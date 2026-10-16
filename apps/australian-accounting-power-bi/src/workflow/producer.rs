use super::{
    PRODUCERS, Result, digest,
    files::{self, Temporary},
    json, schema,
};
use serde_json::Value;
use std::{
    fs::File,
    path::Path,
    process::{Child, Command, ExitStatus, Output, Stdio},
    time::{Duration, Instant},
};

fn executable(bin_dir: &Path, command: &str) -> Result<std::path::PathBuf> {
    if !PRODUCERS.iter().any(|(name, ..)| *name == command) {
        return Err("Unsupported producer command.".into());
    }
    files::ordinary(&bin_dir.join(format!("{command}{}", std::env::consts::EXE_SUFFIX)))
}

fn environment(command: &mut Command, cache: &Path) {
    command.env_clear();
    for name in [
        "PATH",
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .env("PYTHONPYCACHEPREFIX", cache);
}

pub fn manifest(bin_dir: &Path, command: &str) -> Result<Value> {
    inspect(bin_dir, command, |executable| {
        let cache = Temporary::new("review-bytecode-")?;
        let mut process = Command::new(executable);
        process.arg("--version").current_dir(&cache.0);
        environment(&mut process, &cache.0);
        deadline(&mut process, &cache.0, Duration::from_secs(20))
    })
}

fn inspect(bin_dir: &Path, command: &str, run: impl FnOnce(&Path) -> Result<Output>) -> Result<Value> {
    let executable = executable(bin_dir, command)?;
    let launcher = files::read(&executable)?;
    let magics: &[&[u8]] = &[
        b"MZ",
        b"\x7fELF",
        b"\xcf\xfa\xed\xfe",
        b"\xfe\xed\xfa\xcf",
        b"\xca\xfe\xba\xbe",
        b"\xbe\xba\xfe\xca",
    ];
    if !magics.iter().any(|magic| launcher.starts_with(magic)) {
        return Err(format!("{command} requires its native Rust executable."));
    }
    let output = run(&executable)?;
    let text = std::str::from_utf8(&output.stdout).unwrap_or("");
    let version = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .and_then(|text| text.strip_prefix(&format!("{command} ")));
    if !output.status.success() || !output.stderr.is_empty() || !version.is_some_and(schema::native_version) {
        return Err("Native producer version probe refused the installation.".into());
    }
    if files::read(&executable)? != launcher {
        return Err("Producer executable changed during identity inspection.".into());
    }
    let package = PRODUCERS.iter().find(|(name, ..)| *name == command).unwrap().1;
    let value = serde_json::json!({"manifest_version":2,"command":command,"launcher_name":executable.file_name().unwrap().to_string_lossy(),"launcher_sha256":digest(&launcher),"native":{"package":package,"version":version.unwrap()}});
    schema::producer(&value, command)?;
    Ok(value)
}

struct ManagedChild(Child);

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn deadline(command: &mut Command, cache: &Path, limit: Duration) -> Result<Output> {
    let stdout = cache.join("stdout");
    let stderr = cache.join("stderr");
    command.stdout(Stdio::from(File::create(&stdout).map_err(|error| error.to_string())?));
    command.stderr(Stdio::from(File::create(&stderr).map_err(|error| error.to_string())?));
    let mut child = ManagedChild(command.spawn().map_err(|error| error.to_string())?);
    let start = Instant::now();
    loop {
        if let Some(status) = child.0.try_wait().map_err(|error| error.to_string())? {
            return Ok(Output {
                status,
                stdout: files::read(&stdout)?,
                stderr: files::read(&stderr)?,
            });
        }
        if start.elapsed() >= limit {
            child.0.kill().map_err(|error| error.to_string())?;
            child.0.wait().map_err(|error| error.to_string())?;
            return Err("Native producer version probe exceeded its deadline.".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn execute(bin_dir: &Path, command: &str, args: &[String], cwd: Option<&Path>) -> Result<Output> {
    let cache = Temporary::new("review-bytecode-")?;
    let mut process = Command::new(executable(bin_dir, command)?);
    process.args(args);
    if let Some(cwd) = cwd {
        process.current_dir(cwd);
    }
    environment(&mut process, &cache.0);
    process.output().map_err(|error| error.to_string())
}

fn code(status: ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.code().unwrap_or_else(|| -status.signal().unwrap_or(1))
    }
    #[cfg(windows)]
    {
        status.code().unwrap_or(1)
    }
}

pub fn invoke(
    bin_dir: &Path,
    command: &str,
    args: &[String],
    run: &Path,
    label: &str,
    allowed: &[i32],
    expected: &Value,
) -> Result<Value> {
    let installed = manifest(bin_dir, command)?;
    if &installed != expected {
        return Err("Producer installed identity differs from the receipt.".into());
    }
    let started = timestamp()?;
    let output = execute(bin_dir, command, args, Some(run))?;
    if manifest(bin_dir, command)? != installed {
        return Err("Producer installed identity changed during invocation.".into());
    }
    files::write_new(&run.join(format!("validation/{label}.stdout.txt")), &output.stdout)?;
    files::write_new(&run.join(format!("validation/{label}.stderr.txt")), &output.stderr)?;
    let exit = code(output.status);
    if !allowed.contains(&exit) {
        return Err(format!(
            "{command} {label} refused the case (exit {exit}); see validation logs."
        ));
    }
    Ok(
        serde_json::json!({"command":command,"args":args,"executable_sha256":installed["launcher_sha256"],"producer_manifest_sha256":digest(&json::bytes(&installed)),"started":started,"finished":timestamp()?,"exit":exit}),
    )
}

pub fn pinned(bin_dir: &Path, command: &str, args: &[String], expected: &Value) -> Result<Vec<u8>> {
    schema::producer(expected, command)?;
    if digest(&files::read(&executable(bin_dir, command)?)?) != expected["launcher_sha256"].as_str().unwrap() {
        return Err("Producer executable hash differs from the receipt.".into());
    }
    if &manifest(bin_dir, command)? != expected {
        return Err("Producer installed identity differs from the receipt.".into());
    }
    let output = execute(bin_dir, command, args, None)?;
    if &manifest(bin_dir, command)? != expected {
        return Err("Producer installed identity changed during invocation.".into());
    }
    if !output.status.success() {
        return Err("Producer verification refused portable display.".into());
    }
    Ok(output.stdout)
}

fn timestamp() -> Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    let seconds = now.as_secs();
    let date = crate::date::Date::new(1970, 1, 1).add_days((seconds / 86400) as i64);
    let time = format!(
        "{:02}:{:02}:{:02}",
        (seconds / 3600) % 24,
        (seconds / 60) % 60,
        seconds % 60
    );
    let fraction = if now.subsec_micros() == 0 {
        String::new()
    } else {
        format!(".{:06}", now.subsec_micros())
    };
    Ok(format!("{date}T{time}{fraction}+00:00"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_binds_native_version_and_executable_after_inspection() {
        let cache = Temporary::new("pbi-producer-test-").unwrap();
        let executable = executable(&cache.0, "review-ready").unwrap();
        std::fs::write(&executable, b"MZfabricated native image").unwrap();
        let result = inspect(&cache.0, "review-ready", |_| {
            Ok(Output {
                status: success(),
                stdout: b"review-ready 1.0.0\n".to_vec(),
                stderr: vec![],
            })
        })
        .unwrap();
        assert_eq!(result["native"]["package"], "review-ready-gate");
        assert!(
            inspect(&cache.0, "review-ready", |_| {
                std::fs::write(&executable, b"MZchanged").unwrap();
                Ok(Output {
                    status: success(),
                    stdout: b"review-ready 1.0.0\n".to_vec(),
                    stderr: vec![],
                })
            })
            .is_err()
        );
    }

    fn success() -> ExitStatus {
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            ExitStatus::from_raw(0)
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            ExitStatus::from_raw(0)
        }
    }

    #[test]
    fn native_probes_refuse_wrong_commands_versions_stderr_and_exit_status() {
        let cache = Temporary::new("pbi-probe-refusal-").unwrap();
        for command in ["close-control", "review-ready"] {
            std::fs::write(executable(&cache.0, command).unwrap(), b"MZfixture").unwrap();
            for (stdout, stderr, code) in [
                (format!("{command} 0.3.0\n"), vec![], 0),
                ("other-command 1.0.0\n".into(), vec![], 0),
                (format!("{command} 1.0.0\n"), b"warning".to_vec(), 0),
                (format!("{command} 1.0.0\n"), vec![], 1),
                (format!("{command} 1.0.0\nextra\n"), vec![], 0),
            ] {
                #[cfg(windows)]
                let status = {
                    use std::os::windows::process::ExitStatusExt;
                    ExitStatus::from_raw(code)
                };
                #[cfg(unix)]
                let status = {
                    use std::os::unix::process::ExitStatusExt;
                    ExitStatus::from_raw(code << 8)
                };
                assert!(
                    inspect(&cache.0, command, |_| Ok(Output {
                        status,
                        stdout: stdout.into_bytes(),
                        stderr
                    }))
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn children_get_only_allowlisted_environment_and_an_empty_private_cache() {
        let cache = Temporary::new("pbi-environment-").unwrap();
        let mut command = Command::new("unused-fixture");
        command
            .env("PYTHONPATH", "fixture-shadow")
            .env("PYTHONHOME", "fixture-shadow")
            .env("UNRELATED_FIXTURE", "fixture");
        environment(&mut command, &cache.0);
        let entries: std::collections::BTreeMap<_, _> = command.get_envs().collect();
        for name in ["PYTHONPATH", "PYTHONHOME", "UNRELATED_FIXTURE"] {
            assert!(!entries.contains_key(std::ffi::OsStr::new(name)));
        }
        assert_eq!(
            entries[std::ffi::OsStr::new("PYTHONPYCACHEPREFIX")],
            Some(cache.0.as_os_str())
        );
        assert_eq!(std::fs::read_dir(&cache.0).unwrap().count(), 0);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&cache.0).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }

    #[test]
    fn deadline_stops_and_reaps_its_owned_child() {
        let cache = Temporary::new("pbi-producer-test-").unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap());
        child.args(["--ignored", "--exact", "workflow::producer::tests::slow_child"]);
        let start = Instant::now();
        assert!(
            deadline(&mut child, &cache.0, Duration::from_millis(50))
                .unwrap_err()
                .contains("deadline")
        );
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    #[ignore = "child process fixture, invoked only by the deadline test"]
    fn slow_child() {
        std::thread::sleep(Duration::from_secs(60));
    }
}
