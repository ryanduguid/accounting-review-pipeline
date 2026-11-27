// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Set up and run the fabricated accounting examples in a new workspace.

use grant_acquittal_workpapers::{
    oserror,
    pyjson::{self, Json},
    pytext,
};
#[cfg(not(windows))]
use std::collections::HashMap;
#[cfg(windows)]
use std::collections::HashSet;
#[cfg(any(windows, test))]
use std::ffi::OsStr;
#[cfg(windows)]
use std::ffi::OsString;
use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
#[path = "support/pyprocess.rs"]
mod pyprocess;
#[path = "support/pyusage.rs"]
mod pyusage;
#[cfg(test)]
use pyprocess::which_on_path_in;
use pyprocess::{path_override, resolve_program, run_tree, which_on_path};

const PIPELINE: &str = "accounting-review-pipeline";
const GRANT_PROJECT: &str = "apps/grant-acquittal-workpapers";
const DRIVER_PY: &str = "packages/monthly-close-control-plane/examples/utility_workflows.py";
const DRIVER_RS: &str = "packages/monthly-close-control-plane/examples/utility_workflows.rs";
const CLOSE_CRATE: &str = "packages/monthly-close-control-plane";
const COMPANIONS: [(&str, &str); 2] = [
    ("au-fpa-pack", "OzFPA"),
    ("australian-accounting", "australian-accounting"),
];

const HELP_BODY: &str = "Set up and run the fabricated accounting examples in a new workspace.\n\noptions:\n  -h, --help            show this help message and exit\n  --workspace WORKSPACE\n                        New directory outside every Git checkout\n  --replay-manifest REPLAY_MANIFEST\n                        Replay clean source commits from an all-route manifest\n                        or replay.json\n  --summary SUMMARY     Append a result summary outside source and workspace\n                        directories";

/// argparse's usage line for this parser.
fn usage_line(prog: &str) -> String {
    const OPTIONS: [&str; 4] = [
        "[-h]",
        "--workspace WORKSPACE",
        "[--replay-manifest REPLAY_MANIFEST]",
        "[--summary SUMMARY]",
    ];
    pyusage::usage_line(prog, &OPTIONS)
}

#[derive(Clone, Debug)]
struct Captured {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

trait Ops {
    fn which(&self, tool: &str) -> bool;
    fn run_status(
        &self,
        command: &[String],
        cwd: Option<&Path>,
        env_vars: &[(&str, &str)],
        timeout_secs: f64,
    ) -> Result<(), String>;
    fn run_capture(&self, command: &[String], env_vars: &[(&str, &str)], timeout_secs: f64)
    -> Result<Captured, String>;
}

struct SystemOps;

impl Ops for SystemOps {
    fn which(&self, tool: &str) -> bool {
        which_on_path(tool).is_some()
    }

    fn run_status(
        &self,
        command: &[String],
        cwd: Option<&Path>,
        env_vars: &[(&str, &str)],
        timeout_secs: f64,
    ) -> Result<(), String> {
        let mut cmd = system_command(command, env_vars)?;
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let done = finish(command, &mut cmd, timeout_secs)?;
        match done.code {
            Some(0) => Ok(()),
            code => Err(called_process_error(command, code)),
        }
    }

    fn run_capture(
        &self,
        command: &[String],
        env_vars: &[(&str, &str)],
        timeout_secs: f64,
    ) -> Result<Captured, String> {
        let mut cmd = system_command(command, env_vars)?;
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let done = finish(command, &mut cmd, timeout_secs)?;
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).replace("\r\n", "\n");
        Ok(Captured {
            exit_code: done.code,
            stdout: text(&done.stdout),
            stderr: text(&done.stderr),
        })
    }
}

/// `command` resolved on the child's `PATH`, with its environment and no stdin.
fn system_command(command: &[String], env_vars: &[(&str, &str)]) -> Result<Command, String> {
    let Some((program, args)) = command.split_first() else {
        return Err("empty command".into());
    };
    let mut cmd = Command::new(resolve_program(program, path_override(env_vars)));
    cmd.args(args).stdin(Stdio::null());
    for &(k, v) in env_vars {
        cmd.env(k, v);
    }
    Ok(cmd)
}

/// Runs the tree; a timeout reads as `subprocess.TimeoutExpired` does.
fn finish(command: &[String], cmd: &mut Command, timeout_secs: f64) -> Result<pyprocess::Finished, String> {
    let done = run_tree(cmd, timeout_secs).map_err(|e| format!("Failed to launch {}: {e}", command[0]))?;
    if done.timed_out {
        return Err(format!(
            "Command '{}' timed out after {timeout_secs} seconds",
            command_repr(command)
        ));
    }
    Ok(done)
}

/// Python's `repr` of the argument list, as the subprocess errors print it.
fn command_repr(command: &[String]) -> String {
    let items: Vec<String> = command.iter().map(String::as_str).map(pytext::repr_os).collect();
    format!("[{}]", items.join(", "))
}

/// Signal names as Python's `signal.Signals` gives them; real-time signals
/// read as unknown.
#[cfg(unix)]
const SIGNALS: &[(i32, &str)] = &[
    (libc::SIGHUP, "SIGHUP"),
    (libc::SIGINT, "SIGINT"),
    (libc::SIGQUIT, "SIGQUIT"),
    (libc::SIGILL, "SIGILL"),
    (libc::SIGTRAP, "SIGTRAP"),
    (libc::SIGABRT, "SIGABRT"),
    (libc::SIGBUS, "SIGBUS"),
    (libc::SIGFPE, "SIGFPE"),
    (libc::SIGKILL, "SIGKILL"),
    (libc::SIGUSR1, "SIGUSR1"),
    (libc::SIGSEGV, "SIGSEGV"),
    (libc::SIGUSR2, "SIGUSR2"),
    (libc::SIGPIPE, "SIGPIPE"),
    (libc::SIGALRM, "SIGALRM"),
    (libc::SIGTERM, "SIGTERM"),
    #[cfg(target_os = "linux")]
    (libc::SIGSTKFLT, "SIGSTKFLT"),
    (libc::SIGCHLD, "SIGCHLD"),
    (libc::SIGCONT, "SIGCONT"),
    (libc::SIGSTOP, "SIGSTOP"),
    (libc::SIGTSTP, "SIGTSTP"),
    (libc::SIGTTIN, "SIGTTIN"),
    (libc::SIGTTOU, "SIGTTOU"),
    (libc::SIGURG, "SIGURG"),
    (libc::SIGXCPU, "SIGXCPU"),
    (libc::SIGXFSZ, "SIGXFSZ"),
    (libc::SIGVTALRM, "SIGVTALRM"),
    (libc::SIGPROF, "SIGPROF"),
    (libc::SIGWINCH, "SIGWINCH"),
    (libc::SIGIO, "SIGIO"),
    #[cfg(target_os = "linux")]
    (libc::SIGPWR, "SIGPWR"),
    (libc::SIGSYS, "SIGSYS"),
];

#[cfg(not(unix))]
const SIGNALS: &[(i32, &str)] = &[];

/// `str(subprocess.CalledProcessError(code, command))`: a POSIX signal death
/// arrives as the negative signal number, and Windows exit codes are unsigned.
fn called_process_error(command: &[String], code: Option<i32>) -> String {
    let shown = command_repr(command);
    // A finished child always has an exit code or, on POSIX, a signal.
    let code = code.unwrap_or(-1);
    if code < 0 && cfg!(unix) {
        let signal = -code;
        return match SIGNALS.iter().find(|(number, _)| *number == signal) {
            Some((_, name)) => format!("Command '{shown}' died with <Signals.{name}: {signal}>."),
            None => format!("Command '{shown}' died with unknown signal {signal}."),
        };
    }
    let status = if cfg!(windows) {
        u32::from_ne_bytes(code.to_ne_bytes()).to_string()
    } else {
        code.to_string()
    };
    format!("Command '{shown}' returned non-zero exit status {status}.")
}

