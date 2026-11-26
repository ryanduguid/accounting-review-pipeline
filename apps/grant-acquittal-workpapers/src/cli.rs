// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! `main(argv)` with the argparse behaviour the script had: usage, help,
//! option abbreviations, and exit 2 for a refused command line.

use crate::textwrap::wrap;
use std::io::Write;
use std::path::Path;

use crate::argtheme::{self, Stream, Theme};
use crate::workpaper::{self, Failure};
use crate::{oserror, pytext};

const DESCRIPTION: &str = "Local grant allocation and funding workpapers from explicitly reviewed inputs.";
const FLAGS: [&str; 5] = ["-h", "--help", "--input", "--output", "--ignore-file"];
const IGNORE_HELP: &str = "CSV of ledger_id,reason naming ledger rows dated outside the ledger period to leave out of every total; each is listed on the workpaper with its reason";

/// argparse's formatter width: `shutil.get_terminal_size().columns` minus 2.
fn width() -> usize {
    usize::try_from(crate::termsize::columns() - 2).unwrap_or(0)
}

/// A usage part or help header: its plain text, which sets every width, and
/// the coloured text argparse prints.
struct Part {
    plain: String,
    coloured: String,
}

/// `_format_usage`'s `get_lines`: wrap parts under `indent`; with a prefix
/// length, the first line starts after the prefix instead of the indent.
fn usage_lines(parts: &[Part], indent: &str, prefix_len: Option<usize>, text_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line: Vec<&str> = Vec::new();
    let mut line_len = prefix_len.unwrap_or(indent.len()) - 1;
    for part in parts {
        let part_len = pytext::py_len(&part.plain);
        if line_len + 1 + part_len > text_width && !line.is_empty() {
            lines.push(format!("{indent}{}", line.join(" ")));
            line.clear();
            line_len = indent.len() - 1;
        }
        line.push(&part.coloured);
        line_len += part_len + 1;
    }
    if !line.is_empty() {
        lines.push(format!("{indent}{}", line.join(" ")));
    }
    if prefix_len.is_some() && !lines.is_empty() {
        lines[0] = lines[0][indent.len()..].to_string();
    }
    lines
}

/// `HelpFormatter._format_usage` for this parser's four optionals.
fn usage(prog: &str, t: &Theme) -> String {
    let prefix = "usage: ";
    let text_width = width();
    let option = |flag: &str, label: &str| Part {
        plain: format!("{flag} {label}"),
        coloured: format!("{}{flag} {}{label}{}", t.summary_long_option, t.summary_label, t.reset),
    };
    let parts = [
        Part {
            plain: "[-h]".into(),
            coloured: format!("[{}-h{}]", t.summary_short_option, t.reset),
        },
        option("--input", "INPUT"),
        option("--output", "OUTPUT"),
        Part {
            plain: "[--ignore-file IGNORE_FILE]".into(),
            coloured: format!(
                "[{}--ignore-file {}IGNORE_FILE{}]",
                t.summary_long_option, t.summary_label, t.reset
            ),
        },
    ];
    let plain: Vec<&str> = parts.iter().map(|p| p.plain.as_str()).collect();
    let rest = if prefix.len() + pytext::py_len(&format!("{prog} {}", plain.join(" "))) <= text_width {
        format!(
            " {}",
            parts.iter().map(|p| p.coloured.as_str()).collect::<Vec<_>>().join(" ")
        )
    } else if (prefix.len() + pytext::py_len(prog)) as f64 <= 0.75 * text_width as f64 {
        let indent = " ".repeat(prefix.len() + pytext::py_len(prog) + 1);
        let mut head = vec![Part {
            plain: prog.to_string(),
            coloured: prog.to_string(),
        }];
        head.extend(parts);
        let joined = usage_lines(&head, &indent, Some(prefix.len()), text_width).join("\n");
        joined[prog.len()..].to_string()
    } else {
        let indent = " ".repeat(prefix.len());
        format!("\n{}", usage_lines(&parts, &indent, None, text_width).join("\n"))
    };
    format!("{}{prefix}{}{}{prog}{}{rest}", t.usage, t.reset, t.prog, t.reset)
}

