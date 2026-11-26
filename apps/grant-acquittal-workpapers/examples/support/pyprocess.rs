// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Python's `shutil.which` lookup and `subprocess.run(timeout=...)` process
//! handling for the utility drivers: `PATH` and `PATHEXT` resolution, and a
//! process tree per command (a POSIX session, a Windows job object) that the
//! deadline bounds, pipe collection included, and that is killed before the
//! run returns.
//!
//! The setup and utility workflow examples each keep an identical copy of
//! this file, as the standalone crates do for their other support modules.

#[cfg(windows)]
use std::ffi::OsString;
use std::{
    env,
    ffi::OsStr,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    sync::{Arc, Mutex, PoisonError},
    thread,
    time::{Duration, Instant},
};

/// How long the readers get once the tree is killed. A descendant that left
/// the tree (a new POSIX session, say) can hold a pipe open indefinitely; its
/// output is abandoned rather than waited for.
const READ_GRACE: Duration = Duration::from_millis(200);

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
    let meta = std::fs::metadata(&candidate).ok()?;
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

/// One finished run.
pub struct Finished {
    /// The deadline passed (or waiting failed) and the tree was killed.
    pub timed_out: bool,
    /// The child's exit code; a POSIX signal death reads as the negative
    /// signal number.
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Runs `cmd` as the root of its own process tree. Waits until the child has
/// exited and every piped stream is closed, or until the timeout passes, then
/// kills whatever remains of the tree, so no descendant outlives the call and
/// no read waits past the deadline (plus [`READ_GRACE`] after a kill). As with
/// Python's `communicate`, a descendant that keeps a pipe open holds the run
/// until the deadline.
pub fn run_tree(cmd: &mut Command, timeout_secs: f64) -> std::io::Result<Finished> {
    let deadline = Instant::now() + Duration::from_secs_f64(timeout_secs);
    let mut tree = Tree::spawn(cmd)?;
    let stdout = Reader::start(tree.child.stdout.take());
    let stderr = Reader::start(tree.child.stderr.take());
    let mut exited = None;
    let timed_out = loop {
        if exited.is_none() {
            match tree.child.try_wait() {
                Ok(Some(status)) => exited = Some(status_code(status)),
                Ok(None) => {}
                Err(_) => break true,
            }
        }
        if exited.is_some() && stdout.done() && stderr.done() {
            break false;
        }
        if Instant::now() >= deadline {
            break true;
        }
        thread::sleep(Duration::from_millis(10));
    };
    tree.kill();
    if exited.is_none() {
        let _ = tree.child.wait();
    }
    let grace = Instant::now() + READ_GRACE;
    Ok(Finished {
        timed_out,
        code: exited.flatten(),
        stdout: stdout.finish(grace),
        stderr: stderr.finish(grace),
    })
}

/// A pipe drained on its own thread into a buffer the caller can take at any
/// time, so an abandoned reader keeps what it had read.
struct Reader {
    buffer: Arc<Mutex<Vec<u8>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Reader {
    fn start(stream: Option<impl Read + Send + 'static>) -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let thread = stream.map(|mut stream| {
            let sink = Arc::clone(&buffer);
            thread::spawn(move || {
                let mut chunk = [0u8; 8192];
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => sink
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .extend_from_slice(&chunk[..n]),
                        Err(err) if err.kind() == ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
            })
        });
        Self { buffer, thread }
    }