#[cfg(windows)]
fn strip_verbatim(path: PathBuf) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let units = |text: &str| text.encode_utf16().collect::<Vec<u16>>();
    if let Some(unc) = wide.strip_prefix(units(r"\\?\UNC\").as_slice()) {
        let mut out = units(r"\\");
        out.extend_from_slice(unc);
        PathBuf::from(OsString::from_wide(&out))
    } else if let Some(local) = wide.strip_prefix(units(r"\\?\").as_slice()) {
        PathBuf::from(OsString::from_wide(local))
    } else {
        path
    }
}

#[cfg(windows)]
fn read_link_deep(path: &Path) -> Option<PathBuf> {
    let mut current = path.to_path_buf();
    let mut seen = HashSet::new();
    while seen.insert(oserror::decode_os(current.as_os_str()).to_lowercase()) {
        let Ok(target) = fs::read_link(&current) else {
            break;
        };
        let joined = if target.is_absolute() {
            target
        } else {
            current.parent()?.join(target)
        };
        current = std::path::absolute(joined).ok()?;
    }
    Some(current)
}

#[cfg(windows)]
fn resolve_absolute(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    let mut tail = Vec::new();
    loop {
        let base = match fs::canonicalize(&current) {
            Ok(found) => Some(strip_verbatim(found)),
            Err(_) => read_link_deep(&current).filter(|followed| *followed != current),
        };
        if let Some(mut result) = base {
            for name in tail.iter().rev() {
                result.push(name);
            }
            return result;
        }
        let Some(name) = current.file_name().map(OsStr::to_os_string) else {
            return path.to_path_buf();
        };
        tail.push(name);
        current.pop();
    }
}

#[cfg(not(windows))]
struct RealPath {
    rest: Vec<Option<Vec<u8>>>,
    part_count: usize,
    current: Vec<u8>,
    seen: HashMap<Vec<u8>, Option<Vec<u8>>>,
}

#[cfg(not(windows))]
fn os_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(windows))]
impl RealPath {
    fn push_parts(&mut self, text: &[u8]) {
        let parts: Vec<&[u8]> = text.split(|&b| b == b'/').collect();
        self.part_count += parts.len();
        self.rest.extend(parts.into_iter().rev().map(|p| Some(p.to_vec())));
    }

    fn step(&mut self, name: &[u8]) {
        if name.is_empty() || name == b"." {
            return;
        }
        if name == b".." {
            let cut = self.current.iter().rposition(|&b| b == b'/').unwrap_or(0);
            self.current.truncate(cut.max(1));
            return;
        }
        let mut newpath = self.current.clone();
        if newpath != b"/" {
            newpath.push(b'/');
        }
        newpath.extend_from_slice(name);
        match fs::symlink_metadata(os_path(&newpath)) {
            Ok(meta) if meta.file_type().is_symlink() => self.step_symlink(newpath),
            _ => self.current = newpath,
        }
    }

    fn step_symlink(&mut self, newpath: Vec<u8>) {
        match self.seen.get(&newpath) {
            Some(Some(resolved)) => self.current = resolved.clone(),
            Some(None) => self.current = newpath,
            None => match fs::read_link(os_path(&newpath)) {
                Ok(target) => self.follow(newpath, &target),
                Err(_) => self.current = newpath,
            },
        }
    }

    fn follow(&mut self, link: Vec<u8>, target: &Path) {
        use std::os::unix::ffi::OsStrExt;
        let bytes = target.as_os_str().as_bytes();
        if bytes.starts_with(b"/") {
            self.current = b"/".to_vec();
        }
        self.seen.insert(link.clone(), None);
        self.rest.push(Some(link));
        self.rest.push(None);
        self.push_parts(bytes);
    }
}

#[cfg(not(windows))]
fn resolve_absolute(path: &Path) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    let filename = path.as_os_str().as_bytes();
    let trimmed = &filename[..filename.len() - filename.iter().rev().take_while(|&&b| b == b'/').count()];
    let mut walk = RealPath {
        rest: Vec::new(),
        part_count: 0,
        current: b"/".to_vec(),
        seen: HashMap::new(),
    };
    walk.push_parts(trimmed);
    while walk.part_count > 0 {
        let Some(name) = walk.rest.pop().expect("part_count tracks entries") else {
            let link = walk.rest.pop().flatten().expect("link follows marker");
            walk.seen.insert(link, Some(walk.current.clone()));
            continue;
        };
        walk.part_count -= 1;
        walk.step(&name);
    }
    os_path(&walk.current)
}

fn resolve_path(path: &Path) -> Result<PathBuf, String> {
    if oserror::decode_os(path.as_os_str()).contains('\0') {
        return Err("embedded null byte".into());
    }
    let abs = std::path::absolute(path).map_err(|e| e.to_string())?;
    Ok(resolve_absolute(&abs))
}

fn is_within(resolved: &Path, root: &Path) -> bool {
    let norm = |p: &Path| -> Vec<String> {
        p.components()
            .map(|c| {
                let text = oserror::decode_os(c.as_os_str());
                if cfg!(windows) { text.to_lowercase() } else { text }
            })
            .collect()
    };
    let (inner, outer) = (norm(resolved), norm(root));
    inner.len() >= outer.len() && inner[..outer.len()] == outer[..]
}

fn git_command(arguments: &[&str]) -> Vec<String> {
    let mut cmd = Vec::with_capacity(arguments.len() + 1);
    cmd.push("git".to_owned());
    cmd.extend(arguments.iter().map(|s| (*s).to_owned()));
    cmd
}

fn read_strict_json(path: &Path) -> Result<Json, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    if text.starts_with('\u{FEFF}') {
        return Err("Unexpected UTF-8 BOM".into());
    }
    pyjson::loads_bytes(text.as_bytes()).map_err(|e| match e {
        pyjson::LoadError::Value(msg) => msg,
        pyjson::LoadError::Recursion => "JSON recursion limit exceeded".into(),
    })
}

fn is_full_sha(value: &Json) -> Option<String> {
    let Json::Str(s) = value else {
        return None;
    };
    let text = s.to_utf8()?;
    (text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).then_some(text)
}

/// Read only commit identities from an all-workflow provenance manifest.
fn replay_revisions(manifest: &Path) -> Result<BTreeMap<String, String>, String> {
    let data = read_strict_json(manifest)?;
    let schema_ok = data
        .get("schema_version")
        .is_some_and(|v| v.is_str("utility-workflows.v2"));
    let workflow_ok = data.get("workflow").is_some_and(|v| v.is_str("all"));
    if !schema_ok || !workflow_ok {
        return Err("Replay needs an all-workflow v2 manifest".into());
    }
    let projects = data.get("projects").ok_or("Invalid replay provenance manifest")?;
    if !matches!(projects, Json::Object(_)) {
        return Err("Invalid replay provenance manifest".into());
    }
    if !projects.has_exact_keys(&["close", "fpa", "wip", "grants"]) {
        return Err("Replay needs all four source revisions".into());
    }
    let grants_proj = projects
        .get("grants")
        .and_then(|g| g.get("project"))
        .ok_or("Invalid replay provenance manifest")?;
    if !grants_proj.is_str(GRANT_PROJECT) {
        return Err("Replay needs a manifest from a run inside the pipeline checkout; manifests from the separate grant repository cannot be replayed".into());
    }
    collect_replay_map(projects)
}

fn collect_replay_map(projects: &Json) -> Result<BTreeMap<String, String>, String> {
    let mut result: BTreeMap<String, String> = BTreeMap::new();
    for (owner, repository, expected_project) in [
        ("close", PIPELINE, "packages/monthly-close-control-plane"),
        ("fpa", "au-fpa-pack", "."),
        ("wip", "australian-accounting", "packages/the-wip-tally"),
        ("grants", PIPELINE, GRANT_PROJECT),
    ] {
        let evidence = projects.get(owner).ok_or("Invalid replay provenance manifest")?;
        if !matches!(evidence, Json::Object(_)) {
            return Err("Invalid replay provenance manifest".into());
        }
        let rev = evidence.get("revision").ok_or("Invalid replay provenance manifest")?;
        let proj = evidence.get("project").ok_or("Invalid replay provenance manifest")?;
        let status = evidence
            .get("working_tree_status")
            .ok_or("Invalid replay provenance manifest")?;
        let clean = matches!(status, Json::Array(items) if items.is_empty());
        let Some(revision) = is_full_sha(rev) else {
            return Err("Replay requires clean source checkouts and full commit IDs".into());
        };
        if !proj.is_str(expected_project) || !clean {
            return Err("Replay requires clean source checkouts and full commit IDs".into());
        }
        if let Some(prev) = result.insert(repository.to_owned(), revision.clone())
            && prev != revision
        {
            return Err("Replay needs the close controls and grant workpapers from one pipeline commit".into());
        }
    }
    Ok(result)
}

