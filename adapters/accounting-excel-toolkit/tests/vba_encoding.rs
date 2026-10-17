use accounting_excel_toolkit::{check_bytes, collect_targets, default_vba_dir, run};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const CLEAN: &[u8] = b"Attribute VB_Name = \"modClean\"\r\nOption Explicit\r\n";
static SERIAL: AtomicUsize = AtomicUsize::new(0);

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap();
        for _ in 0..100 {
            let path = root.join(format!(
                "excel-encoding-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("{e}"),
            }
        }
        panic!("could not allocate test directory");
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let root = std::env::temp_dir().canonicalize().unwrap();
        assert_eq!(self.0.parent(), Some(root.as_path()));
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn names(directory: &Path) -> Vec<String> {
    collect_targets(&[], directory)
        .unwrap()
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn every_text_suffix_and_case_is_collected_but_binary_and_unrelated_files_are_skipped() {
    let temp = Temp::new();
    for name in ["modA.bas", "clsB.CLS", "frmC.frm"] {
        temp.write(name, CLEAN);
    }
    temp.write("frmC.frx", b"\x00\x01\xff\xfe");
    temp.write("README.md", b"\xff\n");
    assert_eq!(names(&temp.0), ["clsB.CLS", "frmC.frm", "modA.bas"]);
}

#[test]
fn nested_dirty_form_is_reported_without_losing_clean_stdout() {
    let temp = Temp::new();
    temp.write("modTop.bas", CLEAN);
    let dirty = temp.write("forms/frmNested.frm", b"' tolerance \xe2\x80\x94 wide\r\n");
    let (mut out, mut err) = (Vec::new(), Vec::new());
    assert_eq!(
        run(&[], &temp.0, &mut out, &mut err).unwrap_err(),
        "1 of 2 file(s) are not VBE-importable as written"
    );
    let out = String::from_utf8(out).unwrap();
    let err = String::from_utf8(err).unwrap();
    assert!(out.contains("ok   "));
    assert!(!out.contains("FAIL"));
    assert!(!out.contains("file(s) pass"));
    assert!(err.contains(&dirty.display().to_string()));
    assert!(err.contains("3 non-ASCII byte(s)"));
}

#[test]
fn bom_non_ascii_and_both_line_ending_errors_keep_their_order() {
    let problems = check_bytes(b"\xef\xbb\xbfOption Explicit\nEnd Sub\r");
    assert_eq!(problems.len(), 4);
    for (problem, text) in problems.iter().zip([
        "starts with a UTF-8 BOM",
        "3 non-ASCII byte(s)",
        "1 bare LF line ending(s)",
        "1 bare CR (no following LF)",
    ]) {
        assert!(problem.starts_with(text), "{problem}");
    }
    assert!(check_bytes(CLEAN).is_empty());
    assert!(check_bytes(b"").is_empty());
}

#[test]
fn bare_cr_at_start_middle_and_end_is_not_confused_with_crlf() {
    assert_eq!(
        check_bytes(b"\rOption Explicit\rEnd Sub\r")[0],
        "3 bare CR (no following LF) at 0x0000 (byte 0x0d), 0x0010 (byte 0x0d), 0x0018 (byte 0x0d)"
    );
    assert!(check_bytes(b"\r\n").is_empty());
    assert!(check_bytes(b"\n")[0].starts_with("1 bare LF"));
}

#[test]
fn diagnostic_offsets_truncate_but_total_counts_do_not() {
    let problems = check_bytes(&[0xff; 25]);
    assert_eq!(problems.len(), 1);
    assert!(problems[0].starts_with("25 non-ASCII byte(s) at 0x0000 (byte 0xff)"));
    assert!(problems[0].ends_with("0x0013 (byte 0xff), ... 5 more"));
    assert!(!problems[0].contains("0x0014"));
    let mut large = vec![b' '; 0x10000];
    large.push(0x80);
    assert_eq!(
        check_bytes(&large),
        ["1 non-ASCII byte(s) at 0x10000 (byte 0x80)"]
    );
}

#[test]
fn missing_empty_and_unrelated_only_directories_fail_loudly() {
    let temp = Temp::new();
    assert!(
        collect_targets(&[], &temp.0)
            .unwrap_err()
            .starts_with("no .bas, .cls or .frm files found")
    );
    temp.write("modA.txt", CLEAN);
    assert!(collect_targets(&[], &temp.0).is_err());
    let missing = temp.0.join("gone");
    assert_eq!(
        collect_targets(&[], &missing).unwrap_err(),
        format!("no vba directory at {}", missing.display())
    );
}

#[test]
fn explicit_files_ignore_suffix_keep_order_and_duplicates_and_prevalidate_missing_paths() {
    let temp = Temp::new();
    let file = temp.write("anything.txt", CLEAN);
    let arguments = [file.clone().into_os_string(), file.clone().into_os_string()];
    assert_eq!(
        collect_targets(&arguments, &temp.0).unwrap(),
        [file.clone(), file]
    );
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let missing = [
        temp.0.join("z.cls").into_os_string(),
        temp.0.join("a.bas").into_os_string(),
    ];
    let error = run(&missing, &temp.0, &mut out, &mut err).unwrap_err();
    assert!(error.starts_with(&format!(
        "no such file: {}, {}",
        temp.0.join("a.bas").display(),
        temp.0.join("z.cls").display()
    )));
    assert!(out.is_empty() && err.is_empty());
}

#[test]
fn clean_directory_reports_the_full_count() {
    let temp = Temp::new();
    temp.write("modA.bas", CLEAN);
    temp.write("clsB.cls", CLEAN);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    run(&[], &temp.0, &mut out, &mut err).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap().lines().last(),
        Some("2 file(s) pass: pure ASCII, CRLF, no BOM")
    );
    assert!(err.is_empty());
}

#[test]
fn default_directory_uses_only_the_executable_and_known_build_layout() {
    let root = Temp::new();
    for suffix in [
        "check-vba-encoding",
        "tools/check-vba-encoding",
        "target/debug/check-vba-encoding",
        "target/release/check-vba-encoding",
    ] {
        assert_eq!(default_vba_dir(&root.0.join(suffix)), root.0.join("vba"));
    }
    assert_eq!(
        default_vba_dir(&root.0.join("elsewhere/debug/check-vba-encoding")),
        root.0.join("elsewhere/debug/vba")
    );
}

#[test]
fn installed_binary_is_independent_of_cwd_and_build_checkout() {
    let temp = Temp::new();
    let wrong = Temp::new();
    temp.write("vba/forms/Form.FRM", CLEAN);
    wrong.write("vba/Wrong.bas", b"\xff\n");
    let binary = temp.0.join(format!(
        "check-vba-encoding{}",
        std::env::consts::EXE_SUFFIX
    ));
    fs::copy(env!("CARGO_BIN_EXE_check-vba-encoding"), &binary).unwrap();
    let output = Command::new(binary).current_dir(&wrong.0).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 file(s) pass"));
    assert!(output.stderr.is_empty());
}

#[test]
fn cli_errors_have_no_traceback_and_do_not_mix_diagnostics_into_stdout() {
    let temp = Temp::new();
    let file = temp.write("clsProbe.cls", b"\xef\xbb\xbf\n");
    let output = Command::new(env!("CARGO_BIN_EXE_check-vba-encoding"))
        .arg(file)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.starts_with("FAIL "));
    assert_eq!(
        error.lines().last(),
        Some("error: 1 of 1 file(s) are not VBE-importable as written")
    );
    assert!(!error.contains("Traceback"));
}

