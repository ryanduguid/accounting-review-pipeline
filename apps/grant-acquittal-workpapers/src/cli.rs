//! `main(argv)` with the argparse behaviour the script had: usage, help,
//! option abbreviations, and exit 2 for a refused command line.

use crate::textwrap::wrap;
use std::io::Write;
use std::path::Path;

use crate::argtheme::{self, Stream, Theme};
use crate::pytext;
use crate::workpaper::{self, Failure};

const DESCRIPTION: &str = "Local grant allocation and funding workpapers from explicitly reviewed inputs.";
const FLAGS: [&str; 4] = ["-h", "--help", "--input", "--output"];

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

/// `HelpFormatter._format_usage` for this parser's three optionals.
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
fn help_rows(t: &Theme) -> [(Part, Option<&'static str>); 3] {
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

fn help(prog: &str, t: &Theme) -> String {
    let width = width();
    let options = format_rows(&help_rows(t), width);
    let description = wrap(DESCRIPTION, width.max(11)).join("\n");
    format!(
        "{}\n\n{description}\n\n{}options:{}\n{options}",
        usage(prog, t),
        t.heading,
        t.reset
    )
}

enum Class {
    Flag(&'static str, Option<(String, bool)>),
    UnknownOption,
    Positional,
}

/// argparse's `_parse_optional` for this parser.
fn classify(token: &str) -> Result<Class, String> {
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
        return Ok(Class::Flag(exact, Some((value.to_string(), true))));
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

type PrefixMatch = (&'static str, Option<(String, bool)>);

/// argparse's `_get_option_tuples`: the flags a token abbreviates, with any explicit value.
fn option_prefix_matches(token: &str) -> Vec<PrefixMatch> {
    if token.starts_with("--") {
        let (prefix, explicit) = match token.split_once('=') {
            Some((name, value)) => (name, Some((value.to_string(), true))),
            None => (token, None),
        };
        return FLAGS
            .iter()
            .filter(|f| f.starts_with(prefix))
            .map(|f| (*f, explicit.clone()))
            .collect();
    }
    let short: String = token.chars().take(2).collect();
    let rest: String = token.chars().skip(2).collect();
    FLAGS
        .iter()
        .filter_map(|f| {
            if *f == short {
                Some((*f, Some((rest.clone(), false))))
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

fn parse(args: &[String]) -> Result<(String, String), Exit> {
    let mut extras: Vec<String> = Vec::new();
    let mut input = None;
    let mut output = None;
    let mut index = 0;
    while let Some(token) = args.get(index) {
        index += 1;
        if token == "--" {
            extras.extend(args[index - 1..].iter().cloned());
            break;
        }
        let (flag, explicit) = match classify(token).map_err(Exit::Error)? {
            Class::Flag(flag, explicit) => (flag, explicit),
            Class::UnknownOption | Class::Positional => {
                extras.push(token.clone());
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
                    next.clone()
                }
                _ => return Err(Exit::Error(format!("argument {flag}: expected one argument"))),
            },
        };
        if flag == "--input" {
            input = Some(value);
        } else {
            output = Some(value);
        }
    }
    match (input, output) {
        (Some(input), Some(output)) if extras.is_empty() => Ok((input, output)),
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
fn help_exit(flag: &str, explicit: Option<(String, bool)>) -> Exit {
    match explicit {
        None => Exit::Help,
        // `-hX` is `-h` followed by the short flags in X, so help still wins.
        Some((rest, false)) if flag == "-h" && !rest.is_empty() && !rest.starts_with('-') => Exit::Help,
        Some((rest, _)) => Exit::Error(format!(
            "argument -h/--help: ignored explicit argument {}",
            pytext::repr_os(&rest)
        )),
    }
}

/// `main(argv)`: the process exit code.
pub fn main(prog: &str, args: &[String], cwd: &Path, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    let (input, output) = match parse(args) {
        Ok(parsed) => parsed,
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
    let input = if input.is_empty() { ".".to_string() } else { input };
    let output = if output.is_empty() { ".".to_string() } else { output };
    let result = workpaper::review(&input, cwd);
    let outcome = result.and_then(|result| workpaper::write(&result, &output, cwd).map(|()| result));
    match outcome {
        Ok(result) => {
            let status = result.get("status").and_then(|s| match s {
                crate::pyjson::Json::Str(text) => text.to_utf8(),
                _ => None,
            });
            let count = match result.get("findings") {
                Some(crate::pyjson::Json::Array(items)) => items.len(),
                _ => 0,
            };
            let _ = writeln!(
                stdout,
                "grant-workpaper: {}; {count} review items",
                status.unwrap_or_default()
            );
            if count > 0 { 2 } else { 0 }
        }
        Err(Failure::Error(message)) => {
            let _ = writeln!(stderr, "grant-workpaper: {message}");
            1
        }
        Err(Failure::Crash(message)) => {
            let _ = writeln!(stderr, "grant-workpaper: internal error: {message}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            help("grant_workpaper.py", &argtheme::PLAIN),
            "usage: grant_workpaper.py [-h] --input INPUT --output OUTPUT\n\nLocal grant allocation and funding workpapers from explicitly reviewed inputs.\n\noptions:\n  -h, --help       show this help message and exit\n  --input INPUT\n  --output OUTPUT\n"
        );
    }
}
