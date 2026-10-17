use super::{Json, evidence, inspect_export, json, path, replay};
use crate::transactions::{
    cli::{inline_value, negative},
    text::terminal,
};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Verifier,
    Evidence,
    Replay,
}
struct Arguments {
    positional: Vec<OsString>,
    values: BTreeMap<&'static str, OsString>,
}
fn usage(tool: Tool) -> &'static str {
    match tool {
        Tool::Verifier => "usage: xero-aged-receivables SOURCE --manifest FILE [--control FILE]\n",
        Tool::Evidence => {
            "usage: debtor-evidence {draft,review} SOURCE [--manifest FILE] [--control FILE] --output DIRECTORY\n"
        }
        Tool::Replay => "usage: replay-aged-review [--case CASE] --output FILE\n",
    }
}
fn options(tool: Tool) -> &'static [&'static str] {
    match tool {
        Tool::Verifier => &["manifest", "control", "help"],
        Tool::Evidence => &["manifest", "control", "output", "help"],
        Tool::Replay => &["case", "output", "help"],
    }
}
fn name<'a>(options: &'a [&'static str], token: &str) -> Result<Option<&'a &'static str>, String> {
    if let Some(exact) = options.iter().find(|&&s| s == token) {
        return Ok(Some(exact));
    }
    let mut matches = options.iter().filter(|s| s.starts_with(token));
    let first = matches.next();
    if matches.next().is_some() {
        return Err(format!("ambiguous option: --{token}"));
    }
    Ok(first)
}
fn parse(tool: Tool, args: &[OsString]) -> Result<Arguments, (u8, String)> {
    let options = options(tool);
    let bad = |s: String| (2, format!("{}error: {s}\n", usage(tool)));
    for argument in args.iter().take_while(|s| s.as_os_str() != "--") {
        if let Some(option) = argument.to_string_lossy().strip_prefix("--") {
            name(options, option.split('=').next().unwrap()).map_err(bad)?;
        }
    }
    let (mut values, mut positional, mut extra) = (BTreeMap::new(), Vec::new(), Vec::new());
    let (mut literal, mut index) = (false, 0);
    while index < args.len() {
        let raw = &args[index];
        let token = raw.to_string_lossy();
        index += 1;
        if !literal && token == "--" {
            literal = true;
            continue;
        }
        if !literal && let Some(tail) = token.strip_prefix("-h") {
            if tail.trim_start_matches('h').starts_with(['-', '=']) {
                return Err(bad("-h does not take a value".into()));
            }
            return Err((0, usage(tool).into()));
        }
        if !literal && let Some(option) = token.strip_prefix("--") {
            let (key, inline) = option
                .split_once('=')
                .map_or((option, false), |(key, _)| (key, true));
            let Some(&key) = name(options, key).map_err(bad)? else {
                extra.push(token.into_owned());
                continue;
            };
            if key == "help" {
                return if inline {
                    Err(bad("--help does not take a value".into()))
                } else {
                    Err((0, usage(tool).into()))
                };
            }
            let value = read_value(tool, args, &mut index, key, inline)?;
            values.insert(key, value);
        } else if !literal && token.starts_with('-') && token != "-" && !negative(&token) {
            extra.push(token.into_owned());
        } else {
            positional.push(raw.clone());
        }
    }
    finish(tool, positional, values, extra)
}

fn read_value(
    tool: Tool,
    args: &[OsString],
    index: &mut usize,
    key: &str,
    inline: bool,
) -> Result<OsString, (u8, String)> {
    if inline {
        return Ok(inline_value(&args[*index - 1]));
    }
    let missing = || {
        (
            2,
            format!("{}error: --{key} requires a value\n", usage(tool)),
        )
    };
    let value = args.get(*index).ok_or_else(missing)?;
    let text = value.to_string_lossy();
    if text.starts_with('-') && text != "-" && !negative(&text) {
        return Err(missing());
    }
    *index += 1;
    Ok(value.clone())
}

fn finish(
    tool: Tool,
    positional: Vec<OsString>,
    values: BTreeMap<&'static str, OsString>,
    extra: Vec<String>,
) -> Result<Arguments, (u8, String)> {
    let bad = |s: String| (2, format!("{}error: {s}\n", usage(tool)));
    let count = match tool {
        Tool::Verifier => 1,
        Tool::Evidence => 2,
        Tool::Replay => 0,
    };
    if !extra.is_empty() || positional.len() != count {
        return Err(bad("invalid positional arguments".into()));
    }
    let required = if tool == Tool::Verifier {
        "manifest"
    } else {
        "output"
    };
    if !values.contains_key(required) {
        return Err(bad(format!("--{required} is required")));
    }
    if tool == Tool::Evidence && !matches!(positional[0].to_str(), Some("draft" | "review")) {
        return Err(bad("mode must be draft or review".into()));
    }
    if tool == Tool::Replay
        && let Some(case) = values.get("case")
    {
        let case = case
            .to_str()
            .ok_or_else(|| bad("case requires Unicode text".into()))?;
        if case != "all" && !replay::CASES.iter().any(|(s, _, _)| *s == case) {
            return Err(bad("unknown replay case".into()));
        }
    }
    Ok(Arguments { positional, values })
}

