//! The command line (`cli.py`), including the argparse behaviour it relied on:
//! usage and help text, option abbreviations, and exit code 2 for usage errors.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::gateway;
use crate::persist;
use crate::pyjson::Json;
use crate::pytext;
use crate::util::{Env, Failure, Location};

struct Opt {
    flag: &'static str,
    dest: &'static str,
    required: bool,
    help: Option<&'static str>,
}

struct Command {
    name: &'static str,
    help: &'static str,
    options: &'static [Opt],
}

const fn opt(flag: &'static str, dest: &'static str, required: bool, help: Option<&'static str>) -> Opt {
    Opt {
        flag,
        dest,
        required,
        help,
    }
}

const COMMANDS: [Command; 2] = [
    Command {
        name: "evaluate",
        help: "evaluate the one allowlisted variance review",
        options: &[
            opt("--context", "CONTEXT", true, None),
            opt("--request", "REQUEST", true, None),
            opt("--policy", "POLICY", true, None),
            opt("--out", "OUT", true, None),
        ],
    },
    Command {
        name: "validate-review",
        help: "validate a human acknowledgement/escalation record",
        options: &[
            opt("--evidence", "EVIDENCE", true, None),
            opt("--receipt", "RECEIPT", true, None),
            opt(
                "--decision",
                "DECISION",
                true,
                Some("decision JSON under the bundled samples/ data or the working directory's build/"),
            ),
            opt(
                "--out",
                "OUT",
                false,
                Some("optional JSON validation output below build/"),
            ),
        ],
    },
];

const DESCRIPTION: &str = "Run a fixed-policy, synthetic Xero review boundary.";
const HELP_HELP: &str = "show this help message and exit";

/// argparse's formatter width: `$COLUMNS` (or 80 when unset) minus 2.
fn width() -> usize {
    let columns = std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.parse::<usize>().ok())
        .filter(|&c| c > 0)
        .unwrap_or(80);
    columns.saturating_sub(2)
}

/// `textwrap.wrap(text, width)` for single-spaced text.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// `HelpFormatter._format_usage` for a parser whose positionals are given as parts.
fn usage(prog: &str, opt_parts: &[String], pos_parts: &[String]) -> String {
    let prefix = "usage: ";
    let text_width = width();
    let mut all = vec![prog.to_string()];
    all.extend(opt_parts.iter().cloned());
    all.extend(pos_parts.iter().cloned());
    let flat = all.join(" ");
    if prefix.len() + flat.chars().count() <= text_width {
        return format!("{prefix}{flat}");
    }
    let get_lines = |parts: &[String], indent: &str, with_prefix: bool| -> Vec<String> {
        let mut lines = Vec::new();
        let mut line: Vec<&str> = Vec::new();
        let indent_length = indent.len();
        let mut line_len = if with_prefix {
            prefix.len() - 1
        } else {
            indent_length - 1
        };
        for part in parts {
            if line_len + 1 + part.chars().count() > text_width && !line.is_empty() {
                lines.push(format!("{indent}{}", line.join(" ")));
                line.clear();
                line_len = indent_length - 1;
            }
            line.push(part);
            line_len += part.chars().count() + 1;
        }
        if !line.is_empty() {
            lines.push(format!("{indent}{}", line.join(" ")));
        }
        if with_prefix && !lines.is_empty() {
            lines[0] = lines[0][indent_length..].to_string();
        }
        lines
    };
    let lines = if (prefix.len() + prog.chars().count()) as f64 <= 0.75 * text_width as f64 {
        let indent = " ".repeat(prefix.len() + prog.chars().count() + 1);
        let mut head = vec![prog.to_string()];
        if !opt_parts.is_empty() {
            head.extend(opt_parts.iter().cloned());
            let mut lines = get_lines(&head, &indent, true);
            lines.extend(get_lines(pos_parts, &indent, false));
            lines
        } else {
            head.extend(pos_parts.iter().cloned());
            get_lines(&head, &indent, true)
        }
    } else {
        let indent = " ".repeat(prefix.len());
        let mut parts = opt_parts.to_vec();
        parts.extend(pos_parts.iter().cloned());
        let mut lines = get_lines(&parts, &indent, false);
        if lines.len() > 1 {
            lines = get_lines(opt_parts, &indent, false);
            lines.extend(get_lines(pos_parts, &indent, false));
        }
        let mut with_prog = vec![prog.to_string()];
        with_prog.extend(lines);
        with_prog
    };
    format!("{prefix}{}", lines.join("\n"))
}