/// The options section's rows: the invocation and its help, if any.
fn help_rows(t: &Theme) -> [(Part, Option<&'static str>); 4] {
    let flag = |text: &str, colour: &str| format!("{colour}{text}{}", t.reset);
    let option = |name: &str, label: &str| Part {
        plain: format!("{name} {label}"),
        coloured: format!("{} {}", flag(name, t.long_option), flag(label, t.label)),
    };
    let help = Part {
        plain: "-h, --help".into(),
        coloured: format!("{}, {}", flag("-h", t.short_option), flag("--help", t.long_option)),
    };
    [
        (help, Some("show this help message and exit")),
        (option("--input", "INPUT"), None),
        (option("--output", "OUTPUT"), None),
        (option("--ignore-file", "IGNORE_FILE"), Some(IGNORE_HELP)),
    ]
}

/// `HelpFormatter._format_action` for each row, aligned across the section.
fn format_rows(rows: &[(Part, Option<&str>)], width: usize) -> String {
    let max_help_position = 24.min(width.saturating_sub(20).max(4));
    let action_max = rows
        .iter()
        .map(|(header, _)| 2 + pytext::py_len(&header.plain))
        .max()
        .unwrap_or(0);
    let help_position = (action_max + 2).min(max_help_position);
    let help_width = width.saturating_sub(help_position).max(11);
    let action_width = help_position.saturating_sub(4);
    let mut out = String::new();
    for (header, text) in rows {
        let Some(text) = text else {
            out.push_str(&format!("  {}\n", header.coloured));
            continue;
        };
        let lines = wrap(text, help_width);
        if pytext::py_len(&header.plain) <= action_width {
            // argparse pads the plain header, then swaps the coloured one in.
            let padded = format!("  {:<action_width$}  {}\n", header.plain, lines[0]);
            out.push_str(&padded.replacen(&header.plain, &header.coloured, 1));
        } else {
            out.push_str(&format!(
                "  {}\n{}{}\n",
                header.coloured,
                " ".repeat(help_position),
                lines[0]
            ));
        }
        for line in &lines[1..] {
            out.push_str(&format!("{}{line}\n", " ".repeat(help_position)));
        }
    }
    out
}

const LICENCE_NOTICE: &str = "\n\
    Copyright 2026 Ryan Duguid. No warranty.\n\
    AGPL-3.0-or-later with additional terms (see NOTICE).\n\
    Licence: https://github.com/ryanduguid/accounting-review-pipeline/blob/main/apps/grant-acquittal-workpapers/LICENSE\n";

fn help(prog: &str, t: &Theme) -> String {
    let width = width();
    let options = format_rows(&help_rows(t), width);
    let description = wrap(DESCRIPTION, width.max(11)).join("\n");
    format!(
        "{}\n\n{description}\n\n{}options:{}\n{options}{LICENCE_NOTICE}",
        usage(prog, t),
        t.heading,
        t.reset
    )
}

enum Class<'a> {
    Flag(&'static str, Option<(&'a str, bool)>),
    UnknownOption,
    Positional,
}

/// argparse's `_parse_optional` for this parser.
fn classify(token: &str) -> Result<Class<'_>, String> {
    if token.is_empty() || !token.starts_with('-') {
        return Ok(Class::Positional);
    }
    if let Some(exact) = FLAGS.iter().find(|f| **f == token) {
        return Ok(Class::Flag(exact, None));
    }
    if pytext::py_len(token) == 1 {
        return Ok(Class::Positional);
    }
    if let Some((name, value)) = token.split_once('=')
        && let Some(exact) = FLAGS.iter().find(|f| **f == name)
    {
        return Ok(Class::Flag(exact, Some((value, true))));
    }
    let mut found = option_prefix_matches(token);
    match found.len() {
        0 => {}
        1 => {
            let (flag, explicit) = found.remove(0);
            return Ok(Class::Flag(flag, explicit));
        }
        _ => {
            let names: Vec<&str> = found.iter().map(|(f, _)| *f).collect();
            return Err(format!("ambiguous option: {token} could match {}", names.join(", ")));
        }
    }
    if looks_like_negative_number(&token[1..]) || token.contains(' ') {
        return Ok(Class::Positional);
    }
    Ok(Class::UnknownOption)
}

type PrefixMatch<'a> = (&'static str, Option<(&'a str, bool)>);

/// argparse's `_get_option_tuples`: the flags a token abbreviates, with any explicit value.
fn option_prefix_matches(token: &str) -> Vec<PrefixMatch<'_>> {
    if token.starts_with("--") {
        let (prefix, explicit) = match token.split_once('=') {
            Some((name, value)) => (name, Some((value, true))),
            None => (token, None),
        };
        return FLAGS
            .iter()
            .filter(|f| f.starts_with(prefix))
            .map(|f| (*f, explicit))
            .collect();
    }
    let short_end = token.char_indices().nth(2).map_or(token.len(), |(index, _)| index);
    let (short, rest) = token.split_at(short_end);
    FLAGS
        .iter()
        .filter_map(|f| {
            if *f == short {
                Some((*f, Some((rest, false))))
            } else {
                f.starts_with(token).then_some((*f, None))
            }
        })
        .collect()
}

/// argparse's `_negative_number_matcher` on the text after the leading '-'.
fn looks_like_negative_number(body: &str) -> bool {
    let digit = |c: char| pytext::decimal_value(c as u32).is_some();
    (!body.is_empty() && body.chars().all(digit))
        || body
            .split_once('.')
            .is_some_and(|(int, frac)| int.chars().all(digit) && !frac.is_empty() && frac.chars().all(digit))
}