fn load(path: &OsString) -> Result<Json, String> {
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    json::loads(&text).map_err(|error| match error {
        json::LoadError::DuplicateKey(_) => "Duplicate JSON field.".into(),
        json::LoadError::Crash(text) => text,
        _ => "Malformed JSON evidence.".into(),
    })
}
fn output(path: &OsString) -> Result<PathBuf, String> {
    let resolved = path::resolve(Path::new(path), false)
        .map_err(|e| format!("Cannot resolve output: {e:?}"))?;
    let checkout = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let checkout =
        path::resolve(checkout, false).map_err(|e| format!("Cannot resolve checkout: {e:?}"))?;
    if path::is_within(&resolved, &checkout) {
        return Err("Evidence output must be outside the checkout.".into());
    }
    if resolved.exists() {
        return Err("Output already exists; preserve the previous evidence.".into());
    }
    Ok(resolved)
}
fn write_new(path: &Path, text: &str) -> Result<(), String> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    file.write_all(terminal(text).as_bytes())
        .map_err(|e| e.to_string())
}
fn review(arguments: &Arguments, source: &[u8]) -> Result<Json, String> {
    let manifest = load(
        arguments
            .values
            .get("manifest")
            .ok_or("Review mode requires a manifest.")?,
    )?;
    let control = arguments.values.get("control").map(load).transpose()?;
    inspect_export(source, &manifest, control.as_ref())
}
fn verifier(arguments: &Arguments) -> Result<(u8, String), String> {
    let manifest = load(&arguments.values["manifest"])?;
    let control = arguments.values.get("control").map(load).transpose()?;
    let source = fs::read(&arguments.positional[0]).map_err(|e| e.to_string())?;
    let result = inspect_export(&source, &manifest, control.as_ref())?;
    Ok((
        if result.at("summary_status").is_str("PASS") {
            0
        } else {
            2
        },
        json::pretty_utf8(&result)? + "\n",
    ))
}
fn evidence(arguments: &Arguments) -> Result<(u8, String), String> {
    let output = output(&arguments.values["output"])?;
    let source = fs::read(&arguments.positional[1]).map_err(|e| e.to_string())?;
    let draft = arguments.positional[0] == "draft";
    let (code, files, line) = if draft {
        if arguments.values.contains_key("manifest") || arguments.values.contains_key("control") {
            return Err("Draft mode extracts source declarations only.".into());
        }
        let result = evidence::draft(&source)?;
        (
            2,
            vec![("manifest-draft.json", json::pretty_ordered(&result) + "\n")],
            "Source-only draft: REVIEW.\n".into(),
        )
    } else {
        let result = review(arguments, &source)?;
        let status = json::str_value(result.at("summary_status"));
        let files = vec![
            ("summary-review.json", json::pretty_ordered(&result) + "\n"),
            (
                "evidence-requests.json",
                json::pretty_ordered(&evidence::requests(&result)) + "\n",
            ),
            ("summary-review.txt", evidence::briefing(&result)),
        ];
        (
            if status == "PASS" { 0 } else { 2 },
            files,
            format!("Summary: {status}; debtor decisions: REVIEW.\n"),
        )
    };
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::create_dir(&output).map_err(|e| e.to_string())?;
    for (name, text) in files {
        write_new(&output.join(name), &text)?;
    }
    Ok((code, line))
}
fn replay(arguments: &Arguments) -> Result<(u8, String), String> {
    let output = output(&arguments.values["output"])?;
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let verifier = executable.parent().unwrap().join(if cfg!(windows) {
        "xero-aged-receivables.exe"
    } else {
        "xero-aged-receivables"
    });
    let case = arguments
        .values
        .get("case")
        .and_then(|s| s.to_str())
        .unwrap_or("all");
    let pack = replay::pack(case, &verifier)?;
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    write_new(&output, &(json::pretty_ordered(&pack) + "\n"))?;
    Ok((0, "FAILURE REPLAY QUALIFIED\n".into()))
}
pub fn run(tool: Tool, args: &[OsString]) -> (u8, String, String) {
    let arguments = match parse(tool, args) {
        Ok(value) => value,
        Err((0, text)) => return (0, terminal(&text), String::new()),
        Err((code, text)) => return (code, String::new(), terminal(&text)),
    };
    let result = match tool {
        Tool::Verifier => verifier(&arguments),
        Tool::Evidence => evidence(&arguments),
        Tool::Replay => replay(&arguments),
    };
    match result {
        Ok((code, text)) => (code, terminal(&text), String::new()),
        Err(error)
            if tool == Tool::Replay
                && (error.contains("outside the checkout")
                    || error.contains("Output already exists")) =>
        {
            (
                2,
                String::new(),
                terminal(&format!("{}error: {error}\n", usage(tool))),
            )
        }
        Err(error) => (
            1,
            String::new(),
            terminal(
                &(json::dumps(
                    &Json::object([
                        ("summary_status", Json::str("INVALID_INPUT")),
                        ("error", Json::str(&error)),
                    ]),
                    false,
                ) + "\n"),
            ),
        ),
    }
}