type Row<'a> = (usize, String, Option<&'a str>);

/// The rows of one help section; argparse aligns help text across every section of a parser.
fn action_rows(rows: &[Row], all: &[Row]) -> String {
    let width = width();
    let max_help_position = 24.min(width.saturating_sub(20).max(4));
    let action_max = all
        .iter()
        .map(|(indent, header, _)| indent + header.chars().count())
        .max()
        .unwrap_or(0);
    let help_position = (action_max + 2).min(max_help_position);
    let help_width = width.saturating_sub(help_position).max(11);
    let mut out = String::new();
    for (indent, header, help) in rows {
        let action_width = help_position.saturating_sub(indent + 2);
        let pad = " ".repeat(*indent);
        match help {
            None => out.push_str(&format!("{pad}{header}\n")),
            Some(help) => {
                let lines = wrap(help, help_width);
                if header.chars().count() <= action_width {
                    out.push_str(&format!("{pad}{header:<action_width$}  {}\n", lines[0]));
                } else {
                    out.push_str(&format!("{pad}{header}\n{}{}\n", " ".repeat(help_position), lines[0]));
                }
                for line in &lines[1..] {
                    out.push_str(&format!("{}{line}\n", " ".repeat(help_position)));
                }
            }
        }
    }
    out
}

fn choices() -> String {
    format!("{{{}}}", COMMANDS.iter().map(|c| c.name).collect::<Vec<_>>().join(","))
}

fn main_usage(prog: &str) -> String {
    usage(prog, &["[-h]".into()], &[choices(), "...".into()])
}

fn main_help(prog: &str) -> String {
    let mut positional: Vec<Row> = vec![(2, choices(), None)];
    positional.extend(COMMANDS.iter().map(|c| (4, c.name.to_string(), Some(c.help))));
    let options: Vec<Row> = vec![(2, "-h, --help".into(), Some(HELP_HELP))];
    let all: Vec<Row> = positional.iter().chain(&options).cloned().collect();
    let (positional, options) = (action_rows(&positional, &all), action_rows(&options, &all));
    let description = wrap(DESCRIPTION, width()).join("\n");
    format!(
        "{}\n\n{description}\n\npositional arguments:\n{positional}\noptions:\n{options}",
        main_usage(prog)
    )
}

fn command_usage(prog: &str, command: &Command) -> String {
    let mut parts = vec!["[-h]".to_string()];
    for option in command.options {
        let part = format!("{} {}", option.flag, option.dest);
        parts.push(if option.required { part } else { format!("[{part}]") });
    }
    usage(&format!("{prog} {}", command.name), &parts, &[])
}

fn command_help(prog: &str, command: &Command) -> String {
    let mut rows = vec![(2, "-h, --help".to_string(), Some(HELP_HELP))];
    rows.extend(
        command
            .options
            .iter()
            .map(|o| (2, format!("{} {}", o.flag, o.dest), o.help)),
    );
    format!(
        "{}\n\noptions:\n{}",
        command_usage(prog, command),
        action_rows(&rows, &rows)
    )
}

