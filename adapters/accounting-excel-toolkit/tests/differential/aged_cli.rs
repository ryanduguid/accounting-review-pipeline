use accounting_excel_toolkit::aged::{self, Json, json};
use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
struct Comparison<'a> {
    python: &'a Path,
    tools: PathBuf,
    bins: PathBuf,
    scratch: aged::replay::Scratch,
    count: usize,
}
fn os(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
fn option(args: &mut Vec<OsString>, name: &str, path: &Path) {
    args.extend([name.into(), path.as_os_str().to_owned()]);
}
impl Comparison<'_> {
    fn check(
        &mut self,
        tool: &str,
        left: &[OsString],
        right: &[OsString],
        exact: bool,
    ) -> Result<(), String> {
        let expected = Command::new(self.python)
            .arg("-B")
            .arg(self.tools.join(format!("{}.py", tool.replace('-', "_"))))
            .args(left)
            .current_dir(&self.scratch.root)
            .output()
            .map_err(|e| e.to_string())?;
        let actual = Command::new(
            self.bins
                .join(format!("{tool}{}", std::env::consts::EXE_SUFFIX)),
        )
        .args(right)
        .current_dir(&self.scratch.root)
        .output()
        .map_err(|e| e.to_string())?;
        let streams = if exact {
            expected.stdout == actual.stdout && expected.stderr == actual.stderr
        } else if expected.status.success() {
            !actual.stdout.is_empty() && actual.stderr.is_empty()
        } else {
            actual.stdout.is_empty() && !actual.stderr.is_empty()
        };
        if expected.status.code() != actual.status.code() || !streams {
            return Err(format!(
                "CLI {tool} {left:?}/{right:?}: Python {expected:?}; Rust {actual:?}"
            ));
        }
        self.count += 1;
        Ok(())
    }
    fn same(&mut self, tool: &str, args: &[OsString], exact: bool) -> Result<(), String> {
        self.check(tool, args, args, exact)
    }
    fn inputs(
        &self,
        source: &[u8],
        manifest: &Json,
        control: Option<&Json>,
    ) -> Result<Vec<OsString>, String> {
        let root = &self.scratch.root;
        fs::write(root.join("source.csv"), source).map_err(|e| e.to_string())?;
        fs::write(root.join("manifest.json"), json::dumps(manifest, false))
            .map_err(|e| e.to_string())?;
        let mut args = vec![root.join("source.csv").into_os_string()];
        option(&mut args, "--manifest", &root.join("manifest.json"));
        if let Some(value) = control {
            fs::write(root.join("control.json"), json::dumps(value, false))
                .map_err(|e| e.to_string())?;
            option(&mut args, "--control", &root.join("control.json"));
        }
        Ok(args)
    }
    fn evidence_bytes(&self, left: &Path, right: &Path, names: &[&str]) -> Result<(), String> {
        for name in names {
            if fs::read(left.join(name)).map_err(|e| e.to_string())?
                != fs::read(right.join(name)).map_err(|e| e.to_string())?
            {
                return Err(format!("Evidence bytes differ: {name}"));
            }
        }
        if fs::read_dir(left).map_err(|e| e.to_string())?.count() != names.len()
            || fs::read_dir(right).map_err(|e| e.to_string())?.count() != names.len()
        {
            return Err("Evidence file inventory differs".into());
        }
        Ok(())
    }
}
fn verifier(c: &mut Comparison<'_>) -> Result<(), String> {
    for &(name, _, _) in aged::replay::CASES {
        let (source, manifest, control) = aged::replay::case(name)?;
        let args = c.inputs(&source, &manifest, control.as_ref())?;
        c.same("xero-aged-receivables", &args, true)?;
        if fs::read(c.scratch.root.join("source.csv")).map_err(|e| e.to_string())? != source {
            return Err("Verifier changed source".into());
        }
    }
    let (source, manifest, control) = aged::replay::case("clean")?;
    let args = c.inputs(&source, &manifest, control.as_ref())?;
    for body in ["{\"x\":1,\"x\":2}", "{\"nested\":{\"x\":1,\"x\":2}}"] {
        fs::write(c.scratch.root.join("manifest.json"), body).map_err(|e| e.to_string())?;
        c.same("xero-aged-receivables", &args, true)?;
    }
    for body in ["{", "\u{feff}{}"] {
        fs::write(c.scratch.root.join("manifest.json"), body).map_err(|e| e.to_string())?;
        c.same("xero-aged-receivables", &args, false)?;
    }
    let args = c.inputs(&[0xff], &manifest, control.as_ref())?;
    c.same("xero-aged-receivables", &args, false)
}
fn evidence(c: &mut Comparison<'_>) -> Result<(), String> {
    let (source, manifest, control) = aged::replay::case("clean")?;
    let base = c.inputs(&source, &manifest, control.as_ref())?;
    for mode in ["draft", "review"] {
        let mut left = vec![mode.into(), base[0].clone()];
        if mode == "review" {
            left.extend_from_slice(&base[1..]);
        }
        let mut right = left.clone();
        let (a, b) = (
            c.scratch.root.join(format!("python-{mode}")),
            c.scratch.root.join(format!("rust-{mode}")),
        );
        option(&mut left, "--output", &a);
        option(&mut right, "--output", &b);
        c.check("debtor-evidence", &left, &right, true)?;
        c.evidence_bytes(
            &a,
            &b,
            if mode == "draft" {
                &["manifest-draft.json"]
            } else {
                &[
                    "summary-review.json",
                    "evidence-requests.json",
                    "summary-review.txt",
                ]
            },
        )?;
        c.check("debtor-evidence", &left, &right, true)?;
    }
    let mut missing = vec!["review".into(), base[0].clone()];
    option(&mut missing, "--output", &c.scratch.root.join("missing"));
    c.same("debtor-evidence", &missing, true)?;
    let mut draft = vec!["draft".into()];
    draft.extend_from_slice(&base);
    option(&mut draft, "--output", &c.scratch.root.join("invalid"));
    c.same("debtor-evidence", &draft, true)?;
    let existing = c.scratch.root.join("preserve");
    fs::write(&existing, b"operator evidence").map_err(|e| e.to_string())?;
    let mut args = vec!["draft".into(), base[0].clone()];
    option(&mut args, "--output", &existing);
    c.same("debtor-evidence", &args, true)?;
    if fs::read(existing).map_err(|e| e.to_string())? != b"operator evidence" {
        return Err("Existing evidence overwritten".into());
    }
    Ok(())
}
fn replay(c: &mut Comparison<'_>) -> Result<(), String> {
    for case in ["all", "clean"] {
        let (a, b) = (
            c.scratch.root.join(format!("python-{case}.json")),
            c.scratch.root.join(format!("rust-{case}.json")),
        );
        let mut left = os(&["--case", case]);
        let mut right = left.clone();
        option(&mut left, "--output", &a);
        option(&mut right, "--output", &b);
        c.check("replay-aged-review", &left, &right, true)?;
        if fs::read(a).map_err(|e| e.to_string())? != fs::read(b).map_err(|e| e.to_string())? {
            return Err(format!("Replay bytes differ: {case}"));
        }
        c.check("replay-aged-review", &left, &right, false)?;
    }
    Ok(())
}
fn usage(c: &mut Comparison<'_>) -> Result<(), String> {
    for tool in [
        "xero-aged-receivables",
        "debtor-evidence",
        "replay-aged-review",
    ] {
        for args in [
            vec![],
            vec!["--help"],
            vec!["-hh"],
            vec!["-hfoo"],
            vec!["-h-h"],
            vec!["--help=x"],
            vec!["--unknown", "--help"],
            vec!["--manifest"],
            vec!["--", "--help"],
        ] {
            c.same(tool, &os(&args), false)?;
        }
    }
    for tool in ["debtor-evidence", "replay-aged-review"] {
        let a = c
            .tools
            .ancestors()
            .nth(3)
            .unwrap()
            .join(format!(".aged-no-write-{}", std::process::id()));
        let b = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap()
            .join(format!(".aged-no-write-{}", std::process::id()));
        let mut left = if tool == "debtor-evidence" {
            os(&["draft", "missing.csv"])
        } else {
            Vec::new()
        };
        let mut right = left.clone();
        option(&mut left, "--output", &a);
        option(&mut right, "--output", &b);
        c.check(tool, &left, &right, false)?;
        if a.exists() || b.exists() {
            return Err("Repository refusal created output".into());
        }
    }
    Ok(())
}
fn raw_paths(c: &mut Comparison<'_>) -> Result<(), String> {
    #[cfg(unix)]
    let name = {
        use std::os::unix::ffi::OsStringExt;
        OsString::from_vec(b"path-\xff".to_vec())
    };
    #[cfg(windows)]
    let name = {
        use std::os::windows::ffi::OsStringExt;
        OsString::from_wide(&[110, 0xd800])
    };
    let source = c.scratch.root.join(&name).with_extension("csv");
    let manifest = c.scratch.root.join(name).with_extension("json");
    let (bytes, map, _) = aged::replay::case("clean")?;
    fs::write(&source, bytes).map_err(|e| e.to_string())?;
    fs::write(&manifest, json::dumps(&map, false)).map_err(|e| e.to_string())?;
    for prefix in ["--manifest=", "--man="] {
        let mut inline = OsString::from(prefix);
        inline.push(&manifest);
        c.same(
            "xero-aged-receivables",
            &[source.as_os_str().to_owned(), inline],
            true,
        )?;
    }
    Ok(())
}
pub fn run(python: &Path, tools: &Path, binaries: &Path, _seed: u64) -> Result<usize, String> {
    let mut c = Comparison {
        python,
        tools: fs::canonicalize(tools).map_err(|e| e.to_string())?,
        bins: fs::canonicalize(binaries).map_err(|e| e.to_string())?,
        scratch: aged::replay::Scratch::new()?,
        count: 0,
    };
    verifier(&mut c)?;
    evidence(&mut c)?;
    replay(&mut c)?;
    usage(&mut c)?;
    raw_paths(&mut c)?;
    Ok(c.count)
}
