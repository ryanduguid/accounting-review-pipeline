// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use crate::{
    Failure, Result,
    config::Environment,
    export,
    http::{self, NativeHttp},
    paths, pypath, pytext,
    tokens::TokenSession,
};
use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    thread,
};

pub struct Options {
    pub date: String,
    pub tenant: Option<String>,
    pub out: Option<PathBuf>,
    pub token: Option<PathBuf>,
    pub cash: bool,
    pub quiet: bool,
    pub no_manifest: bool,
}

const OPTIONS: [(&str, Option<&str>, &str); 8] = [
    ("--help", None, "show this help message and exit"),
    ("--date", Some("DATE"), "Report date YYYY-MM-DD"),
    (
        "--tenant",
        Some("TENANT"),
        "Tenant name substring or exact tenantId (required when multiple orgs are connected)",
    ),
    (
        "--out",
        Some("OUT"),
        "Output CSV path relative to the current working directory",
    ),
    ("--payments-only", None, "Cash-basis report"),
    (
        "--quiet",
        None,
        "Do not print the organisation name, the export totals or the written filenames, which carry the organisation name by default. The output directory, the account count and every warning still print.",
    ),
    (
        "--no-manifest",
        None,
        "Do not write the <out>.manifest.json that records the tenant, basis and SHA-256 of the export",
    ),
    (
        "--token-file",
        Some("TOKEN_FILE"),
        "Path to the token cache (token.json). This flag takes precedence over the XERO_TOKEN_FILE environment variable; the default is the per-user state directory (or a leftover executable-adjacent token.json).",
    ),
];

fn width() -> usize {
    usize::try_from(crate::termsize::columns() - 2).unwrap_or(0)
}

fn usage(prog: &str, auth: bool, width: usize) -> String {
    let parts: Vec<_> = if auth {
        vec!["[-h]".to_string()]
    } else {
        OPTIONS
            .iter()
            .map(|(flag, value, _)| match (flag, value) {
                (&"--help", _) => "[-h]".into(),
                (_, Some(value)) => format!("[{flag} {value}]"),
                _ => format!("[{flag}]"),
            })
            .collect()
    };
    let full = format!("usage: {prog} {}", parts.join(" "));
    if pytext::py_len(&full) <= width {
        return format!("{full}\n");
    }
    let long = (7 + pytext::py_len(prog)) * 4 > width * 3;
    let indent = " ".repeat(if long {
        7
    } else {
        7 + pytext::py_len(prog) + 1
    });
    let mut line = if long {
        indent.clone()
    } else {
        format!("usage: {prog}")
    };
    let mut lines = Vec::new();
    if long {
        lines.push(format!("usage: {prog}"));
    }
    for part in parts {
        if pytext::py_len(&line) + 1 + part.len() > width && line != indent {
            lines.push(line);
            line = indent.clone();
        }
        if !line.ends_with(' ') {
            line.push(' ');
        }
        line.push_str(&part);
    }
    lines.push(line);
    format!("{}\n", lines.join("\n"))
}

const LICENCE_NOTICE: &str = "\n\
    Copyright 2026 Ryan Duguid. No warranty.\n\
    AGPL-3.0-or-later with additional terms (see NOTICE).\n\
    Licence: https://github.com/ryanduguid/accounting-review-pipeline/blob/main/packages/xero-trial-balance-export/LICENSE\n";