/// The ratio difflib.SequenceMatcher(None, a, b).ratio() computes.
fn sequence_ratio(a: &[char], b: &[char]) -> f64 {
    let mut b2j: BTreeMap<char, Vec<usize>> = BTreeMap::new();
    for (j, c) in b.iter().enumerate() {
        b2j.entry(*c).or_default().push(j);
    }
    if b.len() >= 200 {
        let ntest = b.len() / 100 + 1;
        b2j.retain(|_, indices| indices.len() <= ntest);
    }
    fn longest(
        a: &[char],
        b: &[char],
        b2j: &BTreeMap<char, Vec<usize>>,
        alo: usize,
        ahi: usize,
        blo: usize,
        bhi: usize,
    ) -> (usize, usize, usize) {
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0);
        let mut j2len: BTreeMap<usize, usize> = BTreeMap::new();
        for (i, ch) in a.iter().enumerate().take(ahi).skip(alo) {
            let mut new: BTreeMap<usize, usize> = BTreeMap::new();
            for &j in b2j.get(ch).map(Vec::as_slice).unwrap_or(&[]) {
                if j < blo {
                    continue;
                }
                if j >= bhi {
                    break;
                }
                let k = j.checked_sub(1).and_then(|p| j2len.get(&p)).copied().unwrap_or(0) + 1;
                new.insert(j, k);
                if k > bestsize {
                    (besti, bestj, bestsize) = (i + 1 - k, j + 1 - k, k);
                }
            }
            j2len = new;
        }
        // Extend over popular characters the autojunk heuristic dropped from b2j.
        while besti > alo && bestj > blo && a[besti - 1] == b[bestj - 1] {
            (besti, bestj, bestsize) = (besti - 1, bestj - 1, bestsize + 1);
        }
        while besti + bestsize < ahi && bestj + bestsize < bhi && a[besti + bestsize] == b[bestj + bestsize] {
            bestsize += 1;
        }
        (besti, bestj, bestsize)
    }
    let mut matches = 0;
    let mut queue = vec![(0, a.len(), 0, b.len())];
    while let Some((alo, ahi, blo, bhi)) = queue.pop() {
        let (i, j, k) = longest(a, b, &b2j, alo, ahi, blo, bhi);
        if k > 0 {
            matches += k;
            if alo < i && blo < j {
                queue.push((alo, i, blo, j));
            }
            if i + k < ahi && j + k < bhi {
                queue.push((i + k, ahi, j + k, bhi));
            }
        }
    }
    let total = a.len() + b.len();
    if total == 0 {
        1.0
    } else {
        2.0 * matches as f64 / total as f64
    }
}