/// Fetch only the requested commit from a caller-selected fixed source.
fn checkout_revision(
    ops: &dyn Ops,
    destination: &Path,
    source: &str,
    revision: &str,
    environment: &[(&str, &str)],
) -> Result<(), String> {
    let dest = oserror::path_text(destination);
    ops.run_status(&git_command(&["init", "--", &dest]), None, environment, 30.0)?;
    ops.run_status(
        &git_command(&["-C", &dest, "fetch", "--depth", "1", "--", source, revision]),
        None,
        environment,
        180.0,
    )?;
    ops.run_status(
        &git_command(&["-C", &dest, "checkout", "--detach", "FETCH_HEAD"]),
        None,
        environment,
        30.0,
    )?;
    let rev_parse = git_command(&["-C", &dest, "rev-parse", "HEAD"]);
    let actual = ops.run_capture(&rev_parse, environment, 30.0)?;
    if actual.exit_code != Some(0) {
        return Err(called_process_error(&rev_parse, actual.exit_code));
    }
    if actual.stdout.trim() != revision {
        return Err("Fetched commit does not match the recorded revision".into());
    }
    Ok(())
}

fn pipeline_root(source: &Path) -> Result<PathBuf, String> {
    let resolved = resolve_path(source)?;
    resolved
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| {
            "Run setup from apps/grant-acquittal-workpapers in an accounting-review-pipeline checkout".to_owned()
        })
}

fn has_driver(pipeline: &Path) -> bool {
    (pipeline.join(DRIVER_RS).is_file() && pipeline.join(CLOSE_CRATE).join("Cargo.toml").is_file())
        || pipeline.join(DRIVER_PY).is_file()
}

fn verify_workspace_isolation(
    ops: &dyn Ops,
    workspace: &Path,
    pipeline: &Path,
    git_env: &[(&str, &str)],
) -> Result<(), String> {
    if is_within(workspace, pipeline) || is_within(pipeline, workspace) {
        return Err("The workspace must be separate from the pipeline checkout".into());
    }
    let mut ancestor = workspace.parent().unwrap_or(workspace);
    while !ancestor.exists() {
        ancestor = match ancestor.parent() {
            Some(parent) => parent,
            None => break,
        };
    }
    let anc_text = oserror::path_text(ancestor);
    let probe = ops.run_capture(
        &git_command(&["-C", &anc_text, "rev-parse", "--show-toplevel"]),
        git_env,
        30.0,
    )?;
    if probe.exit_code == Some(0) {
        return Err("The workspace must be outside existing Git checkouts".into());
    }
    if probe.exit_code != Some(128) || !probe.stderr.starts_with("fatal: not a git repository") {
        return Err("Git could not verify workspace isolation; fix its error before setup".into());
    }
    Ok(())
}

fn python_command(ops: &dyn Ops) -> Option<&'static str> {
    let order = if cfg!(windows) {
        ["python", "python3"]
    } else {
        ["python3", "python"]
    };
    order.into_iter().find(|&candidate| ops.which(candidate))
}

fn run_driver(ops: &dyn Ops, pipeline: &Path, sources: &Path, output: &Path, envs: &Path) -> Result<(), String> {
    let fpa = oserror::path_text(&sources.join("au-fpa-pack"));
    let acc = oserror::path_text(&sources.join("australian-accounting"));
    let grants = oserror::path_text(&pipeline.join(GRANT_PROJECT));
    let out = oserror::path_text(output);
    let env_root = oserror::path_text(envs);
    let route_args = [
        "--fpa",
        &fpa,
        "--accounting",
        &acc,
        "--grants",
        &grants,
        "--output",
        &out,
        "--environment-root",
        &env_root,
    ];
    if pipeline.join(DRIVER_RS).is_file() && pipeline.join(CLOSE_CRATE).join("Cargo.toml").is_file() {
        let mut cmd: Vec<String> = [
            "cargo",
            "run",
            "--locked",
            "--quiet",
            "--example",
            "utility_workflows",
            "--",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        cmd.extend(route_args.iter().map(|s| (*s).to_owned()));
        return ops.run_status(&cmd, Some(&pipeline.join(CLOSE_CRATE)), &[], 3600.0);
    }
    let python = python_command(ops).ok_or("Install python and put it on PATH before running setup")?;
    let driver = oserror::path_text(&pipeline.join(DRIVER_PY));
    let mut cmd = vec![python.to_owned(), driver];
    cmd.extend(route_args.iter().map(|s| (*s).to_owned()));
    ops.run_status(&cmd, None, &[], 3600.0)
}

fn setup(ops: &dyn Ops, source: &Path, workspace: &Path, replay_manifest: Option<&Path>) -> Result<PathBuf, String> {
    if fs::symlink_metadata(workspace).is_ok() {
        return Err("Choose a new workspace; existing paths are never reused".into());
    }
    for tool in ["git", "uv", "cargo"] {
        if !ops.which(tool) {
            return Err(format!("Install {tool} and put it on PATH before running setup"));
        }
    }
    let revisions = match replay_manifest {
        Some(path) => Some(replay_revisions(path)?),
        None => None,
    };
    let workspace = resolve_path(workspace)?;
    let mut pipeline = pipeline_root(source)?;
    if revisions.is_none() && !has_driver(&pipeline) {
        return Err("Run setup from apps/grant-acquittal-workpapers in an accounting-review-pipeline checkout".into());
    }
    let git_env = [("GIT_TERMINAL_PROMPT", "0"), ("LC_ALL", "C")];
    verify_workspace_isolation(ops, &workspace, &pipeline, &git_env)?;
    fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;
    let sources = workspace.join("sources");
    fs::create_dir(&sources).map_err(|e| e.to_string())?;
    fetch_sources(ops, &sources, &mut pipeline, revisions.as_ref(), &git_env)?;
    let output = workspace.join("results");
    run_driver(ops, &pipeline, &sources, &output, &workspace.join("environments"))?;
    verify_output(&output, revisions.as_ref())?;
    Ok(output)
}

fn fetch_sources(
    ops: &dyn Ops,
    sources: &Path,
    pipeline: &mut PathBuf,
    revisions: Option<&BTreeMap<String, String>>,
    git_env: &[(&str, &str)],
) -> Result<(), String> {
    for (name, repo) in COMPANIONS {
        let url = format!("https://github.com/ryanduguid/{repo}.git");
        let dest = sources.join(name);
        match revisions {
            None => {
                let dest_str = oserror::path_text(&dest);
                ops.run_status(
                    &git_command(&[
                        "clone",
                        "--depth",
                        "1",
                        "--branch",
                        "main",
                        "--single-branch",
                        "--",
                        &url,
                        &dest_str,
                    ]),
                    None,
                    git_env,
                    180.0,
                )?;
            }
            Some(revs) => checkout_revision(ops, &dest, &url, &revs[name], git_env)?,
        }
    }
    if let Some(revs) = revisions {
        let snapshot = sources.join(PIPELINE);
        let pipe_str = oserror::path_text(pipeline);
        checkout_revision(ops, &snapshot, &pipe_str, &revs[PIPELINE], git_env)?;
        *pipeline = snapshot;
        if !has_driver(pipeline) || !pipeline.join(GRANT_PROJECT).is_dir() {
            return Err("The recorded pipeline commit lacks the joined runner or the grant workpapers".into());
        }
    }
    Ok(())
}

fn verify_output(output: &Path, revisions: Option<&BTreeMap<String, String>>) -> Result<(), String> {
    let manifest_path = output.join("manifest.json");
    if !manifest_path.is_file() {
        return Err("The workflow returned without its success manifest".into());
    }
    let manifest = read_strict_json(&manifest_path)?;
    if !matches!(manifest, Json::Object(_)) || !manifest.get("fixture_validation").is_some_and(|v| v.is_str("passed")) {
        return Err("The pipeline checkout lacks the required fixture result checks".into());
    }
    if let Some(revs) = revisions
        && replay_revisions(&manifest_path)? != *revs
    {
        return Err("Run provenance does not match the replay revisions".into());
    }
    Ok(())
}

struct CliArgs {
    workspace: PathBuf,
    replay_manifest: Option<PathBuf>,
    summary: Option<PathBuf>,
}

fn parse_cli(args: &[String]) -> Result<Option<CliArgs>, String> {
    let (mut workspace, mut replay, mut summary, mut extras) = (None, None, None, Vec::new());
    let mut i = 0;
    while i < args.len() {
        let token = &args[i];
        i += 1;
        if token == "-h" || token == "--help" {
            return Ok(None);
        }
        let (flag, explicit) = match token.split_once('=') {
            Some((k, v)) if k.starts_with("--") => (k, Some(v)),
            _ if token.starts_with("--") => (token.as_str(), None),
            _ => {
                extras.push(token.as_str());
                continue;
            }
        };
        if !matches!(flag, "--workspace" | "--replay-manifest" | "--summary") {
            extras.push(token.as_str());
            continue;
        }
        let value = match explicit {
            Some(v) => v,
            None => match args.get(i) {
                Some(next) if !next.starts_with('-') || next == "-" => {
                    i += 1;
                    next.as_str()
                }
                _ => return Err(format!("argument {flag}: expected one argument")),
            },
        };
        let path = PathBuf::from(value);
        match flag {
            "--workspace" => workspace = Some(path),
            "--replay-manifest" => replay = Some(path),
            "--summary" => summary = Some(path),
            _ => {}
        }
    }
    if !extras.is_empty() {
        return Err(format!("unrecognized arguments: {}", extras.join(" ")));
    }
    let Some(ws) = workspace else {
        return Err("the following arguments are required: --workspace".into());
    };
    Ok(Some(CliArgs {
        workspace: ws,
        replay_manifest: replay,
        summary,
    }))
}

fn write_stream(stream: &mut dyn Write, text: &str) {
    let payload = if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text.to_owned()
    };
    let _ = stream.write_all(payload.as_bytes());
}

