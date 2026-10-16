use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

fn line(stream: &mut dyn Write, text: std::fmt::Arguments<'_>) -> Result<(), String> {
    write!(
        stream,
        "{text}{}",
        if cfg!(windows) { "\r\n" } else { "\n" }
    )
    .map_err(|e| e.to_string())
}

fn describe(offsets: &[usize], data: &[u8]) -> String {
    let mut parts: Vec<_> = offsets
        .iter()
        .take(20)
        .map(|&offset| format!("0x{offset:04x} (byte 0x{:02x})", data[offset]))
        .collect();
    if offsets.len() > parts.len() {
        parts.push(format!("... {} more", offsets.len() - parts.len()));
    }
    parts.join(", ")
}

/// Check the three VBE text formats without decoding their source bytes.
pub fn check_bytes(data: &[u8]) -> Vec<String> {
    let mut problems = Vec::new();
    if data.starts_with(b"\xef\xbb\xbf") {
        problems.push(
            "starts with a UTF-8 BOM; the VBE reads it as literal text on the Attribute line"
                .into(),
        );
    }
    let non_ascii: Vec<_> = data
        .iter()
        .enumerate()
        .filter_map(|(i, &b)| (b > 0x7f).then_some(i))
        .collect();
    if !non_ascii.is_empty() {
        problems.push(format!(
            "{} non-ASCII byte(s) at {}",
            non_ascii.len(),
            describe(&non_ascii, data)
        ));
    }
    let bare_lf: Vec<_> = data
        .iter()
        .enumerate()
        .filter_map(|(i, &b)| (b == b'\n' && (i == 0 || data[i - 1] != b'\r')).then_some(i))
        .collect();
    if !bare_lf.is_empty() {
        problems.push(format!(
            "{} bare LF line ending(s) at {}; the VBE needs CRLF",
            bare_lf.len(),
            describe(&bare_lf, data)
        ));
    }
    let bare_cr: Vec<_> = data
        .iter()
        .enumerate()
        .filter_map(|(i, &b)| (b == b'\r' && data.get(i + 1) != Some(&b'\n')).then_some(i))
        .collect();
    if !bare_cr.is_empty() {
        problems.push(format!(
            "{} bare CR (no following LF) at {}",
            bare_cr.len(),
            describe(&bare_cr, data)
        ));
    }
    problems
}

fn normalise(path: &Path) -> PathBuf {
    let path: PathBuf = path
        .components()
        .filter(|part| *part != Component::CurDir)
        .collect();
    if path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        path
    }
}

/// Locate sources beside an installed binary, in tools/, or in a Cargo build.
pub fn default_vba_dir(executable: &Path) -> PathBuf {
    let directory = executable.parent().unwrap_or_else(|| Path::new("."));
    let root = if directory.file_name().is_some_and(|name| name == "tools") {
        directory.parent().unwrap_or(directory)
    } else if directory
        .file_name()
        .is_some_and(|name| name == "debug" || name == "release")
        && directory
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "target")
    {
        directory
            .parent()
            .and_then(Path::parent)
            .unwrap_or(directory)
    } else {
        directory
    };
    root.join("vba")
}

fn walk(directory: &Path, targets: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries =
        fs::read_dir(directory).map_err(|e| format!("cannot list {}: {e}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot list {}: {e}", directory.display()))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|e| format!("cannot list {}: {e}", directory.display()))?;
        if kind.is_dir() {
            walk(&path, targets)?;
        } else if path.is_file()
            && path.extension().is_some_and(|suffix| {
                ["bas", "cls", "frm"]
                    .iter()
                    .any(|text| suffix.eq_ignore_ascii_case(text))
            })
        {
            targets.push(path);
        }
    }
    Ok(())
}

/// Explicit paths are checked regardless of suffix, including repeated paths.
pub fn collect_targets(arguments: &[OsString], directory: &Path) -> Result<Vec<PathBuf>, String> {
    if !arguments.is_empty() {
        let targets: Vec<_> = arguments
            .iter()
            .map(|arg| normalise(Path::new(arg)))
            .collect();
        let mut missing: Vec<_> = targets
            .iter()
            .filter(|path| !path.is_file())
            .map(|path| path.display().to_string())
            .collect();
        missing.sort();
        return if missing.is_empty() {
            Ok(targets)
        } else {
            Err(format!("no such file: {}", missing.join(", ")))
        };
    }
    if !directory.is_dir() {
        return Err(format!("no vba directory at {}", directory.display()));
    }
    let mut targets = Vec::new();
    walk(directory, &mut targets)?;
    #[cfg(windows)]
    targets.sort_by_key(|path| PathBuf::from(path.to_string_lossy().to_lowercase()));
    #[cfg(not(windows))]
    targets.sort();
    if targets.is_empty() {
        return Err(format!(
            "no .bas, .cls or .frm files found in {}",
            directory.display()
        ));
    }
    Ok(targets)
}

pub fn run(
    arguments: &[OsString],
    directory: &Path,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> Result<(), String> {
    let targets = collect_targets(arguments, directory)?;
    let mut failures = Vec::new();
    for path in &targets {
        let data = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let problems = check_bytes(&data);
        if problems.is_empty() {
            line(out, format_args!("ok   {}", path.display()))?;
        } else {
            failures.push((path, problems));
        }
    }
    if !failures.is_empty() {
        out.flush().map_err(|e| e.to_string())?;
        for (path, problems) in &failures {
            for problem in problems {
                line(err, format_args!("FAIL {}: {problem}", path.display()))?;
            }
        }
        return Err(format!(
            "{} of {} file(s) are not VBE-importable as written",
            failures.len(),
            targets.len()
        ));
    }
    line(
        out,
        format_args!("{} file(s) pass: pure ASCII, CRLF, no BOM", targets.len()),
    )
}
