// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Python's `shutil.which` lookup and `subprocess.run(timeout=...)` process
//! handling for the utility drivers: `PATH` and `PATHEXT` resolution, a
//! process group per child and killing that group on timeout.
//!
//! The setup and utility workflow examples each keep an identical copy of
//! this file, as the standalone crates do for their other support modules.

#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::process::Stdio;
use std::{
    env,
    ffi::OsStr,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    thread,
    time::{Duration, Instant},
};

/// The last `PATH` entry in a child's environment overrides, matched without
/// case on Windows as the OS does.
pub fn path_override<K: AsRef<str>, V: AsRef<str>>(env_vars: &[(K, V)]) -> Option<&OsStr> {
    env_vars
        .iter()
        .rfind(|(k, _)| {
            if cfg!(windows) {
                k.as_ref().eq_ignore_ascii_case("PATH")
            } else {
                k.as_ref() == "PATH"
            }
        })
        .map(|(_, v)| OsStr::new(v.as_ref()))
}

pub fn resolve_program(program: &str, env_path: Option<&OsStr>) -> PathBuf {
    if Path::new(program).components().count() > 1 {
        return PathBuf::from(program);
    }
    which_on_path_in(program, env_path).unwrap_or_else(|| PathBuf::from(program))
}

pub fn which_on_path(tool: &str) -> Option<PathBuf> {
    which_on_path_in(tool, None)
}

pub fn which_on_path_in(tool: &str, path_override: Option<&OsStr>) -> Option<PathBuf> {
    let owned;
    let path_var = match path_override {
        Some(v) => v,
        None => {
            owned = env::var_os("PATH")?;
            owned.as_os_str()
        }
    };
    env::split_paths(path_var).find_map(|dir| executable_in(&dir, tool))
}

#[cfg(unix)]
pub fn executable_in(dir: &Path, tool: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let candidate = dir.join(tool);
    let meta = fs::metadata(&candidate).ok()?;
    (meta.is_file() && meta.permissions().mode() & 0o111 != 0).then_some(candidate)
}

#[cfg(windows)]
pub fn executable_in(dir: &Path, tool: &str) -> Option<PathBuf> {
    let pathext = env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD"));
    let text = pathext.to_string_lossy();
    let exts: Vec<&str> = text.split(';').filter(|e| !e.is_empty()).collect();
    let lower = tool.to_ascii_lowercase();
    let listed = |ext: &&str| lower.ends_with(&ext.to_ascii_lowercase());
    if exts.iter().any(listed) {
        let direct = dir.join(tool);
        if direct.is_file() {
            return Some(direct);
        }
    }
    exts.iter()
        .map(|ext| dir.join(format!("{tool}{ext}")))
        .find(|candidate| candidate.is_file())
}

#[cfg(unix)]
pub fn configure_group(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: setsid is async-signal-safe and isolates the child in its own session/group.
    // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(windows)]
pub fn configure_group(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
}

#[cfg(unix)]
fn kill_tree(pid: u32) {
    if let Ok(pgid) = libc::pid_t::try_from(pid) {
        // SAFETY: pgid equals the session leader PID created via setsid.
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
        unsafe {
            libc::killpg(pgid, libc::SIGKILL);
        }
    }
}

#[cfg(windows)]
fn kill_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn spawn_reader(stream: Option<impl Read + Send + 'static>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut reader) = stream {
            let _ = reader.read_to_end(&mut buf);
        }
        buf
    })
}

#[cfg(unix)]
fn status_code(status: ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.code().or_else(|| status.signal().map(|s| -s))
}

#[cfg(not(unix))]
fn status_code(status: ExitStatus) -> Option<i32> {
    status.code()
}

/// Waits for the child; on timeout or a wait error kills its process group
/// and returns `Err(())`. A signal death reads as the negative signal number.
pub fn wait_child(mut child: Child, timeout_secs: f64) -> Result<Option<i32>, ()> {
    let deadline = Instant::now() + Duration::from_secs_f64(timeout_secs);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status_code(status)),
            Ok(None) if Instant::now() >= deadline => {
                kill_tree(child.id());
                let _ = child.wait();
                return Err(());
            }
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(_) => {
                kill_tree(child.id());
                let _ = child.wait();
                return Err(());
            }
        }
    }
}

#[cfg(unix)]
fn is_busy_exec(err: &std::io::Error) -> bool {
    err.raw_os_error() == Some(libc::ETXTBSY)
}

#[cfg(not(unix))]
fn is_busy_exec(_err: &std::io::Error) -> bool {
    false
}

/// Retries a spawn briefly whilst a freshly written executable is still open
/// for writing elsewhere (Unix `ETXTBSY`).
pub fn spawn_retrying_busy(cmd: &mut Command) -> std::io::Result<Child> {
    for _ in 0..10 {
        match cmd.spawn() {
            Ok(child) => return Ok(child),
            Err(err) if is_busy_exec(&err) => thread::sleep(Duration::from_millis(5)),
            Err(err) => return Err(err),
        }
    }
    cmd.spawn()
}
