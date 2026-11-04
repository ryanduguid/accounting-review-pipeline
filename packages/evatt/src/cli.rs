//! The command line (`cli.py`), including the argparse behaviour it relied on:
//! usage, help and abbreviations. Copied from elizabeth-anne-alexander/src/cli.rs,
//! adapted for CPython 3.14 and evatt's exit 1 and private disclosure errors.

use crate::argtheme::{self, Stream, Theme};
use crate::textwrap::wrap;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use crate::pytext;

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

static COMMANDS: [Command; 5] = [
    Command {
        name: "redact",
        help: "raw markdown to sanitised markdown",
        options: &[
            opt("--in", "SOURCE", true, None),
            opt("--map", "ENTITY_MAP", true, None),
            opt("--out", "OUT", true, None),
        ],
    },
    Command {
        name: "restore",
        help: "reverse the entity map over an answer",
        options: &[
            opt("--in", "SOURCE", true, None),
            opt("--map", "ENTITY_MAP", true, None),
            opt("--out", "OUT", true, None),
        ],
    },
    Command {
        name: "verify",
        help: "re-scan a sanitised file",
        options: &[
            opt("--in", "SOURCE", true, None),
            opt("--map", "ENTITY_MAP", true, None),
        ],
    },
    Command {
        name: "disclosure-record",
        help: "record local evidence for an intended disclosure",
        options: &[
            opt("--in", "SOURCE", true, None),
            opt("--map", "ENTITY_MAP", true, None),
            opt("--destination", "DESTINATION", true, None),
            opt("--decision-ref", "DECISION_REF", true, None),
            opt("--out", "OUT", true, None),
        ],
    },
    Command {
        name: "disclosure-check",
        help: "check local evidence; does not authenticate permission",
        options: &[
            opt("--in", "SOURCE", true, None),
            opt("--map", "ENTITY_MAP", true, None),
            opt("--destination", "DESTINATION", true, None),
            opt("--decision-ref", "DECISION_REF", true, None),
            opt("--record", "RECORD", true, None),
        ],
    },
];

const DESCRIPTION: &str = "Pseudonymise Australian client data in markdown, locally, before sending it.";
const HELP_HELP: &str = "show this help message and exit";

/// argparse's formatter width: `shutil.get_terminal_size().columns` minus 2.
fn width() -> usize {
    usize::try_from(crate::termsize::columns() - 2).unwrap_or(0)
}

/// A usage part or help header: its plain text, which sets every width, and
/// the coloured text argparse prints.
#[derive(Clone)]
struct Part {
    plain: String,
    coloured: String,
}

/// `HelpFormatter._format_usage` for a parser whose usage parts are given.
fn usage(prog: &str, opt_parts: &[Part], pos_parts: &[Part], t: &Theme) -> String {
    let prefix = "usage: ";
    let text_width = width();
    let plain: Vec<&str> = opt_parts.iter().chain(pos_parts).map(|p| p.plain.as_str()).collect();
    let rest = if prefix.len() + pytext::py_len(&format!("{prog} {}", plain.join(" "))) <= text_width {
        let coloured: Vec<&str> = opt_parts.iter().chain(pos_parts).map(|p| p.coloured.as_str()).collect();
        format!(" {}", coloured.join(" "))
    } else {
        wrapped_usage(prog, opt_parts, pos_parts, text_width)
    };
    format!("{}{prefix}{}{}{prog}{}{rest}", t.usage, t.reset, t.prog, t.reset)
}

