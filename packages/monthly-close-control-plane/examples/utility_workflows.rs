// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Run fabricated workflows with explicit lock authority and source evidence.

#[cfg(test)]
#[path = "../tests/support/temp.rs"]
pub(crate) mod temp;
#[path = "utility_results.rs"]
mod utility_results;

use monthly_close_control_plane::{
    pyjson::{self, Json, PyStr},
    pypath, sha256,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    ffi::OsStr,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(windows)]
use std::ffi::OsString;
use utility_results::CallRecord;

const WIP_BIN: &[&str] = &[
    "cargo",
    "run",
    "--locked",
    "--quiet",
    "--bin",
    "wip-tally",
    "--",
];
const WIP_EXAMPLE: &[&str] = &[
    "cargo",
    "run",
    "--locked",
    "--quiet",
    "--example",
    "job_to_cash",
    "--",
];
const CLOSE_BIN: &[&str] = &[
    "cargo",
    "run",
    "--locked",
    "--release",
    "--quiet",
    "--bin",
    "close-control",
    "--",
];
const GRANTS_BIN: &[&str] = &[
    "cargo",
    "run",
    "--locked",
    "--release",
    "--quiet",
    "--bin",
    "grant-workpaper",
    "--",
];
const SOURCE_EXTENSIONS: &[&str] = &[
    ".py", ".rs", ".toml", ".lock", ".json", ".csv", ".yaml", ".yml", ".md", ".in", ".txt",
];
const PYTHON_PROBE: &str = "import sys,json,importlib.metadata as m; print(json.dumps({'python':sys.version,'packages':sorted((d.metadata['Name'],d.version) for d in m.distributions())}))";

#[derive(Clone, Debug)]
struct Captured {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

#[derive(Debug)]
enum CaptureError {
    Timeout { stdout: String, stderr: String },
    Launch(String),
}

trait Ops {
    fn which(&self, tool: &str) -> bool;
    fn run(
        &self,
        command: &[String],
        cwd: &Path,
        env_vars: &[(String, String)],
        timeout_secs: f64,
    ) -> Result<Captured, CaptureError>;
}

struct SystemOps;

impl Ops for SystemOps {
    fn which(&self, tool: &str) -> bool {
        which_on_path(tool).is_some()
    }