    fn done(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    fn finish(self, until: Instant) -> Vec<u8> {
        while !self.done() && Instant::now() < until {
            thread::sleep(Duration::from_millis(10));
        }
        std::mem::take(&mut *self.buffer.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// The child and, on Windows, the job object holding it and its descendants.
struct Tree {
    child: Child,
    #[cfg(windows)]
    job: Option<job::Job>,
}

#[cfg(unix)]
impl Tree {
    fn spawn(cmd: &mut Command) -> std::io::Result<Self> {
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
        let child = spawn_retrying_busy(cmd)?;
        Ok(Self { child })
    }

    /// Kills every process left in the child's group. The group outlives an
    /// exited leader whilst any member remains, and POSIX keeps its ID from
    /// being reused until then.
    fn kill(&self) {
        if let Ok(pgid) = libc::pid_t::try_from(self.child.id()) {
            // SAFETY: pgid equals the session leader PID created via setsid.
            // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
            unsafe {
                libc::killpg(pgid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(windows)]
impl Tree {
    /// Starts the child suspended, so nothing it starts can escape the job
    /// before the job holds it, then lets it run.
    fn spawn(cmd: &mut Command) -> std::io::Result<Self> {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_SUSPENDED};
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_SUSPENDED);
        let mut child = spawn_retrying_busy(cmd)?;
        let job = job::Job::holding(&child);
        if let Err(err) = job::resume(child.id()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(err);
        }
        Ok(Self { child, job })
    }

    /// Terminates the job; without one (assignment refused), falls back to
    /// `taskkill /T`, which reaches only descendants whose parent still runs.
    fn kill(&self) {
        match &self.job {
            Some(job) => job.terminate(),
            None => {
                let _ = Command::new("taskkill")
                    .args(["/PID", &self.child.id().to_string(), "/T", "/F"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        }
    }
}

#[cfg(windows)]
mod job {
    use std::{os::windows::io::AsRawHandle, process::Child, ptr};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp as toolhelp;
    use windows_sys::Win32::System::JobObjects as jobs;
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};

    /// An owned job object handle. Processes started inside a job join it, and
    /// the job refuses breakaway as its limits allow none.
    pub struct Job(HANDLE);

    impl Job {
        /// A new unnamed job holding `child`, or `None` if Windows refuses.
        pub fn holding(child: &Child) -> Option<Self> {
            // SAFETY: null attributes and name create an unnamed job; the
            // handle is owned by the returned value and closed on drop.
            // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
            unsafe {
                let handle = jobs::CreateJobObjectW(ptr::null(), ptr::null());
                if handle.is_null() {
                    return None;
                }
                let job = Self(handle);
                (jobs::AssignProcessToJobObject(job.0, child.as_raw_handle()) != 0).then_some(job)
            }
        }

        pub fn terminate(&self) {
            // SAFETY: the handle is a live job handle owned by self.
            // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
            unsafe {
                jobs::TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: closes the handle this value owns, once.
            // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// Resumes the suspended threads of process `pid` (its one primary thread).
    pub fn resume(pid: u32) -> std::io::Result<()> {
        // SAFETY: the snapshot and thread handles are checked before use and
        // closed after it; the entry's size field is set as Thread32First requires.
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
        unsafe {
            let snapshot = toolhelp::CreateToolhelp32Snapshot(toolhelp::TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error());
            }
            let mut entry: toolhelp::THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = u32::try_from(size_of::<toolhelp::THREADENTRY32>()).unwrap_or(u32::MAX);
            let mut resumed = false;
            let mut more = toolhelp::Thread32First(snapshot, &mut entry) != 0;
            while more {
                if entry.th32OwnerProcessID == pid {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if !thread.is_null() {
                        resumed |= ResumeThread(thread) != u32::MAX;
                        CloseHandle(thread);
                    }
                }
                more = toolhelp::Thread32Next(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
            if resumed {
                Ok(())
            } else {
                Err(std::io::Error::other("could not resume the new process"))
            }
        }
    }
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
fn spawn_retrying_busy(cmd: &mut Command) -> std::io::Result<Child> {
    for _ in 0..10 {
        match cmd.spawn() {
            Ok(child) => return Ok(child),
            Err(err) if is_busy_exec(&err) => thread::sleep(Duration::from_millis(5)),
            Err(err) => return Err(err),
        }
    }
    cmd.spawn()
}

/// A process tree for the timeout tests: the test binary runs itself as a
/// parent that starts a long-lived grandchild sharing its stdout and stderr.
#[cfg(test)]
pub mod tests {
    use std::{
        env, fs,
        path::Path,
        process::Command,
        thread,
        time::{Duration, Instant},
    };

    const ROLE: &str = "PYPROCESS_TREE_ROLE";
    const PIDFILE: &str = "PYPROCESS_TREE_PIDFILE";
    const HELPER: &str = "pyprocess::tests::tree_helper";

    /// Plays one role when the variables are set; otherwise does nothing.
    /// `parent-exits` exits straight after starting the grandchild,
    /// `parent-waits` outlives any test timeout, and the grandchild sleeps.
    #[test]
    #[expect(clippy::zombie_processes, reason = "the grandchild outlives it")]
    fn tree_helper() {
        let Ok(role) = env::var(ROLE) else { return };
        if role != "grandchild" {
            let exe = env::current_exe().unwrap();
            let child = Command::new(exe)
                .args(["--exact", HELPER, "--nocapture", "--quiet"])
                .env(ROLE, "grandchild")
                .spawn()
                .unwrap();
            let pidfile = env::var(PIDFILE).unwrap();
            let partial = format!("{pidfile}.part");
            fs::write(&partial, child.id().to_string()).unwrap();
            fs::rename(partial, pidfile).unwrap();
            if role == "parent-exits" {
                std::process::exit(0);
            }
        }
        thread::sleep(Duration::from_secs(60));
    }

    /// The command and environment that start the tree in `role`.
    pub fn tree_command(role: &str, pidfile: &Path) -> (Vec<String>, Vec<(String, String)>) {
        let exe = env::current_exe().unwrap().to_string_lossy().into_owned();
        let command = [exe.as_str(), "--exact", HELPER, "--nocapture", "--quiet"];
        let env_vars = vec![
            (ROLE.to_owned(), role.to_owned()),
            (PIDFILE.to_owned(), pidfile.to_string_lossy().into_owned()),
        ];
        (command.map(str::to_owned).to_vec(), env_vars)
    }

    /// Panics unless the grandchild recorded in `pidfile` is gone within 5 s
    /// (kills are asynchronous), stopping it first so a failure leaves nothing.
    pub fn assert_tree_gone(pidfile: &Path) {
        let pid: u32 = fs::read_to_string(pidfile)
            .expect("the tree's parent never recorded its grandchild")
            .parse()
            .unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while alive(pid) && Instant::now() < until {
            thread::sleep(Duration::from_millis(20));
        }
        if alive(pid) {
            stop(pid);
            panic!("descendant {pid} outlived the call");
        }
    }

    #[cfg(target_os = "linux")]
    fn alive(pid: u32) -> bool {
        // A killed orphan stays a zombie until init reaps it.
        let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        let state = stat.rsplit(") ").next().unwrap_or("Z");
        !state.starts_with('Z')
    }

    #[cfg(all(unix, not(target_os = "linux")))]
    fn alive(pid: u32) -> bool {
        // SAFETY: signal 0 only checks that the process exists.
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
        libc::pid_t::try_from(pid).is_ok_and(|pid| unsafe { libc::kill(pid, 0) } == 0)
    }

    #[cfg(unix)]
    fn stop(pid: u32) {
        if let Ok(pid) = libc::pid_t::try_from(pid) {
            // SAFETY: kills only the test's own grandchild.
            // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }

    #[cfg(windows)]
    fn alive(pid: u32) -> bool {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, WAIT_TIMEOUT},
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };
        // SAFETY: the handle is checked before use and closed after it.
        // nosemgrep: rust.lang.security.unsafe-usage.unsafe-usage
        unsafe {
            let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if handle.is_null() {
                return false;
            }
            let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
            CloseHandle(handle);
            running
        }
    }

    #[cfg(windows)]
    fn stop(pid: u32) {
        let pid = pid.to_string();
        let _ = Command::new("taskkill").args(["/PID", &pid, "/F"]).output();
    }
}
