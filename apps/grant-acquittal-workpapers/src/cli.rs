//! `main(argv)` with the argparse behaviour the script had: usage, help,
//! option abbreviations, and exit 2 for a refused command line.

use std::io::Write;
use std::path::Path;

use crate::pytext;
use crate::workpaper::{self, Failure};

const DESCRIPTION: &str = "Local grant allocation and funding workpapers from explicitly reviewed inputs.";
const FLAGS: [&str; 4] = ["-h", "--help", "--input", "--output"];

/// argparse's formatter width: `$COLUMNS` (or 80) minus 2.
fn width() -> usize {
    let columns = std::env::var("COLUMNS")
        .ok()
        .and_then(|c| c.parse::<usize>().ok())
        .filter(|&c| c > 0)
        .unwrap_or(80);
    columns.saturating_sub(2)
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && pytext::py_len(&line) + 1 + pytext::py_len(word) > width {
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

/// `HelpFormatter._format_usage` for this parser's three optionals.
fn usage(prog: &str) -> String {
    let prefix = "usage: ";
    let text_width = width();
    let parts = ["[-h]", "--input INPUT", "--output OUTPUT"];
    let flat = format!("{prog} {}", parts.join(" "));
    if prefix.len() + pytext::py_len(&flat) <= text_width {
        return format!("{prefix}{flat}");
    }
    let get_lines = |parts: &[&str], indent: &str, with_prefix: bool| -> Vec<String> {
        let mut lines = Vec::new();
        let mut line: Vec<&str> = Vec::new();
        let mut line_len = if with_prefix {
            prefix.len() - 1
        } else {
            indent.len() - 1
        };
        for part in parts {
            if line_len + 1 + pytext::py_len(part) > text_width && !line.is_empty() {
                lines.push(format!("{indent}{}", line.join(" ")));
                line.clear();
                line_len = indent.len() - 1;
            }
            line.push(part);
            line_len += pytext::py_len(part) + 1;
        }
        if !line.is_empty() {
            lines.push(format!("{indent}{}", line.join(" ")));
        }
        if with_prefix && !lines.is_empty() {
            lines[0] = lines[0][indent.len()..].to_string();
        }
        lines
    };
    let lines = if (prefix.len() + pytext::py_len(prog)) as f64 <= 0.75 * text_width as f64 {
        let indent = " ".repeat(prefix.len() + pytext::py_len(prog) + 1);
        let mut head = vec![prog];
        head.extend(parts);
        get_lines(&head, &indent, true)
    } else {
        let indent = " ".repeat(prefix.len());
        let mut lines = vec![prog.to_string()];
        lines.extend(get_lines(&parts, &indent, false));
        lines
    };
    format!("{prefix}{}", lines.join("\n"))
}

fn help(prog: &str) -> String {
    let width = width();
    let rows: [(&str, Option<&str>); 3] = [
        ("-h, --help", Some("show this help message and exit")),
        ("--input INPUT", None),
        ("--output OUTPUT", None),
    ];
    let max_help_position = 24.min(width.saturating_sub(20).max(4));
    let action_max = rows
        .iter()
        .map(|(header, _)| 2 + pytext::py_len(header))
        .max()
        .unwrap_or(0);
    let help_position = (action_max + 2).min(max_help_position);
    let help_width = width.saturating_sub(help_position).max(11);
    let action_width = help_position.saturating_sub(4);
    let mut options = String::new();
    for (header, text) in rows {
        match text {
            None => options.push_str(&format!("  {header}\n")),
            Some(text) => {
                let lines = wrap(text, help_width);
                if pytext::py_len(header) <= action_width {
                    options.push_str(&format!("  {header:<action_width$}  {}\n", lines[0]));
                } else {
                    options.push_str(&format!("  {header}\n{}{}\n", " ".repeat(help_position), lines[0]));
                }
                for line in &lines[1..] {
                    options.push_str(&format!("{}{line}\n", " ".repeat(help_position)));
                }
            }
        }
    }
    format!(
        "{}\n\n{}\n\noptions:\n{options}",
        usage(prog),
        wrap(DESCRIPTION, width).join("\n")
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
            let _ = write!(stdout, "{}", help(prog));
            return 0;
        }
        Err(Exit::Error(message)) => {
            let _ = write!(stderr, "{}\n{prog}: error: {message}\n", usage(prog));
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
        assert_eq!(
            usage("grant_workpaper.py"),
            "usage: grant_workpaper.py [-h] --input INPUT --output OUTPUT"
        );
        assert_eq!(
            help("grant_workpaper.py"),
            "usage: grant_workpaper.py [-h] --input INPUT --output OUTPUT\n\nLocal grant allocation and funding workpapers from explicitly reviewed inputs.\n\noptions:\n  -h, --help       show this help message and exit\n  --input INPUT\n  --output OUTPUT\n"
        );
    }
}