/// `difflib.get_close_matches(word, possibilities, n=1)`.
fn close_match(word: &str, possibilities: &[&'static str]) -> Option<&'static str> {
    let b: Vec<char> = word.chars().collect();
    possibilities
        .iter()
        .map(|p| (sequence_ratio(&p.chars().collect::<Vec<_>>(), &b), *p))
        .filter(|(score, _)| *score >= 0.6)
        .max_by(|x, y| x.0.partial_cmp(&y.0).unwrap().then_with(|| x.1.cmp(y.1)))
        .map(|(_, p)| p)
}

enum Parsed {
    Evaluate {
        context: PathBuf,
        request: PathBuf,
        policy: PathBuf,
        out: PathBuf,
    },
    Validate {
        evidence: PathBuf,
        receipt: PathBuf,
        decision: PathBuf,
        out: Option<PathBuf>,
    },
}

/// What argparse does instead of returning a namespace.
enum Exit {
    Help(String),
    Error {
        usage: String,
        prog: String,
        message: String,
    },
}

/// How argparse's `_parse_optional` classifies one token against a parser's flags.
enum Class<'a> {
    /// A flag, with an explicit argument (`--flag=value`, or `-hvalue` for a short
    /// flag) and whether `=` introduced it.
    Flag(&'a str, Option<(String, bool)>),
    UnknownOption,
    Positional,
}

fn classify<'a>(token: &str, flags: &[&'a str]) -> Result<Class<'a>, String> {
    if token.is_empty() || !token.starts_with('-') {
        return Ok(Class::Positional);
    }
    if let Some(exact) = flags.iter().find(|f| **f == token) {
        return Ok(Class::Flag(exact, None));
    }
    if token.chars().count() == 1 {
        return Ok(Class::Positional);
    }
    if let Some((name, value)) = token.split_once('=')
        && let Some(exact) = flags.iter().find(|f| **f == name)
    {
        return Ok(Class::Flag(exact, Some((value.to_string(), true))));
    }
    let found: Vec<(&'a str, Option<(String, bool)>)> = if token.starts_with("--") {
        let (prefix, explicit) = match token.split_once('=') {
            Some((name, value)) => (name, Some((value.to_string(), true))),
            None => (token, None),
        };
        flags
            .iter()
            .filter(|f| f.starts_with(prefix))
            .map(|f| (*f, explicit.clone()))
            .collect()
    } else {
        let short: String = token.chars().take(2).collect();
        let rest: String = token.chars().skip(2).collect();
        flags
            .iter()
            .filter_map(|f| {
                if *f == short {
                    Some((*f, Some((rest.clone(), false))))
                } else {
                    f.starts_with(token).then_some((*f, None))
                }
            })
            .collect()
    };
    match found.len() {
        0 => {}
        1 => {
            let (flag, explicit) = found.into_iter().next().unwrap();
            return Ok(Class::Flag(flag, explicit));
        }
        _ => {
            let names: Vec<&str> = found.iter().map(|(f, _)| *f).collect();
            return Err(format!("ambiguous option: {token} could match {}", names.join(", ")));
        }
    }
    let body = &token[1..];
    let digit = |c: char| pytext::decimal_value(c as u32).is_some();
    let negative_number = (!body.is_empty() && body.chars().all(digit))
        || body
            .split_once('.')
            .is_some_and(|(int, frac)| int.chars().all(digit) && !frac.is_empty() && frac.chars().all(digit));
    if negative_number || token.contains(' ') {
        return Ok(Class::Positional);
    }
    Ok(Class::UnknownOption)
}

/// argparse's handling of an explicit argument given to the zero-argument help flag.
fn help_or_error(flag: &str, explicit: Option<(String, bool)>) -> Result<(), String> {
    match explicit {
        None => Ok(()),
        // "-hx": the rest is read as further short flags, and help runs first.
        Some((rest, false)) if flag == "-h" && !rest.is_empty() && !rest.starts_with('-') => Ok(()),
        Some((rest, _)) => Err(format!(
            "argument -h/--help: ignored explicit argument {}",
            pytext::repr(&rest)
        )),
    }
}