pub fn help(prog: &str, auth: bool) -> String {
    let width = width();
    let description = if auth {
        "Authorise this exporter against Xero once, in a browser, and cache the token."
    } else {
        "Export a Xero Trial Balance to CSV."
    };
    let mut out = format!(
        "{}\n{}\n\noptions:\n",
        usage(prog, auth, width),
        crate::textwrap::wrap(description, width.max(11)).join("\n")
    );
    let options = if auth { &OPTIONS[..1] } else { &OPTIONS[..] };
    let position = if auth { 14 } else { 24 }.min(width.saturating_sub(20).max(4));
    for (flag, argument, text) in options {
        let header = if *flag == "--help" {
            "-h, --help".into()
        } else {
            argument
                .map(|a| format!("{flag} {a}"))
                .unwrap_or_else(|| flag.to_string())
        };
        let lines = crate::textwrap::wrap(text, width.saturating_sub(position).max(11));
        if header.len() <= position.saturating_sub(4) {
            out.push_str(&format!(
                "  {:width$}  {}\n",
                header,
                lines[0],
                width = position - 4
            ));
        } else {
            out.push_str(&format!(
                "  {header}\n{}{}\n",
                " ".repeat(position),
                lines[0]
            ));
        }
        for line in &lines[1..] {
            out.push_str(&format!("{}{line}\n", " ".repeat(position)));
        }
    }
    out.push_str(LICENCE_NOTICE);
    out
}

fn error(prog: &str, auth: bool, message: String) -> Failure {
    Failure {
        code: 2,
        stdout: String::new(),
        stderr: format!("{}{prog}: error: {message}\n", usage(prog, auth, width())),
    }
}

fn option_operand(value: &str) -> bool {
    let Some(suffix) = value.strip_prefix('-') else {
        return true;
    };
    // CPython 3.15 argparse recognises a negative number by its prefix.
    let number = suffix.strip_prefix('.').unwrap_or(suffix);
    suffix.is_empty()
        || value.contains(' ')
        || number
            .chars()
            .next()
            .is_some_and(|ch| pytext::decimal_value(ch as u32).is_some())
}

#[expect(
    clippy::too_many_lines,
    reason = "parses every command-line option in one loop"
)]
pub fn parse(args: &[OsString], prog: &str, auth: bool) -> Result<Option<Options>> {
    let mut options = Options {
        date: jiff::ZonedDateTime::now().date().to_string(),
        tenant: None,
        out: None,
        token: None,
        cash: false,
        quiet: false,
        no_manifest: false,
    };
    let values: Vec<_> = args.iter().map(|v| pypath::decode_os(v)).collect();
    let mut unknown = Vec::new();
    let mut index = 0;
    let mut positional = false;
    while index < values.len() {
        let value = &values[index];
        index += 1;
        if positional {
            unknown.push(value.clone());
            continue;
        }
        if value == "--" {
            positional = true;
            if index < values.len() {
                unknown.push(value.clone());
            }
            continue;
        }
        if let Some(explicit) = value.strip_prefix("-h=") {
            return Err(error(
                prog,
                auth,
                format!(
                    "argument -h/--help: ignored explicit argument {}",
                    pytext::repr(explicit)
                ),
            ));
        }
        if value == "-h" || (value.starts_with("-h") && !value.starts_with("--")) {
            return Ok(None);
        }
        let (flag, explicit) = value
            .split_once('=')
            .map(|(a, b)| (a, Some(b)))
            .unwrap_or((value, None));
        let available = if auth { &OPTIONS[..1] } else { &OPTIONS[..] };
        let mut found = available.iter().filter(|(name, _, _)| {
            *name == flag || (flag.starts_with("--") && name.starts_with(flag))
        });
        let Some((name, argument, _)) = found.next() else {
            unknown.push(value.clone());
            continue;
        };
        if let Some((second, _, _)) = found.next() {
            let names = [*name, *second]
                .into_iter()
                .chain(found.map(|(other, _, _)| *other))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(error(
                prog,
                auth,
                format!("ambiguous option: {flag} could match {names}"),
            ));
        }
        if *name == "--help" {
            if let Some(value) = explicit {
                return Err(error(
                    prog,
                    auth,
                    format!(
                        "argument -h/--help: ignored explicit argument {}",
                        pytext::repr(value)
                    ),
                ));
            }
            return Ok(None);
        }
        if argument.is_none() {
            if let Some(value) = explicit {
                return Err(error(
                    prog,
                    auth,
                    format!(
                        "argument {name}: ignored explicit argument {}",
                        pytext::repr(value)
                    ),
                ));
            }
            options.cash |= *name == "--payments-only";
            options.quiet |= *name == "--quiet";
            options.no_manifest |= *name == "--no-manifest";
            continue;
        }
        let value = if let Some(value) = explicit {
            value.to_string()
        } else {
            let Some(value) = values.get(index).filter(|s| option_operand(s)) else {
                return Err(error(
                    prog,
                    auth,
                    format!("argument {name}: expected one argument"),
                ));
            };
            index += 1;
            value.clone()
        };
        match *name {
            "--date" => options.date = iso_date(&value).map_err(|_| error(prog, auth, format!("argument --date: \"{value}\" is not YYYY-MM-DD (e.g. 2026-06-30). Note the order: ISO year-month-day, not the AU DD/MM/YYYY.")))?,
            "--tenant" => options.tenant = Some(value),
            "--out" => options.out = Some(PathBuf::from(pypath::encode_os(&value))),
            "--token-file" => options.token = Some(PathBuf::from(pypath::encode_os(&value))), _ => {}
        }
    }
    if !unknown.is_empty() {
        return Err(error(
            prog,
            auth,
            format!("unrecognized arguments: {}", unknown.join(" ")),
        ));
    }
    Ok(Some(options))
}

