use super::{convert, inspect_export, output_csv, text};
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs,
    path::PathBuf,
};

const USAGE: &str = "usage: xero-account-transactions SOURCE --tenant TENANT --account-name ACCOUNT [--mapping FILE --account-id ID --currency CODE]\n";
const OPTIONS: &[&str] = &[
    "tenant",
    "account-name",
    "mapping",
    "account-id",
    "currency",
    "help",
];

struct Arguments {
    source: PathBuf,
    values: BTreeMap<&'static str, OsString>,
}

fn usage(message: &str) -> (u8, String) {
    (
        2,
        format!("{USAGE}xero-account-transactions: error: {message}\n"),
    )
}

fn negative(text: &str) -> bool {
    let Some(text) = text.strip_prefix('-') else {
        return false;
    };
    let digits = |s: &str| s.chars().all(|ch| super::text::digit(ch).is_some());
    if let Some((whole, fraction)) = text.split_once('.') {
        digits(whole) && !fraction.is_empty() && digits(fraction)
    } else {
        !text.is_empty() && digits(text)
    }
}

fn option(name: &str) -> Result<&'static str, (u8, String)> {
    if let Some(&exact) = OPTIONS.iter().find(|&&value| value == name) {
        return Ok(exact);
    }
    let options: Vec<_> = OPTIONS
        .iter()
        .copied()
        .filter(|value| value.starts_with(name))
        .collect();
    match options.as_slice() {
        [only] => Ok(only),
        [] => Err(usage(&format!("unrecognised argument: --{name}"))),
        _ => Err(usage(&format!("ambiguous option: --{name}"))),
    }
}

fn inline_value(argument: &OsStr) -> OsString {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let bytes = argument.as_bytes();
        let equals = bytes
            .iter()
            .position(|&byte| byte == b'=')
            .expect("inline option");
        OsString::from_vec(bytes[equals + 1..].to_vec())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        let units: Vec<_> = argument.encode_wide().collect();
        let equals = units
            .iter()
            .position(|&unit| unit == u16::from(b'='))
            .expect("inline option");
        OsString::from_wide(&units[equals + 1..])
    }
}

fn read_option(
    arguments: &[OsString],
    index: &mut usize,
    token: &str,
    argument: &OsStr,
) -> Result<(&'static str, OsString), (u8, String)> {
    let (name, inline) = token[2..]
        .split_once('=')
        .map_or((&token[2..], None), |(a, b)| (a, Some(b)));
    let name = option(name)?;
    if name == "help" {
        return if inline.is_none() {
            Err((0, USAGE.to_owned()))
        } else {
            Err(usage("--help does not take a value"))
        };
    }
    let value = if inline.is_some() {
        inline_value(argument)
    } else {
        let value = arguments
            .get(*index)
            .ok_or_else(|| usage(&format!("--{name} requires a value")))?;
        if value
            .to_str()
            .is_some_and(|s| s.starts_with('-') && s != "-" && !negative(s))
        {
            return Err(usage(&format!("--{name} requires a value")));
        }
        *index += 1;
        value.clone()
    };
    Ok((name, value))
}

fn parse(arguments: &[OsString]) -> Result<Arguments, (u8, String)> {
    let (mut source, mut values, mut extra) = (None, BTreeMap::new(), Vec::new());
    let (mut index, mut literal) = (0, false);
    while index < arguments.len() {
        let argument = &arguments[index];
        index += 1;
        let token = argument.to_string_lossy();
        if !literal && token == "--" {
            literal = true;
            continue;
        }
        if !literal && let Some(tail) = token.strip_prefix("-h") {
            if tail.trim_start_matches('h').starts_with(['=', '-']) {
                return Err(usage("-h does not take a value"));
            }
            return Err((0, USAGE.to_owned()));
        }
        if !literal && token.starts_with("--") {
            let name = token[2..].split('=').next().expect("option text");
            if !OPTIONS.iter().any(|option| option.starts_with(name)) {
                extra.push(token.into_owned());
                continue;
            }
            let (name, value) = read_option(arguments, &mut index, &token, argument)?;
            values.insert(name, value);
        } else if !literal && token.starts_with('-') && token != "-" && !negative(&token) {
            extra.push(token.into_owned());
        } else if source.is_none() {
            source = Some(PathBuf::from(argument));
        } else {
            extra.push(argument.to_string_lossy().into_owned());
        }
    }
    finish(source, values, extra)
}

fn finish(
    source: Option<PathBuf>,
    values: BTreeMap<&'static str, OsString>,
    extra: Vec<String>,
) -> Result<Arguments, (u8, String)> {
    if !extra.is_empty() {
        return Err(usage(&format!(
            "unrecognised arguments: {}",
            extra.join(" ")
        )));
    }
    let source = source.ok_or_else(|| usage("source is required"))?;
    for name in ["tenant", "account-name"] {
        if !values.contains_key(name) {
            return Err(usage(&format!("--{name} is required")));
        }
    }
    for (key, value) in &values {
        if *key != "mapping" && value.to_str().is_none() {
            return Err(usage("text arguments require valid Unicode"));
        }
    }
    Ok(Arguments { source, values })
}

fn execute(arguments: &Arguments) -> Result<String, String> {
    let value = |key| {
        arguments
            .values
            .get(key)
            .and_then(|s| s.to_str())
            .unwrap_or("")
    };
    let source = fs::read(&arguments.source).map_err(|e| e.to_string())?;
    let Some(mapping) = arguments.values.get("mapping") else {
        return inspect_export(&source, value("tenant"), value("account-name"))
            .map(|value| text::pretty(&value) + "\n");
    };
    if value("account-id").is_empty() || value("currency").is_empty() {
        return Err("Conversion requires --account-id and --currency.".into());
    }
    let mapping = fs::read_to_string(PathBuf::from(mapping)).map_err(|e| e.to_string())?;
    let mapping = serde_json::from_str(&mapping).map_err(|e| e.to_string())?;
    let rows = convert(
        &source,
        &mapping,
        value("tenant"),
        value("account-name"),
        value("account-id"),
        value("currency"),
    )?;
    output_csv(&rows)
}

pub fn run(arguments: &[OsString]) -> (u8, String, String) {
    let arguments = match parse(arguments) {
        Ok(arguments) => arguments,
        Err((0, help)) => return (0, text::terminal(&help), String::new()),
        Err((code, error)) => return (code, String::new(), text::terminal(&error)),
    };
    match execute(&arguments) {
        Ok(output) => (0, text::terminal(&output), String::new()),
        Err(error) => (
            1,
            String::new(),
            text::terminal(&format!("Account Transactions: {error}\n")),
        ),
    }
}