fn append_summary(summary_target: &Path, workspace: &Path, was_present: bool, succeeded: bool) -> Result<(), String> {
    let summary = workspace.join("results/summary.md");
    let fallback = if succeeded {
        "# Joined accounting examples\n\nFixture checks passed. Detailed summary unavailable; see results/manifest.json.\n"
    } else {
        "# Joined accounting examples\n\nSetup failed before verified results were available. See the failed setup step and retained diagnostics.\n"
    };
    let mut text = if !was_present && summary.is_file() {
        fs::read_to_string(&summary)
            .map_err(|e| e.to_string())?
            .replace("\r\n", "\n")
    } else {
        fallback.to_owned()
    };
    if !succeeded && text.contains("Fixture checks passed.") {
        text = fallback.to_owned();
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(summary_target)
        .map_err(|e| e.to_string())?;
    let payload = format!("{text}\n");
    let encoded = if cfg!(windows) {
        payload.replace('\n', "\r\n")
    } else {
        payload
    };
    file.write_all(encoded.as_bytes()).map_err(|e| e.to_string())
}

fn check_summary_target(source: &Path, summary: &Path, workspace: &Path) -> Result<(), String> {
    let target = resolve_path(summary)?;
    let pipe = pipeline_root(source)?;
    let ws = resolve_path(workspace)?;
    if is_within(&target, &pipe) || is_within(&target, &ws) {
        return Err("Summary must be outside the pipeline checkout and workspace".into());
    }
    Ok(())
}

fn run_main(
    ops: &dyn Ops,
    source: &Path,
    prog: &str,
    args: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let usage = usage_line(prog);
    let parsed = match parse_cli(args) {
        Ok(Some(cli)) => cli,
        Ok(None) => {
            write_stream(out, &format!("{usage}\n\n{HELP_BODY}\n"));
            return 0;
        }
        Err(msg) => {
            write_stream(err, &format!("{usage}\n{prog}: error: {msg}\n"));
            return 2;
        }
    };
    if let Some(summary_path) = &parsed.summary
        && let Err(error) = check_summary_target(source, summary_path, &parsed.workspace)
    {
        write_stream(err, &format!("{usage}\n{prog}: error: {error}\n"));
        return 2;
    }
    run_checked(ops, source, &parsed, out, err)
}

fn run_checked(ops: &dyn Ops, source: &Path, parsed: &CliArgs, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let was_present = fs::symlink_metadata(&parsed.workspace).is_ok();
    let (mut result, succeeded, output) = match setup(ops, source, &parsed.workspace, parsed.replay_manifest.as_deref())
    {
        Ok(output) => (0, true, Some(output)),
        Err(error) => {
            write_stream(
                err,
                &format!(
                    "Setup failed: {error}. Existing checkouts were not updated. Keep any partial workspace for diagnosis and retry with a new path.\n"
                ),
            );
            (1, false, None)
        }
    };
    if let Some(summary_target) = &parsed.summary
        && let Err(error) = append_summary(summary_target, &parsed.workspace, was_present, succeeded)
    {
        write_stream(
            err,
            &format!("Could not append the requested result summary: {error}\n"),
        );
        result = 1;
    }
    if let Some(out_dir) = output {
        write_stream(
            out,
            &format!(
                "Accounting examples verified. Results: {}\n",
                oserror::path_text(&out_dir)
            ),
        );
    }
    result
}

/// This crate's source directory, as setup_utility.py's `Path(__file__).resolve().parent`:
/// fixed at build time, so a shared or relocated target directory cannot move it.
fn default_source() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn main() -> std::process::ExitCode {
    // argv is the user's own command line, decoded as Python decodes sys.argv; it grants nothing.
    // nosemgrep: rust.lang.security.args-os.args-os
    let mut argv = env::args_os();
    let prog = argv
        .next()
        .and_then(|first| Path::new(&first).file_name().map(oserror::decode_os))
        .unwrap_or_else(|| "setup_utility".to_owned());
    let args: Vec<String> = argv.map(|a| oserror::decode_os(&a)).collect();
    let mut out = std::io::stdout().lock();
    let mut err = std::io::stderr().lock();
    let code = run_main(&SystemOps, &default_source(), &prog, &args, &mut out, &mut err);
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(prefix: &str) -> Self {
            static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let id = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            // a test scratch directory with a unique name; it holds only fabricated files.
            // nosemgrep: rust.lang.security.temp-dir.temp-dir
            let path = env::temp_dir().join(format!("{prefix}-{}-{id}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn join(&self, p: impl AsRef<Path>) -> PathBuf {
            self.0.join(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    type StatusHook = dyn Fn(&[String], Option<&Path>) -> Result<(), String>;
    type CaptureHook = dyn Fn(&[String]) -> Result<Captured, String>;

    /// Runs real commands but reports every tool as installed, so tests that
    /// are not about tool lookup pass on hosts without uv.
    struct HostOps;

    impl Ops for HostOps {
        fn which(&self, _tool: &str) -> bool {
            true
        }
        fn run_status(
            &self,
            command: &[String],
            cwd: Option<&Path>,
            env_vars: &[(&str, &str)],
            timeout_secs: f64,
        ) -> Result<(), String> {
            SystemOps.run_status(command, cwd, env_vars, timeout_secs)
        }
        fn run_capture(
            &self,
            command: &[String],
            env_vars: &[(&str, &str)],
            timeout_secs: f64,
        ) -> Result<Captured, String> {
            SystemOps.run_capture(command, env_vars, timeout_secs)
        }
    }

    struct StubOps {
        missing: Option<&'static str>,
        status_hook: Box<StatusHook>,
        capture_hook: Box<CaptureHook>,
        seen: RefCell<Vec<Vec<String>>>,
    }

    impl Ops for StubOps {
        fn which(&self, tool: &str) -> bool {
            self.missing != Some(tool)
        }
        fn run_status(&self, cmd: &[String], cwd: Option<&Path>, _: &[(&str, &str)], _: f64) -> Result<(), String> {
            self.seen.borrow_mut().push(cmd.to_vec());
            (self.status_hook)(cmd, cwd)
        }
        fn run_capture(&self, cmd: &[String], _: &[(&str, &str)], _: f64) -> Result<Captured, String> {
            self.seen.borrow_mut().push(cmd.to_vec());
            (self.capture_hook)(cmd)
        }
    }

    fn manifest_json() -> Json {
        let project = |path: &str, rev: &str| {
            Json::object([
                ("revision", Json::str(rev)),
                ("working_tree_status", Json::Array(Vec::new())),
                ("project", Json::str(path)),
            ])
        };
        Json::object([
            ("schema_version", Json::str("utility-workflows.v2")),
            ("workflow", Json::str("all")),
            (
                "projects",
                Json::object([
                    (
                        "close",
                        project("packages/monthly-close-control-plane", &"1".repeat(40)),
                    ),
                    ("fpa", project(".", &"2".repeat(40))),
                    ("wip", project("packages/the-wip-tally", &"3".repeat(40))),
                    ("grants", project(GRANT_PROJECT, &"1".repeat(40))),
                ]),
            ),
        ])
    }

    fn fake_pipeline(root: &Path, runner: bool) {
        fs::create_dir_all(root.join(GRANT_PROJECT)).unwrap();
        if runner {
            let driver = root.join(DRIVER_PY);
            fs::create_dir_all(driver.parent().unwrap()).unwrap();
            fs::write(driver, "").unwrap();
        }
    }

    fn default_stub(expected_manifest: Option<Json>) -> StubOps {
        StubOps {
            missing: None,
            seen: RefCell::new(Vec::new()),
            capture_hook: Box::new(|cmd| {
                if cmd.iter().any(|s| s == "rev-parse") {
                    return Ok(Captured {
                        exit_code: Some(128),
                        stdout: String::new(),
                        stderr: "fatal: not a git repository (or any of the parent directories): .git\n".into(),
                    });
                }
                Ok(Captured {
                    exit_code: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                })
            }),
            status_hook: Box::new(move |cmd, _| {
                if cmd.iter().any(|s| s == "clone") {
                    fs::create_dir(Path::new(cmd.last().unwrap())).unwrap();
                } else if let Some(idx) = cmd.iter().position(|s| s == "--output") {
                    let out = Path::new(&cmd[idx + 1]);
                    fs::create_dir_all(out).unwrap();
                    let body = expected_manifest
                        .clone()
                        .unwrap_or_else(|| Json::object([("fixture_validation", Json::str("passed"))]));
                    fs::write(out.join("manifest.json"), pyjson::dumps_indent2(&body)).unwrap();
                }
                Ok(())
            }),
        }
    }

    #[test]
    fn manifest_uses_fixed_repository_mapping_and_refuses_invalid() {
        let temp = TempDir::new("setup-replay");
        let path = temp.join("manifest.json");
        fs::write(&path, pyjson::dumps_indent2(&manifest_json())).unwrap();
        let revs = replay_revisions(&path).unwrap();
        assert_eq!(
            revs.keys().cloned().collect::<Vec<_>>(),
            vec![PIPELINE, "au-fpa-pack", "australian-accounting"]
        );
        assert_eq!(revs[PIPELINE], "1".repeat(40));
        assert_eq!(revs["au-fpa-pack"], "2".repeat(40));
        for bad in [
            r#"{"schema_version":"utility-workflows.v2","workflow":"quarter","projects":{}}"#,
            r#"{"schema_version":"future","workflow":"all","projects":{}}"#,
            r#"{"schema_version":"utility-workflows.v2","workflow":"all","projects":["close","fpa","wip","grants"]}"#,
        ] {
            fs::write(&path, bad).unwrap();
            assert!(replay_revisions(&path).is_err(), "{bad}");
        }
        let mut base = manifest_json();
        if let Json::Object(ref mut pairs) = base
            && let Json::Object(ref mut projs) = pairs[2].1
        {
            projs.pop();
        }
        fs::write(&path, pyjson::dumps_indent2(&base)).unwrap();
        assert!(replay_revisions(&path).is_err());
    }

    #[test]
    fn invalid_manifest_fields_and_mismatched_pipeline_revisions_are_refused() {
        let temp = TempDir::new("setup-bad-fields");
        let path = temp.join("manifest.json");
        let cases = [
            ("close", "revision", Json::str("--upload-pack=untrusted")),
            ("fpa", "revision", Json::str("main")),
            ("fpa", "revision", Json::str(&"A".repeat(40))),
            (
                "wip",
                "working_tree_status",
                Json::Array(vec![Json::str(" M source.py")]),
            ),
            ("grants", "project", Json::str("../../private")),
            ("grants", "project", Json::str(".")),
            ("grants", "revision", Json::str(&"4".repeat(40))),
        ];
        for (owner, key, val) in cases {
            let mut m = manifest_json();
            if let Json::Object(ref mut pairs) = m
                && let Json::Object(ref mut projs) = pairs[2].1
            {
                let entry = &mut projs.iter_mut().find(|(k, _)| k.is(owner)).unwrap().1;
                if let Json::Object(epairs) = entry {
                    epairs.iter_mut().find(|(k, _)| k.is(key)).unwrap().1 = val;
                }
            }
            fs::write(&path, pyjson::dumps_indent2(&m)).unwrap();
            assert!(replay_revisions(&path).is_err(), "{owner}.{key}");
        }
    }

    #[test]
    fn invalid_replay_and_existing_workspace_create_or_modify_nothing() {
        let temp = TempDir::new("setup-guard");
        let pipe = temp.join("pipeline");
        fake_pipeline(&pipe, true);
        let manifest = temp.join("manifest.json");
        fs::write(&manifest, "{}").unwrap();
        let target = temp.join("new");
        let stub = default_stub(None);
        assert!(setup(&stub, &pipe.join(GRANT_PROJECT), &target, Some(&manifest)).is_err());
        assert!(!target.exists());
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep.txt"), "original").unwrap();
        let no_tools = StubOps {
            missing: Some("git"),
            ..default_stub(None)
        };
        let err = setup(&no_tools, &pipe.join(GRANT_PROJECT), &target, None).unwrap_err();
        assert!(err.contains("existing paths are never reused"), "{err}");
        assert_eq!(fs::read_to_string(target.join("keep.txt")).unwrap(), "original");
    }

    #[test]
    fn real_git_fetches_recorded_commit_without_changing_source() {
        let temp = TempDir::new("recorded revision");
        let source = temp.join("original repo");
        fs::create_dir(&source).unwrap();
        let (rev1, head) = seed_two_commit_repo(&source);
        fs::write(source.join("fixture.txt"), "uncommitted work").unwrap();
        let ops = SystemOps;
        let src_text = oserror::path_text(&source);
        checkout_revision(&ops, &temp.join("snapshot"), &src_text, &rev1, &[]).unwrap();
        assert_eq!(fs::read_to_string(temp.join("snapshot/fixture.txt")).unwrap(), "old");
        let cur = ops
            .run_capture(&git_command(&["-C", &src_text, "rev-parse", "HEAD"]), &[], 30.0)
            .unwrap();
        assert_eq!(cur.stdout.trim(), head);
        assert_eq!(
            fs::read_to_string(source.join("fixture.txt")).unwrap(),
            "uncommitted work"
        );
        assert!(checkout_revision(&ops, &temp.join("missing"), &src_text, &"f".repeat(40), &[]).is_err());
    }

    fn seed_two_commit_repo(source: &Path) -> (String, String) {
        let ops = SystemOps;
        let src_s = oserror::path_text(source);
        let git = |args: &[&str]| -> String {
            let mut full = vec!["git".into(), "-C".into(), src_s.clone()];
            full.extend(args.iter().map(|s| (*s).to_owned()));
            let out = ops.run_capture(&full, &[], 30.0).unwrap();
            assert_eq!(out.exit_code, Some(0), "{}", out.stderr);
            out.stdout.trim().to_owned()
        };
        git(&["init"]);
        fs::write(source.join("fixture.txt"), "old").unwrap();
        git(&["add", "fixture.txt"]);
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "first",
        ]);
        let rev1 = git(&["rev-parse", "HEAD"]);
        fs::write(source.join("fixture.txt"), "new").unwrap();
        git(&["add", "fixture.txt"]);
        git(&[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "second",
        ]);
        (rev1, git(&["rev-parse", "HEAD"]))
    }

    #[test]
    fn checkout_rejects_unexpected_commit() {
        let stub = StubOps {
            missing: None,
            seen: RefCell::new(Vec::new()),
            status_hook: Box::new(|_, _| Ok(())),
            capture_hook: Box::new(|_| {
                Ok(Captured {
                    exit_code: Some(0),
                    stdout: format!("{}\n", "b".repeat(40)),
                    stderr: String::new(),
                })
            }),
        };
        let temp = TempDir::new("setup-checkout");
        let err = checkout_revision(&stub, &temp.join("new"), "fixed", &"a".repeat(40), &[]).unwrap_err();
        assert!(err.contains("does not match"), "{err}");
    }

    fn replay_stub() -> StubOps {
        StubOps {
            missing: None,
            seen: RefCell::new(Vec::new()),
            capture_hook: Box::new(|cmd| {
                if cmd.iter().any(|s| s == "HEAD") {
                    let rev = if cmd.iter().any(|s| s.contains("au-fpa-pack")) {
                        "2".repeat(40)
                    } else if cmd.iter().any(|s| s.contains("australian-accounting")) {
                        "3".repeat(40)
                    } else {
                        "1".repeat(40)
                    };
                    return Ok(Captured {
                        exit_code: Some(0),
                        stdout: format!("{rev}\n"),
                        stderr: String::new(),
                    });
                }
                Ok(Captured {
                    exit_code: Some(128),
                    stdout: String::new(),
                    stderr: "fatal: not a git repository\n".into(),
                })
            }),
            status_hook: Box::new(|cmd, _| {
                if cmd.get(1).is_some_and(|s| s == "init") {
                    let dest = Path::new(cmd.last().unwrap());
                    fs::create_dir_all(dest).unwrap();
                    if dest.file_name().is_some_and(|n| n == PIPELINE) {
                        fake_pipeline(dest, true);
                    }
                } else if let Some(idx) = cmd.iter().position(|s| s == "--output") {
                    let out = Path::new(&cmd[idx + 1]);
                    fs::create_dir_all(out).unwrap();
                    let mut m = manifest_json();
                    if let Json::Object(ref mut p) = m {
                        p.push(("fixture_validation".into(), Json::str("passed")));
                    }
                    fs::write(out.join("manifest.json"), pyjson::dumps_indent2(&m)).unwrap();
                }
                Ok(())
            }),
        }
    }

    #[test]
    fn replay_uses_recorded_runner() {
        let temp = TempDir::new("setup-replay-runner");
        let current = temp.join("current");
        fake_pipeline(&current, false);
        let manifest = temp.join("manifest.json");
        fs::write(&manifest, pyjson::dumps_indent2(&manifest_json())).unwrap();
        let ws = temp.join("ws");
        assert!(setup(&replay_stub(), &current.join(GRANT_PROJECT), &ws, Some(&manifest)).is_ok());
    }

    #[test]
    fn isolation_checks_and_missing_tools_stop_before_creating_workspace() {
        let temp = TempDir::new("setup-isolation");
        let pipe = temp.join("pipeline");
        fake_pipeline(&pipe, true);
        let source = pipe.join(GRANT_PROJECT);
        for nested in [source.join("outputs"), pipe.join("outputs")] {
            assert!(
                setup(&default_stub(None), &source, &nested, None)
                    .unwrap_err()
                    .contains("separate")
            );
        }
        for tool in ["git", "uv", "cargo"] {
            let stub = StubOps {
                missing: Some(tool),
                ..default_stub(None)
            };
            let ws = temp.join("ws");
            assert!(setup(&stub, &source, &ws, None).unwrap_err().contains(tool));
            assert!(!ws.exists());
        }
    }

    #[test]
    fn git_probe_errors_and_existing_checkouts_do_not_bypass_isolation() {
        let temp = TempDir::new("setup-git-probe");
        let pipe = temp.join("pipeline");
        fake_pipeline(&pipe, true);
        let source = pipe.join(GRANT_PROJECT);
        for stderr in [
            "fatal: detected dubious ownership in repository",
            "fatal: Permission denied",
            "",
        ] {
            let msg = stderr.to_owned();
            let stub = StubOps {
                missing: None,
                seen: RefCell::new(Vec::new()),
                status_hook: Box::new(|_, _| Ok(())),
                capture_hook: Box::new(move |_| {
                    Ok(Captured {
                        exit_code: Some(128),
                        stdout: String::new(),
                        stderr: msg.clone(),
                    })
                }),
            };
            let ws = temp.join("bad-probe");
            assert!(
                setup(&stub, &source, &ws, None)
                    .unwrap_err()
                    .contains("verify workspace isolation")
            );
            assert!(!ws.exists());
        }
        let in_git = StubOps {
            missing: None,
            seen: RefCell::new(Vec::new()),
            status_hook: Box::new(|_, _| Ok(())),
            capture_hook: Box::new(|_| {
                Ok(Captured {
                    exit_code: Some(0),
                    stdout: "/repo\n".into(),
                    stderr: String::new(),
                })
            }),
        };
        assert!(
            setup(&in_git, &source, &temp.join("in-git"), None)
                .unwrap_err()
                .contains("Git checkouts")
        );
    }

    #[test]
    fn summary_appends_and_failed_setup_never_reads_old_results() {
        let temp = TempDir::new("setup-summary");
        let pipe = temp.join("pipeline");
        fake_pipeline(&pipe, true);
        let existing_ws = temp.join("old ws");
        fs::create_dir_all(existing_ws.join("results")).unwrap();
        fs::write(existing_ws.join("results/summary.md"), "Fixture checks passed.").unwrap();
        let target = temp.join("summary.md");
        fs::write(&target, "Earlier step\n").unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let args = [
            "--workspace".into(),
            oserror::path_text(&existing_ws),
            "--summary".into(),
            oserror::path_text(&target),
        ];
        let code = run_main(
            &default_stub(None),
            &pipe.join(GRANT_PROJECT),
            "setup_utility",
            &args,
            &mut out,
            &mut err,
        );
        assert_eq!(code, 1);
        let text = fs::read_to_string(&target).unwrap();
        assert!(
            text.starts_with("Earlier step")
                && text.contains("Setup failed")
                && !text.contains("Fixture checks passed")
        );
    }

    #[test]
    fn summary_cannot_write_into_sources_or_workspace() {
        let temp = TempDir::new("setup-summary-guard");
        let pipe = temp.join("pipeline");
        fake_pipeline(&pipe, true);
        let ws = temp.join("new ws");
        for bad_target in [pipe.join("summary.md"), ws.join("summary.md")] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let args = [
                "--workspace".into(),
                oserror::path_text(&ws),
                "--summary".into(),
                oserror::path_text(&bad_target),
            ];
            let code = run_main(
                &default_stub(None),
                &pipe.join(GRANT_PROJECT),
                "setup_utility",
                &args,
                &mut out,
                &mut err,
            );
            assert_eq!(code, 2);
            assert!(
                String::from_utf8(err)
                    .unwrap()
                    .contains("outside the pipeline checkout and workspace")
            );
        }
    }

    #[test]
    fn summary_fallback_covers_missing_detail() {
        let temp = TempDir::new("setup-sum-fallback");
        let pipe = temp.join("pipeline");
        fake_pipeline(&pipe, true);
        let target = temp.join("summary.md");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let args = [
            "--workspace".into(),
            oserror::path_text(&temp.join("ws-ok")),
            "--summary".into(),
            oserror::path_text(&target),
        ];
        let code = run_main(
            &default_stub(None),
            &pipe.join(GRANT_PROJECT),
            "setup_utility",
            &args,
            &mut out,
            &mut err,
        );
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
        let text = fs::read_to_string(&target).unwrap();
        assert!(text.contains("Fixture checks passed") && text.contains("Detailed summary unavailable"));
    }

    #[test]
    fn unreadable_generated_summary_reports_controlled_error() {
        let temp = TempDir::new("setup-sum-utf8");
        let pipe = temp.join("pipeline");
        fake_pipeline(&pipe, true);
        let bad_stub = StubOps {
            missing: None,
            seen: RefCell::new(Vec::new()),
            capture_hook: default_stub(None).capture_hook,
            status_hook: Box::new(|cmd, _| {
                if cmd.iter().any(|s| s == "clone") {
                    fs::create_dir(Path::new(cmd.last().unwrap())).unwrap();
                } else if let Some(idx) = cmd.iter().position(|s| s == "--output") {
                    let out = Path::new(&cmd[idx + 1]);
                    fs::create_dir_all(out).unwrap();
                    fs::write(out.join("manifest.json"), r#"{"fixture_validation":"passed"}"#).unwrap();
                    fs::write(out.join("summary.md"), [0xFFu8]).unwrap();
                }
                Ok(())
            }),
        };
        let bad_target = temp.join("bad-sum.md");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let args = [
            "--workspace".into(),
            oserror::path_text(&temp.join("ws-bad")),
            "--summary".into(),
            oserror::path_text(&bad_target),
        ];
        let code = run_main(
            &bad_stub,
            &pipe.join(GRANT_PROJECT),
            "setup_utility",
            &args,
            &mut out,
            &mut err,
        );
        assert_eq!(code, 1);
        assert!(String::from_utf8(out).unwrap().contains("Accounting examples verified"));
        assert!(
            String::from_utf8(err)
                .unwrap()
                .contains("Could not append the requested result summary")
        );
        assert!(!bad_target.exists());
    }

    #[test]
    fn failed_setup_cannot_append_success_claim_from_partial_results() {
        let temp = TempDir::new("setup-partial-claim");
        let pipe = temp.join("pipeline");
        fake_pipeline(&pipe, true);
        let stub = StubOps {
            missing: None,
            seen: RefCell::new(Vec::new()),
            capture_hook: default_stub(None).capture_hook,
            status_hook: Box::new(|cmd, _| {
                if cmd.iter().any(|s| s == "clone") {
                    fs::create_dir(Path::new(cmd.last().unwrap())).unwrap();
                } else if let Some(idx) = cmd.iter().position(|s| s == "--output") {
                    let out = Path::new(&cmd[idx + 1]);
                    fs::create_dir_all(out).unwrap();
                    fs::write(out.join("summary.md"), "Fixture checks passed.").unwrap();
                    return Err("failed after summary".into());
                }
                Ok(())
            }),
        };
        let target = temp.join("summary.md");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let args = [
            "--workspace".into(),
            oserror::path_text(&temp.join("partial")),
            "--summary".into(),
            oserror::path_text(&target),
        ];
        assert_eq!(
            run_main(
                &stub,
                &pipe.join(GRANT_PROJECT),
                "setup_utility",
                &args,
                &mut out,
                &mut err
            ),
            1
        );
        let text = fs::read_to_string(&target).unwrap();
        assert!(text.contains("Setup failed") && !text.contains("Fixture checks passed"));
    }

    #[test]
    fn manifest_must_confirm_fixture_validation() {
        let temp = TempDir::new("setup-bad-manifest");
        let pipe = temp.join("pipeline");
        fake_pipeline(&pipe, true);
        let source = pipe.join(GRANT_PROJECT);
        for (label, invalid) in [
            ("arr", Json::Array(Vec::new())),
            ("fail", Json::object([("fixture_validation", Json::str("failed"))])),
        ] {
            let ws_bad = temp.join(format!("ws-{label}"));
            assert!(
                setup(&default_stub(Some(invalid)), &source, &ws_bad, None)
                    .unwrap_err()
                    .contains("fixture result checks")
            );
        }
    }

    #[test]
    fn rust_driver_branch_invokes_cargo_when_example_source_exists() {
        let temp = TempDir::new("setup-rust-driver");
        let pipe = temp.join("pipeline");
        fs::create_dir_all(pipe.join(GRANT_PROJECT)).unwrap();
        let rs = pipe.join(DRIVER_RS);
        fs::create_dir_all(rs.parent().unwrap()).unwrap();
        fs::write(&rs, "fn main() {}\n").unwrap();
        fs::write(pipe.join(CLOSE_CRATE).join("Cargo.toml"), "[package]\n").unwrap();
        let stub = default_stub(None);
        let ws = temp.join("ws");
        setup(&stub, &pipe.join(GRANT_PROJECT), &ws, None).unwrap();
        let seen = stub.seen.borrow();
        let last = seen.last().unwrap();
        assert!(last.starts_with(&[
            "cargo".into(),
            "run".into(),
            "--locked".into(),
            "--quiet".into(),
            "--example".into(),
            "utility_workflows".into()
        ]));
    }

    #[test]
    fn success_clones_fixed_main_into_a_new_nested_workspace_with_spaces() {
        let temp = TempDir::new("setup-success");
        let pipe = temp.join("pipe line");
        fake_pipeline(&pipe, true);
        let stub = default_stub(None);
        let ws = temp.join("new parent").join("new workspace");
        let output = setup(&stub, &pipe.join(GRANT_PROJECT), &ws, None).unwrap();
        assert!(output.ends_with("results") && ws.join("results").join("manifest.json").is_file());
        let seen = stub.seen.borrow();
        let clones: Vec<_> = seen.iter().filter(|cmd| cmd.iter().any(|s| s == "clone")).collect();
        assert_eq!(clones.len(), COMPANIONS.len());
        for ((name, repo), clone) in COMPANIONS.iter().zip(&clones) {
            assert!(clone.contains(&format!("https://github.com/ryanduguid/{repo}.git")));
            assert_eq!(clone[clone.iter().position(|s| s == "--branch").unwrap() + 1], "main");
            assert!(Path::new(clone.last().unwrap()).ends_with(Path::new("sources").join(name)));
        }
        // The enclosing pipeline checkout supplies the runner; the companions stay separate.
        assert!(Path::new(&seen.last().unwrap()[1]).ends_with(DRIVER_PY));
    }

    #[test]
    fn failed_clone_workflow_or_missing_manifest_is_never_success() {
        let temp = TempDir::new("setup-failures");
        let pipe = temp.join("pipeline");
        fake_pipeline(&pipe, true);
        let source = pipe.join(GRANT_PROJECT);
        let with_status = |status_hook: Box<StatusHook>| StubOps {
            status_hook,
            ..default_stub(None)
        };
        let failed_clone = with_status(Box::new(|cmd, _| {
            assert!(cmd.iter().any(|s| s == "clone"), "a workflow ran after a failed clone");
            Err("Command returned non-zero exit status 1.".into())
        }));
        let ws = temp.join("clone");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let args = ["--workspace".into(), oserror::path_text(&ws)];
        assert_eq!(
            run_main(&failed_clone, &source, "setup_utility", &args, &mut out, &mut err),
            1
        );
        assert!(!ws.join("results").exists());
        assert!(fs::read_dir(ws.join("sources")).unwrap().next().is_none());

        let clone_then = |workflow: Result<(), String>| {
            with_status(Box::new(move |cmd, _| {
                if cmd.iter().any(|s| s == "clone") {
                    fs::create_dir(Path::new(cmd.last().unwrap())).unwrap();
                    return Ok(());
                }
                workflow.clone()
            }))
        };
        let ws = temp.join("workflow");
        let args = ["--workspace".into(), oserror::path_text(&ws)];
        let failed_workflow = clone_then(Err("Command returned non-zero exit status 2.".into()));
        assert_eq!(
            run_main(&failed_workflow, &source, "setup_utility", &args, &mut out, &mut err),
            1
        );
        assert!(!ws.join("results").join("manifest.json").exists());
        let no_manifest = clone_then(Ok(()));
        assert!(
            setup(&no_manifest, &source, &temp.join("silent"), None)
                .unwrap_err()
                .contains("success manifest")
        );
    }

    #[test]
    fn nonexistent_parent_traversal_into_pipeline_and_git_repo_fails() {
        let temp = TempDir::new("setup-dotdot");
        let pipe = temp.join("pipeline");
        fake_pipeline(&pipe, true);
        let git_repo = temp.join("repo");
        fs::create_dir_all(&git_repo).unwrap();
        let ops = HostOps;
        ops.run_status(
            &git_command(&["init", "--", &oserror::path_text(&git_repo)]),
            None,
            &[],
            30.0,
        )
        .unwrap();
        let pipe_ws = temp.join("missing").join("..").join("pipeline").join("ws");
        assert!(
            setup(&ops, &pipe.join(GRANT_PROJECT), &pipe_ws, None)
                .unwrap_err()
                .contains("separate")
        );
        let repo_ws = temp.join("missing").join("..").join("repo").join("ws");
        assert!(
            setup(&ops, &pipe.join(GRANT_PROJECT), &repo_ws, None)
                .unwrap_err()
                .contains("outside existing Git")
        );
        assert!(!pipe.join("ws").exists() && !git_repo.join("ws").exists());
        let sum_dotdot = temp.join("missing").join("..").join("pipeline").join("sum.md");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let args = [
            "--workspace".into(),
            oserror::path_text(&temp.join("ok-ws")),
            "--summary".into(),
            oserror::path_text(&sum_dotdot),
        ];
        assert_eq!(
            run_main(
                &default_stub(None),
                &pipe.join(GRANT_PROJECT),
                "setup_utility",
                &args,
                &mut out,
                &mut err
            ),
            2
        );
        assert!(!pipe.join("sum.md").exists());
    }

    #[test]
    fn invalid_summary_path_fails_before_setup() {
        let temp = TempDir::new("setup-bad-summary");
        let pipe = temp.join("pipeline");
        fake_pipeline(&pipe, true);
        let ws = temp.join("ws");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let args = [
            "--workspace".into(),
            oserror::path_text(&ws),
            "--summary".into(),
            "bad\0path".into(),
        ];
        let code = run_main(
            &default_stub(None),
            &pipe.join(GRANT_PROJECT),
            "setup_utility",
            &args,
            &mut out,
            &mut err,
        );
        assert_eq!(code, 2);
        assert!(!ws.exists());
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlink_summary_inside_guarded_dirs_is_refused() {
        let temp = TempDir::new("setup-dangling-sum");
        let pipe = temp.join("pipeline");
        fake_pipeline(&pipe, true);
        let ws = temp.join("ws");
        let link_pipe = temp.join("link-pipe.md");
        std::os::unix::fs::symlink(pipe.join("missing/summary.md"), &link_pipe).unwrap();
        let link_ws = temp.join("link-ws.md");
        std::os::unix::fs::symlink(ws.join("missing/summary.md"), &link_ws).unwrap();
        for bad in [&link_pipe, &link_ws] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let args = [
                "--workspace".into(),
                oserror::path_text(&ws),
                "--summary".into(),
                oserror::path_text(bad),
            ];
            assert_eq!(
                run_main(
                    &default_stub(None),
                    &pipe.join(GRANT_PROJECT),
                    "setup_utility",
                    &args,
                    &mut out,
                    &mut err
                ),
                2
            );
            assert!(String::from_utf8(err).unwrap().contains("Summary must be outside"));
        }
        assert!(!ws.exists());
    }

    /// The cases recorded from setup_utility.py before the Rust port replaced it.
    #[test]
    fn recorded_python_cases_match() {
        let fixture = pyjson::loads_bytes(include_bytes!("../tests/fixtures/setup_utility_cli.json")).unwrap();
        let text = |value: &Json| match value {
            Json::Str(s) => s.lossy(),
            other => panic!("expected a string, got {}", pyjson::repr(other)),
        };
        let Json::Array(cases) = fixture.at("cases") else {
            panic!("cases must be an array");
        };
        assert!(cases.len() >= 8);
        let prog = text(fixture.at("prog"));
        for case in cases {
            let name = text(case.at("name"));
            let temp = TempDir::new("setup-parity");
            let root = oserror::path_text(&temp.0).replace('\\', "/");
            let pipe = temp.join("pipeline");
            fake_pipeline(&pipe, true);
            fs::create_dir(temp.join("existing")).unwrap();
            let Json::Array(raw) = case.at("args") else {
                panic!("{name}: args must be an array");
            };
            let args: Vec<String> = raw.iter().map(|a| text(a).replace("{root}", &root)).collect();
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let code = run_main(
                &default_stub(None),
                &pipe.join(GRANT_PROJECT),
                &prog,
                &args,
                &mut out,
                &mut err,
            );
            let Json::Int(exit) = case.at("exit") else {
                panic!("{name}: exit must be an integer");
            };
            assert_eq!(code.to_string(), *exit, "{name}: exit code");
            // The streams translate newlines as Python's text mode does on
            // Windows; the recorded cases hold the untranslated text.
            let lines = |bytes: Vec<u8>| {
                let written = String::from_utf8(bytes).unwrap();
                if cfg!(windows) {
                    written.replace("\r\n", "\n")
                } else {
                    written
                }
            };
            assert_eq!(lines(out), text(case.at("stdout")), "{name}: stdout");
            assert_eq!(lines(err), text(case.at("stderr")), "{name}: stderr");
        }
    }

    #[cfg(unix)]
    fn write_exec_shim(bin: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = bin.join("fake-tool");
        fs::write(&p, "#!/bin/sh\necho \"$1\"\n").unwrap();
        let mut perms = fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&p, perms).unwrap();
        p
    }

    #[cfg(not(unix))]
    fn write_exec_shim(bin: &Path) -> PathBuf {
        let p = bin.join("fake-tool.cmd");
        fs::write(&p, "@echo off\r\necho %1\r\n").unwrap();
        p
    }

    #[test]
    fn system_ops_resolves_and_runs_path_shim() {
        let temp = TempDir::new("setup-shim");
        let bin = temp.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let script_path = write_exec_shim(&bin);
        let path_val = oserror::path_text(&bin);
        let found = which_on_path_in("fake-tool", Some(OsStr::new(&path_val))).unwrap();
        if cfg!(windows) {
            let (got, exp) = (oserror::path_text(&found), oserror::path_text(&script_path));
            assert!(got.eq_ignore_ascii_case(&exp));
        } else {
            assert_eq!(found, script_path);
        }
        let cap = SystemOps
            .run_capture(&["fake-tool".into(), "ok".into()], &[("PATH", &path_val)], 10.0)
            .unwrap();
        assert_eq!(cap.exit_code, Some(0));
        assert_eq!(cap.stdout.trim(), "ok");
    }

    /// Failed commands read as `subprocess.CalledProcessError` prints them.
    #[test]
    fn failed_commands_report_python_called_process_errors() {
        let temp = TempDir::new("setup-called-process");
        let missing = oserror::path_text(&temp.join("missing"));
        let cmd = git_command(&["-C", &missing, "status"]);
        let err = SystemOps.run_status(&cmd, None, &[], 30.0).unwrap_err();
        let shown = format!("['git', '-C', {}, 'status']", pytext::repr_os(&missing));
        assert_eq!(err, format!("Command '{shown}' returned non-zero exit status 128."));
        let shell = |script: &str| {
            let cmd: Vec<String> = if cfg!(windows) {
                vec!["cmd".into(), "/C".into(), script.into()]
            } else {
                vec!["sh".into(), "-c".into(), script.into()]
            };
            SystemOps.run_status(&cmd, None, &[], 30.0).unwrap_err()
        };
        if cfg!(windows) {
            assert_eq!(
                shell("exit -1"),
                "Command '['cmd', '/C', 'exit -1']' returned non-zero exit status 4294967295."
            );
        } else {
            assert_eq!(
                shell("kill -9 $$"),
                "Command '['sh', '-c', 'kill -9 $$']' died with <Signals.SIGKILL: 9>."
            );
        }
        if cfg!(target_os = "linux") {
            assert_eq!(
                shell("kill -35 $$"),
                "Command '['sh', '-c', 'kill -35 $$']' died with unknown signal 35."
            );
        }
    }

    fn tree_env(env_vars: &[(String, String)]) -> Vec<(&str, &str)> {
        env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
    }

    /// A child that exits whilst its grandchild holds the output pipes: the
    /// capture stops at the timeout and the grandchild is killed.
    #[test]
    fn capture_is_bounded_when_a_descendant_holds_the_pipes() {
        let temp = TempDir::new("setup-tree-capture");
        let pidfile = temp.join("pid");
        let (cmd, env_vars) = pyprocess::tests::tree_command("parent-exits", &pidfile);
        let started = std::time::Instant::now();
        let err = SystemOps.run_capture(&cmd, &tree_env(&env_vars), 5.0).unwrap_err();
        assert!(started.elapsed().as_secs_f64() < 15.0, "{:?}", started.elapsed());
        assert!(err.contains("timed out after 5 seconds"), "{err}");
        pyprocess::tests::assert_tree_gone(&pidfile);
    }

    /// A child that exits 0 leaving a grandchild behind: the status call
    /// succeeds and the grandchild does not outlive it.
    #[test]
    fn status_leaves_no_descendant_running() {
        let temp = TempDir::new("setup-tree-status");
        let pidfile = temp.join("pid");
        let (cmd, env_vars) = pyprocess::tests::tree_command("parent-exits", &pidfile);
        SystemOps.run_status(&cmd, None, &tree_env(&env_vars), 30.0).unwrap();
        pyprocess::tests::assert_tree_gone(&pidfile);
    }

    /// A timeout kills the grandchild as well as the child.
    #[test]
    fn status_timeout_kills_descendants() {
        let temp = TempDir::new("setup-tree-timeout");
        let pidfile = temp.join("pid");
        let (cmd, env_vars) = pyprocess::tests::tree_command("parent-waits", &pidfile);
        let err = SystemOps.run_status(&cmd, None, &tree_env(&env_vars), 5.0).unwrap_err();
        assert!(err.contains("timed out after 5 seconds"), "{err}");
        pyprocess::tests::assert_tree_gone(&pidfile);
    }
}
