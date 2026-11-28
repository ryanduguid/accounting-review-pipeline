// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Reversible identifier placeholders at the agent tool seam:
//! - `post-tool` masks check-digit-valid identifiers into an owner-only session map;
//! - `pre-tool` restores placeholders in tool arguments and refuses unmapped ones.
use crate::{
    files, oserror,
    patterns::{self, Result},
    persist,
    pyjson::{self, Json},
    pypath, pytext,
};
use fancy_regex::Regex;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    sync::LazyLock,
};

#[cfg(test)]
mod tests;

const HOOK_KINDS: [&str; 4] = ["tfn", "abn", "acn", "medicare"];
const PLACEHOLDER_PREFIXES: [&str; 4] = ["TFN", "ABN", "ACN", "MEDICARE"];

static HOOK_PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| {
    fancy_regex::RegexBuilder::new(r"(?<![A-Za-z0-9_])(?:TFN|ABN|ACN|MEDICARE)_[0-9]{2,}(?![A-Za-z0-9_])")
        .backtrack_limit(patterns::BACKTRACK_LIMIT)
        .build()
        .expect("valid hook placeholder regex")
});

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionEntry {
    pub kind: String,
    pub normalised: String,
    pub placeholder: String,
    pub value: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionMap {
    pub entries: Vec<SessionEntry>,
}

impl SessionMap {
    fn find_by_key(&self, kind: &str, normalised: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.kind == kind && e.normalised == normalised)
            .map(|e| e.placeholder.as_str())
    }

    fn assign(&mut self, kind: &str, normalised: String, raw_value: &str) -> String {
        if let Some(existing) = self.find_by_key(kind, &normalised) {
            return existing.to_owned();
        }
        let prefix = kind.to_ascii_uppercase();
        let next = self
            .entries
            .iter()
            .filter(|e| e.kind == kind)
            .filter_map(|e| e.placeholder.strip_prefix(&format!("{prefix}_")))
            .filter_map(|n| n.parse::<usize>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        let placeholder = format!("{prefix}_{next:02}");
        self.entries.push(SessionEntry {
            kind: kind.to_owned(),
            normalised,
            placeholder: placeholder.clone(),
            value: raw_value.to_owned(),
        });
        placeholder
    }

    fn by_placeholder(&self) -> BTreeMap<&str, &str> {
        self.entries
            .iter()
            .map(|e| (e.placeholder.as_str(), e.value.as_str()))
            .collect()
    }

    fn to_json(&self) -> Json {
        let entries = self
            .entries
            .iter()
            .map(|e| {
                Json::object([
                    ("kind", Json::str(&e.kind)),
                    ("normalised", Json::str(&e.normalised)),
                    ("placeholder", Json::str(&e.placeholder)),
                    ("value", Json::str(&e.value)),
                ])
            })
            .collect();
        Json::object([("schema_version", Json::int(1)), ("entries", Json::Array(entries))])
    }
}

fn check_owner_only(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|e| oserror::file_error(&e, &files::text(path)))?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err("session map must be a regular file, not a symlink or directory".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err("session map permissions must be owner-only (0600)".into());
        }
    }
    Ok(())
}

fn parse_entry(entry: &Json) -> Result<SessionEntry> {
    let field = |key: &str| {
        entry
            .get(key)
            .and_then(Json::as_pystr)
            .and_then(|s| s.to_utf8())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("session map entry missing valid {key}"))
    };
    let kind = field("kind")?;
    let normalised = field("normalised")?;
    let placeholder = field("placeholder")?;
    let value = field("value")?;
    if !HOOK_KINDS.contains(&kind.as_str())
        || !PLACEHOLDER_PREFIXES
            .iter()
            .any(|p| placeholder.starts_with(&format!("{p}_")))
    {
        return Err("session map entry has unsupported kind or placeholder".into());
    }
    Ok(SessionEntry {
        kind,
        normalised,
        placeholder,
        value,
    })
}

pub fn load_session_map(path: &Path, create_if_missing: bool) -> Result<SessionMap> {
    files::ignored(path, "the hook session map")?;
    if !path.exists() {
        return if create_if_missing {
            Ok(SessionMap::default())
        } else {
            Err(format!("session map does not exist: {}", files::text(path)))
        };
    }
    check_owner_only(path)?;
    let (text, _) = files::read(path)?;
    let doc = pyjson::loads_plain(&text).map_err(|_| "session map is not valid JSON".to_owned())?;
    if !matches!(doc.get("schema_version"), Some(Json::Int(n)) if n == &num_bigint::BigInt::from(1)) {
        return Err("session map schema_version must be 1".into());
    }
    let Some(Json::Array(items)) = doc.get("entries") else {
        return Err("session map entries must be a list".into());
    };
    let entries = items.iter().map(parse_entry).collect::<Result<Vec<_>>>()?;
    Ok(SessionMap { entries })
}

pub fn save_session_map(path: &Path, map: &SessionMap) -> Result<()> {
    files::ignored(path, "the hook session map")?;
    if path.exists() {
        check_owner_only(path)?;
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    files::mkdir(parent)?;
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|e| oserror::file_error(&e, &files::text(path)))?;
    let encoded = pyjson::pretty_ordered(&map.to_json()) + "\n";
    persist::text(file, encoded.as_bytes()).map_err(|e| oserror::file_error(&e, &files::text(path)))
}

fn token_delimiter(ch: char) -> bool {
    pytext::is_space(ch as u32)
        || matches!(
            ch,
            '"' | '\'' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | '`' | '|' | '='
        )
}