#[expect(
    clippy::too_many_lines,
    reason = "one date validation with its error messages"
)]
pub fn iso_date(value: &str) -> Result<String> {
    let failure = || {
        Failure::message(format!(
            "\"{value}\" is not YYYY-MM-DD (e.g. 2026-06-30). Note the order: ISO year-month-day, not the AU DD/MM/YYYY."
        ))
    };
    if !value.is_ascii()
        || !matches!(value.len(), 7 | 8 | 10)
        || !value[..4].bytes().all(|b| b.is_ascii_digit())
    {
        return Err(failure());
    }
    let year: i16 = value[..4].parse().map_err(|_| failure())?;
    if !(1..=9999).contains(&year) {
        return Err(failure());
    }
    let date = if value.len() == 10 && value.as_bytes()[4] == b'-' && value.as_bytes()[7] == b'-' {
        jiff::civil::Date::new(
            year,
            value[5..7].parse().map_err(|_| failure())?,
            value[8..10].parse().map_err(|_| failure())?,
        )
        .ok()
    } else if value.len() == 8 && value.bytes().all(|b| b.is_ascii_digit()) {
        jiff::civil::Date::new(
            year,
            value[4..6].parse().map_err(|_| failure())?,
            value[6..8].parse().map_err(|_| failure())?,
        )
        .ok()
    } else {
        let remainder = value[4..].strip_prefix('-').unwrap_or(&value[4..]);
        let week = remainder.strip_prefix('W').ok_or_else(failure)?;
        if week.len() < 2 {
            return Err(failure());
        }
        let number: i8 = week[..2].parse().map_err(|_| failure())?;
        let remaining = &week[2..];
        if !remaining.is_empty() && remaining.starts_with('-') != (value.as_bytes()[4] == b'-') {
            return Err(failure());
        }
        let rest = remaining.strip_prefix('-').unwrap_or(remaining);
        if rest.len() > 1 {
            return Err(failure());
        }
        let day = if rest.is_empty() {
            1
        } else {
            rest[..1].parse().map_err(|_| failure())?
        };
        let weekday = jiff::civil::Weekday::from_monday_one_offset(day).map_err(|_| failure())?;
        jiff::civil::ISOWeekDate::new(year, number, weekday)
            .ok()
            .map(jiff::civil::ISOWeekDate::date)
    };
    date.filter(|d| (1..=9999).contains(&d.year()))
        .map(|d| d.to_string())
        .ok_or_else(failure)
}

