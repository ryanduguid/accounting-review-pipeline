use super::{Result, export, files, html, json, projection, run};
use std::{collections::BTreeMap, ffi::OsString, fs, io::Write, path::Path};

const COMMANDS: [&str; 6] = ["build", "html", "fixtures", "verify", "export", "verify-export"];
const OPTIONS: [&str; 6] = ["run", "bin-dir", "month", "review-note", "previous", "output"];
const USAGE: &str = "usage: review-workflow [-h] --run RUN [--bin-dir BIN_DIR] [--month MONTH] [--review-note REVIEW_NOTE] [--previous PREVIOUS] [--output OUTPUT] {build,html,fixtures,verify,export,verify-export}";
const HELP: &str = "Build and display one pinned, offline fabricated review case through existing CLIs.\n\npositional arguments:\n  {build,html,fixtures,verify,export,verify-export}\n\noptions:\n  -h, --help            show this help message and exit\n  --run RUN\n  --bin-dir BIN_DIR\n  --month MONTH         default: 9\n  --review-note REVIEW_NOTE\n  --previous PREVIOUS\n  --output OUTPUT\n";

struct Arguments {
    command: String,
    values: BTreeMap<&'static str, OsString>,
    month: u32,
}

impl Arguments {
    fn path(&self, name: &str) -> Option<&Path> {
        self.values.get(name).map(Path::new)
    }
}

fn parse(args: &[OsString]) -> Result<Arguments> {
    let (mut values, mut command, mut end) = (BTreeMap::new(), None, false);
    let mut args = args.iter().peekable();
    while let Some(raw) = args.next() {
        let text = raw.to_str().ok_or("Command options require valid Unicode.")?;
        if text == "--" && !end {
            end = true;
            continue;
        }
        if !end && text.starts_with('-') {
            let (name, value) = text
                .split_once('=')
                .map_or((text, None), |(key, value)| (key, Some(value)));
            let key = option(name)?;
            let value = if let Some(value) = value {
                OsString::from(value)
            } else {
                if args.peek().is_none_or(|value| {
                    value
                        .to_str()
                        .is_some_and(|value| value != "-" && value.starts_with('-') && !negative_number(value))
                }) {
                    return Err(format!("argument --{key}: expected one argument"));
                }
                args.next().unwrap().clone()
            };
            values.insert(key, value);
        } else if command.replace(text.to_owned()).is_some() {
            return Err(format!("unrecognised arguments: {text}"));
        }
    }
    let command = command.ok_or("the following arguments are required: command")?;
    if !COMMANDS.contains(&command.as_str()) {
        return Err(format!("argument command: invalid choice: {command:?}"));
    }
    if !values.contains_key("run") {
        return Err("the following arguments are required: --run".into());
    }
    let month = values
        .get("month")
        .map(|raw| {
            raw.to_str()
                .ok_or_else(|| "--month requires an integer.".to_owned())
                .and_then(month)
        })
        .transpose()?
        .unwrap_or(9);
    Ok(Arguments { command, values, month })
}

fn option(name: &str) -> Result<&'static str> {
    let prefix = name
        .strip_prefix("--")
        .ok_or_else(|| format!("unrecognised arguments: {name}"))?;
    if let Some(exact) = OPTIONS.iter().find(|option| **option == prefix) {
        return Ok(exact);
    }
    let matches: Vec<_> = OPTIONS
        .iter()
        .copied()
        .filter(|option| option.starts_with(prefix))
        .collect();
    match matches.as_slice() {
        [name] => Ok(name),
        [] => Err(format!("unrecognised arguments: {name}")),
        _ => Err(format!("ambiguous option: {name}")),
    }
}

