use crate::{
    comparison, decimal::Decimal, documents, engine, publication, pypath, pytext, report, viewer,
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

fn usage(command: &str) -> String {
    let options = match command {
        "gate" => {
            "gate [-h] --profile {bas,month_end,year_end} --pack PACK --output OUTPUT [--review-note REVIEW_NOTE] [--document DOCUMENT] [--tieout-tolerance TIEOUT_TOLERANCE]"
        }
        "view" => "view [-h] --pack-dir PACK_DIR [--document DOCUMENT]",
        "compare" => {
            "compare [-h] --previous-pack-dir PREVIOUS_PACK_DIR --current-pack-dir CURRENT_PACK_DIR"
        }
        "intake" => {
            "intake [-h] --synthetic --source SOURCE --text TEXT --text-origin {manual,pdf-inspector} [--text-version TEXT_VERSION] --entity ENTITY --period-end PERIOD_END --output OUTPUT"
        }
        "view-document" => "view-document [-h] --bundle BUNDLE",
        _ => "[-h] {gate,view,compare,intake,view-document} ...",
    };
    format!("usage: review-ready {options}\n")
}
fn help(command: &str) -> String {
    let description = match command {
        "gate" => "Run completeness, tie-out, and self-review controls.",
        "view" => "Display an existing readiness pack after verifying its three files agree.",
        "compare" => "Verify two runs of the same pack and print what moved between them as JSON.",
        "intake" => {
            "Prepare a fabricated labelled-invoice text bundle. --synthetic confirms fabricated input. No PDF extraction runs."
        }
        "view-document" => "Verify and display a document bundle.",
        _ => {
            "Gate a workpaper pack before it reaches manager review. Incomplete, untied, or unbalanced packs are not review-ready."
        }
    };
    let mut text = usage(command)
        + "\n"
        + description
        + "\n\noptions:\n  -h, --help  show this help message and exit\n";
    if command.is_empty() {
        text.push_str("\ncommands:\n  gate           run completeness, tie-out, and self-review controls\n  view           verify and display a readiness pack\n  compare        verify and compare two readiness runs\n  intake         prepare synthetic document observations\n  view-document  verify and display a document bundle\n");
    }
    text
}
fn usage_error(command: &str, message: &str) -> i32 {
    crate::err!(
        "{}review-ready{}: error: {message}\n",
        usage(command),
        if command.is_empty() {
            String::new()
        } else {
            format!(" {command}")
        }
    );
    1
}
fn options(command: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match command {
        "gate" => (
            &[
                "--profile",
                "--pack",
                "--output",
                "--review-note",
                "--document",
                "--tieout-tolerance",
            ],
            &["--profile", "--pack", "--output"],
        ),
        "view" => (&["--pack-dir", "--document"], &["--pack-dir"]),
        "compare" => (
            &["--previous-pack-dir", "--current-pack-dir"],
            &["--previous-pack-dir", "--current-pack-dir"],
        ),
        "intake" => (
            &[
                "--synthetic",
                "--source",
                "--text",
                "--text-origin",
                "--text-version",
                "--entity",
                "--period-end",
                "--output",
            ],
            &[
                "--synthetic",
                "--source",
                "--text",
                "--text-origin",
                "--entity",
                "--period-end",
                "--output",
            ],
        ),
        "view-document" => (&["--bundle"], &["--bundle"]),
        _ => (&[], &[]),
    }
}
struct Arguments {
    values: BTreeMap<String, String>,
    documents: Vec<PathBuf>,
}
impl Arguments {
    fn text(&self, key: &str) -> &str {
        self.values.get(key).map(String::as_str).unwrap_or("")
    }
    fn path(&self, key: &str) -> PathBuf {
        PathBuf::from(pypath::encode_os(&pypath::display(self.text(key))))
    }
    fn unicode(&self, key: &str) -> crate::Result<String> {
        crate::pyjson::PyStr::from_os_text(self.text(key))
            .to_utf8()
            .ok_or_else(|| {
                format!(
                    "{} contains a control or formatting character.",
                    key.trim_start_matches("--").replace('-', "_")
                )
            })
    }
}
fn negative_number(text: &str) -> bool {
    let Some(text) = text.strip_prefix('-') else {
        return false;
    };
    let mut parts = text.split('.');
    let before = parts.next().unwrap();
    let after = parts.next();
    let digits = |s: &str| s.chars().all(|c| pytext::decimal_value(c as u32).is_some());
    parts.next().is_none()
        && digits(before)
        && after.map_or(!before.is_empty(), |a| !a.is_empty() && digits(a))
}
fn parse(command: &str, args: &[String]) -> std::result::Result<Option<Arguments>, String> {
    let (flags, required) = options(command);
    let mut result = Arguments {
        values: BTreeMap::new(),
        documents: Vec::new(),
    };
    let mut unknown = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let raw = &args[i];
        i += 1;
        if raw == "--" {
            unknown.push(raw.clone());
            unknown.extend(args[i..].iter().cloned());
            break;
        }
        if ["-h", "--help"].contains(&raw.as_str()) {
            return Ok(None);
        }
        let (flag, assigned) = raw
            .split_once('=')
            .map_or((raw.as_str(), None), |(flag, value)| (flag, Some(value)));
        let flag = if flags.contains(&flag) {
            flag
        } else {
            let matches: Vec<_> = if flag.starts_with("--") {
                flags
                    .iter()
                    .copied()
                    .chain(std::iter::once("--help"))
                    .filter(|f| f.starts_with(flag))
                    .collect()
            } else {
                Vec::new()
            };
            if matches.len() > 1 {
                return Err(format!(
                    "ambiguous option: {flag} could match {}",
                    matches.join(", ")
                ));
            }
            if let Some(flag) = matches.first() {
                *flag
            } else {
                unknown.push(raw.clone());
                continue;
            }
        };
        if flag == "--help" {
            if let Some(value) = assigned {
                return Err(format!(
                    "argument -h/--help: ignored explicit argument {}",
                    pytext::repr_os(value)
                ));
            }
            return Ok(None);
        }
        if flag == "--synthetic" {
            if let Some(value) = assigned {
                return Err(format!(
                    "argument --synthetic: ignored explicit argument {}",
                    pytext::repr_os(value)
                ));
            }
            result.values.insert(flag.into(), "true".into());
            continue;
        }
        let value = if let Some(value) = assigned {
            value.to_owned()
        } else {
            let Some(value) = args.get(i) else {
                return Err(format!("argument {flag}: expected one argument"));
            };
            if value.starts_with('-') && value != "-" && !negative_number(value) {
                return Err(format!("argument {flag}: expected one argument"));
            }
            i += 1;
            value.clone()
        };
        if flag == "--profile" && !crate::profiles::PROFILE_NAMES.contains(&value.as_str()) {
            return Err(format!(
                "argument --profile: invalid choice: {} (choose from 'bas', 'month_end', 'year_end')",
                pytext::repr_os(&value)
            ));
        }
        if flag == "--text-origin" && !["manual", "pdf-inspector"].contains(&value.as_str()) {
            return Err(format!(
                "argument --text-origin: invalid choice: {} (choose from 'manual', 'pdf-inspector')",
                pytext::repr_os(&value)
            ));
        }
        if flag == "--tieout-tolerance" {
            parse_tolerance(&value).map_err(|e| format!("argument --tieout-tolerance: {e}"))?;
        }
        if flag == "--document" {
            result
                .documents
                .push(PathBuf::from(pypath::encode_os(&pypath::display(&value))));
        } else {
            result.values.insert(flag.into(), value);
        }
    }
    let missing: Vec<_> = required
        .iter()
        .filter(|key| !result.values.contains_key(**key))
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "the following arguments are required: {}",
            missing.join(", ")
        ));
    }
    if !unknown.is_empty() {
        return Err(format!("unrecognized arguments: {}", unknown.join(" ")));
    }
    Ok(Some(result))
}
fn parse_tolerance(value: &str) -> std::result::Result<Decimal, String> {
    let result =
        Decimal::parse(value).map_err(|_| format!("not a decimal: {}", pytext::repr_os(value)))?;
    if !result.is_finite() || result < Decimal::zero() {
        return Err("must be a finite non-negative decimal".into());
    }
    Ok(result)
}
fn source_safe(
    flag: &str,
    source: &Path,
    output: &Path,
    destinations: &[PathBuf],
) -> std::result::Result<(), String> {
    let resolved =
        pypath::resolve(source, false).map_err(|_| "cannot resolve input path".to_owned())?;
    let control = pypath::resolve(&output.join(publication::CONTROL_NAME), false)
        .map_err(|_| "cannot resolve publication control".to_owned())?;
    if pypath::is_within(&resolved, &control) {
        return Err("review-ready: output error: input overlaps publication control".into());
    }
    if destinations.iter().any(|p| pypath::same_path(p, &resolved)) {
        return Err(format!(
            "review-ready: output error: {flag} {} is inside --output and shares a generated pack file name; the run would destroy it.",
            pypath::text(source)
        ));
    }
    if source.is_dir() {
        let children = fs::read_dir(source)
            .and_then(|dir| dir.collect::<std::io::Result<Vec<_>>>())
            .map_err(|e| format!("review-ready: input error: cannot inspect {flag}: {e}"))?;
        for child in children {
            let path = pypath::resolve(&child.path(), false)
                .map_err(|_| "cannot resolve input child".to_owned())?;
            if pypath::same_path(&path, &control)
                || destinations.iter().any(|p| pypath::same_path(p, &path))
            {
                return Err(format!(
                    "review-ready: output error: {flag} contains {}, which is a generated pack file name inside --output.",
                    pypath::decode_os(&child.file_name())
                ));
            }
        }
    }
    Ok(())
}
fn gate(args: &Arguments) -> i32 {
    let output = args.path("--output");
    if let Err(e) = report::require_output_outside_repository(&output) {
        crate::errln!("review-ready: output error: {e}");
        return 1;
    }
    let destinations = report::PACK_NAMES
        .iter()
        .map(|name| pypath::resolve(&output.join(name), false))
        .collect::<std::result::Result<Vec<_>, _>>();
    let Ok(destinations) = destinations else {
        crate::errln!("review-ready: output error: cannot resolve output destinations");
        return 1;
    };
    let pack_dir = args.path("--pack");
    let note = args
        .values
        .get("--review-note")
        .map(|_| args.path("--review-note"));
    for (flag, path) in std::iter::once(("--pack", &pack_dir))
        .chain(note.iter().map(|p| ("--review-note", p)))
        .chain(args.documents.iter().map(|p| ("--document", p)))
    {
        if let Err(e) = source_safe(flag, path, &output, &destinations) {
            crate::errln!("{e}");
            return 1;
        }
    }
    let tolerance = args.values.get("--tieout-tolerance").map_or_else(
        || Decimal::parse("0.01").unwrap(),
        |text| parse_tolerance(text).unwrap(),
    );
    let pack = match engine::review_pack(
        args.text("--profile"),
        &pack_dir,
        note.as_deref(),
        tolerance,
        &args.documents,
    ) {
        Ok(pack) => pack,
        Err(e) => {
            crate::errln!("review-ready: input error: {e}");
            return 1;
        }
    };
    let outputs = {
        let _managed = publication::ManagedPublication::new();
        report::write_review_pack(&pack, &output)
    };
    let outputs = match outputs {
        Ok(paths) => paths,
        Err(e) => {
            crate::errln!("review-ready: output error: {e}");
            return 1;
        }
    };
    crate::outln!(
        "review-ready: {}; {} finding(s)",
        pack.status,
        pack.findings.len()
    );
    if let Some(controls) = &pack.controls_not_run
        && !controls.is_empty()
    {
        crate::outln!(
            "review-ready: {} control(s) did not run: {}",
            controls.len(),
            controls
                .iter()
                .map(|c| c.slot.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if pack.status == "READY" && pack.findings.is_empty() {
        crate::outln!("review-ready: pack may enter manager review. A human still decides.");
    }
    for (name, path) in ["json", "summary", "findings"].iter().zip(outputs) {
        crate::outln!("  {name}: {}", pypath::text(&path));
    }
    if pack.status == "READY" { 0 } else { 2 }
}
pub fn main(args: Vec<OsString>) -> i32 {
    let args: Vec<_> = args.iter().map(|arg| pypath::decode_os(arg)).collect();
    if args == ["--version"] {
        crate::out!("review-ready {}\n", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    let mut command_index = 0;
    let mut unknown = Vec::new();
    while let Some(arg) = args.get(command_index).filter(|s| s.starts_with('-')) {
        if arg == "--" {
            command_index += 1;
            break;
        }
        let (flag, value) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(f, v)| (f, Some(v)));
        if flag == "-h" || (flag.len() > 2 && flag.starts_with("--") && "--help".starts_with(flag))
        {
            if let Some(value) = value {
                return usage_error(
                    "",
                    &format!(
                        "argument -h/--help: ignored explicit argument {}",
                        pytext::repr_os(value)
                    ),
                );
            }
            crate::out!("{}", help(""));
            return 0;
        }
        unknown.push(arg.as_str());
        command_index += 1;
    }
    let Some(command) = args.get(command_index) else {
        return usage_error("", "the following arguments are required: command");
    };
    if !["gate", "view", "compare", "intake", "view-document"].contains(&command.as_str()) {
        return usage_error(
            "",
            &format!(
                "argument command: invalid choice: {} (choose from 'gate', 'view', 'compare', 'intake', 'view-document')",
                pytext::repr_os(command)
            ),
        );
    }
    let parsed = match parse(command, &args[command_index + 1..]) {
        Ok(Some(args)) => args,
        Ok(None) => {
            crate::out!("{}", help(command));
            return 0;
        }
        Err(e) => {
            return usage_error(
                if e.starts_with("unrecognized arguments:") {
                    ""
                } else {
                    command
                },
                &e,
            );
        }
    };
    if !unknown.is_empty() {
        return usage_error(
            "",
            &format!("unrecognized arguments: {}", unknown.join(" ")),
        );
    }
    let args = parsed;
    match command.as_str() {
        "gate" => gate(&args),
        "view" => match viewer::render_review_sheet(&args.path("--pack-dir"), &args.documents) {
            Ok(sheet) => {
                crate::stdio::out_literal(&format!("{sheet}\n"));
                crate::outln!(
                    "review-ready view: display is a review aid; it does not approve a file or change any computed status."
                );
                0
            }
            Err(e) => {
                crate::errln!("review-ready view: verification failed: {e}");
                1
            }
        },
        "compare" => match comparison::compare_packs(
            &args.path("--previous-pack-dir"),
            &args.path("--current-pack-dir"),
        ) {
            Ok(value) => {
                crate::outln!("{}", crate::pyjson::pretty(&value));
                0
            }
            Err(e) => {
                crate::errln!("review-ready compare: verification failed: {e}");
                1
            }
        },
        "view-document" => match documents::load_document(&args.path("--bundle")) {
            Ok(bundle) => {
                crate::stdio::out_literal(&(documents::render_document(&bundle) + "\n"));
                0
            }
            Err(e) => {
                crate::errln!("review-ready view-document: verification failed: {e}");
                1
            }
        },
        "intake" => {
            let source = args.path("--source");
            let text = args.path("--text");
            let output = args.path("--output");
            let (entity, version) = match args.unicode("--entity").and_then(|entity| {
                args.unicode("--text-version")
                    .map(|version| (entity, version))
            }) {
                Ok(values) => values,
                Err(error) => {
                    crate::errln!("review-ready intake: {error}");
                    return 1;
                }
            };
            let options = documents::IntakeOptions {
                source: &source,
                text: &text,
                output: &output,
                entity: &entity,
                period: args.text("--period-end"),
                origin: args.text("--text-origin"),
                version: &version,
            };
            match documents::create_intake(options) {
                Ok(output) => {
                    crate::outln!(
                        "review-ready intake: proposed fields written to {}; human review required.",
                        pypath::text(&output)
                    );
                    0
                }
                Err(e) => {
                    crate::errln!("review-ready intake: {e}");
                    1
                }
            }
        }
        _ => unreachable!("command checked"),
    }
}

/// The bounded JSON decoder needs a consistent stack on both operating systems.
pub fn command_thread(action: impl FnOnce() -> i32 + Send + 'static) -> i32 {
    match std::thread::Builder::new()
        .name("readiness-command".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let code = action();
            if crate::stdio::failed() {
                crate::errln!("review-ready: cannot write command output.");
                1
            } else {
                code
            }
        }) {
        Ok(thread) => thread.join().unwrap_or_else(|_| {
            crate::errln!("review-ready: command failed unexpectedly");
            1
        }),
        Err(error) => {
            crate::errln!("review-ready: cannot start command ({error})");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{self, TempDir};

    #[test]
    fn help_and_usage_errors_leave_no_outputs() {
        let (code, out, err) = crate::stdio::capture(|| main(vec!["--help".into()]));
        assert_eq!(code, 0);
        assert!(out.contains("manager review"));
        assert!(err.is_empty());
        for args in [
            vec![],
            vec!["bad".into()],
            vec!["gate".into()],
            vec!["intake".into(), "--synthetic=false".into()],
        ] {
            let (code, _, err) = crate::stdio::capture(|| main(args));
            assert_eq!(code, 1);
            assert!(err.contains("error:"));
        }
    }
    #[test]
    fn cli_returns_all_readiness_codes_and_verifies_each_pack() {
        for (example, status, expected) in [
            ("bas-ready", "READY", 0),
            ("bas-not-ready", "NOT_READY", 2),
            ("bas-blocked", "BLOCKED", 2),
        ] {
            let tmp = TempDir::new();
            let out = tmp.join("out");
            let argv = vec![
                "gate".into(),
                "--profile".into(),
                "bas".into(),
                "--pack".into(),
                test_support::examples().join(example).into_os_string(),
                "--output".into(),
                out.clone().into_os_string(),
            ];
            let (code, stdout, stderr) = crate::stdio::capture(|| main(argv));
            assert_eq!(code, expected);
            assert!(stderr.is_empty(), "{stderr}");
            assert!(stdout.contains(&format!("review-ready: {status};")));
            let (code, stdout, stderr) = crate::stdio::capture(|| {
                main(vec![
                    "view".into(),
                    "--pack-dir".into(),
                    out.into_os_string(),
                ])
            });
            assert_eq!(code, 0);
            assert!(stdout.contains("Artefact digests"));
            assert!(stderr.is_empty());
        }
    }
    #[test]
    fn tolerance_and_input_collisions_fail_before_publication() {
        let tmp = TempDir::new();
        let output = tmp.join("out");
        for value in ["abc", "-0.01", "NaN", "Infinity"] {
            let args = vec![
                "gate".into(),
                "--profile=bas".into(),
                "--pack".into(),
                test_support::examples().join("bas-ready").into_os_string(),
                "--output".into(),
                output.clone().into_os_string(),
                format!("--tieout-tolerance={value}").into(),
            ];
            let (code, _, err) = crate::stdio::capture(|| main(args));
            assert_eq!(code, 1);
            assert!(err.contains("argument --tieout-tolerance"));
            assert!(!output.exists());
        }
        report::write_review_pack(&test_support::ready_pack(), &output).unwrap();
        let before = report::PACK_NAMES.map(|n| fs::read(output.join(n)).unwrap());
        let args = vec![
            "gate".into(),
            "--profile=bas".into(),
            "--pack".into(),
            output.clone().into_os_string(),
            "--output".into(),
            output.clone().into_os_string(),
        ];
        let (code, _, err) = crate::stdio::capture(|| main(args));
        assert_eq!(code, 1);
        assert!(err.contains("--pack contains "));
        assert_eq!(
            before,
            report::PACK_NAMES.map(|n| fs::read(output.join(n)).unwrap())
        );
    }
}