enum Exit {
    Help,
    Error(String),
}

/// The input folder, the output folder and any ignore file.
type Parsed<'a> = (&'a str, &'a str, Option<&'a str>);

fn parse(args: &[String]) -> Result<Parsed<'_>, Exit> {
    let mut extras = Vec::new();
    let mut input = None;
    let mut output = None;
    let mut ignore = None;
    let mut index = 0;
    while let Some(token) = args.get(index) {
        index += 1;
        if token == "--" {
            extras.extend(args[index - 1..].iter().map(String::as_str));
            break;
        }
        let (flag, explicit) = match classify(token).map_err(Exit::Error)? {
            Class::Flag(flag, explicit) => (flag, explicit),
            Class::UnknownOption | Class::Positional => {
                extras.push(token.as_str());
                continue;
            }
        };
        if flag == "-h" || flag == "--help" {
            return Err(help_exit(flag, explicit));
        }
        let value = match explicit {
            Some((value, _)) => value,
            None => match args.get(index) {
                Some(next) if next != "--" && matches!(classify(next), Ok(Class::Positional)) => {
                    index += 1;
                    next.as_str()
                }
                _ => return Err(Exit::Error(format!("argument {flag}: expected one argument"))),
            },
        };
        match flag {
            "--input" => input = Some(value),
            "--output" => output = Some(value),
            _ => ignore = Some(value),
        }
    }
    match (input, output) {
        (Some(input), Some(output)) if extras.is_empty() => Ok((input, output, ignore)),
        (Some(_), Some(_)) => Err(Exit::Error(format!("unrecognized arguments: {}", extras.join(" ")))),
        (input, output) => {
            let missing: Vec<&str> = [("--input", input.is_none()), ("--output", output.is_none())]
                .into_iter()
                .filter_map(|(flag, absent)| absent.then_some(flag))
                .collect();
            Err(Exit::Error(format!(
                "the following arguments are required: {}",
                missing.join(", ")
            )))
        }
    }
}

/// `-h`/`--help`: print help, unless it was given an explicit value argparse refuses.
fn help_exit(flag: &str, explicit: Option<(&str, bool)>) -> Exit {
    match explicit {
        None => Exit::Help,
        // `-hX` is `-h` followed by the short flags in X, so help still wins.
        Some((rest, false)) if flag == "-h" && !rest.is_empty() && !rest.starts_with('-') => Exit::Help,
        Some((rest, _)) => Exit::Error(format!(
            "argument -h/--help: ignored explicit argument {}",
            pytext::repr_os(rest)
        )),
    }
}

/// The status line for a written workpaper, and its exit code.
fn report(result: &crate::pyjson::Json, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let status = result.get("status").and_then(|s| match s {
        crate::pyjson::Json::Str(text) => text.to_utf8(),
        _ => None,
    });
    let count = match result.get("findings") {
        Some(crate::pyjson::Json::Array(items)) => items.len(),
        _ => 0,
    };
    let ignored = match result.get("ignored_items") {
        Some(crate::pyjson::Json::Array(items)) => format!("; {} out-of-period ledger rows ignored", items.len()),
        _ => String::new(),
    };
    if let Err(error) = writeln!(
        stdout,
        "grant-workpaper: {}; {count} review items{ignored}",
        status.unwrap_or_default()
    ) {
        // The script's last print sat outside its handler, so a failed write
        // ended it with a traceback.
        let _ = writeln!(
            stderr,
            "grant-workpaper: internal error: {}",
            oserror::stream_error(&error)
        );
        return 1;
    }
    if count > 0 { 2 } else { 0 }
}