fn parse(prog: &str, args: &[String]) -> Result<Parsed, Exit> {
    let main_error = |message: String| Exit::Error {
        usage: main_usage(prog),
        prog: prog.to_string(),
        message,
    };
    let mut extras: Vec<String> = Vec::new();
    let mut index = 0;
    let mut options_ended = false;
    let command = loop {
        let Some(token) = args.get(index) else {
            return Err(main_error("the following arguments are required: command".into()));
        };
        index += 1;
        if !options_ended {
            if token == "--" {
                options_ended = true;
                continue;
            }
            match classify(token, &["-h", "--help"]).map_err(main_error)? {
                Class::Flag(flag, explicit) => {
                    help_or_error(flag, explicit).map_err(main_error)?;
                    return Err(Exit::Help(main_help(prog)));
                }
                Class::UnknownOption => {
                    extras.push(token.clone());
                    continue;
                }
                Class::Positional => {}
            }
        }
        match COMMANDS.iter().find(|c| c.name == token) {
            Some(command) => break command,
            None => {
                let names: Vec<&'static str> = COMMANDS.iter().map(|c| c.name).collect();
                let quoted: Vec<String> = names.iter().map(|n| pytext::repr(n)).collect();
                let suggestion = close_match(token, &names)
                    .map(|m| format!(", maybe you meant {}?", pytext::repr(m)))
                    .unwrap_or_default();
                return Err(main_error(format!(
                    "argument command: invalid choice: {}{suggestion} (choose from {})",
                    pytext::repr(token),
                    quoted.join(", ")
                )));
            }
        }
    };
    let sub_prog = format!("{prog} {}", command.name);
    let sub_error = |message: String| Exit::Error {
        usage: command_usage(prog, command),
        prog: sub_prog.clone(),
        message,
    };
    let mut flags: Vec<&str> = vec!["-h", "--help"];
    flags.extend(command.options.iter().map(|o| o.flag));
    let mut values: BTreeMap<&str, String> = BTreeMap::new();
    while let Some(token) = args.get(index) {
        index += 1;
        if token == "--" {
            // Everything from here is positional, and this parser takes none.
            extras.extend(args[index - 1..].iter().cloned());
            break;
        }
        let (flag, explicit) = match classify(token, &flags).map_err(sub_error)? {
            Class::Flag(flag, explicit) => (flag, explicit),
            Class::UnknownOption | Class::Positional => {
                extras.push(token.clone());
                continue;
            }
        };
        if flag == "-h" || flag == "--help" {
            help_or_error(flag, explicit).map_err(sub_error)?;
            return Err(Exit::Help(command_help(prog, command)));
        }
        let value = match explicit {
            Some((value, _)) => value,
            None => match args.get(index) {
                Some(next) if next != "--" && matches!(classify(next, &flags), Ok(Class::Positional)) => {
                    index += 1;
                    next.clone()
                }
                _ => return Err(sub_error(format!("argument {flag}: expected one argument"))),
            },
        };
        values.insert(flag, value);
    }
    let missing: Vec<&str> = command
        .options
        .iter()
        .filter(|o| o.required && !values.contains_key(o.flag))
        .map(|o| o.flag)
        .collect();
    if !missing.is_empty() {
        return Err(sub_error(format!(
            "the following arguments are required: {}",
            missing.join(", ")
        )));
    }
    if !extras.is_empty() {
        return Err(main_error(format!("unrecognized arguments: {}", extras.join(" "))));
    }
    let path = |flag: &str| {
        values
            .get(flag)
            .map(|value| PathBuf::from(crate::pypath::encode_os(value)))
    };
    Ok(match command.name {
        "evaluate" => Parsed::Evaluate {
            context: path("--context").unwrap(),
            request: path("--request").unwrap(),
            policy: path("--policy").unwrap(),
            out: path("--out").unwrap(),
        },
        _ => Parsed::Validate {
            evidence: path("--evidence").unwrap(),
            receipt: path("--receipt").unwrap(),
            decision: path("--decision").unwrap(),
            out: path("--out"),
        },
    })
}

/// Write one line; the text is already free of unencodable characters.
fn emit(stream: &mut dyn Write, text: &str) {
    let _ = writeln!(stream, "{text}");
}

/// Refuse a validation output that names one of the files validate_review just read.
fn refuse_input_collision(out: &Path, inputs: &gateway::ReviewInputs) -> Result<(), Failure> {
    let protected = [&inputs.evidence, &inputs.receipt, &inputs.model];
    let decision = match &inputs.decision {
        Location::Disk(path) => Some(path),
        Location::Bundled(_) => None,
    };
    for path in protected.into_iter().chain(decision) {
        let same = crate::pypath::same_path(out, path)
            || (out.exists() && path.exists() && {
                let ids = crate::pypath::file_id(out).and_then(|a| crate::pypath::file_id(path).map(|b| a == b));
                match ids {
                    Ok(same) => same,
                    Err(error) => {
                        return Err(Failure::Gateway(format!(
                            "validation output cannot be checked against {}: {}.",
                            crate::pypath::text(path),
                            persist::os_error(&error, Some(path), None)
                        )));
                    }
                }
            });
        if same {
            return Err(Failure::Gateway(format!(
                "validation output must not overwrite a review input: {}.",
                crate::pypath::text(path)
            )));
        }
    }
    Ok(())
}

fn protected_locations(inputs: &gateway::ReviewInputs) -> Vec<Location> {
    vec![
        Location::Disk(inputs.evidence.clone()),
        Location::Disk(inputs.receipt.clone()),
        inputs.decision.clone(),
        Location::Disk(inputs.model.clone()),
    ]
}

