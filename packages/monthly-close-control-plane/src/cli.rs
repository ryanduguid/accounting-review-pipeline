use crate::{
    Result, classification, comparison,
    decimal::Decimal,
    drivers, engine, loader,
    models::Status,
    publication,
    pyjson::{self, Json, PyStr},
    pypath, pytext, queue, reconciliation, reconciliation_report, report, schedules, stdio, viewer,
};
use num_bigint::BigInt;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
};

const COMMANDS: [&str; 9] = [
    "review",
    "workbench",
    "view",
    "queue",
    "reconcile",
    "drivers",
    "compare",
    "classify",
    "schedule",
];
fn text(command: &str, usage: bool) -> &'static str {
    macro_rules! saved {
        ($name:literal) => {
            if usage {
                include_str!(concat!("help/", $name, "-usage.txt"))
            } else {
                include_str!(concat!("help/", $name, "-help.txt"))
            }
        };
    }
    match command {
        "review" | "workbench" => saved!("review"),
        "view" => saved!("view"),
        "queue" => saved!("queue"),
        "reconcile" => saved!("reconcile"),
        "drivers" => saved!("drivers"),
        "compare" => saved!("compare"),
        "classify" => saved!("classify"),
        "schedule" => saved!("schedule"),
        _ => saved!("root"),
    }
}
fn usage_error(command: &str, message: &str) -> i32 {
    let command = if command == "workbench" {
        "review"
    } else {
        command
    };
    crate::err!(
        "{}close-control{}: error: {message}\n",
        text(command, true),
        if command.is_empty() {
            String::new()
        } else {
            format!(" {command}")
        }
    );
    1
}
fn options(command: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match command {
        "review" | "workbench" => (
            &[
                "--current",
                "--prior",
                "--mapping",
                "--mapping-policy",
                "--balance-policy",
                "--subledger",
                "--review-note",
                "--equity-schedule",
                "--equity-currency",
                "--equity-currency-evidence",
                "--calculation-evidence",
                "--require-calculation",
                "--output",
                "--absolute-threshold",
                "--percentage-threshold",
                "--reconciliation-tolerance",
            ],
            &["--current", "--prior", "--output"],
        ),
        "view" => (&["--pack-dir"], &["--pack-dir"]),
        "queue" => (
            &["--pack-dir", "--period", "--status", "--reviewer"],
            &["--pack-dir"],
        ),
        "reconcile" => (
            &[
                "--transactions",
                "--tenant",
                "--account-id",
                "--currency",
                "--period-start",
                "--period-end",
                "--opening-balance",
                "--closing-balance",
                "--opening-items",
                "--decisions",
                "--decisions-escaped",
                "--output",
            ],
            &[
                "--transactions",
                "--tenant",
                "--account-id",
                "--currency",
                "--period-start",
                "--period-end",
                "--opening-balance",
                "--closing-balance",
                "--output",
            ],
        ),
        "drivers" => (
            &[
                "--pack-dir",
                "--transactions",
                "--currency",
                "--top",
                "--output",
            ],
            &["--pack-dir", "--transactions", "--currency", "--output"],
        ),
        "compare" => (
            &[
                "--previous-pack",
                "--current-pack",
                "--previous-tb",
                "--current-tb",
                "--responses",
            ],
            &[
                "--previous-pack",
                "--current-pack",
                "--previous-tb",
                "--current-tb",
            ],
        ),
        "classify" => (
            &[
                "--original-transactions",
                "--current-transactions",
                "--coding-evidence",
                "--currency",
            ],
            &[
                "--original-transactions",
                "--current-transactions",
                "--coding-evidence",
                "--currency",
            ],
        ),
        "schedule" => (&["--kind", "--input"], &["--kind", "--input"]),
        _ => unreachable!("command checked"),
    }
}
struct Arguments(BTreeMap<String, Vec<String>>);
impl Arguments {
    fn raw(&self, flag: &str) -> &str {
        self.0
            .get(flag)
            .and_then(|v| v.last())
            .map_or("", String::as_str)
    }
    fn literal(&self, flag: &str) -> Result<String> {
        PyStr::from_os_text(self.raw(flag))
            .to_utf8()
            .ok_or_else(|| format!("{flag} contains a control or formatting character."))
    }
    fn optional_literal(&self, flag: &str) -> Result<Option<String>> {
        self.0
            .contains_key(flag)
            .then(|| self.literal(flag))
            .transpose()
    }
    fn path(&self, flag: &str) -> PathBuf {
        PathBuf::from(pypath::encode_os(&pypath::display(self.raw(flag))))
    }
    fn optional_path(&self, flag: &str) -> Option<PathBuf> {
        self.0.contains_key(flag).then(|| self.path(flag))
    }
    fn paths(&self, flag: &str) -> Vec<PathBuf> {
        self.0
            .get(flag)
            .into_iter()
            .flatten()
            .map(|value| PathBuf::from(pypath::encode_os(&pypath::display(value))))
            .collect()
    }
    fn texts(&self, flag: &str) -> Result<Vec<String>> {
        self.0
            .get(flag)
            .into_iter()
            .flatten()
            .map(|value| {
                PyStr::from_os_text(value)
                    .to_utf8()
                    .ok_or_else(|| format!("{flag} contains a control or formatting character."))
            })
            .collect()
    }
    fn decimal(&self, flag: &str, default: &str) -> Decimal {
        Decimal::parse(if self.0.contains_key(flag) {
            self.raw(flag)
        } else {
            default
        })
        .expect("parser checked decimal")
    }
}
fn negative_number(text: &str) -> bool {
    let Some(text) = text.strip_prefix('-') else {
        return false;
    };
    let mut parts = text.split('.');
    let before = parts.next().unwrap();
    let after = parts.next();
    let digits = |s: &str| {
        s.chars()
            .all(|ch| pytext::decimal_value(ch as u32).is_some())
    };
    parts.next().is_none()
        && digits(before)
        && after.map_or(!before.is_empty(), |after| {
            !after.is_empty() && digits(after)
        })
}
fn integer(value: &str) -> Option<BigInt> {
    let raw = pytext::strip(value);
    let (sign, raw) = if let Some(raw) = raw.strip_prefix('-') {
        ("-", raw)
    } else {
        ("", raw.strip_prefix('+').unwrap_or(raw))
    };
    let mut digits = String::from(sign);
    let mut previous_digit = false;
    let mut count = 0;
    for ch in raw.chars() {
        if ch == '_' && previous_digit {
            previous_digit = false;
            continue;
        }
        let digit = pytext::decimal_value(ch as u32)?;
        digits.push(char::from_digit(digit, 10).unwrap());
        previous_digit = true;
        count += 1;
    }
    (previous_digit && count <= 4300)
        .then(|| digits.parse().ok())
        .flatten()
}
fn threshold(value: &str, fraction: bool) -> Result<Decimal> {
    let result =
        Decimal::parse(value).map_err(|_| format!("not a decimal: {}", pytext::repr_os(value)))?;
    if !result.is_finite() || result < Decimal::zero() {
        return Err("must be a finite non-negative decimal".into());
    }
    if fraction && result > Decimal::parse("1").unwrap() {
        return Err("use a fraction: 0.10 means 10%".into());
    }
    Ok(result)
}
fn parse(command: &str, args: &[String]) -> Result<Option<Arguments>> {
    let (flags, required) = options(command);
    let mut result = Arguments(BTreeMap::new());
    let mut unknown = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let raw = &args[i];
        i += 1;
        if raw == "--" {
            unknown.push(raw.clone());
            unknown.extend(args[i..].iter().cloned());
            break;
        }
        if ["-h", "--help"].contains(&raw.as_str()) {
            return Ok(None);
        }
        let (flag, assigned) = raw
            .split_once('=')
            .map_or((raw.as_str(), None), |(key, value)| (key, Some(value)));
        let flag = if flags.contains(&flag) {
            flag
        } else {
            let matches: Vec<_> = if flag.starts_with("--") {
                flags
                    .iter()
                    .copied()
                    .chain(std::iter::once("--help"))
                    .filter(|candidate| candidate.starts_with(flag))
                    .collect()
            } else {
                vec![]
            };
            if matches.len() > 1 {
                return Err(format!(
                    "ambiguous option: {flag} could match {}",
                    matches.join(", ")
                ));
            }
            if let Some(flag) = matches.first() {
                *flag
            } else {
                unknown.push(raw.clone());
                continue;
            }
        };
        if ["--help", "--decisions-escaped"].contains(&flag) {
            if let Some(value) = assigned {
                return Err(format!(
                    "argument {}: ignored explicit argument {}",
                    if flag == "--help" { "-h/--help" } else { flag },
                    pytext::repr_os(value)
                ));
            }
            if flag == "--help" {
                return Ok(None);
            }
            result.0.insert(flag.into(), vec!["true".into()]);
            continue;
        }
        let value = if let Some(value) = assigned {
            value.to_owned()
        } else {
            let value = args
                .get(i)
                .ok_or_else(|| format!("argument {flag}: expected one argument"))?;
            if value.starts_with('-') && value != "-" && !negative_number(value) {
                return Err(format!("argument {flag}: expected one argument"));
            }
            i += 1;
            value.clone()
        };
        let choices: &[&str] = match flag {
            "--kind" => &["expenses", "migration", "interentity"],
            "--status" => &["PASS", "REVIEW", "BLOCKED"],
            _ => &[],
        };
        if !choices.is_empty() && !choices.contains(&value.as_str()) {
            return Err(format!(
                "argument {flag}: invalid choice: {} (choose from {})",
                pytext::repr_os(&value),
                choices
                    .iter()
                    .map(|value| pytext::repr(value))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if [
            "--absolute-threshold",
            "--percentage-threshold",
            "--reconciliation-tolerance",
        ]
        .contains(&flag)
        {
            threshold(&value, flag == "--percentage-threshold")
                .map_err(|error| format!("argument {flag}: {error}"))?;
        }
        if flag == "--top" && integer(&value).is_none() {
            return Err(format!(
                "argument --top: invalid int value: {}",
                pytext::repr_os(&value)
            ));
        }
        if ["--calculation-evidence", "--require-calculation"].contains(&flag)
            || command == "queue" && flag == "--pack-dir"
        {
            result.0.entry(flag.into()).or_default().push(value);
        } else {
            result.0.insert(flag.into(), vec![value]);
        }
    }
    let missing: Vec<_> = required
        .iter()
        .filter(|flag| !result.0.contains_key(**flag))
        .copied()
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "the following arguments are required: {}",
            missing.join(", ")
        ));
    }
    if !unknown.is_empty() {
        return Err(format!("unrecognized arguments: {}", unknown.join(" ")));
    }
    Ok(Some(result))
}

