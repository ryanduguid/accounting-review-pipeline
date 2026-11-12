// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

#[path = "../../../tests/support/child_profile.rs"]
mod child_profile;

#[cfg(windows)]
#[path = "../../../tests/support/windows_console.rs"]
mod windows_console;

use std::process::Output;

// Captured from CPython 3.15 argparse with this command's published options.
const HELP20: &str = if cfg!(windows) {
    include_str!("fixtures/cli/help-20-windows.txt")
} else {
    include_str!("fixtures/cli/help-20.txt")
};
const HELP40: &str = if cfg!(windows) {
    include_str!("fixtures/cli/help-40-windows.txt")
} else {
    include_str!("fixtures/cli/help-40.txt")
};
const HELP80: &str = if cfg!(windows) {
    include_str!("fixtures/cli/help-80-windows.txt")
} else {
    include_str!("fixtures/cli/help-80.txt")
};
const PROGRAM: &str = if cfg!(windows) {
    "grant-workpaper.exe"
} else {
    "grant-workpaper"
};
const COLOUR_ENV: [&str; 4] = ["PYTHON_COLORS", "NO_COLOR", "FORCE_COLOR", "TERM"];

#[cfg(windows)]
#[test]
fn real_console_uses_native_width_and_colours_on_both_streams() {
    let usage = "usage: grant-workpaper.exe [-h] --input INPUT --output OUTPUT\n\
                 grant-workpaper.exe: error: the following arguments are required: --input, --output\n";
    windows_console::check(
        "real_console_uses_native_width_and_colours_on_both_streams",
        env!("CARGO_BIN_EXE_grant-workpaper"),
        HELP80,
        usage,
        2,
    );
}

fn command(args: &[&str], variables: &[(&str, &str)]) -> Output {
    let mut command = child_profile::command(env!("CARGO_BIN_EXE_grant-workpaper"));
    for name in COLOUR_ENV.into_iter().chain(["COLUMNS", "LINES", "PYTHONUNBUFFERED"]) {
        command.env_remove(name);
    }
    command
        .args(args)
        .env("COLUMNS", "80")
        .envs(variables.iter().copied())
        .output()
        .unwrap()
}

fn plain(bytes: &[u8]) -> String {
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let mut parts = text.split('\x1b');
    let mut result = parts.next().unwrap().to_string();
    for part in parts {
        let (sequence, rest) = part.split_once('m').expect("complete ANSI colour sequence");
        assert!(sequence.starts_with('['));
        assert!(sequence[1..].bytes().all(|byte| byte.is_ascii_digit() || byte == b';'));
        result.push_str(rest);
    }
    result.replace("\r\n", "\n")
}

#[test]
fn help_wraps_like_python_at_narrow_and_normal_widths() {
    for (width, expected) in [("20", HELP20), ("40", HELP40), ("80", HELP80)] {
        let output = command(
            &["--help"],
            &[("PYTHON_COLORS", "0"), ("COLUMNS", width), ("LINES", "24")],
        );
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty(), "{}", plain(&output.stderr));
        assert_eq!(plain(&output.stdout), expected, "COLUMNS={width}");
    }
}

#[test]
fn help_and_argument_refusals_match_the_pinned_argparse_contract() {
    let help = command(&["-hx"], &[("PYTHON_COLORS", "0")]);
    assert_eq!(help.status.code(), Some(0));
    assert_eq!(plain(&help.stdout), HELP80);
    assert!(help.stderr.is_empty());
    for (args, diagnostic) in [
        (vec!["--help=1"], "argument -h/--help: ignored explicit argument '1'"),
        (vec!["-h=1"], "argument -h/--help: ignored explicit argument '1'"),
        (
            vec!["--input", "--output", "out"],
            "argument --input: expected one argument",
        ),
    ] {
        let output = command(&args, &[("PYTHON_COLORS", "0")]);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(
            plain(&output.stderr),
            format!("usage: {PROGRAM} [-h] --input INPUT --output OUTPUT\n{PROGRAM}: error: {diagnostic}\n")
        );
    }
    for token in ["-", "-1", "--bogus", "--"] {
        let output = command(
            &["--input", "missing-input", "--output", "out", token],
            &[("PYTHON_COLORS", "0")],
        );
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(
            plain(&output.stderr),
            format!(
                "usage: {PROGRAM} [-h] --input INPUT --output OUTPUT\n{PROGRAM}: error: unrecognized arguments: {token}\n"
            )
        );
    }
}

#[test]
fn missing_or_invalid_dimensions_keep_the_python_fallbacks() {
    for (columns, lines, expected) in [
        ("40", None, HELP40),
        ("invalid", Some("24"), HELP80),
        ("0", Some("0"), HELP80),
    ] {
        let mut variables = vec![("PYTHON_COLORS", "0"), ("COLUMNS", columns)];
        if let Some(lines) = lines {
            variables.push(("LINES", lines));
        }
        let output = command(&["--help"], &variables);
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty(), "{}", plain(&output.stderr));
        assert_eq!(plain(&output.stdout), expected, "COLUMNS={columns}, LINES={lines:?}");
    }
}

#[test]
fn colour_overrides_have_python_precedence_on_both_streams() {
    let required = format!(
        "usage: {PROGRAM} [-h] --input INPUT --output OUTPUT\n\
         {PROGRAM}: error: the following arguments are required: --input, --output\n"
    );
    let cases = [
        ([Some("0"), None, Some("1"), None], false),
        ([Some("1"), Some("1"), None, Some("dumb")], true),
        ([None, Some("1"), Some("1"), None], false),
        ([None, None, Some("1"), Some("dumb")], true),
        ([None, None, None, Some("dumb")], false),
    ];
    for (values, coloured) in cases {
        let variables: Vec<_> = COLOUR_ENV
            .into_iter()
            .zip(values)
            .filter_map(|(name, value)| value.map(|value| (name, value)))
            .collect();
        for args in [&["--help"][..], &[][..]] {
            let output = command(args, &variables);
            let (stream, other, code, expected) = if args.is_empty() {
                (&output.stderr, &output.stdout, 2, required.as_str())
            } else {
                (&output.stdout, &output.stderr, 0, HELP80)
            };
            assert_eq!(output.status.code(), Some(code));
            assert!(other.is_empty());
            assert_eq!(stream.contains(&b'\x1b'), coloured, "{values:?}");
            assert_eq!(plain(stream), expected, "{values:?}");
        }
    }
}