    fn run(
        &self,
        command: &[String],
        cwd: &Path,
        env_vars: &[(String, String)],
        timeout_secs: f64,
    ) -> Result<Captured, CaptureError> {
        captured(command, cwd, env_vars, timeout_secs)
    }
}

fn path_override(env_vars: &[(String, String)]) -> Option<&OsStr> {
    env_vars
        .iter()
        .rfind(|(k, _)| {
            if cfg!(windows) {
                k.eq_ignore_ascii_case("PATH")
            } else {
                k == "PATH"
            }
        })
        .map(|(_, v)| OsStr::new(v.as_str()))
}

fn resolve_program(program: &str, env_path: Option<&OsStr>) -> PathBuf {
    if Path::new(program).components().count() > 1 {
        return PathBuf::from(program);
    }
    which_on_path_in(program, env_path).unwrap_or_else(|| PathBuf::from(program))
}

fn which_on_path(tool: &str) -> Option<PathBuf> {
    which_on_path_in(tool, None)
}

fn which_on_path_in(tool: &str, path_override: Option<&OsStr>) -> Option<PathBuf> {
    let owned;
    let path_var = match path_override {
        Some(v) => v,
        None => {
            owned = env::var_os("PATH")?;
            owned.as_os_str()
        }
    };
    for dir in env::split_paths(path_var) {
        if let Some(found) = executable_in(&dir, tool) {
            return Some(found);
        }
    }
    None
}

#[cfg(unix)]
fn executable_in(dir: &Path, tool: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let candidate = dir.join(tool);
    let meta = fs::metadata(&candidate).ok()?;
    (meta.is_file() && meta.permissions().mode() & 0o111 != 0).then_some(candidate)
}

#[cfg(windows)]
fn executable_in(dir: &Path, tool: &str) -> Option<PathBuf> {
    let pathext = env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD"));
    let text = pathext.to_string_lossy();
    let exts: Vec<&str> = text.split(';').filter(|e| !e.is_empty()).collect();
    let lower = tool.to_ascii_lowercase();
    if exts
        .iter()
        .any(|e| lower.ends_with(&e.to_ascii_lowercase()))
    {
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
fn configure_group(cmd: &mut Command) {
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
fn configure_group(cmd: &mut Command) {
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

fn spawn_reader(stream: Option<impl Read + Send + 'static>) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut reader) = stream {
            let _ = reader.read_to_end(&mut buf);
        }
        buf
    })
}

fn decode_stream(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
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

fn wait_child(mut child: Child, timeout_secs: f64) -> Result<Option<i32>, ()> {
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

/// Stop the whole command tree on timeout or interruption.
fn captured(
    command: &[String],
    cwd: &Path,
    env_vars: &[(String, String)],
    timeout_secs: f64,
) -> Result<Captured, CaptureError> {
    let Some((program, args)) = command.split_first() else {
        return Err(CaptureError::Launch("empty command".to_owned()));
    };
    let resolved = resolve_program(program, path_override(env_vars));
    let mut cmd = Command::new(&resolved);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env_vars {
        cmd.env(key, value);
    }
    configure_group(&mut cmd);
    let mut child =
        spawn_retrying_busy(&mut cmd).map_err(|e| CaptureError::Launch(e.to_string()))?;
    let out_handle = spawn_reader(child.stdout.take());
    let err_handle = spawn_reader(child.stderr.take());
    let waited = wait_child(child, timeout_secs);
    let stdout = decode_stream(&out_handle.join().unwrap_or_default());
    let stderr = decode_stream(&err_handle.join().unwrap_or_default());
    match waited {
        Ok(exit_code) => Ok(Captured {
            exit_code,
            stdout,
            stderr,
        }),
        Err(()) => Err(CaptureError::Timeout { stdout, stderr }),
    }
}

fn git_output(ops: &dyn Ops, root: &Path, arguments: &[&str]) -> Result<String, String> {
    let mut command = Vec::with_capacity(arguments.len() + 1);
    command.push("git".to_owned());
    command.extend(arguments.iter().map(|s| (*s).to_owned()));
    let result = ops
        .run(&command, root, &[], 30.0)
        .map_err(|e| format!("Git evidence failed: {e:?}"))?;
    if result.exit_code != Some(0) {
        return Err(format!("Git evidence failed: {}", result.stderr));
    }
    Ok(result.stdout)
}

#[derive(Default)]
struct WorkspaceSpec {
    defined: bool,
    members: Vec<String>,
    exclude: Vec<String>,
}

fn strip_toml_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut quote: Option<(char, bool)> = None;
    while i < chars.len() {
        let ch = chars[i];
        if let Some((q, triple)) = quote {
            out.push(ch);
            if q == '"' && ch == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if ch == q
                && (!triple || (i + 2 < chars.len() && chars[i + 1] == q && chars[i + 2] == q))
            {
                if triple {
                    let mut extra = 3;
                    while extra < 5 && chars.get(i + extra) == Some(&q) {
                        extra += 1;
                    }
                    out.extend(&chars[i + 1..i + extra]);
                    i += extra - 1;
                }
                quote = None;
            }
        } else if ch == '#' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        } else {
            out.push(ch);
            if ch == '"' || ch == '\'' {
                let triple = i + 2 < chars.len() && chars[i + 1] == ch && chars[i + 2] == ch;
                if triple {
                    out.push(chars[i + 1]);
                    out.push(chars[i + 2]);
                    i += 2;
                }
                quote = Some((ch, triple));
            }
        }
        i += 1;
    }
    out
}

fn push_toml_escape(src: &str, i: &mut usize, out: &mut String, multiline: bool) -> Option<()> {
    let esc = src[*i..].chars().next()?;
    if multiline && matches!(esc, ' ' | '\t' | '\r' | '\n') {
        let mut saw_newline = false;
        while let Some(c) = src[*i..].chars().next() {
            if c == '\n' {
                saw_newline = true;
            } else if !matches!(c, ' ' | '\t' | '\r') {
                break;
            }
            *i += c.len_utf8();
        }
        return saw_newline.then_some(());
    }
    *i += esc.len_utf8();
    out.push(match esc {
        'b' => '\u{0008}',
        'f' => '\u{000C}',
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        '\\' => '\\',
        '"' => '"',
        other => other,
    });
    Some(())
}

fn parse_multiline_toml_string(src: &str, start: usize, q: char) -> Option<(String, usize)> {
    let mut i = start + 3;
    if src[i..].starts_with("\r\n") {
        i += 2;
    } else if src[i..].starts_with('\n') {
        i += 1;
    }
    let mut out = String::new();
    while i < src.len() {
        let ch = src[i..].chars().next()?;
        if ch == q {
            let count = src[i..].chars().take(6).take_while(|&c| c == q).count();
            if (3..=5).contains(&count) {
                for _ in 0..(count - 3) {
                    out.push(q);
                }
                return Some((out, i + count));
            }
        }
        if q == '"' && ch == '\\' {
            i += 1;
            push_toml_escape(src, &mut i, &mut out, true)?;
        } else {
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    None
}

fn parse_toml_string(src: &str, start: usize) -> Option<(String, usize)> {
    let rest = src.get(start..)?;
    if rest.starts_with("\"\"\"") {
        return parse_multiline_toml_string(src, start, '"');
    }
    if rest.starts_with("'''") {
        return parse_multiline_toml_string(src, start, '\'');
    }
    let q = rest.bytes().next().filter(|&b| b == b'"' || b == b'\'')?;
    let mut i = start + 1;
    let mut out = String::new();
    while i < src.len() {
        let ch = src[i..].chars().next()?;
        if ch as u32 == u32::from(q) {
            return Some((out, i + 1));
        }
        if q == b'"' && ch == '\\' {
            i += 1;
            push_toml_escape(src, &mut i, &mut out, false)?;
        } else {
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    None
}

fn parse_string_array(src: &str, mut pos: usize) -> Option<(Vec<String>, usize)> {
    let bytes = src.as_bytes();
    while bytes.get(pos).is_some_and(u8::is_ascii_whitespace) {
        pos += 1;
    }
    if bytes.get(pos) != Some(&b'[') {
        return None;
    }
    pos += 1;
    let mut items = Vec::new();
    while pos < src.len() {
        while bytes
            .get(pos)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b',')
        {
            pos += 1;
        }
        if bytes.get(pos) == Some(&b']') {
            return Some((items, pos + 1));
        }
        let (value, next) = parse_toml_string(src, pos)?;
        items.push(value);
        pos = next;
    }
    None
}

fn skip_toml_value(src: &str, mut pos: usize) -> usize {
    let bytes = src.as_bytes();
    let mut depth = 0usize;
    while pos < bytes.len() {
        match bytes[pos] {
            b'"' | b'\'' => {
                if let Some((_, next)) = parse_toml_string(src, pos) {
                    pos = next;
                    continue;
                }
            }
            b'[' | b'{' => depth += 1,
            b']' | b'}' => depth = depth.saturating_sub(1),
            b'\n' if depth == 0 => return pos,
            _ => {}
        }
        pos += 1;
    }
    pos
}

fn parse_uv_workspace(text: &str) -> WorkspaceSpec {
    let clean = strip_toml_comments(text);
    let mut spec = WorkspaceSpec::default();
    let mut section = String::new();
    let mut i = 0;
    let bytes = clean.as_bytes();
    while i < clean.len() {
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        if i >= clean.len() {
            break;
        }
        if bytes[i] == b'[' {
            let Some(end) = clean[i..].find(']') else {
                break;
            };
            let raw = clean[i + 1..i + end]
                .trim()
                .trim_matches(|c| c == '[' || c == ']');
            section = raw.split('.').map(str::trim).collect::<Vec<_>>().join(".");
            spec.defined |= section == "tool.uv.workspace";
            i += end + 1;
            continue;
        }
        let Some(eq) = clean[i..].find('=') else {
            break;
        };
        i = apply_toml_kv(&clean, &section, i, eq, &mut spec);
    }
    spec
}

fn skip_inline_value(src: &str, mut pos: usize) -> usize {
    let bytes = src.as_bytes();
    let mut depth = 0usize;
    while pos < bytes.len() {
        match bytes[pos] {
            b'"' | b'\'' => {
                if let Some((_, next)) = parse_toml_string(src, pos) {
                    pos = next;
                    continue;
                }
            }
            b'[' | b'{' => depth += 1,
            b']' => depth = depth.saturating_sub(1),
            b'}' | b',' if depth == 0 => return pos,
            b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        pos += 1;
    }
    pos
}

fn parse_inline_workspace(src: &str, mut pos: usize, spec: &mut WorkspaceSpec) -> Option<usize> {
    let bytes = src.as_bytes();
    while bytes.get(pos).is_some_and(u8::is_ascii_whitespace) {
        pos += 1;
    }
    if bytes.get(pos) != Some(&b'{') {
        return None;
    }
    spec.defined = true;
    pos += 1;
    while pos < src.len() {
        while bytes
            .get(pos)
            .is_some_and(|b| b.is_ascii_whitespace() || *b == b',')
        {
            pos += 1;
        }
        if bytes.get(pos) == Some(&b'}') {
            return Some(pos + 1);
        }
        let eq_rel = src[pos..].find('=')?;
        if src[pos..pos + eq_rel].contains('}') {
            return None;
        }
        let key = src[pos..pos + eq_rel].trim();
        let val_start = pos + eq_rel + 1;
        pos = if key == "members" {
            let (vals, next) = parse_string_array(src, val_start)?;
            spec.members = vals;
            next
        } else if key == "exclude" {
            let (vals, next) = parse_string_array(src, val_start)?;
            spec.exclude = vals;
            next
        } else {
            skip_inline_value(src, val_start)
        };
    }
    None
}

fn apply_toml_kv(
    clean: &str,
    section: &str,
    start: usize,
    eq: usize,
    spec: &mut WorkspaceSpec,
) -> usize {
    let key = clean[start..start + eq]
        .trim()
        .split('.')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(".");
    let full = if section.is_empty() {
        key
    } else {
        format!("{section}.{key}")
    };
    let val_start = start + eq + 1;
    if full == "tool.uv.workspace"
        && let Some(next) = parse_inline_workspace(clean, val_start, spec)
    {
        return next;
    }
    if full == "tool.uv.workspace.members" {
        spec.defined = true;
        if let Some((vals, next)) = parse_string_array(clean, val_start) {
            spec.members = vals;
            return next;
        }
    } else if full == "tool.uv.workspace.exclude" {
        spec.defined = true;
        if let Some((vals, next)) = parse_string_array(clean, val_start) {
            spec.exclude = vals;
            return next;
        }
    }
    skip_toml_value(clean, val_start)
}

fn fold_eq(a: char, b: char, ci: bool) -> bool {
    a == b || (ci && a.to_lowercase().eq(b.to_lowercase()))
}

fn fold_char(c: char, ci: bool) -> char {
    if ci {
        c.to_lowercase().next().unwrap_or(c)
    } else {
        c
    }
}

fn match_char_class(pat: &[char], start: usize, ch: char, ci: bool) -> Option<(bool, usize)> {
    let neg = pat.get(start) == Some(&'!');
    let mut i = start + usize::from(neg);
    let mut first = true;
    let mut matched = false;
    let cf = fold_char(ch, ci);
    while i < pat.len() && (first || pat[i] != ']') {
        first = false;
        if i + 2 < pat.len() && pat[i + 1] == '-' && pat[i + 2] != ']' {
            let (lo, hi) = (fold_char(pat[i], ci), fold_char(pat[i + 2], ci));
            matched |= (lo..=hi).contains(&cf);
            i += 3;
        } else {
            matched |= fold_eq(pat[i], ch, ci);
            i += 1;
        }
    }
    (i < pat.len() && pat[i] == ']').then_some((matched ^ neg, i + 1))
}

fn step_glob_token(p: &[char], ch: char, ci: bool) -> Option<usize> {
    match *p.first()? {
        '?' => Some(1),
        '[' => match match_char_class(p, 1, ch, ci) {
            Some((ok, next)) => ok.then_some(next),
            None => (ch == '[').then_some(1),
        },
        pc if fold_eq(pc, ch, ci) => Some(1),
        _ => None,
    }
}

fn glob_match_ci(pattern: &str, text: &str, ci: bool) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti, mut star, mut match_ti) = (0usize, 0usize, None, 0usize);
    while ti < t.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            match_ti = ti;
            pi += 1;
        } else if let Some(step) = step_glob_token(&p[pi..], t[ti], ci) {
            pi += step;
            ti += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            match_ti += 1;
            ti = match_ti;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn py_path_match_ci(member: &str, pattern: &str, ci: bool) -> bool {
    fn split(s: &str) -> Vec<&str> {
        s.split('/')
            .filter(|part| !part.is_empty() && *part != ".")
            .collect()
    }
    if pattern.is_empty() {
        return false;
    }
    let m_norm = if ci {
        member.replace('\\', "/")
    } else {
        member.to_owned()
    };
    let p_norm = if ci {
        pattern.replace('\\', "/")
    } else {
        pattern.to_owned()
    };
    let (m_anch, p_anch) = (m_norm.starts_with('/'), p_norm.starts_with('/'));
    let (m_parts, p_parts) = (split(&m_norm), split(&p_norm));
    if p_anch {
        return m_anch
            && m_parts.len() == p_parts.len()
            && m_parts
                .iter()
                .zip(p_parts.iter())
                .all(|(m, p)| glob_match_ci(p, m, ci));
    }
    if p_parts.is_empty() || m_parts.len() < p_parts.len() {
        return false;
    }
    let tail = &m_parts[m_parts.len() - p_parts.len()..];
    tail.iter()
        .zip(p_parts.iter())
        .all(|(m, p)| glob_match_ci(p, m, ci))
}

fn py_path_match(member: &str, pattern: &str) -> bool {
    py_path_match_ci(member, pattern, cfg!(windows))
}

fn posix_rel(rel: &Path) -> String {
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    }
}

fn find_lock_owner(project: &Path, root: &Path, native: bool) -> Result<PathBuf, String> {
    if native {
        return Ok(project.to_path_buf());
    }
    for parent in project.ancestors() {
        let manifest = parent.join("pyproject.toml");
        if manifest.is_file() {
            let text = fs::read_to_string(&manifest).map_err(|e| e.to_string())?;
            let spec = parse_uv_workspace(&text);
            if spec.defined {
                let rel = parent_rel(project, parent)?;
                let matched = rel == "."
                    || (spec.members.iter().any(|p| py_path_match(&rel, p))
                        && !spec.exclude.iter().any(|p| py_path_match(&rel, p)));
                if matched {
                    return Ok(parent.to_path_buf());
                }
            }
        }
        if pypath::same_path(parent, root) {
            break;
        }
    }
    Ok(project.to_path_buf())
}

fn parent_rel(child: &Path, parent: &Path) -> Result<String, String> {
    child.strip_prefix(parent).map(posix_rel).map_err(|_| {
        format!(
            "{} is outside {}",
            pypath::text(child),
            pypath::text(parent)
        )
    })
}

fn should_hash_source(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let path = Path::new(name);
    let ext_ok = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        SOURCE_EXTENSIONS.contains(&format!(".{}", e.to_ascii_lowercase()).as_str())
    });
    if !ext_ok {
        return false;
    }
    !path.components().any(|comp| {
        let s = comp.as_os_str().to_string_lossy();
        let lower = s.to_ascii_lowercase();
        s.starts_with(".env")
            || matches!(lower.as_str(), "credentials" | "secrets" | "node_modules")
    })
}

fn collect_source_hashes(ops: &dyn Ops, root: &Path) -> Result<Vec<(PyStr, Json)>, String> {
    let raw = git_output(
        ops,
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    let unique: BTreeSet<&str> = raw.split('\0').collect();
    let mut pairs = Vec::new();
    for name in unique {
        if !should_hash_source(name) {
            continue;
        }
        let full = root.join(name);
        let Ok(meta) = fs::symlink_metadata(&full) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_file() {
            continue;
        }
        let bytes = fs::read(&full).map_err(|e| e.to_string())?;
        pairs.push((PyStr::from(name), Json::str(&sha256(&bytes))));
    }
    Ok(pairs)
}

fn repo_head_and_status(ops: &dyn Ops, root: &Path) -> Result<(String, Json, Vec<Json>), String> {
    let count = git_output(ops, root, &["rev-list", "--all", "--count"])?;
    let head = if count.trim() != "0" {
        Json::str(git_output(ops, root, &["rev-parse", "--verify", "--quiet", "HEAD"])?.trim())
    } else {
        Json::Null
    };
    let status_raw = git_output(
        ops,
        root,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    let status_items = status_raw.lines().map(Json::str).collect();
    let repo_name = root
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    Ok((repo_name, head, status_items))
}

fn project_evidence(ops: &dyn Ops, project: &Path) -> Result<Json, String> {
    let project =
        pypath::resolve(project, false).map_err(|_| "Cannot resolve project".to_owned())?;
    let top = git_output(ops, &project, &["rev-parse", "--show-toplevel"])?;
    let root = pypath::resolve(Path::new(top.trim()), false)
        .map_err(|_| "Cannot resolve root".to_owned())?;
    let relative = parent_rel(&project, &root)?;
    let native = project.join("Cargo.toml").is_file();
    let lock_owner = find_lock_owner(&project, &root, native)?;
    let lock = lock_owner.join(if native { "Cargo.lock" } else { "uv.lock" });
    if !lock.is_file() {
        return Err(format!(
            "Missing authoritative lockfile: {}",
            pypath::text(&lock)
        ));
    }
    let hashes = collect_source_hashes(ops, &root)?;
    let (repo_name, head, status_items) = repo_head_and_status(ops, &root)?;
    let lock_rel = parent_rel(&lock, &root)?;
    let lock_bytes = fs::read(&lock).map_err(|e| e.to_string())?;
    let scope = if pypath::same_path(&lock_owner, &project) {
        "project"
    } else {
        "workspace"
    };
    Ok(Json::object([
        ("repository", Json::str(&repo_name)),
        ("project", Json::str(&relative)),
        ("revision", head),
        ("working_tree_status", Json::Array(status_items)),
        ("lockfile", Json::str(&lock_rel)),
        ("lock_sha256", Json::str(&sha256(&lock_bytes))),
        ("lock_scope", Json::str(scope)),
        ("source_sha256", Json::Object(hashes)),
        (
            "source_scope",
            Json::str(
                "Git-listed source and fixtures with documented text extensions; local secrets, symlinks and ignored files excluded.",
            ),
        ),
    ]))
}

fn native_wip(project: &Path) -> bool {
    project.join("Cargo.toml").is_file()
}

struct Prepared {
    projects: Vec<(&'static str, PathBuf)>,
    output: PathBuf,
    environment_root: Option<PathBuf>,
}

fn resolve_path(path: &Path) -> Result<PathBuf, String> {
    pypath::resolve(path, false).map_err(|_| format!("Cannot resolve path: {}", pypath::text(path)))
}

fn required_owners(workflow: &str) -> &'static [&'static str] {
    match workflow {
        "all" => &["wip", "grants"],
        "job-cash" => &["wip"],
        "grant-cash" => &["grants"],
        _ => &[],
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "validates workflow checkout and output paths"
)]
fn prepare(
    ops: &dyn Ops,
    component: &Path,
    output: &Path,
    fpa: &Path,
    accounting: Option<&Path>,
    grants: Option<&Path>,
    environment_root: Option<&Path>,
    workflow: &str,
) -> Result<Prepared, String> {
    let component_resolved = resolve_path(component)?;
    let fpa_resolved = resolve_path(fpa)?;
    let mut projects = vec![
        ("close", component_resolved.clone()),
        ("fpa", fpa_resolved.clone()),
    ];
    let mut roots = vec![
        component_resolved
            .parent()
            .and_then(Path::parent)
            .map_or_else(|| component_resolved.clone(), Path::to_path_buf),
        fpa_resolved,
    ];
    if let Some(acc) = accounting {
        let acc_resolved = resolve_path(acc)?;
        projects.push((
            "wip",
            resolve_path(&acc_resolved.join("packages/the-wip-tally"))?,
        ));
        roots.push(acc_resolved);
    }
    if let Some(gr) = grants {
        let gr_resolved = resolve_path(gr)?;
        projects.push(("grants", gr_resolved.clone()));
        roots.push(gr_resolved);
    }
    let required = required_owners(workflow);
    if required
        .iter()
        .any(|req| !projects.iter().any(|(o, _)| o == req))
    {
        return Err("This workflow needs --accounting and/or --grants checkouts".to_owned());
    }
    for (_, project) in &projects {
        roots.push(project.clone());
    }
    check_manifests_and_paths(ops, &projects, &roots, output, environment_root, required)
}

fn check_manifests_and_paths(
    ops: &dyn Ops,
    projects: &[(&'static str, PathBuf)],
    roots: &[PathBuf],
    output: &Path,
    environment_root: Option<&Path>,
    required: &[&str],
) -> Result<Prepared, String> {
    for &(owner, ref project) in projects {
        let manifest = if owner == "close" || (owner == "wip" && native_wip(project)) {
            "Cargo.toml"
        } else {
            "pyproject.toml"
        };
        if !project.join(manifest).is_file() {
            return Err(format!(
                "Missing project manifest: {}",
                pypath::text(project)
            ));
        }
    }
    let out_resolved = resolve_path(output)?;
    if roots
        .iter()
        .any(|root| pypath::is_within(&out_resolved, root))
    {
        return Err("Output must be outside all source checkouts".to_owned());
    }
    if fs::symlink_metadata(&out_resolved).is_ok() {
        return Err("Choose a new output directory".to_owned());
    }
    let env_resolved = match environment_root {
        Some(env_root) => Some(check_env_root(env_root, &out_resolved, roots)?),
        None => None,
    };
    if !ops.which("uv") {
        return Err("Install uv before running the locked workflow".to_owned());
    }
    if !ops.which("cargo") {
        return Err("Install Rust (cargo) before running the native close workflow".to_owned());
    }
    if required.contains(&"wip") {
        let wip = &projects
            .iter()
            .find(|(o, _)| *o == "wip")
            .expect("checked")
            .1;
        check_wip_entrypoint(wip)?;
    }
    Ok(Prepared {
        projects: projects.to_vec(),
        output: out_resolved,
        environment_root: env_resolved,
    })
}

fn check_env_root(
    env_root: &Path,
    out_resolved: &Path,
    roots: &[PathBuf],
) -> Result<PathBuf, String> {
    let resolved = resolve_path(env_root)?;
    if fs::symlink_metadata(&resolved).is_ok()
        || pypath::same_path(&resolved, out_resolved)
        || pypath::is_within(&resolved, out_resolved)
        || pypath::is_within(out_resolved, &resolved)
    {
        return Err("Fresh environment root must be a separate new directory".to_owned());
    }
    if roots.iter().any(|root| pypath::is_within(&resolved, root)) {
        return Err("Fresh environments must be outside source checkouts".to_owned());
    }
    Ok(resolved)
}

fn check_wip_entrypoint(wip: &Path) -> Result<(), String> {
    if native_wip(wip) {
        if !wip.join("examples/job_to_cash.rs").is_file() {
            return Err("The WIP checkout needs examples/job_to_cash.rs".to_owned());
        }
    } else if !wip.join("examples/job_to_cash.py").is_file() {
        return Err(
            "The WIP checkout needs examples/job_to_cash.py from australian-accounting PR #236"
                .to_owned(),
        );
    }
    Ok(())
}

struct WorkflowContext<'a> {
    ops: &'a dyn Ops,
    component: &'a Path,
    projects: &'a [(&'static str, PathBuf)],
    provenance: &'a Json,
    project_revs: &'a BTreeMap<String, Option<String>>,
    output: &'a Path,
    environment_root: Option<&'a Path>,
    timeout: f64,
    calls: Vec<CallRecord>,
}

enum RunFailure {
    CallRecorded(String),
    SourceChanged(String),
    Other(String),
}

fn call_to_json(call: &CallRecord) -> Json {
    Json::object([
        ("owner", Json::str(&call.owner)),
        ("kind", Json::str(&call.kind)),
        (
            "arguments",
            Json::Array(call.arguments.iter().map(|s| Json::str(s)).collect()),
        ),
        (
            "exit_code",
            call.exit_code
                .map_or(Json::Null, |c| Json::int(i64::from(c))),
        ),
        ("expected_exit", Json::int(i64::from(call.expected_exit))),
        ("elapsed_seconds", Json::Float(call.elapsed_seconds)),
        (
            "failure",
            call.failure.as_deref().map_or(Json::Null, Json::str),
        ),
    ])
}

fn calls_to_json(calls: &[CallRecord]) -> Json {
    Json::Array(calls.iter().map(call_to_json).collect())
}

fn project_revisions(provenance: &Json) -> BTreeMap<String, Option<String>> {
    let mut map = BTreeMap::new();
    if let Some(pairs) = provenance.as_object() {
        for (owner, evidence) in pairs {
            if let Some(name) = owner.to_utf8() {
                let rev = evidence
                    .get("revision")
                    .and_then(Json::as_pystr)
                    .and_then(|s| s.to_utf8());
                map.insert(name, rev);
            }
        }
    }
    map
}

impl<'a> WorkflowContext<'a> {
    fn project(&self, owner: &str) -> &Path {
        &self
            .projects
            .iter()
            .find(|(o, _)| *o == owner)
            .expect("known owner")
            .1
    }

    fn execute(
        &mut self,
        owner: &str,
        arguments: &[&str],
        expected: i32,
        kind: &str,
        runner_override: Option<&[&str]>,
    ) -> Result<String, RunFailure> {
        let project = self.project(owner).to_path_buf();
        let project_str = pypath::text(&project);
        let mut command: Vec<String> = match runner_override {
            Some(runner) => runner.iter().map(|s| (*s).to_owned()).collect(),
            None if owner == "close" => CLOSE_BIN.iter().map(|s| (*s).to_owned()).collect(),
            None => [
                "uv",
                "run",
                "--project",
                &project_str,
                "--locked",
                "--python",
                "3.14",
                "python",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        };
        command.extend(arguments.iter().map(|s| (*s).to_owned()));
        let env_vars = match self.environment_root {
            Some(root) => vec![(
                "UV_PROJECT_ENVIRONMENT".to_owned(),
                pypath::text(&root.join(owner)),
            )],
            None => Vec::new(),
        };
        self.record_execution(
            owner, &project, &command, arguments, expected, kind, &env_vars,
        )
    }

    fn write_failed_call_report(&self, stdout: &str, stderr: &str) {
        let record = Json::object([
            ("calls", calls_to_json(&self.calls)),
            ("projects", self.provenance.clone()),
            ("stdout", Json::str(stdout)),
            ("stderr", Json::str(stderr)),
        ]);
        let _ = utility_results::write_platform_text(
            &self.output.join("failed-calls.json"),
            &pyjson::pretty_ordered(&record),
        );
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "records one subprocess invocation and failure artefact"
    )]
    fn record_execution(
        &mut self,
        owner: &str,
        cwd: &Path,
        command: &[String],
        arguments: &[&str],
        expected: i32,
        kind: &str,
        env_vars: &[(String, String)],
    ) -> Result<String, RunFailure> {
        let started = Instant::now();
        let (exit_code, stdout, stderr, failure) =
            match self.ops.run(command, cwd, env_vars, self.timeout) {
                Ok(cap) => (cap.exit_code, cap.stdout, cap.stderr, None),
                Err(CaptureError::Timeout { stdout, stderr }) => (
                    None,
                    stdout,
                    stderr,
                    Some("timeout_or_interrupt".to_owned()),
                ),
                Err(CaptureError::Launch(msg)) => {
                    (None, String::new(), msg, Some("launch_error".to_owned()))
                }
            };
        let elapsed = (started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1_000_000.0;
        self.calls.push(CallRecord {
            owner: owner.to_owned(),
            kind: kind.to_owned(),
            arguments: arguments.iter().map(|s| (*s).to_owned()).collect(),
            exit_code,
            expected_exit: expected,
            elapsed_seconds: elapsed,
            failure: failure.clone(),
        });
        if failure.is_some() || exit_code != Some(expected) {
            let reason = failure
                .unwrap_or_else(|| exit_code.map_or_else(|| "None".to_owned(), |c| c.to_string()));
            self.write_failed_call_report(&stdout, &stderr);
            let msg = format!(
                "{owner} command {}: {reason}; see failed-calls.json",
                self.calls.len()
            );
            let _ = utility_results::summary(
                self.output,
                &self.calls,
                self.project_revs,
                None,
                Some(&msg),
            );
            return Err(RunFailure::CallRecorded(format!(
                "{owner} failed ({reason}); see {}",
                pypath::text(&self.output.join("failed-calls.json"))
            )));
        }
        Ok(stdout)
    }

    fn close_pack(
        &mut self,
        current: &Path,
        prior: &Path,
        subledger: &Path,
        target: &Path,
        mapping: Option<&Path>,
    ) -> Result<(), RunFailure> {
        let (c, p, s, t) = (
            pypath::text(current),
            pypath::text(prior),
            pypath::text(subledger),
            pypath::text(target),
        );
        let mut args: Vec<String> = vec![
            "review".into(),
            "--current".into(),
            c,
            "--prior".into(),
            p,
            "--subledger".into(),
            s,
            "--output".into(),
            t.clone(),
        ];
        if let Some(map_path) = mapping {
            args.push("--mapping".into());
            args.push(pypath::text(map_path));
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        self.execute("close", &refs, 2, "workflow", None)?;
        self.execute("close", &["view", "--pack-dir", &t], 0, "workflow", None)?;
        Ok(())
    }
}

fn check_semver_line(text: &str, prefix: &str) -> Option<String> {
    let rest = text.strip_prefix(prefix)?;
    let version = rest
        .strip_suffix("\r\n")
        .or_else(|| rest.strip_suffix('\n'))?;
    let mut parts = version.split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    let patch = parts.next()?;
    if parts.next().is_some()
        || !major.starts_with(|c: char| ('1'..='9').contains(&c))
        || !major.bytes().all(|b| b.is_ascii_digit())
        || minor.is_empty()
        || !minor.bytes().all(|b| b.is_ascii_digit())
        || patch.is_empty()
        || !patch.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some(version.to_owned())
}

fn collect_runtimes(ctx: &mut WorkflowContext<'_>) -> Result<(Json, String), RunFailure> {
    let mut pairs = Vec::new();
    let owners: Vec<&'static str> = ctx.projects.iter().map(|(o, _)| *o).collect();
    for owner in owners {
        let entry = if owner == "close" {
            let out = ctx.execute(owner, &["--version"], 0, "runtime", None)?;
            let ver = check_semver_line(&out, "close-control ").ok_or_else(|| {
                RunFailure::Other("Native close runtime version differs".to_owned())
            })?;
            Json::object([
                ("runtime", Json::str("rust")),
                ("command", Json::str("close-control")),
                ("version", Json::str(&ver)),
            ])
        } else if owner == "wip" && native_wip(ctx.project(owner)) {
            let out = ctx.execute(owner, &["--version"], 0, "runtime", Some(WIP_BIN))?;
            let ver = check_semver_line(&out, "wip-tally ").ok_or_else(|| {
                RunFailure::Other("Native WIP runtime version differs".to_owned())
            })?;
            Json::object([
                ("runtime", Json::str("rust")),
                ("command", Json::str("wip-tally")),
                ("version", Json::str(&ver)),
            ])
        } else {
            let out = ctx.execute(owner, &["-c", PYTHON_PROBE], 0, "runtime", None)?;
            pyjson::loads(&out).map_err(|_| RunFailure::Other("Invalid runtime JSON".to_owned()))?
        };
        pairs.push((PyStr::from(owner), entry));
    }
    let uv = ctx
        .ops
        .run(
            &["uv".to_owned(), "--version".to_owned()],
            ctx.component,
            &[],
            30.0,
        )
        .ok()
        .filter(|c| c.exit_code == Some(0))
        .ok_or_else(|| RunFailure::Other("Unable to record uv version".to_owned()))?;
    Ok((Json::Object(pairs), uv.stdout.trim().to_owned()))
}

fn verify_close_forecast_sources(inputs: &Path, refresh: &Path) -> Result<(), RunFailure> {
    let handoff = utility_results::read_json(inputs, "handoff.json")
        .map_err(|e| RunFailure::Other(e.to_string()))?;
    let refreshed = utility_results::read_json(refresh, "review.json")
        .map_err(|e| RunFailure::Other(e.to_string()))?;
    let h_sha = handoff
        .get("source_sha256")
        .ok_or_else(|| RunFailure::Other("Missing source_sha256".into()))?;
    let r_sha = refreshed
        .get("source_sha256")
        .ok_or_else(|| RunFailure::Other("Missing source_sha256".into()))?;
    for name in ["xero_bs.csv", "invoices.csv", "payments.csv"] {
        let a = h_sha
            .get(name)
            .ok_or_else(|| RunFailure::Other(format!("Missing {name}")))?;
        let b = r_sha
            .get(name)
            .ok_or_else(|| RunFailure::Other(format!("Missing {name}")))?;
        if !pyjson::py_eq(a, b, false) {
            return Err(RunFailure::Other(
                "Close and forecast sources differ".to_owned(),
            ));
        }
    }
    Ok(())
}

fn run_close_forecast(ctx: &mut WorkflowContext<'_>) -> Result<(), RunFailure> {
    let inputs = ctx.output.join("lumbridge-inputs");
    let pack = ctx.output.join("lumbridge-close");
    let refresh = ctx.output.join("lumbridge-refresh");
    let in_s = pypath::text(&inputs);
    let ref_s = pypath::text(&refresh);
    ctx.execute(
        "fpa",
        &[
            "examples/lumbridge-services/models/generated/close_handoff.py",
            "--output",
            &in_s,
        ],
        0,
        "workflow",
        None,
    )?;
    ctx.close_pack(
        &inputs.join("current.csv"),
        &inputs.join("prior.csv"),
        &inputs.join("subledger.csv"),
        &pack,
        Some(&inputs.join("mapping.csv")),
    )?;
    ctx.execute(
        "fpa",
        &[
            "examples/lumbridge-services/models/generated/recurring.py",
            "--output",
            &ref_s,
        ],
        0,
        "workflow",
        None,
    )?;
    verify_close_forecast_sources(&inputs, &refresh)
}

fn compare_quarter_periods(
    ctx: &mut WorkflowContext<'_>,
    prior: &Path,
    current: &Path,
) -> Result<(), RunFailure> {
    let (pp, cp) = (
        pypath::text(&prior.join("close")),
        pypath::text(&current.join("close")),
    );
    let (pt, ct) = (
        pypath::text(&prior.join("trial-balance.csv")),
        pypath::text(&current.join("trial-balance.csv")),
    );
    let args = [
        "compare",
        "--previous-pack",
        &pp,
        "--current-pack",
        &cp,
        "--previous-tb",
        &pt,
        "--current-tb",
        &ct,
    ];
    let cmp = ctx.execute("close", &args, 0, "workflow", None)?;
    utility_results::write_platform_text(&current.join("comparison.json"), &cmp)
        .map_err(|e| RunFailure::Other(e.to_string()))
}

fn run_quarter(ctx: &mut WorkflowContext<'_>) -> Result<(), RunFailure> {
    let quarter = ctx.output.join("quarter");
    let q_str = pypath::text(&quarter);
    ctx.execute(
        "fpa",
        &["examples/quarter-close/quarter.py", "--output", &q_str],
        0,
        "workflow",
        None,
    )?;
    let q_doc = utility_results::read_json(&quarter, "quarter.json")
        .map_err(|e| RunFailure::Other(e.to_string()))?;
    let periods = q_doc
        .get("periods")
        .ok_or_else(|| RunFailure::Other("Missing periods".into()))?;
    utility_results::quarter_periods(periods).map_err(|e| RunFailure::Other(e.to_string()))?;
    let cutoffs: Vec<String> = periods
        .as_array()
        .expect("checked")
        .iter()
        .map(|p| p.at("cutoff").as_pystr().unwrap().to_utf8().unwrap())
        .collect();
    for index in 1..cutoffs.len() {
        let current = quarter.join(&cutoffs[index]);
        let prior = quarter.join(&cutoffs[index - 1]);
        ctx.close_pack(
            &current.join("trial-balance.csv"),
            &prior.join("trial-balance.csv"),
            &current.join("subledger.csv"),
            &current.join("close"),
            None,
        )?;
        if index > 1 {
            compare_quarter_periods(ctx, &prior, &current)?;
        }
    }
    Ok(())
}

fn run_job_cash(ctx: &mut WorkflowContext<'_>) -> Result<(), RunFailure> {
    let wip_native = native_wip(ctx.project("wip"));
    for (label, extra) in [("job-base", "0"), ("job-extra-cost", "100000")] {
        let target = ctx.output.join(label);
        let t_str = pypath::text(&target);
        if wip_native {
            ctx.execute(
                "wip",
                &["--output", &t_str, "--extra-cost-to-complete", extra],
                0,
                "workflow",
                Some(WIP_EXAMPLE),
            )?;
        } else {
            ctx.execute(
                "wip",
                &[
                    "examples/job_to_cash.py",
                    "--output",
                    &t_str,
                    "--extra-cost-to-complete",
                    extra,
                ],
                0,
                "workflow",
                None,
            )?;
        }
        let forecast = ctx.execute(
            "fpa",
            &["examples/job-to-cash/project_cash.py", "--pack", &t_str],
            0,
            "workflow",
            None,
        )?;
        utility_results::write_platform_text(&target.join("project-cash.json"), &forecast)
            .map_err(|e| RunFailure::Other(e.to_string()))?;
    }
    Ok(())
}

fn run_grant_cash(ctx: &mut WorkflowContext<'_>) -> Result<(), RunFailure> {
    let grant = ctx.output.join("grants");
    let g_str = pypath::text(&grant);
    ctx.execute(
        "grants",
        &["--input", "examples/two-grants", "--output", &g_str],
        2,
        "workflow",
        Some(GRANTS_BIN),
    )?;
    let workpaper = grant.join("workpaper.json");
    let bytes = fs::read(&workpaper).map_err(|e| RunFailure::Other(e.to_string()))?;
    let digest = sha256(&bytes);
    let w_str = pypath::text(&workpaper);
    let out = ctx.execute(
        "fpa",
        &[
            "examples/restricted-cash/grant_cash.py",
            "--workpaper",
            &w_str,
            "--workpaper-sha256",
            &digest,
        ],
        0,
        "workflow",
        None,
    )?;
    utility_results::write_platform_text(&ctx.output.join("grant-cash.json"), &out)
        .map_err(|e| RunFailure::Other(e.to_string()))
}

fn collect_output_hashes(output: &Path) -> Result<Vec<(PyStr, Json)>, String> {
    fn walk(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                walk(&path, files)?;
            } else if entry.file_type()?.is_file() {
                files.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(output, &mut files).map_err(|e| e.to_string())?;
    let mut pairs: Vec<(String, String)> = files
        .into_iter()
        .map(|p| {
            let rel = posix_rel(p.strip_prefix(output).expect("within output"));
            let digest = sha256(&fs::read(&p).map_err(|e| e.to_string())?);
            Ok((rel, digest))
        })
        .collect::<Result<_, String>>()?;
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(pairs
        .into_iter()
        .map(|(k, v)| (PyStr::from(k.as_str()), Json::str(&v)))
        .collect())
}

fn run_selected_routes(ctx: &mut WorkflowContext<'_>, workflow: &str) -> Result<(), RunFailure> {
    if matches!(workflow, "all" | "close-forecast") {
        run_close_forecast(ctx)?;
    }
    if matches!(workflow, "all" | "quarter") {
        run_quarter(ctx)?;
    }
    if matches!(workflow, "all" | "job-cash") {
        run_job_cash(ctx)?;
    }
    if matches!(workflow, "all" | "grant-cash") {
        run_grant_cash(ctx)?;
    }
    Ok(())
}

fn execute_and_finalise(
    ctx: &mut WorkflowContext<'_>,
    workflow: &str,
    evidence_fn: &dyn Fn(&dyn Ops, &Path) -> Result<Json, String>,
) -> Result<Json, RunFailure> {
    let (runtimes, uv_ver) = collect_runtimes(ctx)?;
    run_selected_routes(ctx, workflow)?;
    let verified = utility_results::validate(ctx.output, workflow, &ctx.calls, ctx.project_revs)
        .map_err(|e| RunFailure::Other(e.to_string()))?;
    let after_pairs = ctx
        .projects
        .iter()
        .map(|&(owner, ref proj)| evidence_fn(ctx.ops, proj).map(|ev| (PyStr::from(owner), ev)))
        .collect::<Result<Vec<_>, String>>()
        .map_err(RunFailure::Other)?;
    let after = Json::Object(after_pairs);
    if !pyjson::py_eq(&after, ctx.provenance, false) {
        let change = Json::object([("before", ctx.provenance.clone()), ("after", after)]);
        let _ = utility_results::write_platform_text(
            &ctx.output.join("source-changes.json"),
            &pyjson::pretty_ordered(&change),
        );
        let _ = utility_results::summary(
            ctx.output,
            &ctx.calls,
            ctx.project_revs,
            None,
            Some("Source changed during execution; see source-changes.json"),
        );
        return Err(RunFailure::SourceChanged(
            "Source changed during the run; review source-changes.json".to_owned(),
        ));
    }
    utility_results::summary(
        ctx.output,
        &ctx.calls,
        ctx.project_revs,
        Some(&verified),
        None,
    )
    .map_err(|e| RunFailure::Other(e.to_string()))?;
    let output_hashes = collect_output_hashes(ctx.output).map_err(RunFailure::Other)?;
    let manifest = build_manifest(workflow, ctx, runtimes, &uv_ver, output_hashes);
    let tmp = ctx.output.join("manifest.json.tmp");
    utility_results::write_platform_text(&tmp, &pyjson::pretty_ordered(&manifest))
        .map_err(|e| RunFailure::Other(e.to_string()))?;
    fs::rename(&tmp, ctx.output.join("manifest.json"))
        .map_err(|e| RunFailure::Other(e.to_string()))?;
    Ok(manifest)
}

fn build_manifest(
    workflow: &str,
    ctx: &WorkflowContext<'_>,
    runtimes: Json,
    uv_ver: &str,
    output_hashes: Vec<(PyStr, Json)>,
) -> Json {
    Json::object([
        ("fixture_validation", Json::str("passed")),
        ("execution_mode", Json::str("source-project")),
        ("schema_version", Json::str("utility-workflows.v2")),
        ("workflow", Json::str(workflow)),
        ("calls", calls_to_json(&ctx.calls)),
        ("projects", ctx.provenance.clone()),
        ("runtimes", runtimes),
        (
            "tools",
            Json::object([
                (
                    "driver_rust",
                    Json::str(concat!(
                        "monthly-close-control-plane ",
                        env!("CARGO_PKG_VERSION")
                    )),
                ),
                ("uv", Json::str(uv_ver)),
            ]),
        ),
        ("outputs_sha256", Json::Object(output_hashes)),
        (
            "scope",
            Json::str(
                "Fabricated local workflows. Engine review statuses remain unchanged. Fresh environments do not establish a hosted CI result.",
            ),
        ),
    ])
}

#[expect(
    clippy::too_many_arguments,
    reason = "CLI configuration for run_workflows"
)]
fn run_workflows_with(
    ops: &dyn Ops,
    evidence_fn: &dyn Fn(&dyn Ops, &Path) -> Result<Json, String>,
    component: &Path,
    output: &Path,
    fpa: &Path,
    accounting: Option<&Path>,
    grants: Option<&Path>,
    environment_root: Option<&Path>,
    workflow: &str,
    timeout: f64,
) -> Result<(Json, usize), String> {
    if !(timeout > 0.0 && timeout <= 3600.0) {
        return Err("Command timeout must be positive and at most 3600 seconds".to_owned());
    }
    let prepared = prepare(
        ops,
        component,
        output,
        fpa,
        accounting,
        grants,
        environment_root,
        workflow,
    )?;
    let prov_pairs = prepared
        .projects
        .iter()
        .map(|&(owner, ref proj)| evidence_fn(ops, proj).map(|ev| (PyStr::from(owner), ev)))
        .collect::<Result<Vec<_>, String>>()?;
    let provenance = Json::Object(prov_pairs);
    let project_revs = project_revisions(&provenance);
    fs::create_dir_all(&prepared.output).map_err(|e| e.to_string())?;
    let replay = Json::object([
        ("schema_version", Json::str("utility-workflows.v2")),
        ("workflow", Json::str(workflow)),
        ("projects", provenance.clone()),
    ]);
    utility_results::write_platform_text(
        &prepared.output.join("replay.json"),
        &pyjson::pretty_ordered(&replay),
    )
    .map_err(|e| e.to_string())?;
    if let Some(env_root) = &prepared.environment_root {
        fs::create_dir_all(env_root).map_err(|e| e.to_string())?;
    }
    let mut ctx = WorkflowContext {
        ops,
        component,
        projects: &prepared.projects,
        provenance: &provenance,
        project_revs: &project_revs,
        output: &prepared.output,
        environment_root: prepared.environment_root.as_deref(),
        timeout,
        calls: Vec::new(),
    };
    finish_workflow(&mut ctx, workflow, evidence_fn)
}

fn finish_workflow(
    ctx: &mut WorkflowContext<'_>,
    workflow: &str,
    evidence_fn: &dyn Fn(&dyn Ops, &Path) -> Result<Json, String>,
) -> Result<(Json, usize), String> {
    match execute_and_finalise(ctx, workflow, evidence_fn) {
        Ok(manifest) => Ok((manifest, ctx.calls.len())),
        Err(err) => {
            let _ = fs::remove_file(ctx.output.join("manifest.json"));
            let _ = fs::remove_file(ctx.output.join("manifest.json.tmp"));
            match err {
                RunFailure::CallRecorded(msg) | RunFailure::SourceChanged(msg) => Err(msg),
                RunFailure::Other(msg) => {
                    if !ctx.output.join("failed-calls.json").exists()
                        && !ctx.output.join("source-changes.json").exists()
                    {
                        let _ = utility_results::summary(
                            ctx.output,
                            &ctx.calls,
                            ctx.project_revs,
                            None,
                            Some("Fixture result validation failed; see failed-results.json"),
                        );
                        let record = Json::object([
                            ("error", Json::str(&msg)),
                            ("calls", calls_to_json(&ctx.calls)),
                            ("projects", ctx.provenance.clone()),
                        ]);
                        let _ = utility_results::write_platform_text(
                            &ctx.output.join("failed-results.json"),
                            &pyjson::pretty_ordered(&record),
                        );
                        return Err(
                            "Fixture result validation failed; see failed-results.json".to_owned()
                        );
                    }
                    Err(msg)
                }
            }
        }
    }
}

struct CliArgs {
    fpa: PathBuf,
    accounting: Option<PathBuf>,
    grants: Option<PathBuf>,
    output: PathBuf,
    environment_root: Option<PathBuf>,
    timeout: f64,
    workflow: String,
}

fn usage_line(prog: &str) -> String {
    format!(
        "usage: {prog} [-h] --fpa FPA [--accounting ACCOUNTING] [--grants GRANTS] --output OUTPUT [--environment-root ENVIRONMENT_ROOT] [--timeout TIMEOUT] [--workflow {{all,close-forecast,quarter,job-cash,grant-cash}}]"
    )
}
const HELP: &str = "Run fabricated workflows with explicit lock authority and source evidence.\n\noptions:\n  -h, --help            show this help message and exit\n  --fpa FPA\n  --accounting ACCOUNTING\n  --grants GRANTS\n  --output OUTPUT\n  --environment-root ENVIRONMENT_ROOT\n                        optional new directory for isolated installations\n  --timeout TIMEOUT     seconds per command, including setup (default: 300)\n  --workflow {all,close-forecast,quarter,job-cash,grant-cash}";

struct CliBuilder {
    fpa: Option<PathBuf>,
    accounting: Option<PathBuf>,
    grants: Option<PathBuf>,
    output: Option<PathBuf>,
    env_root: Option<PathBuf>,
    timeout: f64,
    workflow: String,
}

fn negative_number(token: &str) -> bool {
    let Some(body) = token.strip_prefix('-') else {
        return false;
    };
    !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}

fn set_cli_flag(flag: &str, value: &str, builder: &mut CliBuilder) -> Result<(), String> {
    match flag {
        "--fpa" => builder.fpa = Some(PathBuf::from(value)),
        "--accounting" => builder.accounting = Some(PathBuf::from(value)),
        "--grants" => builder.grants = Some(PathBuf::from(value)),
        "--output" => builder.output = Some(PathBuf::from(value)),
        "--environment-root" => builder.env_root = Some(PathBuf::from(value)),
        "--timeout" => {
            builder.timeout = value
                .parse::<f64>()
                .map_err(|_| format!("argument --timeout: invalid float value: {value:?}"))?;
        }
        "--workflow" => {
            if !matches!(
                value,
                "all" | "close-forecast" | "quarter" | "job-cash" | "grant-cash"
            ) {
                return Err(format!(
                    "argument --workflow: invalid choice: {value:?} (choose from all, close-forecast, quarter, job-cash, grant-cash)"
                ));
            }
            builder.workflow = value.to_owned();
        }
        _ => {}
    }
    Ok(())
}

fn split_cli_flag<'a>(
    token: &'a str,
    args: &'a [String],
    idx: &mut usize,
) -> Result<Option<(&'a str, &'a str)>, String> {
    let known = [
        "--fpa",
        "--accounting",
        "--grants",
        "--output",
        "--environment-root",
        "--timeout",
        "--workflow",
    ];
    let (flag, explicit) = match token.split_once('=') {
        Some((k, v)) if k.starts_with("--") => (k, Some(v)),
        _ if token.starts_with("--") => (token, None),
        _ => return Ok(None),
    };
    if !known.contains(&flag) {
        return Ok(None);
    }
    let value = match explicit {
        Some(v) => v,
        None => match args.get(*idx) {
            Some(next) if !next.starts_with('-') || next == "-" || negative_number(next) => {
                *idx += 1;
                next.as_str()
            }
            _ => return Err(format!("argument {flag}: expected one argument")),
        },
    };
    Ok(Some((flag, value)))
}

fn parse_cli(args: &[String]) -> Result<Option<CliArgs>, String> {
    let mut b = CliBuilder {
        fpa: None,
        accounting: None,
        grants: None,
        output: None,
        env_root: None,
        timeout: 300.0,
        workflow: "all".to_owned(),
    };
    let mut extras = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let token = &args[i];
        i += 1;
        if token == "-h" || token == "--help" {
            return Ok(None);
        }
        match split_cli_flag(token, args, &mut i)? {
            Some((flag, value)) => set_cli_flag(flag, value, &mut b)?,
            None => extras.push(token.as_str()),
        }
    }
    if !extras.is_empty() {
        return Err(format!("unrecognized arguments: {}", extras.join(" ")));
    }
    let missing: Vec<&str> = [("--fpa", b.fpa.is_none()), ("--output", b.output.is_none())]
        .into_iter()
        .filter_map(|(n, m)| m.then_some(n))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "the following arguments are required: {}",
            missing.join(", ")
        ));
    }
    Ok(Some(CliArgs {
        fpa: b.fpa.unwrap(),
        accounting: b.accounting,
        grants: b.grants,
        output: b.output.unwrap(),
        environment_root: b.env_root,
        timeout: b.timeout,
        workflow: b.workflow,
    }))
}

