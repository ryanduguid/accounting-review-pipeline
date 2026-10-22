use super::{SAMPLE, transactions_cases};
use std::{
    ffi::{OsStr, OsString},
    fmt::Debug,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let root = std::env::temp_dir().canonicalize().unwrap();
        assert_eq!(self.0.parent(), Some(root.as_path()));
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn check<A: AsRef<OsStr> + Debug>(
    python: &Path,
    script: &Path,
    binary: &Path,
    arguments: &[A],
    exact: bool,
) -> Result<(), String> {
    let expected = Command::new(python)
        .args(["-I", "-B"])
        .arg(script)
        .args(arguments)
        .output()
        .map_err(|e| e.to_string())?;
    let actual = Command::new(binary)
        .args(arguments)
        .output()
        .map_err(|e| e.to_string())?;
    let streams_match = if exact {
        expected.stdout == actual.stdout && expected.stderr == actual.stderr
    } else {
        match expected.status.code() {
            Some(0) => !actual.stdout.is_empty() && actual.stderr.is_empty(),
            Some(2) => actual.stdout.is_empty() && !actual.stderr.is_empty(),
            _ => {
                actual.stdout == expected.stdout
                    && actual.stderr.starts_with(b"Account Transactions: ")
            }
        }
    };
    if expected.status.code() != actual.status.code() || !streams_match {
        return Err(format!(
            "CLI {arguments:?}: Python {expected:?}; Rust {actual:?}"
        ));
    }
    Ok(())
}

fn base(source: &Path) -> Vec<String> {
    vec![
        source.to_string_lossy().into_owned(),
        "--tenant".into(),
        "Fabricated Services".into(),
        "--account-name".into(),
        "Clearing".into(),
    ]
}

fn conversion(source: &Path, mapping: &Path) -> Vec<String> {
    let mut args = base(source);
    args.extend([
        "--mapping".into(),
        mapping.to_string_lossy().into_owned(),
        "--account-id".into(),
        "090".into(),
        "--currency".into(),
        "AUD".into(),
    ]);
    args
}

fn success_cases(
    python: &Path,
    script: &Path,
    binary: &Path,
    root: &Path,
) -> Result<usize, String> {
    let source = root.join("source.csv");
    let mapping = root.join("mapping.json");
    let mut count = 0;
    for text in [
        SAMPLE.to_owned(),
        format!("\u{feff}{SAMPLE}"),
        SAMPLE.replace("Receipt", "\u{e9}\u{7f}\u{1f600}"),
        SAMPLE.replace("Receipt", "\"receipt, \"\"quoted\"\"\r\ncontinued\""),
        SAMPLE.replace("Receipt", "\"bare\rreturn\""),
    ] {
        fs::write(&source, text.as_bytes()).map_err(|e| e.to_string())?;
        let map = transactions_cases::mapping(&text);
        fs::write(&mapping, map.to_string()).map_err(|e| e.to_string())?;
        check(python, script, binary, &base(&source), true)?;
        check(python, script, binary, &conversion(&source, &mapping), true)?;
        let duplicate = format!(
            "{{\"source_sha256\":\"wrong\",\"source_sha256\":{},\"lines\":{}}}",
            map["source_sha256"], map["lines"]
        );
        fs::write(&mapping, duplicate).map_err(|e| e.to_string())?;
        check(python, script, binary, &conversion(&source, &mapping), true)?;
        if fs::read(&source).map_err(|e| e.to_string())? != text.as_bytes() {
            return Err("source changed".into());
        }
        count += 3;
    }
    Ok(count)
}

fn refusals(python: &Path, script: &Path, binary: &Path, root: &Path) -> Result<usize, String> {
    let source = root.join("source.csv");
    let mapping = root.join("mapping.json");
    let mut count = 0;
    for prefix in ["=1", "+1", "-00123", "@x", "\u{1c}=SUM(1)", "\u{a0}@x"] {
        let text = SAMPLE.replace("Receipt", prefix);
        fs::write(&source, text.as_bytes()).map_err(|e| e.to_string())?;
        fs::write(&mapping, transactions_cases::mapping(&text).to_string())
            .map_err(|e| e.to_string())?;
        check(python, script, binary, &conversion(&source, &mapping), true)?;
        count += 1;
    }
    for text in [
        SAMPLE.replace("Total Clearing,,,,110.00", "Total Clearing,,,,0"),
        SAMPLE.replace("01/09/2026", "01/10/2026"),
        SAMPLE.replace("Running Balance", "Balance"),
    ] {
        fs::write(&source, text).map_err(|e| e.to_string())?;
        check(python, script, binary, &base(&source), true)?;
        count += 1;
    }
    fs::write(&source, SAMPLE).map_err(|e| e.to_string())?;
    for text in ["{}", "{", "\u{feff}{}", "null", "[]"] {
        fs::write(&mapping, text).map_err(|e| e.to_string())?;
        check(
            python,
            script,
            binary,
            &conversion(&source, &mapping),
            false,
        )?;
        count += 1;
    }
    fs::write(&source, [0xff]).map_err(|e| e.to_string())?;
    check(python, script, binary, &base(&source), false)?;
    Ok(count + 1)
}

fn usage_cases(python: &Path, script: &Path, binary: &Path, root: &Path) -> Result<usize, String> {
    let source = root.join("source.csv");
    fs::write(&source, SAMPLE).map_err(|e| e.to_string())?;
    let cases = [
        vec![],
        vec!["--help"],
        vec!["-h"],
        vec!["-hh"],
        vec!["-hhh"],
        vec!["-hfoo"],
        vec!["-h_-"],
        vec!["-h-h"],
        vec!["-hh-h"],
        vec!["-h=foo"],
        vec!["-h="],
        vec!["--help=yes"],
        vec!["--unknown"],
        vec!["--acc"],
        vec!["--tenant"],
        vec!["--tenant", "-h"],
        vec!["--", "--help"],
        vec!["one", "two", "--help"],
        vec!["--unknown", "--help"],
    ];
    for case in &cases {
        let args: Vec<_> = case.iter().map(|text| (*text).to_owned()).collect();
        check(python, script, binary, &args, false)?;
    }
    for alias in ["--tenant", "--ten", "--tenant=Fabricated Services"] {
        let mut args = vec![source.to_string_lossy().into_owned(), alias.to_owned()];
        if !alias.contains('=') {
            args.push("Fabricated Services".into());
        }
        args.extend(["--account-n".into(), "Clearing".into()]);
        check(python, script, binary, &args, true)?;
    }
    Ok(cases.len() + 3)
}

fn path_cases(python: &Path, script: &Path, binary: &Path, root: &Path) -> Result<usize, String> {
    #[cfg(unix)]
    let name = {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(b"nonunicode-\xff".to_vec())
    };
    #[cfg(windows)]
    let name = {
        use std::os::windows::ffi::OsStringExt;
        OsString::from_wide(&[u16::from(b'n'), 0xd800])
    };
    let source = root.join(&name).with_extension("csv");
    let mapping = root.join(name).with_extension("json");
    let map = transactions_cases::mapping(SAMPLE).to_string();
    fs::write(&source, SAMPLE).map_err(|e| e.to_string())?;
    fs::write(&mapping, &map).map_err(|e| e.to_string())?;
    let mut args = vec![source.as_os_str().to_owned()];
    args.extend(
        [
            "--tenant",
            "Fabricated Services",
            "--account-name",
            "Clearing",
        ]
        .map(OsString::from),
    );
    check(python, script, binary, &args, true)?;
    args.extend(["--account-id", "090", "--currency", "AUD"].map(OsString::from));
    let mut separate = args.clone();
    separate.extend([OsString::from("--mapping"), mapping.as_os_str().to_owned()]);
    check(python, script, binary, &separate, true)?;
    for prefix in ["--mapping=", "--map="] {
        let mut inline = args.clone();
        let mut option = OsString::from(prefix);
        option.push(&mapping);
        inline.push(option);
        check(python, script, binary, &inline, true)?;
    }
    if fs::read(&source).map_err(|e| e.to_string())? != SAMPLE.as_bytes()
        || fs::read(&mapping).map_err(|e| e.to_string())? != map.as_bytes()
    {
        return Err("non-Unicode path inputs changed".into());
    }
    Ok(4)
}

pub fn compare(python: &Path, script: &Path, binary: &Path, seed: u64) -> Result<usize, String> {
    let root = std::env::temp_dir()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let path = root.join(format!("excel-transactions-{}-{seed}", std::process::id()));
    fs::create_dir(&path).map_err(|e| e.to_string())?;
    let scratch = Scratch(path);
    Ok(success_cases(python, script, binary, &scratch.0)?
        + refusals(python, script, binary, &scratch.0)?
        + usage_cases(python, script, binary, &scratch.0)?
        + path_cases(python, script, binary, &scratch.0)?)
}