fn review(command: &str, args: &Arguments) -> i32 {
    let output = args.path("--output");
    let checked = || -> Result<()> {
        publication::require_output_outside_repository(&output)?;
        let destinations = report::PACK_FILE_NAMES
            .iter()
            .map(|name| {
                pypath::resolve(&output.join(name), false)
                    .map_err(|_| "cannot resolve output path".to_owned())
            })
            .collect::<Result<Vec<_>>>()?;
        let mut sources: Vec<_> = [
            "--current",
            "--prior",
            "--mapping",
            "--mapping-policy",
            "--balance-policy",
            "--subledger",
            "--review-note",
            "--equity-schedule",
        ]
        .into_iter()
        .filter_map(|flag| args.optional_path(flag).map(|path| (flag, path)))
        .collect();
        sources.extend(
            args.paths("--calculation-evidence")
                .into_iter()
                .map(|path| ("--calculation-evidence", path)),
        );
        for (flag, source) in sources {
            let resolved = pypath::resolve(&source, false).map_err(|_| {
                format!(
                    "cannot resolve {flag} {}: path is unusable",
                    pypath::text(&source)
                )
            })?;
            if destinations
                .iter()
                .any(|path| pypath::same_path(path, &resolved))
            {
                return Err(format!(
                    "{flag} {} is inside --output and shares a generated pack file name; the run would destroy it.",
                    pypath::text(&source)
                ));
            }
        }
        Ok(())
    };
    if let Err(error) = checked() {
        crate::errln!("close-control: output error: {error}");
        return 1;
    }
    let input = || -> Result<_> {
        let mut config = engine::ReviewConfig::new(args.path("--current"), args.path("--prior"));
        config.mapping_path = args.optional_path("--mapping");
        config.mapping_policy_path = args.optional_path("--mapping-policy");
        config.balance_policy_path = args.optional_path("--balance-policy");
        config.subledger_path = args.optional_path("--subledger");
        config.acknowledgement_path = args.optional_path("--review-note");
        config.equity_schedule_path = args.optional_path("--equity-schedule");
        config.equity_currency = args.optional_literal("--equity-currency")?;
        config.equity_currency_evidence = args.optional_literal("--equity-currency-evidence")?;
        config.calculation_evidence_paths = args.paths("--calculation-evidence");
        config.required_calculations = args.texts("--require-calculation")?;
        config.absolute_threshold = args.decimal("--absolute-threshold", "1000");
        config.percentage_threshold = args.decimal("--percentage-threshold", "0.10");
        config.reconciliation_tolerance = args.decimal("--reconciliation-tolerance", "0.01");
        engine::review_close(&config)
    };
    let pack = match input() {
        Ok(pack) => pack,
        Err(error) => {
            crate::errln!("close-control: input error: {error}");
            return 1;
        }
    };
    let outputs = match publication::write_review_pack(&pack, &output) {
        Ok(outputs) => outputs,
        Err(error) => {
            crate::errln!("close-control: output error: {error}");
            return 1;
        }
    };
    let prefix = if command == "workbench" {
        "close-control workbench"
    } else {
        "close-control"
    };
    let count = pack
        .client_queries()
        .expect("published query register validated")
        .len();
    crate::outln!(
        "{prefix}: {}; {} exception(s); {count} client query(ies) drafted",
        pack.status.as_str(),
        pack.exceptions.len()
    );
    if pack.status == Status::Pass && pack.exceptions.is_empty() {
        crate::outln!("{prefix}: Nothing interesting happens.");
    }
    for (name, path) in ["json", "summary", "exceptions", "client_queries"]
        .iter()
        .zip(outputs)
    {
        crate::outln!("  {name}: {}", pypath::text(&path));
    }
    if command == "workbench" {
        crate::outln!(
            "Review close-summary.md, exceptions.csv, client-queries.csv, and close-review-pack.json."
        );
        crate::outln!(
            "This pack records review evidence only; it does not approve or close a period."
        );
        crate::outln!(
            "client-queries.csv holds draft questions; nothing has been sent to a client."
        );
    }
    if pack.status == Status::Pass { 0 } else { 2 }
}
fn run(command: &str, args: &Arguments) -> i32 {
    if ["review", "workbench"].contains(&command) {
        return review(command, args);
    }
    let result = || -> Result<i32> {
        match command {
            "view" => {
                let (sheet, _) = viewer::render_review_sheet(&args.path("--pack-dir"))?;
                stdio::out(&(sheet + "\n"));
                crate::outln!(
                    "close-control view: display is a review aid; it does not approve a close or change any computed status."
                );
                Ok(0)
            }
            "queue" => {
                let option = |flag| {
                    args.0
                        .contains_key(flag)
                        .then(|| PyStr::from_os_text(args.raw(flag)))
                };
                let filters = queue::Filters {
                    period: option("--period"),
                    status: option("--status"),
                    reviewer: option("--reviewer"),
                };
                stdio::out_literal(
                    &(queue::render_review_queue(&args.paths("--pack-dir"), &filters)? + "\n"),
                );
                Ok(0)
            }
            "compare" => {
                let config = comparison::CompareConfig {
                    previous_pack: args.path("--previous-pack"),
                    current_pack: args.path("--current-pack"),
                    previous_tb: args.path("--previous-tb"),
                    current_tb: args.path("--current-tb"),
                    responses: args.optional_path("--responses"),
                };
                stdio::out_literal(&(pyjson::pretty(&comparison::compare_packs(&config)?) + "\n"));
                Ok(0)
            }
            "classify" | "schedule" => {
                let value = if command == "classify" {
                    classification::classify(
                        &args.path("--original-transactions"),
                        &args.path("--current-transactions"),
                        &args.path("--coding-evidence"),
                        &args.literal("--currency")?,
                    )?
                } else {
                    schedules::review_schedule(&args.path("--input"), args.raw("--kind"))?
                };
                stdio::out_literal(&(pyjson::pretty(&value) + "\n"));
                Ok(if value.at("status").is_str("PASS") {
                    0
                } else {
                    2
                })
            }
            "reconcile" => {
                let destination =
                    publication::require_output_outside_repository(&args.path("--output"))?;
                if destination.exists() {
                    return Err("Output already exists; use a new directory for each run.".into());
                }
                let identity = |flag, field| {
                    loader::json_input_text(
                        &Json::Str(PyStr::from_os_text(args.raw(flag))),
                        field,
                        0,
                        Path::new("configuration"),
                        false,
                    )
                };
                let config = reconciliation::ReconcileConfig {
                    transactions: args.path("--transactions"),
                    tenant: identity("--tenant", "Tenant")?,
                    account_id: identity("--account-id", "AccountID")?,
                    currency: identity("--currency", "Currency")?,
                    period_start: args.literal("--period-start")?,
                    period_end: args.literal("--period-end")?,
                    opening_balance: args.literal("--opening-balance")?,
                    closing_balance: args.literal("--closing-balance")?,
                    opening_items: args.optional_path("--opening-items"),
                    decisions: args.optional_path("--decisions"),
                    decisions_escaped: args.0.contains_key("--decisions-escaped"),
                };
                let value = reconciliation::reconcile(&config)?;
                let path = reconciliation_report::write_reconciliation(&value, &destination)?;
                crate::outln!(
                    "close-control reconcile: {}; {} outstanding items",
                    drivers::field(&value, "status"),
                    value.at("outstanding").as_array().unwrap().len()
                );
                crate::outln!("  Review: {}", pypath::text(&path));
                Ok(if value.at("status").is_str("PASS") {
                    0
                } else {
                    2
                })
            }
            "drivers" => {
                let top = if args.0.contains_key("--top") {
                    integer(args.raw("--top")).unwrap()
                } else {
                    5.into()
                };
                let value = drivers::variance_drivers(
                    &args.path("--pack-dir"),
                    &args.path("--transactions"),
                    &args.literal("--currency")?,
                    &top,
                )?;
                let path = drivers::write_drivers(&value, &args.path("--output"))?;
                let accounts = value.at("accounts").as_array().unwrap();
                let unexplained = accounts
                    .iter()
                    .filter(|account| {
                        !Decimal::parse(&drivers::field(account, "unexplained"))
                            .unwrap()
                            .is_zero()
                    })
                    .count();
                crate::outln!(
                    "close-control drivers: {}; {} variance(s), {unexplained} not fully covered by the supplied transactions",
                    drivers::field(&value, "status"),
                    accounts.len()
                );
                if value.at("financial_year_reset").as_bool() == Some(true) {
                    crate::outln!(
                        "  The pack crosses a 30 June reset; profit-and-loss movements are not comparable."
                    );
                }
                crate::outln!("  CSV: {}", pypath::text(&path));
                Ok(if value.at("status").is_str("PASS") {
                    0
                } else {
                    2
                })
            }
            _ => unreachable!("command checked"),
        }
    };
    match result() {
        Ok(code) => code,
        Err(error) => {
            let prefix = if ["view", "queue", "compare"].contains(&command) {
                ": verification failed: "
            } else {
                ": "
            };
            let error = if command == "queue" {
                let value = pyjson::dumps(&Json::Str(PyStr::from_os_text(&error)), false);
                value[1..value.len() - 1].to_owned()
            } else {
                error
            };
            crate::errln!("close-control {command}{prefix}{error}");
            1
        }
    }
}
pub fn main(args: Vec<OsString>) -> i32 {
    let args: Vec<_> = args.iter().map(|arg| pypath::decode_os(arg)).collect();
    if args == ["--version"] {
        crate::outln!("close-control {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    let mut command_index = 0;
    let mut unknown = Vec::new();
    while let Some(arg) = args.get(command_index).filter(|arg| arg.starts_with('-')) {
        if arg == "--" {
            command_index += 1;
            break;
        }
        let (flag, value) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(key, value)| (key, Some(value)));
        if flag == "-h" || flag.len() > 2 && flag.starts_with("--") && "--help".starts_with(flag) {
            if let Some(value) = value {
                return usage_error(
                    "",
                    &format!(
                        "argument -h/--help: ignored explicit argument {}",
                        pytext::repr_os(value)
                    ),
                );
            }
            stdio::out_literal(text("", false));
            return 0;
        }
        unknown.push(arg.as_str());
        command_index += 1;
    }
    let Some(command) = args.get(command_index) else {
        return usage_error("", "the following arguments are required: command");
    };
    if !COMMANDS.contains(&command.as_str()) {
        return usage_error(
            "",
            &format!(
                "argument command: invalid choice: {} (choose from {})",
                pytext::repr_os(command),
                COMMANDS
                    .iter()
                    .map(|name| pytext::repr(name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
    let parsed = match parse(command, &args[command_index + 1..]) {
        Ok(Some(args)) => args,
        Ok(None) => {
            stdio::out_literal(text(command, false));
            return 0;
        }
        Err(error) => {
            return usage_error(
                if error.starts_with("unrecognized arguments:") {
                    ""
                } else {
                    command
                },
                &error,
            );
        }
    };
    if !unknown.is_empty() {
        return usage_error(
            "",
            &format!("unrecognized arguments: {}", unknown.join(" ")),
        );
    }
    run(command, &parsed)
}
pub fn command_thread(action: impl FnOnce() -> i32 + Send + 'static) -> i32 {
    match std::thread::Builder::new()
        .name("close-command".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let code = action();
            if stdio::failed() {
                crate::errln!("close-control: cannot write command output.");
                1
            } else {
                code
            }
        }) {
        Ok(thread) => thread.join().unwrap_or_else(|_| {
            crate::errln!("close-control: command failed unexpectedly");
            1
        }),
        Err(error) => {
            crate::errln!("close-control: cannot start command ({error})");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_panicking_command_returns_failure() {
        assert_eq!(command_thread(|| panic!("synthetic command failure")), 1);
    }
    #[test]
    fn malformed_optional_inputs_preserve_an_existing_pack() {
        let temp = crate::test_support::TempDir::new();
        let output = temp.join("pack");
        let examples = crate::test_support::examples();
        let args: Vec<OsString> = vec![
            "review".into(),
            "--current".into(),
            examples.join("current_trial_balance.csv").into(),
            "--prior".into(),
            examples.join("prior_trial_balance.csv").into(),
            "--output".into(),
            output.clone().into(),
        ];
        assert_eq!(stdio::capture(|| main(args.clone())).0, 2);
        let before: Vec<_> = crate::report::PACK_FILE_NAMES
            .iter()
            .map(|name| std::fs::read(output.join(name)).unwrap())
            .collect();
        for (flag, content) in [
            (
                "--balance-policy",
                "AccountID,ExpectedBalance,ExpectMovement\nA,invalid,yes\n",
            ),
            (
                "--equity-schedule",
                "{\"schema_version\":1,\"movements\":[{\"credit\":\"NaN\"}]}",
            ),
        ] {
            let source = temp.join("invalid-input");
            std::fs::write(&source, content).unwrap();
            let mut invalid = args.clone();
            invalid.extend([flag.into(), source.into()]);
            if flag == "--equity-schedule" {
                invalid.extend([
                    "--equity-currency=AUD".into(),
                    "--equity-currency-evidence=Synthetic".into(),
                ]);
            }
            assert_eq!(stdio::capture(|| main(invalid)).0, 1);
            for (index, name) in crate::report::PACK_FILE_NAMES.iter().enumerate() {
                assert_eq!(std::fs::read(output.join(name)).unwrap(), before[index]);
            }
        }
    }
    #[test]
    fn help_usage_and_integer_options_keep_their_exit_contracts() {
        let (code, stdout, stderr) = stdio::capture(|| main(vec!["--help".into()]));
        assert_eq!(code, 0);
        assert!(stdout.starts_with("usage: close-control"));
        assert!(stderr.is_empty());
        let (code, _, stderr) = stdio::capture(|| main(vec!["review".into()]));
        assert_eq!(code, 1);
        assert!(stderr.contains("--current, --prior, --output"));
        assert_eq!(integer(" +١_٢ "), Some(12.into()));
        for raw in ["1__2", "_1", "1_", "", "1.0", "0x10"] {
            assert!(integer(raw).is_none());
        }
    }
}
