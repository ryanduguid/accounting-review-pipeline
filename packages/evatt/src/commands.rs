//! CLI workflows in evatt/cli.py order, including leftover-file behaviour.
use crate::{
    cli::Parsed,
    disclosure,
    entities::Entity,
    files,
    patterns::{self, Result},
    pyjson::{self, Json},
    pypath,
    redact::{self, Redaction},
    verify,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::Write,
    path::Path,
};

fn fail(stderr: &mut dyn Write, message: &str) -> i32 {
    let _ = writeln!(stderr, "error: {message}");
    1
}

fn collision(args: &Parsed) -> Result<Option<String>> {
    if args.command == "verify" {
        return Ok(None);
    }
    let mut protected = vec![
        (files::resolve(&args.path("--map"))?, "the entity map"),
        (files::resolve(&args.path("--in"))?, "the input"),
    ];
    let out = args.path("--out");
    let mut written = vec![("--out", out.clone())];
    if args.command == "redact" {
        written.push((
            "the manifest path --out derives",
            files::suffix(&out, ".manifest.json")?,
        ));
        written.push(("the triage path --out derives", files::suffix(&out, ".triage.md")?));
    }
    for (name, path) in written {
        let resolved = files::resolve(&path)?;
        for (candidate, description) in &protected {
            let same = pypath::same_path(&resolved, candidate)
                || resolved.exists()
                    && candidate.exists()
                    && pypath::file_id(&resolved)
                        .and_then(|a| pypath::file_id(candidate).map(|b| a == b))
                        .map_err(|e| e.to_string())?;
            if same {
                return Ok(Some(format!(
                    "{name} is {description}, {}; refusing to overwrite it",
                    files::text(&resolved)
                )));
            }
        }
        protected.push((resolved, name));
    }
    Ok(None)
}

fn halt_message(result: &Redaction) -> String {
    format!(
        "{} unclassified candidate(s); nothing was written",
        result.unknowns.len()
    )
}

fn triage(result: &Redaction) -> String {
    let mut lines = vec![
        "# Triage".into(),
        "".into(),
        format!("{}.", halt_message(result)),
        "".into(),
        "Classify every candidate, then run redact again.".into(),
        "Map a false positive as an entity; it will also be replaced and restored.".into(),
        "For a number: rewrite a TFN, ABN, ACN or Medicare number in a recognised spelling".into(),
        "so pass one replaces it one-way; map any other number as an entity.".into(),
        "".into(),
    ];
    let mut unknowns: Vec<_> = result.unknowns.iter().collect();
    unknowns.sort_by_key(|u| (u.line, &u.kind, &u.value));
    let mut quoted = HashSet::new();
    for u in unknowns {
        let value = u
            .value
            .split(|c: char| crate::pytext::is_space(c as u32))
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        lines.push(format!("- **{value}** ({}, line {})", u.kind, u.line));
        if quoted.insert(u.line) {
            lines.push(format!("  > {}", u.context));
        }
    }
    lines.join("\n") + "\n"
}

fn halt(out: &Path, manifest: &Path, path: &Path, result: &Redaction, stdout: &mut dyn Write) -> Result<i32> {
    files::remove(out)?;
    files::remove(manifest)?;
    files::mkdir(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    files::ignored(path, "the triage file")?;
    files::write(path, &triage(result), "\n")?;
    let _ = writeln!(stdout, "halted: {}. See {}", halt_message(result), files::text(path));
    Ok(2)
}

fn clean(
    out: &Path,
    manifest: &Path,
    triage: &Path,
    result: &Redaction,
    ending: &str,
    stdout: &mut dyn Write,
) -> Result<i32> {
    files::write(out, &result.text, ending)?;
    let counts = Json::Object(
        result
            .counts
            .iter()
            .map(|(k, n)| (k.as_str().into(), Json::int(*n as i64)))
            .collect(),
    );
    let document = Json::object([("schema_version", Json::int(1)), ("counts", counts)]);
    let result =
        files::write(manifest, &(pyjson::pretty_ordered(&document) + "\n"), "\n").and_then(|()| files::remove(triage));
    if let Err(error) = result {
        // The first failed cleanup stops the second, as in the Python block.
        let _ = files::remove(out).and_then(|()| files::remove(manifest));
        return Err(error);
    }
    let _ = writeln!(stdout, "wrote {}", files::text(out));
    Ok(0)
}

fn redact_command(args: &Parsed, text: &str, entries: &[Entity], ending: &str, stdout: &mut dyn Write) -> Result<i32> {
    let out = args.path("--out");
    let manifest = files::suffix(&out, ".manifest.json")?;
    let triage = files::suffix(&out, ".triage.md")?;
    let result = redact::redact(text, entries)?;
    if !result.unknowns.is_empty() {
        halt(&out, &manifest, &triage, &result, stdout)
    } else {
        clean(&out, &manifest, &triage, &result, ending, stdout)
    }
}

fn ambiguous_locations(text: &str) -> Result<HashMap<String, VecDeque<usize>>> {
    let lines = redact::Lines::new(text);
    let mut found = HashMap::new();
    let mut seen = HashSet::new();
    for a in patterns::ambiguous(text)? {
        let (line, _) = lines.locate(a.digit_start);
        if seen.insert((line, a.value.clone())) {
            found.entry(a.value).or_insert_with(VecDeque::new).push_back(line);
        }
    }
    Ok(found)
}

fn verify_command(text: &str, entries: &[Entity], stdout: &mut dyn Write) -> Result<i32> {
    let found = verify::findings(text, entries)?;
    if found.is_empty() {
        let _ = writeln!(stdout, "no findings");
        return Ok(0);
    }
    let mut ambiguous = ambiguous_locations(text)?;
    for finding in &found {
        let locations = if finding.kind == "number" {
            ambiguous.get_mut(&finding.value)
        } else {
            None
        };
        let line = locations.and_then(|q| q.pop_front());
        let location = if let Some(line) = line {
            format!("line {line}")
        } else {
            let pattern = patterns::value_pattern(&finding.value)?;
            let matched = pattern.find(text).map_err(patterns::runtime)?;
            matched.map_or_else(
                || "detected in input".into(),
                |m| format!("line {}", text[..m.start()].bytes().filter(|&b| b == b'\n').count() + 1),
            )
        };
        let _ = writeln!(stdout, "{}: {location}", finding.kind);
    }
    let _ = writeln!(stdout, "{} finding(s); this file is not ready to send", found.len());
    Ok(2)
}

fn normal(args: &Parsed, stdout: &mut dyn Write) -> Result<i32> {
    if let Some(message) = collision(args)? {
        return Err(message);
    }
    files::ignored(&args.path("--map"), "the entity map")?;
    let entries = files::load(&args.path("--map"))?;
    let (text, ending) = files::read(&args.path("--in"))?;
    match args.command {
        "redact" => redact_command(args, &text, &entries, ending, stdout),
        "restore" => {
            let out = args.path("--out");
            let restored = match verify::restore(&text, &entries) {
                Ok(text) => text,
                Err(error) if error.contains("lone surrogate") => {
                    // Python opens/truncates the file before UTF-8 encoding fails.
                    files::write(&out, "", ending)?;
                    return Err(error);
                }
                Err(error) => return Err(error),
            };
            files::write(&out, &restored, ending)?;
            let _ = writeln!(stdout, "wrote {}", files::text(&out));
            Ok(0)
        }
        _ => verify_command(&text, &entries, stdout),
    }
}

fn record_write(path: &Path, record: &[u8]) -> Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    crate::persist::record(file, record).map_err(|e| e.to_string())
}

