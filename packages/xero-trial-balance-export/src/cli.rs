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
    path::{Path, PathBuf},
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

fn usage(prog: &str, auth: bool) -> String {
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
    if pytext::py_len(&full) <= width() {
        return format!("{full}\n");
    }
    let long = (7 + pytext::py_len(prog)) * 4 > width() * 3;
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
        if pytext::py_len(&line) + 1 + part.len() > width() && line != indent {
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

pub fn help(prog: &str, auth: bool) -> String {
    let description = if auth {
        "Authorise this exporter against Xero once, in a browser, and cache the token."
    } else {
        "Export a Xero Trial Balance to CSV."
    };
    let mut out = format!(
        "{}\n{}\n\noptions:\n",
        usage(prog, auth),
        crate::textwrap::wrap(description, width().max(11)).join("\n")
    );
    let options = if auth { &OPTIONS[..1] } else { &OPTIONS[..] };
    let position = if auth { 14 } else { 24 }.min(width().saturating_sub(20).max(4));
    for (flag, argument, text) in options {
        let header = if *flag == "--help" {
            "-h, --help".into()
        } else {
            argument
                .map(|a| format!("{flag} {a}"))
                .unwrap_or_else(|| flag.to_string())
        };
        let lines = crate::textwrap::wrap(text, width().saturating_sub(position).max(11));
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
    out
}

fn error(prog: &str, auth: bool, message: String) -> Failure {
    Failure {
        code: 2,
        stdout: String::new(),
        stderr: format!("{}{prog}: error: {message}\n", usage(prog, auth)),
    }
}

fn negative_number(value: &str) -> bool {
    let Some(value) = value.strip_prefix('-') else {
        return false;
    };
    let mut dots = 0;
    !value.is_empty()
        && value.bytes().all(|b| {
            if b == b'.' {
                dots += 1;
                dots == 1
            } else {
                b.is_ascii_digit()
            }
        })
        && value.bytes().any(|b| b.is_ascii_digit())
}

pub fn parse(args: &[OsString], prog: &str, auth: bool) -> Result<Option<Options>> {
    let mut options = Options {
        date: chrono::Local::now()
            .date_naive()
            .format("%Y-%m-%d")
            .to_string(),
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
        let found: Vec<_> = OPTIONS
            .iter()
            .filter(|(name, _, _)| {
                *name == flag || (flag.starts_with("--") && name.starts_with(flag))
            })
            .filter(|(name, _, _)| !auth || *name == "--help")
            .collect();
        if found.len() > 1 {
            return Err(error(
                prog,
                auth,
                format!(
                    "ambiguous option: {flag} could match {}",
                    found
                        .iter()
                        .map(|(name, _, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        let Some((name, argument, _)) = found.first().copied() else {
            unknown.push(value.clone());
            continue;
        };
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
            match *name {
                "--payments-only" => options.cash = true,
                "--quiet" => options.quiet = true,
                "--no-manifest" => options.no_manifest = true,
                _ => {}
            }
            continue;
        }
        let value = if let Some(value) = explicit {
            value.to_string()
        } else {
            let Some(value) = values.get(index).filter(|s| {
                !s.starts_with('-') || negative_number(s) || *s == "-" || s.contains(' ')
            }) else {
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
    let year: i32 = value[..4].parse().map_err(|_| failure())?;
    if !(1..=9999).contains(&year) {
        return Err(failure());
    }
    let date = if value.len() == 10 && value.as_bytes()[4] == b'-' && value.as_bytes()[7] == b'-' {
        chrono::NaiveDate::from_ymd_opt(
            year,
            value[5..7].parse().map_err(|_| failure())?,
            value[8..10].parse().map_err(|_| failure())?,
        )
    } else if value.len() == 8 && value.bytes().all(|b| b.is_ascii_digit()) {
        chrono::NaiveDate::from_ymd_opt(
            year,
            value[4..6].parse().map_err(|_| failure())?,
            value[6..8].parse().map_err(|_| failure())?,
        )
    } else {
        let remainder = value[4..].strip_prefix('-').unwrap_or(&value[4..]);
        let week = remainder.strip_prefix('W').ok_or_else(failure)?;
        if week.len() < 2 {
            return Err(failure());
        }
        let number: u32 = week[..2].parse().map_err(|_| failure())?;
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
            rest[..1].parse::<u32>().map_err(|_| failure())?
        };
        let weekday = match day {
            1 => chrono::Weekday::Mon,
            2 => chrono::Weekday::Tue,
            3 => chrono::Weekday::Wed,
            4 => chrono::Weekday::Thu,
            5 => chrono::Weekday::Fri,
            6 => chrono::Weekday::Sat,
            7 => chrono::Weekday::Sun,
            _ => return Err(failure()),
        };
        chrono::NaiveDate::from_isoywd_opt(year, number, weekday)
    };
    date.filter(|d| (1..=9999).contains(&chrono::Datelike::year(d)))
        .map(|d| d.format("%Y-%m-%d").to_string())
        .ok_or_else(failure)
}

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

pub fn entry(auth: bool) {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = std::thread::Builder::new()
        .name("xero-command".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || run(&args, auth))
        .map_err(|error| {
            Failure::message(format!("error: cannot start the Xero command ({error})"))
        })
        .and_then(|thread| {
            thread.join().unwrap_or_else(|_| {
                Err(Failure::message(
                    "error: the Xero command failed unexpectedly.",
                ))
            })
        });
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
}