// Copied from the gate's CPython 3.15 / Unicode 17 decimal-digit table.
const DECIMAL_ZEROS: &[u32] = &[
    0x30, 0x660, 0x6F0, 0x7C0, 0x966, 0x9E6, 0xA66, 0xAE6, 0xB66, 0xBE6, 0xC66, 0xCE6, 0xD66, 0xDE6, 0xE50, 0xED0,
    0xF20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620,
    0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0, 0xFF10, 0x104A0, 0x10D30, 0x10D40, 0x11066, 0x110F0, 0x11136,
    0x111D0, 0x112F0, 0x11450, 0x114D0, 0x11650, 0x116C0, 0x116D0, 0x116DA, 0x11730, 0x118E0, 0x11950, 0x11BF0,
    0x11C50, 0x11D50, 0x11DA0, 0x11DE0, 0x11F50, 0x16130, 0x16A60, 0x16AC0, 0x16B50, 0x16D70, 0x1CCF0, 0x1D7CE,
    0x1D7D8, 0x1D7E2, 0x1D7EC, 0x1D7F6, 0x1E140, 0x1E2F0, 0x1E4F0, 0x1E5F1, 0x1E950, 0x1FBF0,
];

fn month(text: &str) -> Result<u32> {
    let text = text.trim_matches(char::is_whitespace);
    let (negative, digits) = text
        .strip_prefix('-')
        .map_or((false, text.strip_prefix('+').unwrap_or(text)), |text| (true, text));
    let (mut value, mut previous_digit, mut count) = (0u32, false, 0);
    for character in digits.chars() {
        if character == '_' && previous_digit {
            previous_digit = false;
            continue;
        }
        let digit = decimal_digit(character).ok_or("--month requires an integer.")?;
        value = value.saturating_mul(10).saturating_add(digit);
        count += 1;
        previous_digit = true;
    }
    if !previous_digit || count > 4300 {
        return Err("--month requires an integer.".into());
    }
    Ok(if negative { 0 } else { value })
}

fn decimal_digit(character: char) -> Option<u32> {
    let cp = character as u32;
    let index = DECIMAL_ZEROS.partition_point(|zero| *zero <= cp);
    index
        .checked_sub(1)
        .map(|index| DECIMAL_ZEROS[index])
        .filter(|zero| cp - zero < 10)
        .map(|zero| cp - zero)
}

fn negative_number(text: &str) -> bool {
    let Some(text) = text.strip_prefix('-') else {
        return false;
    };
    let digits = |text: &str| text.chars().all(|character| decimal_digit(character).is_some());
    if let Some((whole, fraction)) = text.split_once('.') {
        digits(whole) && !fraction.is_empty() && digits(fraction)
    } else {
        !text.is_empty() && digits(text)
    }
}

fn execute(args: &Arguments) -> Result<String> {
    let run = args.path("run").unwrap();
    let previous = args.path("previous");
    let note = args.path("review-note");
    let output = args.path("output");
    if args.command == "verify-export" {
        if previous.is_some() || note.is_some() || output.is_some() {
            return Err("verify-export does not support --previous, --review-note or --output.".into());
        }
        return Ok(json::inline(&export::verify(run)?));
    }
    // The executable directory locates sibling producers. It grants no privilege.
    // nosemgrep: rust.lang.security.current-exe.current-exe
    let default = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .parent()
        .ok_or("Cannot locate the native producer directory.")?
        .to_path_buf();
    let bin_dir = args.path("bin-dir").unwrap_or(&default);
    match args.command.as_str() {
        "export" => {
            if output.is_none() || previous.is_some() || note.is_some() {
                return Err(
                    "Export requires --output and supports one run without --previous or --review-note.".into(),
                );
            }
            export::export(run, bin_dir, output.unwrap())?;
        }
        "build" => run::build(run, bin_dir, args.month, note)?,
        "verify" => {
            run::verify(run)?;
        }
        "html" => html_output(run, bin_dir, previous, output.ok_or("--output is required.")?)?,
        "fixtures" => fixture_output(run, output.ok_or("--output is required.")?)?,
        _ => unreachable!("validated command"),
    }
    Ok("Review workflow verification completed.".into())
}

fn html_output(run: &Path, bin_dir: &Path, previous: Option<&Path>, output: &Path) -> Result<()> {
    let output = files::new_output(output)?;
    let content = html::render(run, bin_dir, previous)?;
    fs::create_dir_all(output.parent().unwrap()).map_err(|error| error.to_string())?;
    let content = if cfg!(windows) {
        content.replace('\n', "\r\n")
    } else {
        content
    };
    files::write_new(&output, content.as_bytes())
}