#[expect(clippy::too_many_lines, reason = "runs the export or sign-in command")]
pub fn run(args: &[OsString], auth: bool) -> Result<()> {
    let prog = if auth { "xero-tb-auth" } else { "export-tb" };
    let Some(options) = parse(args, prog, auth)? else {
        crate::out!("{}", help(prog, auth));
        return Ok(());
    };
    let cwd = std::env::current_dir().map_err(|e| Failure::message(e.to_string()))?;
    if !auth {
        let preflight = if options.out.is_some() {
            paths::output_path(options.out.as_deref(), "unused.csv", &cwd).and_then(|out| {
                paths::require_ignored_output(&out, !options.no_manifest, options.quiet, false)
            })
        } else {
            paths::require_output_directory(&cwd, !options.no_manifest, options.quiet)
        };
        preflight.map_err(|e| error(prog, false, e.stderr.trim_end().into()))?;
    }
    let environment = Environment::load(Path::new(".env"))?;
    let token = options.token.or_else(|| {
        environment
            .get("XERO_TOKEN_FILE")
            .filter(|v| !pytext::strip(v).is_empty())
            .map(|v| PathBuf::from(pypath::encode_os(&v)))
    });
    let session = TokenSession::new(&paths::token_path(token.as_deref())?)?;
    let credentials = environment.credentials()?;
    let http = NativeHttp::new(&environment)?;
    if auth {
        return crate::auth::authorise(
            &http,
            &session,
            &credentials,
            &environment
                .get("XERO_REDIRECT_URI")
                .unwrap_or_else(|| "http://localhost:8400/callback".into()),
        );
    }
    let connections = http::api_get(
        &http,
        &session,
        &credentials,
        http::CONNECTIONS_URL,
        None,
        &[],
    )?;
    let connections = export::validated_connections(&connections)?;
    let tenant = export::select_tenant(connections, options.tenant.as_deref())?;
    if !options.quiet {
        crate::outln!("Tenant: {}", crate::report::text(tenant.at("tenantName")));
    }
    let rows = export::fetch_rows(
        &http,
        &session,
        &credentials,
        tenant,
        &options.date,
        options.cash,
    )?;
    let name = export::default_filename(
        tenant,
        &options.date,
        if options.cash { "cash" } else { "accrual" },
    );
    let out = paths::output_path(options.out.as_deref(), &name, &cwd)
        .map_err(|e| error(prog, false, e.stderr.trim_end().into()))?;
    paths::require_ignored_output(&out, !options.no_manifest, options.quiet, false)
        .map_err(|e| error(prog, false, e.stderr.trim_end().into()))?;
    export::publish(
        &rows,
        tenant,
        &options.date,
        &out,
        options.cash,
        options.quiet,
        options.no_manifest,
    )
}

fn finish_worker(worker: io::Result<thread::JoinHandle<Result<()>>>) -> Result<()> {
    let worker = worker.map_err(|error| {
        Failure::message(format!("error: cannot start the Xero command ({error})"))
    })?;
    worker.join().unwrap_or_else(|_| {
        Err(Failure::message(
            "error: the Xero command failed unexpectedly.",
        ))
    })
}