/// The wrapped usage after the program name, as `_format_usage` lays it out.
fn wrapped_usage(prog: &str, opt_parts: &[Part], pos_parts: &[Part], text_width: usize) -> String {
    let prefix_len = "usage: ".len();
    let get_lines = |parts: &[Part], indent: &str, with_prefix: bool| {
        usage_lines(parts, indent, with_prefix.then_some(prefix_len), text_width)
    };
    if (prefix_len + pytext::py_len(prog)) as f64 <= 0.75 * text_width as f64 {
        let indent = " ".repeat(prefix_len + pytext::py_len(prog) + 1);
        let mut head = vec![Part {
            plain: prog.to_string(),
            coloured: prog.to_string(),
        }];
        let lines = if !opt_parts.is_empty() {
            head.extend(opt_parts.iter().cloned());
            let mut lines = get_lines(&head, &indent, true);
            lines.extend(get_lines(pos_parts, &indent, false));
            lines
        } else {
            head.extend(pos_parts.iter().cloned());
            get_lines(&head, &indent, true)
        };
        return lines.join("\n")[prog.len()..].to_string();
    }
    let indent = " ".repeat(prefix_len);
    let all: Vec<Part> = opt_parts.iter().chain(pos_parts).cloned().collect();
    let mut lines = get_lines(&all, &indent, false);
    if lines.len() > 1 {
        lines = get_lines(opt_parts, &indent, false);
        lines.extend(get_lines(pos_parts, &indent, false));
    }
    format!("\n{}", lines.join("\n"))
}

/// `_format_usage`'s `get_lines`: wrap parts under `indent`; with a prefix
/// length, the first line starts after the prefix instead of the indent.
fn usage_lines(parts: &[Part], indent: &str, prefix_len: Option<usize>, text_width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line: Vec<&str> = Vec::new();
    let indent_length = indent.len();
    let mut line_len = prefix_len.unwrap_or(indent_length) - 1;
    for part in parts {
        let part_len = pytext::py_len(&part.plain);
        if line_len + 1 + part_len > text_width && !line.is_empty() {
            lines.push(format!("{indent}{}", line.join(" ")));
            line.clear();
            line_len = indent_length - 1;
        }
        line.push(&part.coloured);
        line_len += part_len + 1;
    }
    if !line.is_empty() {
        lines.push(format!("{indent}{}", line.join(" ")));
    }
    if prefix_len.is_some() && !lines.is_empty() {
        lines[0] = lines[0][indent_length..].to_string();
    }
    lines
}

type Row<'a> = (usize, Part, Option<&'a str>);