fn has_file_extension(token: &str) -> bool {
    let trimmed = token.trim_end_matches(['.', ':', '!', '?']);
    let Some((stem, ext)) = trimmed.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && (1..=6).contains(&ext.len())
        && ext.bytes().all(|b| b.is_ascii_alphanumeric())
        && ext.bytes().any(|b| b.is_ascii_alphabetic())
}

fn is_path_or_filename_exempt(text: &str, start: usize, end: usize) -> bool {
    let left = text[..start]
        .char_indices()
        .rev()
        .find(|&(_, ch)| token_delimiter(ch))
        .map_or(0, |(idx, ch)| idx + ch.len_utf8());
    let right = text[end..]
        .char_indices()
        .find(|&(_, ch)| token_delimiter(ch))
        .map_or(text.len(), |(idx, _)| end + idx);
    let prefix = &text[left..start];
    let suffix = &text[end..right];
    prefix.contains(['/', '\\']) || suffix.contains(['/', '\\']) || has_file_extension(&text[left..right])
}

fn normalise_identifier(kind: &str, value: &str) -> String {
    let digits = patterns::digits(value);
    if kind == "medicare" {
        digits.chars().take(10).collect()
    } else {
        digits
    }
}

pub fn mask_tool_result(text: &str, map: &mut SessionMap) -> Result<String> {
    let text = pytext::nfc(text);
    let spans = patterns::structured_spans(&text)?;
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for span in spans {
        if !HOOK_KINDS.contains(&span.kind.as_str()) {
            continue;
        }
        let normalised = normalise_identifier(&span.kind, &span.value);
        if !patterns::valid(&span.kind, &normalised)? {
            continue;
        }
        if is_path_or_filename_exempt(&text, span.start, span.end) {
            continue;
        }
        let placeholder = map.assign(&span.kind, normalised, &span.value);
        out.push_str(&text[cursor..span.start]);
        out.push_str(&placeholder);
        cursor = span.end;
    }
    out.push_str(&text[cursor..]);
    Ok(out)
}

#[derive(Debug, PartialEq, Eq)]
pub enum RestoreOutcome {
    Restored(String),
    Unmapped(Vec<String>),
}

pub fn restore_tool_arguments(text: &str, map: &SessionMap) -> Result<RestoreOutcome> {
    let lookup = map.by_placeholder();
    let mut missing = BTreeSet::new();
    let mut matches_found = Vec::new();
    for item in HOOK_PLACEHOLDER.find_iter(text) {
        let matched = item.map_err(patterns::runtime)?;
        let token = matched.as_str();
        if !lookup.contains_key(token) {
            missing.insert(token.to_owned());
        }
        matches_found.push((matched.start(), matched.end(), token));
    }
    if !missing.is_empty() {
        return Ok(RestoreOutcome::Unmapped(missing.into_iter().collect()));
    }
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end, token) in matches_found {
        out.push_str(&text[cursor..start]);
        out.push_str(lookup[token]);
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    Ok(RestoreOutcome::Restored(out))
}

fn read_input(path: Option<&Path>, mut reader: impl std::io::Read) -> Result<(String, &'static str)> {
    match path {
        Some(path) => files::read(path),
        None => {
            let mut text = String::new();
            reader
                .read_to_string(&mut text)
                .map_err(|e| format!("cannot read hook input: {e}"))?;
            Ok((text.replace("\r\n", "\n"), "\n"))
        }
    }
}

fn write_output(path: Option<&Path>, text: &str, ending: &str, stdout: &mut dyn Write) -> Result<()> {
    match path {
        Some(path) => files::write(path, text, ending),
        None => write!(stdout, "{text}").map_err(|e| format!("cannot write hook output: {e}")),
    }
}

fn dispatch_hook(
    action: &str,
    session_path: &Path,
    input_path: Option<&Path>,
    output_path: Option<&Path>,
    reader: impl std::io::Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> Result<i32> {
    let (input, ending) = read_input(input_path, reader)?;
    if let Some(out) = output_path
        && pypath::same_path(&files::resolve(out)?, &files::resolve(session_path)?)
    {
        return Err("refusing to overwrite the hook session map with --out".into());
    }
    match action {
        "post-tool" => {
            let mut map = load_session_map(session_path, true)?;
            let masked = mask_tool_result(&input, &mut map)?;
            save_session_map(session_path, &map)?;
            write_output(output_path, &masked, ending, stdout)?;
            Ok(0)
        }
        "pre-tool" => {
            let map = load_session_map(session_path, false)?;
            match restore_tool_arguments(&input, &map)? {
                RestoreOutcome::Restored(restored) => {
                    write_output(output_path, &restored, ending, stdout)?;
                    Ok(0)
                }
                RestoreOutcome::Unmapped(missing) => {
                    let _ = writeln!(
                        stderr,
                        "refused: unmapped placeholder(s) {}; re-read the source before calling this tool",
                        missing.join(", ")
                    );
                    Ok(2)
                }
            }
        }
        _ => Err(format!("unknown hook mode: {action}")),
    }
}

pub fn run_hook_io(
    action: &str,
    session_path: &Path,
    input_path: Option<&Path>,
    output_path: Option<&Path>,
    reader: impl std::io::Read,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    match dispatch_hook(action, session_path, input_path, output_path, reader, stdout, stderr) {
        Ok(code) => code,
        Err(message) => {
            let _ = writeln!(stderr, "error: {message}");
            1
        }
    }
}