pub fn entry(auth: bool) {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let worker = thread::Builder::new()
        .name("xero-command".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || run(&args, auth));
    let result = finish_worker(worker);
    if let Err(error) = result {
        crate::stdio::out(&error.stdout);
        crate::stdio::err(&error.stderr);
        std::process::exit(error.code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_failure_and_worker_panic_return_distinct_command_errors() {
        let start = finish_worker(Err(io::Error::other("synthetic startup failure")))
            .err()
            .unwrap();
        assert_eq!(start.code, 1);
        assert!(start.stdout.is_empty());
        assert_eq!(
            start.stderr,
            "error: cannot start the Xero command (synthetic startup failure)\n"
        );

        let worker = thread::spawn(|| panic!("synthetic worker panic"));
        let joined = finish_worker(Ok(worker)).err().unwrap();
        assert_eq!(joined.code, 1);
        assert!(joined.stdout.is_empty());
        assert_eq!(
            joined.stderr,
            "error: the Xero command failed unexpectedly.\n"
        );
    }

    #[test]
    fn options_preflight_and_iso_forms() {
        for (value, date) in [
            ("2026-06-30", "2026-06-30"),
            ("20260630", "2026-06-30"),
            ("2026-W27-2", "2026-06-30"),
        ] {
            assert_eq!(iso_date(value).unwrap(), date);
        }
        for invalid in ["30/06/2026", "0000-01-01", "2026-02-29"] {
            assert!(iso_date(invalid).is_err());
        }
        assert!(
            parse(&["--qu".into()], "export-tb", false)
                .unwrap()
                .unwrap()
                .quiet
        );
        assert!(parse(&["--out".into(), "--quiet".into()], "export-tb", false).is_err());
        assert!(
            parse(&["unknown".into(), "--help".into()], "export-tb", false)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn option_prefixes_keep_diagnostics_and_command_scope() {
        for flag in ["--t", "--t=Demo"] {
            let failure = parse(&[flag.into()], "export-tb", false).err().unwrap();
            assert_eq!(failure.code, 2);
            assert!(
                failure
                    .stderr
                    .ends_with("ambiguous option: --t could match --tenant, --token-file\n")
            );
        }
        let options = parse(&["--te=Demo".into(), "--qu".into()], "export-tb", false)
            .unwrap()
            .unwrap();
        assert_eq!(options.tenant.as_deref(), Some("Demo"));
        assert!(options.quiet);
        let failure = parse(&["--t".into()], "xero-auth", true).err().unwrap();
        assert_eq!(failure.code, 2);
        assert!(failure.stderr.ends_with("unrecognized arguments: --t\n"));
        assert!(
            parse(&["--he".into()], "xero-auth", true)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn option_operands_follow_argparse_315_negative_number_prefixes() {
        for value in [
            "-7.5.2", "-.5part", "-４", "-.٤", "-7.5", "-", "- demo", "Demo",
        ] {
            let options = parse(
                &["--tenant".into(), value.into(), "--quiet".into()],
                "export-tb",
                false,
            )
            .unwrap()
            .unwrap();
            assert_eq!(options.tenant.as_deref(), Some(value));
            assert!(options.quiet, "{value}");
        }
        for value in ["-.", "-.x", "-²", "--"] {
            let failure = parse(&["--tenant".into(), value.into()], "export-tb", false)
                .err()
                .unwrap();
            assert_eq!(failure.code, 2, "{value}");
            assert!(
                failure
                    .stderr
                    .contains("argument --tenant: expected one argument"),
                "{value}: {}",
                failure.stderr
            );
        }
    }

    #[test]
    fn iso_dates_keep_the_calendar_and_years_1_to_9999() {
        for (value, date) in [
            ("0001-01-01", "0001-01-01"),
            ("00010101", "0001-01-01"),
            ("0001-W01-1", "0001-01-01"),
            ("9999-12-31", "9999-12-31"),
            ("9999-W52-5", "9999-12-31"),
            ("2024-02-29", "2024-02-29"),
            ("2000-02-29", "2000-02-29"),
            ("2026-W01", "2025-12-29"),
            ("2026W272", "2026-06-30"),
            ("2026-+1-05", "2026-01-05"),
        ] {
            assert_eq!(iso_date(value).unwrap(), date, "{value}");
        }
        for invalid in [
            "9999-W52-6",
            "1900-02-29",
            "2026-04-31",
            "2026-13-01",
            "2026-00-10",
            "2026--1-05",
            "2026-W00-1",
            "2026-W27-0",
            "2026-W27-8",
            "2026W27080",
            "0000-W01-1",
        ] {
            assert!(iso_date(invalid).is_err(), "{invalid}");
        }
        let today = parse(&[], "export-tb", false).unwrap().unwrap().date;
        assert_eq!(iso_date(&today).unwrap(), today);
    }
}