fn run(env: &Env, parsed: Parsed, stdout: &mut dyn Write) -> Result<(), Failure> {
    match parsed {
        Parsed::Evaluate {
            context,
            request,
            policy,
            out,
        } => {
            let evaluation = gateway::evaluate(env, &context, &request, &policy)?;
            let outputs =
                persist::write_evaluation(env, &evaluation.model, &evaluation.evidence, &evaluation.receipt, &out)?;
            let total = evaluation.model.at("total_findings");
            let truncated = matches!(evaluation.model.at("truncated"), Json::Bool(true));
            let truncation = if truncated {
                format!(
                    "; TRUNCATED at max_results, {} total finding(s)",
                    crate::pyjson::canonical(total)
                )
            } else {
                String::new()
            };
            let shown = match evaluation.model.at("findings") {
                Json::Array(items) => items.len(),
                _ => 0,
            };
            emit(
                stdout,
                &format!("elizabeth-anne-alexander: REVIEW_READY; {shown} bounded finding(s){truncation}"),
            );
            for (name, path) in [
                ("model", &outputs.model),
                ("evidence", &outputs.evidence),
                ("receipt", &outputs.receipt),
            ] {
                emit(stdout, &format!("  {name}: {}", crate::pypath::text(path)));
            }
            Ok(())
        }
        Parsed::Validate {
            evidence,
            receipt,
            decision,
            out,
        } => {
            let mut target = None;
            if let Some(out) = out {
                let resolved = env.within_build(&out, "validation output", false)?;
                refuse_input_collision(
                    &resolved,
                    &gateway::review_input_paths(env, &evidence, &receipt, &decision)?,
                )?;
                target = Some(resolved);
            }
            let validation = gateway::validate_review(env, &evidence, &receipt, &decision)?;
            if let Some(target) = target {
                let inputs = gateway::review_input_paths(env, &evidence, &receipt, &decision)?;
                persist::write_validation(&validation, &target, &protected_locations(&inputs))?;
            }
            let text = |key: &str| {
                crate::pyjson::canonical(validation.at(key))
                    .trim_matches('"')
                    .to_string()
            };
            emit(
                stdout,
                &format!(
                    "elizabeth-anne-alexander: {}; {} decision(s); {} undecided finding(s)",
                    text("status"),
                    text("decision_count"),
                    text("undecided_count")
                ),
            );
            Ok(())
        }
    }
}

/// `main(argv)`: the process exit code.
pub fn main(args: &[String], env: &Env, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let prog = "elizabeth-anne-alexander";
    let parsed = match parse(prog, args) {
        Ok(parsed) => parsed,
        Err(Exit::Help(text)) => {
            let _ = write!(stdout, "{text}");
            return 0;
        }
        Err(Exit::Error { usage, prog, message }) => {
            let _ = write!(stderr, "{usage}\n{prog}: error: {message}\n");
            return 2;
        }
    };
    match run(env, parsed, stdout) {
        Ok(()) => 0,
        Err(Failure::Gateway(message)) => {
            emit(stderr, &format!("elizabeth-anne-alexander: blocked: {message}"));
            2
        }
        Err(Failure::Crash(message)) => {
            emit(stderr, &format!("elizabeth-anne-alexander: internal error: {message}"));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_wraps_like_argparse_at_80_columns() {
        let evaluate = &COMMANDS[0];
        assert_eq!(
            command_usage("elizabeth-anne-alexander", evaluate),
            "usage: elizabeth-anne-alexander evaluate [-h] --context CONTEXT\n                                         --request REQUEST --policy POLICY\n                                         --out OUT"
        );
    }

    #[test]
    fn difflib_ratio_suggests_like_python() {
        assert_eq!(
            close_match("evalute", &["evaluate", "validate-review"]),
            Some("evaluate")
        );
        assert_eq!(close_match("zzz", &["evaluate", "validate-review"]), None);
    }
}