/// `main(argv)`: the process exit code.
pub fn main(prog: &str, args: &[String], cwd: &Path, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let (input, output, ignore) = match parse(args) {
        Ok(parsed) => parsed,
        // argparse's _print_message ignores an OSError from either stream.
        Err(Exit::Help) => {
            let _ = write!(stdout, "{}", help(prog, argtheme::theme(Stream::Stdout)));
            return 0;
        }
        Err(Exit::Error(message)) => {
            let t = argtheme::theme(Stream::Stderr);
            let _ = write!(
                stderr,
                "{}\n{prog}: {}error:{} {}{message}{}\n",
                usage(prog, t),
                t.error,
                t.reset,
                t.message,
                t.reset
            );
            return 2;
        }
    };
    // Path("") is ".", as argparse's type=Path made it.
    let input = if input.is_empty() { "." } else { input };
    let output = if output.is_empty() { "." } else { output };
    // Path("") is ".", which names a folder, so an empty ignore path fails to read.
    let result = workpaper::review_ignoring(input, ignore, cwd);
    let outcome = result.and_then(|result| workpaper::write(&result, output, cwd).map(|()| result));
    match outcome {
        Ok(result) => report(&result, stdout, stderr),
        // A diagnostic that cannot be written raised from the handler: exit 1,
        // as here. main settles what that leaves pending on stderr.
        Err(Failure::Error(message)) => {
            let _ = writeln!(stderr, "grant-workpaper: {message}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_path_arguments_treat_the_working_directory_as_dot() {
        let fixture = crate::tests::Fixture::new();
        let cwd = fixture.source();
        let run = |input, output| {
            let args = ["--input", input, "--output", output].map(String::from);
            let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
            let code = main("grant-workpaper", &args, &cwd, &mut stdout, &mut stderr);
            (code, stdout, stderr)
        };

        let explicit = run(".", "explicit");
        let implicit = run("", "implicit");
        assert_eq!(implicit, explicit);
        assert!(matches!(implicit.0, 0 | 2));
        assert_eq!(
            std::fs::read(cwd.join("explicit/workpaper.json")).unwrap(),
            std::fs::read(cwd.join("implicit/workpaper.json")).unwrap()
        );

        let marker = cwd.join("agreements.json");
        let original = std::fs::read(&marker).unwrap();
        let refused = run(".", "");
        assert_eq!(refused, run(".", "."));
        assert_eq!(refused.0, 1);
        assert_eq!(std::fs::read(marker).unwrap(), original);
        assert!(!cwd.join("workpaper.json").exists());
    }

    #[test]
    fn a_failed_status_write_keeps_the_published_workpaper() {
        // A full buffer refuses the status line, as a closed stream would.
        let mut full: &mut [u8] = &mut [];
        let refusal = full.write_all(b"status").unwrap_err();

        let fixture = crate::tests::Fixture::new();
        let args = ["--input", crate::tests::SOURCE, "--output", "published"].map(String::from);
        let mut stderr = Vec::new();
        assert_eq!(
            main("grant-workpaper", &args, fixture.root(), &mut full, &mut stderr),
            1
        );
        let diagnostic = oserror::stream_error(&refusal);
        assert_eq!(
            stderr,
            format!("grant-workpaper: internal error: {diagnostic}\n").as_bytes()
        );

        let saved = std::fs::read(fixture.root().join("published/workpaper.json")).unwrap();
        assert_eq!(
            crate::pyjson::dumps_indent2(&crate::pyjson::loads_bytes(&saved).unwrap()),
            crate::pyjson::dumps_indent2(&fixture.review().unwrap())
        );
    }

    #[test]
    fn a_result_without_a_status_or_findings_reports_an_empty_status_and_no_items() {
        let (mut out, mut error) = (Vec::new(), Vec::new());
        let result = crate::pyjson::Json::object([
            ("status", crate::pyjson::Json::Null),
            ("findings", crate::pyjson::Json::object([])),
        ]);
        assert_eq!(report(&result, &mut out, &mut error), 0);
        assert_eq!(out, b"grant-workpaper: ; 0 review items\n");
        assert!(error.is_empty());
    }

    #[test]
    fn an_empty_long_option_name_reports_all_candidates_in_order() {
        let (mut out, mut error) = (Vec::new(), Vec::new());
        assert_eq!(
            main(
                "grant-workpaper",
                &["--=fabricated".into()],
                Path::new("."),
                &mut out,
                &mut error
            ),
            2
        );
        assert!(out.is_empty());
        assert!(
            String::from_utf8(error)
                .unwrap()
                .contains("ambiguous option: --=fabricated could match --help, --input, --output")
        );
    }

    #[test]
    fn usage_and_help_match_argparse() {
        // Python 3.15 with FORCE_COLOR=1 and 80 columns.
        assert_eq!(
            usage("grant_workpaper.py", &argtheme::COLOURED),
            "\x1b[1;34musage: \x1b[0m\x1b[1;35mgrant_workpaper.py\x1b[0m [\x1b[32m-h\x1b[0m] \x1b[36m--input \x1b[33mINPUT\x1b[0m \x1b[36m--output \x1b[33mOUTPUT\x1b[0m"
        );
        assert_eq!(
            usage("grant_workpaper.py", &argtheme::PLAIN),
            "usage: grant_workpaper.py [-h] --input INPUT --output OUTPUT"
        );
        assert_eq!(
            help("grant_workpaper.py", &argtheme::PLAIN)
                .strip_suffix(LICENCE_NOTICE)
                .expect("help ends with the licence notice"),
            "usage: grant_workpaper.py [-h] --input INPUT --output OUTPUT\n\nLocal grant allocation and funding workpapers from explicitly reviewed inputs.\n\noptions:\n  -h, --help       show this help message and exit\n  --input INPUT\n  --output OUTPUT\n"
        );
    }
}