fn fixture_output(run: &Path, output: &Path) -> Result<()> {
    let output = files::new_output(output)?;
    let values = projection::projection(run)?;
    fs::create_dir_all(output.parent().unwrap()).map_err(|error| error.to_string())?;
    fs::create_dir(&output).map_err(|error| error.to_string())?;
    for (name, value) in values {
        files::write_new(&output.join(name), &value)?;
    }
    Ok(())
}

pub fn entry(args: Vec<OsString>, stdout: &mut dyn Write, stderr: &mut dyn Write) -> u8 {
    let result = if args
        .iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--help" || arg == "-h")
    {
        Ok(format!("{USAGE}\n\n{HELP}"))
    } else if args == [OsString::from("--version")] {
        Ok(format!("review-workflow {}\n", env!("CARGO_PKG_VERSION")))
    } else {
        match parse(&args) {
            Err(message) => Err((2, format!("{USAGE}\nreview-workflow: error: {message}\n"))),
            Ok(args) => execute(&args)
                .map(|text| format!("{text}\n"))
                .map_err(|message| (1, format!("Review workflow refused: {message}\n"))),
        }
    };
    let (code, stream, text): (u8, &mut dyn Write, _) = match result {
        Ok(text) => (0, stdout, text),
        Err((code, text)) => (code, stderr, text),
    };
    let text = if cfg!(windows) {
        text.replace('\n', "\r\n")
    } else {
        text
    };
    if stream.write_all(text.as_bytes()).is_err() {
        1
    } else {
        code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_preserve_option_aliases_integer_forms_and_pre_access_refusals() {
        for text in ["8", "+08", "\u{2000}８\u{2000}", "0_9", "۹"] {
            assert!([8, 9].contains(&month(text).unwrap()));
        }
        for text in ["", "+", "8_", "8__0", "_8", "8.0", "\u{1c}8"] {
            assert!(month(text).is_err(), "{text:?}");
        }
        for arguments in [
            vec!["verify"],
            vec!["verify", "--run", "unused", "--month", "bad"],
            vec!["verify", "--r", "unused"],
            vec!["verify-export", "--run", "unused", "--previous", "unused"],
        ] {
            let (mut out, mut err) = (Vec::new(), Vec::new());
            assert_ne!(
                entry(arguments.into_iter().map(OsString::from).collect(), &mut out, &mut err),
                0
            );
            assert!(out.is_empty());
            assert!(!err.is_empty());
        }
        let args = [
            "verify",
            "--run=unused",
            "--month",
            "999999999999999999999",
            "--prev",
            "unused",
        ]
        .map(OsString::from);
        let parsed = parse(&args).unwrap();
        assert_eq!(parsed.path("previous"), Some(Path::new("unused")));
        assert!(parse(&["verify", "--run", "unused", "--month", "8"].map(OsString::from)).is_ok());
    }

    #[test]
    fn failed_output_is_observable() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert_eq!(entry(vec!["--help".into()], &mut Closed, &mut Vec::new()), 1);
    }

    #[test]
    fn option_values_and_end_marker_follow_argument_classification() {
        for value in ["-8", "-۸", "-.5", "-1.2"] {
            assert!(negative_number(value));
            assert!(parse(&["verify", "--run", value].map(OsString::from)).is_ok());
        }
        for value in ["-other", "-1e2", "--unknown", "-1."] {
            assert!(parse(&["verify", "--run", value].map(OsString::from)).is_err());
        }
        assert!(parse(&["verify", "--run=-other"].map(OsString::from)).is_ok());
        assert!(parse(&["verify", "--run", "-"].map(OsString::from)).is_ok());
        assert_eq!(
            entry(
                ["--", "--help"].map(OsString::from).to_vec(),
                &mut Vec::new(),
                &mut Vec::new()
            ),
            2
        );
    }
}