/// The rows of one help section; argparse aligns help text across every section of a parser.
fn action_rows(rows: &[Row], all: &[Row]) -> String {
    let width = width();
    let max_help_position = 24.min(width.saturating_sub(20).max(4));
    let action_max = all
        .iter()
        .map(|(indent, header, _)| indent + pytext::py_len(&header.plain))
        .max()
        .unwrap_or(0);
    let help_position = (action_max + 2).min(max_help_position);
    let help_width = width.saturating_sub(help_position).max(11);
    let mut out = String::new();
    for (indent, header, help) in rows {
        let action_width = help_position.saturating_sub(indent + 2);
        let pad = " ".repeat(*indent);
        let Some(help) = help else {
            out.push_str(&format!("{pad}{}\n", header.coloured));
            continue;
        };
        let lines = wrap(help, help_width);
        if pytext::py_len(&header.plain) <= action_width {
            // argparse pads the plain header, then swaps the coloured one in.
            let padded = format!("{pad}{:<action_width$}  {}\n", header.plain, lines[0]);
            out.push_str(&padded.replacen(&header.plain, &header.coloured, 1));
        } else {
            out.push_str(&format!(
                "{pad}{}\n{}{}\n",
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

fn choices() -> String {
    format!("{{{}}}", COMMANDS.iter().map(|c| c.name).collect::<Vec<_>>().join(","))
}

fn coloured(text: &str, colour: &str, t: &Theme) -> Part {
    Part {
        plain: text.to_string(),
        coloured: format!("{colour}{text}{}", t.reset),
    }
}

/// `-h, --help` as `_format_action_invocation` colours it.
fn help_header(t: &Theme) -> Part {
    Part {
        plain: "-h, --help".into(),
        coloured: format!("{}-h{}, {}--help{}", t.short_option, t.reset, t.long_option, t.reset),
    }
}

fn main_usage(prog: &str, t: &Theme) -> String {
    let help = Part {
        plain: "[-h]".into(),
        coloured: format!("[{}-h{}]", t.summary_short_option, t.reset),
    };
    // The subparsers action is one usage part: "{choices} ...".
    usage(
        prog,
        &[
            help,
            Part {
                plain: "[--version]".into(),
                coloured: format!("[{}--version{}]", t.summary_long_option, t.reset),
            },
        ],
        &[coloured(&format!("{} ...", choices()), t.summary_action, t)],
        t,
    )
}

fn main_help(prog: &str, t: &Theme) -> String {
    let mut positional: Vec<Row> = vec![(2, coloured(&choices(), t.action, t), None)];
    positional.extend(
        COMMANDS
            .iter()
            .map(|c| (4, coloured(c.name, t.action, t), Some(c.help))),
    );
    let options: Vec<Row> = vec![
        (2, help_header(t), Some(HELP_HELP)),
        (
            2,
            coloured("--version", t.long_option, t),
            Some("show program's version number and exit"),
        ),
    ];
    let all: Vec<Row> = positional.iter().chain(&options).cloned().collect();
    let (positional, options) = (action_rows(&positional, &all), action_rows(&options, &all));
    let description = wrap(DESCRIPTION, width().max(11)).join("\n");
    let heading = |text: &str| format!("{}{text}:{}", t.heading, t.reset);
    format!(
        "{}\n\n{description}\n\n{}\n{positional}\n{}\n{options}",
        main_usage(prog, t),
        heading("positional arguments"),
        heading("options")
    )
}

fn command_usage(prog: &str, command: &Command, t: &Theme) -> String {
    let mut parts = vec![Part {
        plain: "[-h]".into(),
        coloured: format!("[{}-h{}]", t.summary_short_option, t.reset),
    }];
    for option in command.options {
        let plain = format!("{} {}", option.flag, option.dest);
        let colour = format!(
            "{}{} {}{}{}",
            t.summary_long_option, option.flag, t.summary_label, option.dest, t.reset
        );
        parts.push(if option.required {
            Part {
                plain,
                coloured: colour,
            }
        } else {
            Part {
                plain: format!("[{plain}]"),
                coloured: format!("[{colour}]"),
            }
        });
    }
    usage(&format!("{prog} {}", command.name), &parts, &[], t)
}

fn command_help(prog: &str, command: &Command, t: &Theme) -> String {
    let mut rows = vec![(2, help_header(t), Some(HELP_HELP))];
    rows.extend(command.options.iter().map(|o| {
        let plain = format!("{} {}", o.flag, o.dest);
        let colour = format!(
            "{}{}{} {}{}{}",
            t.long_option, o.flag, t.reset, t.label, o.dest, t.reset
        );
        (
            2,
            Part {
                plain,
                coloured: colour,
            },
            o.help,
        )
    }));
    format!(
        "{}\n\n{}options:{}\n{}",
        command_usage(prog, command, t),
        t.heading,
        t.reset,
        action_rows(&rows, &rows)
    )
}

pub struct Parsed {
    pub command: &'static str,
    pub values: BTreeMap<&'static str, String>,
}

impl Parsed {
    pub fn path(&self, flag: &str) -> PathBuf {
        let text = crate::pypath::display(&self.values[flag]);
        PathBuf::from(crate::pypath::encode_os(&text))
    }
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
    if pytext::py_len(token) == 1 {
        return Ok(Class::Positional);
    }
    if let Some((name, value)) = token.split_once('=')
        && let Some(exact) = flags.iter().find(|f| **f == name)
    {
        return Ok(Class::Flag(exact, Some((value.to_string(), true))));
    }
    let mut found = option_prefix_matches(token, flags);
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

/// argparse's `_get_option_tuples`: the flags a token abbreviates, with any explicit value.
fn option_prefix_matches<'a>(token: &str, flags: &[&'a str]) -> Vec<(&'a str, Option<(String, bool)>)> {
    if token.starts_with("--") {
        let (prefix, explicit) = match token.split_once('=') {
            Some((name, value)) => (name, Some((value.to_string(), true))),
            None => (token, None),
        };
        return flags
            .iter()
            .filter(|f| f.starts_with(prefix))
            .map(|f| (*f, explicit.clone()))
            .collect();
    }
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
}

/// argparse's `_negative_number_matcher` on the text after the leading '-'.
fn looks_like_negative_number(body: &str) -> bool {
    let digit = |c: char| pytext::decimal_value(c as u32).is_some();
    (!body.is_empty() && body.chars().all(digit))
        || body
            .split_once('.')
            .is_some_and(|(int, frac)| int.chars().all(digit) && !frac.is_empty() && frac.chars().all(digit))
}

/// argparse's handling of an explicit argument given to the zero-argument help flag.
fn help_or_error(flag: &str, explicit: Option<(String, bool)>) -> Result<(), String> {
    match explicit {
        None => Ok(()),
        // "-hx": the rest is read as further short flags, and help runs first.
        Some((rest, false)) if flag == "-h" && !rest.is_empty() && !rest.starts_with('-') => Ok(()),
        Some((rest, _)) => Err(format!(
            "argument -h/--help: ignored explicit argument {}",
            pytext::repr_os(&rest)
        )),
    }
}

fn main_exit(prog: &str, message: String) -> Exit {
    Exit::Error {
        usage: main_usage(prog, argtheme::theme(Stream::Stderr)),
        prog: prog.to_string(),
        message,
    }
}

fn sub_exit(prog: &str, command: &Command, message: String) -> Exit {
    Exit::Error {
        usage: command_usage(prog, command, argtheme::theme(Stream::Stderr)),
        prog: format!("{prog} {}", command.name),
        message,
    }
}

fn parse(prog: &str, args: &[String]) -> Result<Parsed, Exit> {
    let mut extras: Vec<String> = Vec::new();
    let (command, index) = parse_command(prog, args, &mut extras)?;
    let values = parse_options(prog, command, &args[index..], &mut extras)?;
    let missing: Vec<&str> = command
        .options
        .iter()
        .filter(|o| o.required && !values.contains_key(o.flag))
        .map(|o| o.flag)
        .collect();
    if !missing.is_empty() {
        return Err(sub_exit(
            prog,
            command,
            format!("the following arguments are required: {}", missing.join(", ")),
        ));
    }
    if !extras.is_empty() {
        return Err(main_exit(prog, format!("unrecognized arguments: {}", extras.join(" "))));
    }
    Ok(Parsed {
        command: command.name,
        values,
    })
}

/// The main parser up to the subcommand: its help flag, unknown options kept as
/// extras, and the command choice. Returns the command and the index after it.
fn parse_command(prog: &str, args: &[String], extras: &mut Vec<String>) -> Result<(&'static Command, usize), Exit> {
    let mut index = 0;
    let mut options_ended = false;
    loop {
        let Some(token) = args.get(index) else {
            return Err(main_exit(prog, "the following arguments are required: command".into()));
        };
        index += 1;
        if !options_ended {
            if token == "--" {
                options_ended = true;
                continue;
            }
            match classify(token, &["-h", "--help", "--version"]).map_err(|m| main_exit(prog, m))? {
                Class::Flag(flag, explicit) => {
                    if flag == "--version" {
                        if let Some((value, _)) = explicit {
                            return Err(main_exit(
                                prog,
                                format!(
                                    "argument --version: ignored explicit argument {}",
                                    pytext::repr_os(&value)
                                ),
                            ));
                        }
                        return Err(Exit::Help(format!("{}\n", crate::disclosure::VERSION)));
                    }
                    help_or_error(flag, explicit).map_err(|m| main_exit(prog, m))?;
                    return Err(Exit::Help(main_help(prog, argtheme::theme(Stream::Stdout))));
                }
                Class::UnknownOption => {
                    extras.push(token.clone());
                    continue;
                }
                Class::Positional => {}
            }
        }
        return match COMMANDS.iter().find(|c| c.name == token) {
            Some(command) => Ok((command, index)),
            None => Err(main_exit(prog, invalid_choice(token))),
        };
    }
}

/// argparse's invalid-choice message, with Python 3.15's close-match suggestion.
fn invalid_choice(token: &str) -> String {
    let names: Vec<&'static str> = COMMANDS.iter().map(|c| c.name).collect();
    let quoted: Vec<String> = names.iter().map(|n| pytext::repr(n)).collect();
    format!(
        "argument command: invalid choice: {} (choose from {})",
        pytext::repr_os(token),
        quoted.join(", ")
    )
}

/// The subcommand's options after its name.
fn parse_options(
    prog: &str,
    command: &Command,
    args: &[String],
    extras: &mut Vec<String>,
) -> Result<BTreeMap<&'static str, String>, Exit> {
    let sub_error = |message: String| sub_exit(prog, command, message);
    let mut flags: Vec<&'static str> = vec!["-h", "--help"];
    flags.extend(command.options.iter().map(|o| o.flag));
    let mut values: BTreeMap<&'static str, String> = BTreeMap::new();
    let mut index = 0;
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
            return Err(Exit::Help(command_help(prog, command, argtheme::theme(Stream::Stdout))));
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
    Ok(values)
}

fn private(args: &[String]) -> bool {
    args.iter().any(|arg| {
        arg.starts_with("disclosure-")
            || arg.starts_with("--")
                && arg != "--"
                && ["--destination", "--decision-ref", "--record"]
                    .iter()
                    .any(|option| option.starts_with(arg.split('=').next().unwrap()))
    })
}

pub struct Outcome {
    pub code: i32,
    pub argparse: bool,
}

pub fn main(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    run(args, stdout, stderr).code
}

pub fn run(args: &[String], stdout: &mut dyn Write, stderr: &mut dyn Write) -> Outcome {
    let parsed = match parse("evatt", args) {
        Ok(parsed) => parsed,
        Err(Exit::Help(text)) => {
            let _ = write!(stdout, "{text}");
            return Outcome {
                code: 0,
                argparse: true,
            };
        }
        Err(Exit::Error { usage, prog, message }) => {
            let message = if prog == "evatt" && private(args)
                || prog.ends_with(" disclosure-record")
                || prog.ends_with(" disclosure-check")
            {
                "invalid disclosure arguments".into()
            } else {
                message
            };
            let _ = write!(stderr, "{usage}\n{prog}: error: {message}\n");
            return Outcome {
                code: 1,
                argparse: true,
            };
        }
    };
    Outcome {
        code: crate::commands::run(&parsed, stdout, stderr),
        argparse: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostics(args: &[&str]) -> String {
        let args = args.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        assert_eq!(main(&args, &mut stdout, &mut stderr), 1);
        assert!(stdout.is_empty());
        String::from_utf8(stderr).unwrap()
    }

    #[test]
    fn private_parent_diagnostics_do_not_change_ordinary_child_parsers() {
        let child = diagnostics(&["redact", "--record"]);
        assert!(child.contains("the following arguments are required: --in, --map, --out"));
        assert!(!child.contains("invalid disclosure arguments"));
        let parent = diagnostics(&[
            "redact",
            "--in",
            "in.md",
            "--map",
            "entities.json",
            "--out",
            "out.md",
            "--record",
            "Sample Sensitive Sentinel",
        ]);
        assert!(parent.contains("invalid disclosure arguments"));
        assert!(!parent.contains("Sample Sensitive Sentinel"));
        assert!(diagnostics(&["disclosure-record", "--in"]).contains("invalid disclosure arguments"));
    }
}