fn default_component() -> PathBuf {
    if let Ok(exe) = env::current_exe()
        && let Some(dir) = exe.parent()
        && dir.file_name().is_some_and(|n| n == "examples")
    {
        let parent = dir.parent();
        let in_target = parent
            .and_then(Path::file_name)
            .is_some_and(|n| n == "debug" || n == "release");
        if !in_target && let Some(comp) = parent {
            return comp.to_path_buf();
        }
        if in_target
            && let Some(crate_dir) = parent.and_then(Path::parent).and_then(Path::parent)
            && crate_dir.join("Cargo.toml").is_file()
        {
            return crate_dir.to_path_buf();
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run_main(
    ops: &dyn Ops,
    component: &Path,
    prog: &str,
    args: &[String],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let usage = usage_line(prog);
    let parsed = match parse_cli(args) {
        Ok(Some(cli)) => cli,
        Ok(None) => {
            let _ = write_stream(out, &format!("{usage}\n\n{HELP}\n"));
            return 0;
        }
        Err(message) => {
            let _ = write_stream(err, &format!("{usage}\n{prog}: error: {message}\n"));
            return 2;
        }
    };
    match run_workflows_with(
        ops,
        &project_evidence,
        component,
        &parsed.output,
        &parsed.fpa,
        parsed.accounting.as_deref(),
        parsed.grants.as_deref(),
        parsed.environment_root.as_deref(),
        &parsed.workflow,
        parsed.timeout,
    ) {
        Ok((_, count)) => {
            let _ = write_stream(
                out,
                &format!("UTILITY WORKFLOWS VERIFIED: {count} successful commands\n"),
            );
            0
        }
        Err(message) => {
            let _ = write_stream(err, &format!("Utility workflows: {message}\n"));
            1
        }
    }
}

fn write_stream(stream: &mut dyn Write, text: &str) -> std::io::Result<()> {
    let payload = if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text.to_owned()
    };
    stream.write_all(payload.as_bytes())
}

fn main() -> std::process::ExitCode {
    let mut argv = env::args_os();
    let prog = argv
        .next()
        .and_then(|first| {
            Path::new(&first)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "utility_workflows".to_owned());
    let args: Vec<String> = argv.map(|a| pypath::decode_os(&a)).collect();
    let mut out = std::io::stdout().lock();
    let mut err = std::io::stderr().lock();
    let code = run_main(
        &SystemOps,
        &default_component(),
        &prog,
        &args,
        &mut out,
        &mut err,
    );
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::temp;
    use super::*;
    use std::cell::RefCell;

    fn sys_ops() -> SystemOps {
        SystemOps
    }

    type StubHandler =
        Box<dyn Fn(&[String], &Path, &[(String, String)], f64) -> Result<Captured, CaptureError>>;
    type SeenCall = (Vec<String>, Vec<(String, String)>);

    struct StubOps {
        missing_tool: Option<&'static str>,
        handler: StubHandler,
        seen: RefCell<Vec<SeenCall>>,
    }

    impl Ops for StubOps {
        fn which(&self, tool: &str) -> bool {
            self.missing_tool != Some(tool)
        }
        fn run(
            &self,
            command: &[String],
            cwd: &Path,
            env_vars: &[(String, String)],
            timeout_secs: f64,
        ) -> Result<Captured, CaptureError> {
            self.seen
                .borrow_mut()
                .push((command.to_vec(), env_vars.to_vec()));
            (self.handler)(command, cwd, env_vars, timeout_secs)
        }
    }

    fn init_git_repo(root: &Path) {
        let ops = sys_ops();
        git_output(&ops, root, &["init"]).unwrap();
    }

    fn seed_wip_and_fpa_only(root: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let accounting = root.join("accounting");
        let wip = accounting.join("packages/the-wip-tally");
        let fpa = root.join("fpa");
        for project in [&wip, &fpa] {
            fs::create_dir_all(project).unwrap();
            fs::write(project.join("pyproject.toml"), "[project]\n").unwrap();
        }
        let close = root.join("repo/packages/close");
        fs::create_dir_all(&close).unwrap();
        fs::write(close.join("Cargo.toml"), "[package]\n").unwrap();
        (close, fpa, accounting, wip)
    }

    #[test]
    fn missing_companion_fails_before_creating_outputs() {
        let temp = temp::TempDir::new();
        let (close, fpa, accounting, _) = seed_wip_and_fpa_only(&temp.0);
        let ops = sys_ops();
        let output = temp.join("output");
        let envs = temp.join("environments");
        let err = prepare(
            &ops,
            &close,
            &output,
            &fpa,
            Some(&accounting),
            None,
            Some(&envs),
            "job-cash",
        )
        .err()
        .unwrap();
        assert!(
            err.contains("job_to_cash.py") && err.contains("#236"),
            "{err}"
        );
        assert!(!output.exists() && !envs.exists());
    }

    #[test]
    fn dest_guards_fail_before_creating_outputs() {
        let temp = temp::TempDir::new();
        let (close, fpa, _, _) = seed_wip_and_fpa_only(&temp.0);
        let ops = sys_ops();
        assert!(
            prepare(
                &ops,
                &close,
                &temp.join("new"),
                &fpa,
                None,
                None,
                None,
                "all"
            )
            .is_err()
        );
        assert!(
            prepare(
                &ops,
                &close,
                &fpa.join("generated"),
                &fpa,
                None,
                None,
                None,
                "quarter"
            )
            .is_err()
        );
        let existing = temp.join("existing");
        fs::create_dir(&existing).unwrap();
        assert!(prepare(&ops, &close, &existing, &fpa, None, None, None, "quarter").is_err());
    }

    #[test]
    fn fresh_environments_cannot_overlap_outputs() {
        let temp = temp::TempDir::new();
        let close = temp.join("repo/packages/close");
        let fpa = temp.join("fpa");
        fs::create_dir_all(&close).unwrap();
        fs::write(close.join("Cargo.toml"), "[package]\n").unwrap();
        fs::create_dir_all(&fpa).unwrap();
        fs::write(fpa.join("pyproject.toml"), "[project]\n").unwrap();
        let output = temp.join("results");
        for envs in [output.clone(), output.join("envs"), temp.0.clone()] {
            let err = prepare(
                &sys_ops(),
                &close,
                &output,
                &fpa,
                None,
                None,
                Some(&envs),
                "quarter",
            )
            .err()
            .unwrap();
            assert!(err.contains("separate new directory"), "{err}");
        }
    }

    #[test]
    fn provenance_uses_workspace_lock_and_tracks_uncommitted_and_deleted_sources() {
        let temp = temp::TempDir::new();
        init_git_repo(&temp.0);
        fs::write(
            temp.join("pyproject.toml"),
            "[tool.uv.workspace]\nmembers = [\"packages/*\"]\n",
        )
        .unwrap();
        fs::write(temp.join("uv.lock"), "workspace resolution").unwrap();
        let child = temp.join("packages/component");
        fs::create_dir_all(&child).unwrap();
        fs::write(
            child.join("pyproject.toml"),
            "[project]\nname = \"component\"\n",
        )
        .unwrap();
        fs::write(child.join("uv.lock"), "standalone resolution").unwrap();
        let source = child.join("example.py");
        fs::write(&source, "first = 1\n").unwrap();
        let ops = sys_ops();
        let before = project_evidence(&ops, &child).unwrap();
        assert!(
            before.at("lockfile").is_str("uv.lock") && before.at("lock_scope").is_str("workspace")
        );
        assert!(matches!(before.at("revision"), Json::Null));
        fs::write(&source, "first = 2\n").unwrap();
        let after = project_evidence(&ops, &child).unwrap();
        assert_ne!(
            before
                .at("source_sha256")
                .at("packages/component/example.py")
                .as_pystr(),
            after
                .at("source_sha256")
                .at("packages/component/example.py")
                .as_pystr()
        );
        git_output(
            &ops,
            &temp.0,
            &["add", "--", "packages/component/example.py"],
        )
        .unwrap();
        fs::remove_file(&source).unwrap();
        let removed = project_evidence(&ops, &child).unwrap();
        assert!(
            removed
                .at("source_sha256")
                .get("packages/component/example.py")
                .is_none()
        );
    }

    #[test]
    fn native_project_provenance_uses_cargo_lock_and_missing_lock_fails() {
        let temp = temp::TempDir::new();
        init_git_repo(&temp.0);
        fs::write(
            temp.join("pyproject.toml"),
            "[tool.uv.workspace]\nmembers = [\"packages/*\"]\n",
        )
        .unwrap();
        fs::write(temp.join("uv.lock"), "Python workspace resolution").unwrap();
        let native = temp.join("packages/native");
        fs::create_dir_all(&native).unwrap();
        fs::write(native.join("Cargo.toml"), "[package]\nname = \"fixture\"\n").unwrap();
        fs::write(native.join("Cargo.lock"), "Native resolution").unwrap();
        let ops = sys_ops();
        let ev = project_evidence(&ops, &native).unwrap();
        assert!(
            ev.at("lockfile").is_str("packages/native/Cargo.lock")
                && ev.at("lock_scope").is_str("project")
        );
        fs::remove_file(native.join("Cargo.lock")).unwrap();
        assert!(
            project_evidence(&ops, &native)
                .unwrap_err()
                .contains("Missing authoritative lockfile")
        );
    }

    #[test]
    fn command_timeout_stops_process_tree() {
        let temp = temp::TempDir::new();
        let cmd = if cfg!(windows) {
            vec![
                "cmd".to_owned(),
                "/C".to_owned(),
                "echo ready & ping -n 10 127.0.0.1 >NUL".to_owned(),
            ]
        } else {
            vec![
                "sh".to_owned(),
                "-c".to_owned(),
                "echo ready; sleep 10".to_owned(),
            ]
        };
        let err = captured(&cmd, &temp.0, &[], 0.15).unwrap_err();
        assert!(
            matches!(err, CaptureError::Timeout { ref stdout, .. } if stdout.contains("ready")),
            "{err:?}"
        );
    }

    #[test]
    fn native_wip_requires_job_to_cash_rs() {
        let temp = temp::TempDir::new();
        let (close, fpa, acc, _) = seed_all_tree(&temp.0);
        let wip = acc.join("packages/the-wip-tally");
        let out = temp.join("out");
        assert!(
            prepare(
                &sys_ops(),
                &close,
                &out,
                &fpa,
                Some(&acc),
                None,
                None,
                "job-cash"
            )
            .is_ok()
        );
        fs::remove_file(wip.join("examples/job_to_cash.rs")).unwrap();
        assert!(
            prepare(
                &sys_ops(),
                &close,
                &out,
                &fpa,
                Some(&acc),
                None,
                None,
                "job-cash"
            )
            .err()
            .unwrap()
            .contains("job_to_cash.rs")
        );
    }

    #[test]
    fn missing_tools_fail_before_creating_outputs() {
        let temp = temp::TempDir::new();
        let (close, fpa, _, _) = seed_all_tree(&temp.0);
        let out = temp.join("out");
        for tool in ["uv", "cargo"] {
            let stub = StubOps {
                missing_tool: Some(tool),
                handler: Box::new(|_, _, _, _| unreachable!()),
                seen: RefCell::new(Vec::new()),
            };
            assert!(
                prepare(&stub, &close, &out, &fpa, None, None, None, "quarter")
                    .err()
                    .unwrap()
                    .contains(tool)
            );
        }
        assert!(!out.exists());
    }

    fn make_failing_stub(mode: &str) -> StubOps {
        let m = mode.to_owned();
        StubOps {
            missing_tool: None,
            seen: RefCell::new(Vec::new()),
            handler: Box::new(move |_, _, _, _| match m.as_str() {
                "timeout" => Err(CaptureError::Timeout {
                    stdout: "partial".into(),
                    stderr: "timed out".into(),
                }),
                "launch" => Err(CaptureError::Launch("unavailable".into())),
                _ => Ok(Captured {
                    exit_code: Some(1),
                    stdout: "partial".into(),
                    stderr: "failure".into(),
                }),
            }),
        }
    }

    #[test]
    fn failed_steps_preserve_diagnostics_and_never_write_manifest() {
        let temp = temp::TempDir::new();
        let (close, fpa, _, _) = seed_all_tree(&temp.0);
        let fake_ev =
            |_: &dyn Ops, _: &Path| Ok(Json::object([("revision", Json::str("fixture"))]));
        for mode in ["exit", "timeout", "launch"] {
            let out = temp.join(format!("results-{mode}"));
            let stub = make_failing_stub(mode);
            let err = run_workflows_with(
                &stub, &fake_ev, &close, &out, &fpa, None, None, None, "quarter", 30.0,
            )
            .unwrap_err();
            assert!(err.contains("failed"), "{err}");
            assert!(
                out.join("failed-calls.json").is_file()
                    && out.join("replay.json").is_file()
                    && !out.join("manifest.json").exists()
            );
        }
    }

    fn seed_all_tree(root: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let close = root.join("repo/packages/monthly-close-control-plane");
        let fpa = root.join("au-fpa-pack");
        let acc = root.join("australian-accounting");
        let wip = acc.join("packages/the-wip-tally");
        let grants = root.join("grants");
        fs::create_dir_all(&close).unwrap();
        fs::write(close.join("Cargo.toml"), "[package]\n").unwrap();
        for dir in [&fpa, &grants] {
            fs::create_dir_all(dir).unwrap();
            fs::write(dir.join("pyproject.toml"), "[project]\n").unwrap();
        }
        fs::create_dir_all(wip.join("examples")).unwrap();
        fs::write(wip.join("Cargo.toml"), "[package]\n").unwrap();
        fs::write(wip.join("examples/job_to_cash.rs"), "fn main() {}\n").unwrap();
        (close, fpa, acc, grants)
    }

    fn stub_version_reply(cmd: &[String]) -> Captured {
        let text = if cmd.iter().any(|s| s == "close-control") {
            "close-control 1.2.0\n"
        } else if cmd.iter().any(|s| s == "wip-tally") {
            "wip-tally 1.0.0\n"
        } else {
            "uv 0.13.9\n"
        };
        Captured {
            exit_code: Some(0),
            stdout: text.into(),
            stderr: String::new(),
        }
    }

    fn stub_compare_reply(cmd: &[String]) -> Captured {
        let cur = cmd
            .windows(2)
            .find(|w| w[0] == "--current-pack")
            .map(|w| PathBuf::from(&w[1]))
            .unwrap();
        let period = cur
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let prev = if period == "2026-11-30" {
            "2026-10-31"
        } else {
            "2026-11-30"
        };
        let json = Json::object([
            ("previous_period", Json::str(prev)),
            ("current_period", Json::str(&period)),
            ("previous_status", Json::str("REVIEW")),
            ("current_status", Json::str("REVIEW")),
        ]);
        Captured {
            exit_code: Some(0),
            stdout: pyjson::pretty_ordered(&json),
            stderr: String::new(),
        }
    }

    fn handle_stub_command(cmd: &[String]) -> Result<Captured, CaptureError> {
        if cmd.last().map(String::as_str) == Some("--version") {
            return Ok(stub_version_reply(cmd));
        }
        if cmd.iter().any(|s| s == "-c") {
            return Ok(Captured {
                exit_code: Some(0),
                stdout: r#"{"python":"3.14","packages":[]}"#.into(),
                stderr: String::new(),
            });
        }
        let out_path = cmd
            .windows(2)
            .find(|w| w[0] == "--output")
            .map(|w| PathBuf::from(&w[1]));
        if cmd.iter().any(|s| s == "compare") {
            return Ok(stub_compare_reply(cmd));
        }
        if cmd.iter().any(|s| s == "review") {
            let target = out_path.unwrap();
            fs::create_dir_all(&target).unwrap();
            fs::write(
                target.join("close-review-pack.json"),
                r#"{"overall_status":"REVIEW","exceptions":[{}]}"#,
            )
            .unwrap();
            return Ok(Captured {
                exit_code: Some(2),
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        respond_remaining_commands(cmd, out_path)
    }

    fn respond_remaining_commands(
        cmd: &[String],
        out_path: Option<PathBuf>,
    ) -> Result<Captured, CaptureError> {
        let joined = cmd.join(" ");
        if let Some(target) = out_path {
            return respond_output_command(&joined, &target);
        }
        respond_stdout_command(cmd, &joined)
    }

    fn respond_output_command(joined: &str, target: &Path) -> Result<Captured, CaptureError> {
        fs::create_dir_all(target).unwrap();
        if joined.contains("close_handoff.py") {
            fs::write(
                target.join("handoff.json"),
                r#"{"source_sha256":{"xero_bs.csv":"h","invoices.csv":"h","payments.csv":"h"}}"#,
            )
            .unwrap();
        } else if joined.contains("recurring.py") {
            fs::write(target.join("review.json"), r#"{"closed_through":"2026-10-31","actuals":{"ending_cash":"-1960"},"source_sha256":{"xero_bs.csv":"h","invoices.csv":"h","payments.csv":"h"}}"#).unwrap();
        } else if joined.contains("quarter.py") {
            let mut periods = Vec::new();
            for (c, cash) in utility_results::CUTOFFS
                .iter()
                .zip(utility_results::CASH.iter())
            {
                periods.push(Json::object([
                    ("cutoff", Json::str(c)),
                    ("balances", Json::object([("cash", Json::str(cash))])),
                ]));
                fs::create_dir_all(target.join(c)).unwrap();
            }
            fs::write(
                target.join("quarter.json"),
                pyjson::pretty_ordered(&Json::object([("periods", Json::Array(periods))])),
            )
            .unwrap();
        } else if joined.contains("grant-workpaper") {
            let findings: Vec<Json> = utility_results::GRANT_FINDINGS
                .iter()
                .map(|(c, r)| Json::object([("code", Json::str(c)), ("reference", Json::str(r))]))
                .collect();
            fs::write(
                target.join("workpaper.json"),
                pyjson::pretty_ordered(&Json::object([
                    ("status", Json::str("REVIEW")),
                    ("findings", Json::Array(findings)),
                ])),
            )
            .unwrap();
            return Ok(Captured {
                exit_code: Some(2),
                stdout: String::new(),
                stderr: String::new(),
            });
        }
        Ok(Captured {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        })
    }

    fn stub_project_cash(cmd: &[String]) -> Captured {
        let pack = cmd.windows(2).find(|w| w[0] == "--pack").unwrap()[1].clone();
        let cash = if pack.ends_with("job-base") {
            "-434500"
        } else {
            "-544500"
        };
        let weeks: Vec<Json> = (1..=13)
            .map(|i| {
                Json::object([
                    ("week", Json::int(i)),
                    (
                        "ending_cash",
                        Json::str(if i <= 4 {
                            "-140000"
                        } else if i <= 10 {
                            "5500"
                        } else {
                            cash
                        }),
                    ),
                ])
            })
            .collect();
        Captured {
            exit_code: Some(0),
            stdout: pyjson::pretty_ordered(&Json::object([("weeks", Json::Array(weeks))])),
            stderr: String::new(),
        }
    }

    fn stub_grant_cash() -> Captured {
        let findings: Vec<Json> = utility_results::GRANT_FINDINGS
            .iter()
            .map(|(c, r)| Json::object([("code", Json::str(c)), ("reference", Json::str(r))]))
            .collect();
        let body = Json::object([
            ("workpaper_status", Json::str("REVIEW")),
            ("source_findings", Json::Array(findings)),
            (
                "forecast",
                Json::object([("closing_cash", Json::str("34800"))]),
            ),
            (
                "liquidity",
                Json::object([("status", Json::str("NO_SHORTFALL"))]),
            ),
            ("known_unpaid_allocations", Json::str("3200")),
            ("planned_commitment_cash", Json::str("200")),
            (
                "unplanned_commitments",
                Json::Array(vec![Json::object([
                    ("allocation_id", Json::str("A4")),
                    ("amount", Json::str("3000")),
                ])]),
            ),
        ]);
        Captured {
            exit_code: Some(0),
            stdout: pyjson::pretty_ordered(&body),
            stderr: String::new(),
        }
    }

    fn respond_stdout_command(cmd: &[String], joined: &str) -> Result<Captured, CaptureError> {
        if joined.contains("project_cash.py") {
            return Ok(stub_project_cash(cmd));
        }
        if joined.contains("grant_cash.py") {
            return Ok(stub_grant_cash());
        }
        Ok(Captured {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
        })
    }

    #[test]
    fn all_routes_validate_and_write_manifest() {
        let temp = temp::TempDir::new();
        let (close, fpa, acc, grants) = seed_all_tree(&temp.0);
        let output = temp.join("results");
        let env_root = temp.join("envs");
        let stub = StubOps {
            missing_tool: None,
            seen: RefCell::new(Vec::new()),
            handler: Box::new(|cmd, _, _, _| handle_stub_command(cmd)),
        };
        let stable =
            |_: &dyn Ops, _: &Path| Ok(Json::object([("revision", Json::str(&"a".repeat(40)))]));
        let (manifest, count) = run_workflows_with(
            &stub,
            &stable,
            &close,
            &output,
            &fpa,
            Some(&acc),
            Some(&grants),
            Some(&env_root),
            "all",
            300.0,
        )
        .unwrap();
        assert_eq!(count, 23);
        assert!(manifest.at("fixture_validation").is_str("passed"));
        assert!(
            output.join("manifest.json").is_file() && !output.join("failed-results.json").exists()
        );
    }

    #[test]
    fn source_mutation_during_run_is_refused() {
        let temp = temp::TempDir::new();
        let (close, fpa, acc, grants) = seed_all_tree(&temp.0);
        let stub = StubOps {
            missing_tool: None,
            seen: RefCell::new(Vec::new()),
            handler: Box::new(|cmd, _, _, _| handle_stub_command(cmd)),
        };
        let mutate_calls = RefCell::new(0usize);
        let mutated = |_: &dyn Ops, _: &Path| {
            let mut n = mutate_calls.borrow_mut();
            *n += 1;
            let rev = if *n <= 4 {
                "a".repeat(40)
            } else {
                "b".repeat(40)
            };
            Ok(Json::object([("revision", Json::str(&rev))]))
        };
        let mut_out = temp.join("mutated-results");
        let err = run_workflows_with(
            &stub,
            &mutated,
            &close,
            &mut_out,
            &fpa,
            Some(&acc),
            Some(&grants),
            None,
            "all",
            300.0,
        )
        .unwrap_err();
        assert!(err.contains("Source changed"), "{err}");
        assert!(
            mut_out.join("source-changes.json").is_file()
                && !mut_out.join("manifest.json").exists()
        );
    }

    fn run_cli_case(root: &Path, args: &[&str]) -> (i32, String, String) {
        let vec_args: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run_main(
            &sys_ops(),
            root,
            "utility_workflows",
            &vec_args,
            &mut out,
            &mut err,
        );
        (
            code,
            String::from_utf8(out).unwrap(),
            String::from_utf8(err).unwrap(),
        )
    }

    #[test]
    fn cli_help_errors_and_invalid_timeout_keep_contracts() {
        let temp = temp::TempDir::new();
        let (code, out, err) = run_cli_case(&temp.0, &["--help"]);
        assert!(code == 0 && out.contains("--workflow") && err.is_empty());
        let (code, _, err) = run_cli_case(&temp.0, &["--fpa", "x"]);
        assert!(code == 2 && err.contains("--output"));
        let bad_out = temp.join("results");
        let fpa_s = temp.0.to_string_lossy().into_owned();
        let out_s = bad_out.to_string_lossy().into_owned();
        let (code, _, err) = run_cli_case(
            &temp.0,
            &["--fpa", &fpa_s, "--output", &out_s, "--timeout", "0"],
        );
        assert_eq!(code, 1);
        assert_eq!(
            err.replace("\r\n", "\n"),
            "Utility workflows: Command timeout must be positive and at most 3600 seconds\n"
        );
        assert!(!bad_out.exists());
    }

    #[test]
    fn toml_parser_handles_multiline_strings_and_inline_workspace() {
        let toml = concat!(
            "[package]\n",
            "description = \"\"\"\n[tool.uv.workspace]\nmembers = [\"bogus\"]\n\"\"\"\n",
            "note = '''\n[ignored]\nquoted \"\"\"stays\"\"\" literal\n'''\n",
            "[tool.uv]\n",
            "workspace = { extra = { nested = \"}\" }, members = [\"packages/*\"], exclude = [\"packages/ignored\"] }\n",
        );
        let spec = parse_uv_workspace(toml);
        assert!(spec.defined);
        assert_eq!(spec.members, vec!["packages/*".to_owned()]);
        assert_eq!(spec.exclude, vec!["packages/ignored".to_owned()]);
        let sample = "\"\"\"\nhello \\\n  \"world\"\"\"\"\"";
        let (parsed, end) = parse_toml_string(sample, 0).unwrap();
        assert_eq!(parsed, "hello \"world\"\"");
        assert_eq!(end, sample.len());
    }

    #[test]
    fn strip_toml_comments_handles_four_and_five_quote_closing_delimiters() {
        assert_eq!(
            strip_toml_comments("\"\"\"x\"\"\"\" # comment\n'''y''''' # comment"),
            "\"\"\"x\"\"\"\" \n'''y''''' "
        );
        assert_eq!(
            strip_toml_comments(
                "a = \"\"\"x # kept\"\"\"\" # stripped\nb = '''y # kept''''' # stripped\nc = \"z\" # stripped"
            ),
            "a = \"\"\"x # kept\"\"\"\" \nb = '''y # kept''''' \nc = \"z\" "
        );
        let toml = concat!(
            "a = \"\"\"x\"\"\"\" # comment\n",
            "b = '''y''''' # comment\n",
            "[tool.uv.workspace] # real\n",
            "members = [\"packages/*\"] # keep\n",
        );
        let spec = parse_uv_workspace(toml);
        assert!(spec.defined);
        assert_eq!(spec.members, vec!["packages/*".to_owned()]);
    }

    #[test]
    fn py_path_match_matches_purepath_brackets_anchors_and_case() {
        assert!(py_path_match_ci("a/b/c", "**", false));
        assert!(py_path_match_ci("a/b/c", "a/**/c", false));
        assert!(!py_path_match_ci("packages/x/y", "packages/**", false));
        assert!(py_path_match_ci("packages/a1", "packages/a[0-9]", false));
        assert!(!py_path_match_ci("packages/a1", "packages/a[!0-9]", false));
        assert!(py_path_match_ci("a*b", "a[*]b", false));
        assert!(py_path_match_ci("a^", "a[^b]", false));
        assert!(py_path_match_ci("a[", "a[", false));
        assert!(!py_path_match_ci("packages/a", "/packages/a", false));
        assert!(py_path_match_ci("/packages/a", "/packages/a", false));
        assert!(!py_path_match_ci("/x/packages/a", "/packages/a", false));
        assert!(py_path_match_ci("Packages/Foo", "packages/foo", true));
        assert!(!py_path_match_ci("Packages/Foo", "packages/foo", false));
    }

    #[cfg(unix)]
    fn write_exec_shim(bin: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = bin.join("fake-wf-tool");
        fs::write(&p, "#!/bin/sh\necho \"$1\"\n").unwrap();
        let mut perms = fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&p, perms).unwrap();
        p
    }

    #[cfg(not(unix))]
    fn write_exec_shim(bin: &Path) -> PathBuf {
        let p = bin.join("fake-wf-tool.cmd");
        fs::write(&p, "@echo off\r\necho %1\r\n").unwrap();
        p
    }

    #[test]
    fn captured_resolves_path_shim_and_reports_signal_exit() {
        let temp = temp::TempDir::new();
        let bin = temp.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let shim = write_exec_shim(&bin);
        let path_val = pypath::text(&bin);
        let found = which_on_path_in("fake-wf-tool", Some(OsStr::new(&path_val))).unwrap();
        if cfg!(windows) {
            let (got, exp) = (pypath::text(&found), pypath::text(&shim));
            assert!(got.eq_ignore_ascii_case(&exp));
        } else {
            assert_eq!(found, shim);
        }
        let env_vars = vec![("PATH".to_owned(), path_val)];
        let cap = captured(
            &["fake-wf-tool".into(), "ok".into()],
            &temp.0,
            &env_vars,
            10.0,
        )
        .unwrap();
        assert_eq!((cap.exit_code, cap.stdout.trim()), (Some(0), "ok"));
        #[cfg(unix)]
        {
            let sig = captured(
                &["sh".into(), "-c".into(), "kill -9 $$".into()],
                &temp.0,
                &[],
                10.0,
            )
            .unwrap();
            assert_eq!(sig.exit_code, Some(-9));
        }
    }
}