#[test]
fn dash_arguments_are_literal_paths_as_in_the_original_guard() {
    let temp = Temp::new();
    temp.write("--help", CLEAN);
    let output = Command::new(env!("CARGO_BIN_EXE_check-vba-encoding"))
        .arg("--help")
        .current_dir(&temp.0)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).lines().next(),
        Some("ok   --help")
    );
}

#[test]
fn tracked_sources_pass_and_guard_sources_remain_ascii() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    run(&[], &root.join("vba"), &mut Vec::new(), &mut Vec::new()).unwrap();
    for path in ["src/lib.rs", "src/main.rs", "tests/vba_encoding.rs"] {
        assert!(fs::read(root.join(path)).unwrap().is_ascii());
    }
}

#[test]
fn explicit_paths_normalise_dot_segments_and_windows_separators() {
    let temp = Temp::new();
    let path = temp.write("sub/mod.bas", CLEAN);
    let argument: OsString = temp
        .0
        .join("sub")
        .join(".")
        .join("mod.bas")
        .into_os_string();
    assert_eq!(collect_targets(&[argument], &temp.0).unwrap(), [path]);
}

#[cfg(unix)]
#[test]
fn linked_files_are_checked_and_linked_directories_are_not_recursed() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let elsewhere = Temp::new();
    let file = elsewhere.write("linked.txt", b"\xff");
    symlink(&file, temp.0.join("linked.bas")).unwrap();
    symlink(&elsewhere.0, temp.0.join("directory")).unwrap();
    assert_eq!(names(&temp.0), ["linked.bas"]);
    assert!(
        run(&[], &temp.0, &mut Vec::new(), &mut Vec::new())
            .unwrap_err()
            .starts_with("1 of 1")
    );
}