fn record(args: &Parsed, entries: &[Entity], payload: &[u8], stdout: &mut dyn Write) -> Result<()> {
    let content = disclosure::create(
        payload,
        entries,
        &args.values["--destination"],
        &args.values["--decision-ref"],
    )
    .map_err(|e| format!("disclosure:{e:?}"))?;
    let out = args.path("--out");
    let source = args.path("--in");
    let protected = [
        source.clone(),
        args.path("--map"),
        files::suffix(&source, ".manifest.json")?,
        files::suffix(&source, ".triage.md")?,
    ];
    let resolved = files::resolve(&out)?;
    for path in protected {
        if pypath::same_path(&resolved, &files::resolve(&path)?) {
            return Err("invalid:path".into());
        }
    }
    files::record_path(&out).map_err(|error| {
        if error.starts_with("disclosure record") {
            "invalid:path".to_string()
        } else {
            error
        }
    })?;
    files::mkdir(
        out.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    files::ignored(&out, "the disclosure record").map_err(|_| "invalid:path".to_string())?;
    record_write(&out, &content)?;
    let _ = writeln!(
        stdout,
        "Local disclosure evidence recorded. External authorisation has not been checked."
    );
    Ok(())
}

fn disclosure_result(error: &str, stderr: &mut dyn Write) -> i32 {
    if error.contains("Refused(\"the disclosure payload has verifier findings\")") {
        let _ = writeln!(stderr, "refused: the disclosure payload has verifier findings");
        return 2;
    }
    if error.contains("Refused(\"disclosure evidence does not match the supplied context\")") {
        let _ = writeln!(
            stderr,
            "refused: disclosure evidence does not match the supplied context"
        );
        return 2;
    }
    if error.starts_with("invalid:") || error.starts_with("disclosure:") {
        fail(
            stderr,
            "invalid disclosure input or record path; use local ignored, untracked records",
        )
    } else {
        fail(
            stderr,
            "cannot read or create disclosure files; existing files are never overwritten",
        )
    }
}

fn disclosure_command(args: &Parsed, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let entries = files::ignored(&args.path("--map"), "the entity map").and_then(|()| files::load(&args.path("--map")));
    let Ok(entries) = entries else {
        return fail(stderr, "cannot use entity map; it must be valid, ignored and untracked");
    };
    let result = (|| -> Result<()> {
        let payload = files::bytes(&args.path("--in"))?;
        if args.command == "disclosure-record" {
            return record(args, &entries, &payload, stdout);
        }
        let path = args.path("--record");
        files::ignored(&path, "the disclosure record").map_err(|_| "invalid:path".to_string())?;
        let record = files::record_read(&path).map_err(|error| {
            if error.starts_with("disclosure record") {
                "invalid:path".to_string()
            } else {
                error
            }
        })?;
        disclosure::check(
            &payload,
            &entries,
            &record,
            &args.values["--destination"],
            &args.values["--decision-ref"],
        )
        .map_err(|e| format!("disclosure:{e:?}"))?;
        let _ = writeln!(
            stdout,
            "Local disclosure evidence matches. External authorisation has not been checked."
        );
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(error) => disclosure_result(&error, stderr),
    }
}

pub fn run(args: &Parsed, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    if args.command.starts_with("disclosure-") {
        return disclosure_command(args, stdout, stderr);
    }
    match normal(args, stdout) {
        Ok(code) => code,
        Err(error) => fail(stderr, &error),
    }
}
