// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! A read-only Model Context Protocol server over stdio. Its tools run the
//! commands that only read local files, on paths inside the directories the
//! user allowed when starting it; none writes a file or reaches a network.
use crate::{
    Result, classification, comparison, journal_entries,
    pyjson::{self, Json, PyStr},
    queue, schedules, viewer,
};
use std::{
    io::{BufRead, Write},
    path::{Component, Path, PathBuf, Prefix},
};

/// Newest first; `initialize` answers with the newest handshake version
/// when the client's is not listed.
pub const VERSIONS: [&str; 3] = ["2026-07-28", "2025-11-25", "2025-06-18"];
const HANDSHAKE_VERSION: &str = "2025-11-25";
const INSTRUCTIONS: &str = "Read-only close review tools over local files. Paths must be absolute and inside the directories the user allowed. Results are review evidence for a person; they approve nothing.";
const VIEW_NOTE: &str = "close-control view: display is a review aid; it does not approve a close or change any computed status.";

fn string_property(description: &str) -> Json {
    Json::object([
        ("type", Json::str("string")),
        ("description", Json::str(description)),
    ])
}

fn tool(name: &str, description: &str, properties: Vec<(&str, Json)>, required: &[&str]) -> Json {
    Json::object([
        ("name", Json::str(name)),
        ("description", Json::str(description)),
        (
            "inputSchema",
            Json::object([
                ("type", Json::str("object")),
                (
                    "properties",
                    Json::Object(
                        properties
                            .into_iter()
                            .map(|(key, value)| (PyStr::from(key), value))
                            .collect(),
                    ),
                ),
                (
                    "required",
                    Json::Array(required.iter().map(|key| Json::str(key)).collect()),
                ),
                ("additionalProperties", Json::Bool(false)),
            ]),
        ),
        (
            "annotations",
            Json::object([
                ("readOnlyHint", Json::Bool(true)),
                ("openWorldHint", Json::Bool(false)),
            ]),
        ),
    ])
}

#[expect(clippy::too_many_lines, reason = "one schema per tool")]
fn tools() -> Json {
    let path = string_property;
    let currency = || string_property("three-letter currency, e.g. AUD");
    Json::Array(vec![
        tool(
            "view",
            "Verify a saved close review pack's 4 files agree and display it.",
            vec![("pack_dir", path("absolute path of the pack directory"))],
            &["pack_dir"],
        ),
        tool(
            "queue",
            "Verify several saved close packs and list them, optionally filtered or summarised.",
            vec![
                (
                    "pack_dirs",
                    Json::object([
                        ("type", Json::str("array")),
                        (
                            "items",
                            string_property("absolute path of a pack directory"),
                        ),
                        ("minItems", Json::int(1)),
                    ]),
                ),
                ("period", string_property("report date to keep, YYYY-MM-DD")),
                (
                    "status",
                    Json::object([
                        ("type", Json::str("string")),
                        (
                            "enum",
                            Json::Array(["PASS", "REVIEW", "BLOCKED"].map(Json::str).to_vec()),
                        ),
                    ]),
                ),
                ("reviewer", string_property("reviewer to keep")),
                ("summary", Json::object([("type", Json::str("boolean"))])),
            ],
            &["pack_dirs"],
        ),
        tool(
            "compare",
            "Compare 2 verified close packs and their trial balances.",
            vec![
                ("previous_pack", path("absolute path of the earlier pack")),
                ("current_pack", path("absolute path of the later pack")),
                (
                    "previous_tb",
                    path("absolute path of the earlier trial balance CSV"),
                ),
                (
                    "current_tb",
                    path("absolute path of the later trial balance CSV"),
                ),
                (
                    "responses",
                    path("optional absolute path of a responses file"),
                ),
            ],
            &["previous_pack", "current_pack", "previous_tb", "current_tb"],
        ),
        tool(
            "classify",
            "Compare postings before and after a recode with supplied coding evidence.",
            vec![
                (
                    "original_transactions",
                    path("absolute path of the original CSV"),
                ),
                (
                    "current_transactions",
                    path("absolute path of the current CSV"),
                ),
                (
                    "coding_evidence",
                    path("absolute path of the coding evidence JSON"),
                ),
                ("currency", currency()),
            ],
            &[
                "original_transactions",
                "current_transactions",
                "coding_evidence",
                "currency",
            ],
        ),
        tool(
            "schedule",
            "Review a canonical expense, migration or inter-entity schedule.",
            vec![
                (
                    "kind",
                    Json::object([
                        ("type", Json::str("string")),
                        (
                            "enum",
                            Json::Array(
                                ["expenses", "migration", "interentity"]
                                    .map(Json::str)
                                    .to_vec(),
                            ),
                        ),
                    ]),
                ),
                ("input", path("absolute path of the schedule JSON")),
            ],
            &["kind", "input"],
        ),
        tool(
            "jet",
            "Journal-entry tests and a Benford first-digit test over one tenant's general-ledger listing.",
            vec![
                (
                    "transactions",
                    path("absolute path of the canonical transaction CSV"),
                ),
                ("currency", currency()),
                (
                    "period_end",
                    string_property("last day of the period, YYYY-MM-DD"),
                ),
            ],
            &["transactions", "currency", "period_end"],
        ),
    ])
}

/// Arguments of one tool call, with every path confined to the allowed
/// directories.
struct Arguments<'a> {
    values: &'a Json,
    allowed: &'a [PathBuf],
}

impl Arguments<'_> {
    fn optional_text(&self, key: &str) -> Result<Option<String>> {
        match self.values.get(key) {
            None => Ok(None),
            Some(value) => value
                .as_pystr()
                .and_then(PyStr::to_utf8)
                .map(Some)
                .ok_or_else(|| format!("{key} must be text.")),
        }
    }
    fn text(&self, key: &str) -> Result<String> {
        self.optional_text(key)?
            .ok_or_else(|| format!("{key} is required."))
    }
    fn path(&self, key: &str) -> Result<PathBuf> {
        file(&self.text(key)?, self.allowed)
    }
    fn optional_path(&self, key: &str) -> Result<Option<PathBuf>> {
        self.optional_text(key)?
            .map(|raw| file(&raw, self.allowed))
            .transpose()
    }
    fn pack(&self, key: &str) -> Result<PathBuf> {
        pack(&self.text(key)?, self.allowed)
    }
    fn paths(&self, key: &str) -> Result<Vec<PathBuf>> {
        let items = self
            .values
            .get(key)
            .and_then(Json::as_array)
            .filter(|items| !items.is_empty())
            .ok_or_else(|| format!("{key} must be a non-empty list of paths."))?;
        items
            .iter()
            .map(|item| {
                let raw = item
                    .as_pystr()
                    .and_then(PyStr::to_utf8)
                    .ok_or_else(|| format!("{key} must be a non-empty list of paths."))?;
                pack(&raw, self.allowed)
            })
            .collect()
    }
}

/// Refuse network and device paths (`\\server\share`, `//server/share`,
/// `\\?\UNC\` and `\\.\`) before anything resolves them; a local drive
/// in verbatim form (`\\?\C:\`) is allowed.
fn local(raw: &str) -> Result<()> {
    let bytes = raw.as_bytes();
    let separator = |index: usize| matches!(bytes.get(index), Some(b'/' | b'\\'));
    // `\\?\C:\` is a local drive in Windows' verbatim form.
    let verbatim_disk = raw.starts_with("\\\\?\\")
        && bytes.get(4).is_some_and(u8::is_ascii_alphabetic)
        && bytes.get(5) == Some(&b':');
    if separator(0) && separator(1) && !verbatim_disk {
        return Err(format!(
            "{raw} is a network or device path; give a local path."
        ));
    }
    Ok(())
}

/// Whether the link at `link` sits inside an allowed directory. Its parent
/// holds no link the walk has not already followed, so resolving it follows
/// nothing new.
fn link_inside(link: &Path, allowed: &[PathBuf]) -> std::io::Result<bool> {
    let parent = link.parent().expect("a link the walk named has a parent");
    let name = link.file_name().expect("a link the walk named has a name");
    let location = std::fs::canonicalize(parent)?.join(name);
    Ok(allowed
        .iter()
        .any(|directory| location.starts_with(directory)))
}

/// `path` in Windows' verbatim form when it starts with a drive, so that
/// each name is looked up as written, without the trimming of trailing dots
/// and spaces the system applies to other paths. Other paths are unchanged.
fn verbatim(path: PathBuf) -> PathBuf {
    match path.components().next() {
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_)) => {
            let mut text = std::ffi::OsString::from("\\\\?\\");
            text.push(path.as_os_str());
            PathBuf::from(text)
        }
        _ => path,
    }
}

/// The existing file or directory `raw` names when it lies inside an
/// allowed directory. The path is resolved one name at a time, the way the
/// system would, and a link is followed only where it sits inside an allowed
/// directory, so no link elsewhere can lead to another folder or a share.
fn confine(raw: &str, allowed: &[PathBuf]) -> Result<PathBuf> {
    local(raw)?;
    if !Path::new(raw).is_absolute() {
        return Err(format!("{raw} is not an absolute path."));
    }
    let failed = |error: std::io::Error| format!("{raw}: {error}");
    let outside = || format!("{raw} is outside the directories this server may read.");
    let mut current = PathBuf::new();
    let absolute = verbatim(std::path::absolute(raw).map_err(failed)?);
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // As when the system resolves it, `..` must follow a directory.
                if !current.is_dir() {
                    return Err(format!("{raw}: a name before .. is not a directory."));
                }
                current.pop();
            }
            Component::Normal(name) => {
                current.push(name);
                let metadata = std::fs::symlink_metadata(&current).map_err(failed)?;
                if metadata.file_type().is_symlink() {
                    if !link_inside(&current, allowed).map_err(failed)? {
                        return Err(outside());
                    }
                    current = std::fs::canonicalize(&current).map_err(failed)?;
                }
            }
            other => current.push(other),
        }
    }
    let resolved = std::fs::canonicalize(&current).map_err(failed)?;
    let directory = raw.ends_with(['/', '\\']) || raw.ends_with("/.") || raw.ends_with("\\.");
    if directory && !resolved.is_dir() {
        return Err(format!("{raw} is not a directory."));
    }
    if allowed
        .iter()
        .any(|directory| resolved.starts_with(directory))
    {
        Ok(resolved)
    } else {
        Err(outside())
    }
}

/// A confined regular file: not a directory, pipe or device.
fn file(raw: &str, allowed: &[PathBuf]) -> Result<PathBuf> {
    let resolved = confine(raw, allowed)?;
    if resolved.is_file() {
        Ok(resolved)
    } else {
        Err(format!("{raw} is not a regular file."))
    }
}

/// A confined review-pack directory whose pack files, where present, are
/// regular files rather than links, pipes or directories, so reading them
/// by name stays inside the directory.
fn pack(raw: &str, allowed: &[PathBuf]) -> Result<PathBuf> {
    let resolved = confine(raw, allowed)?;
    if !resolved.is_dir() {
        return Err(format!("{raw} is not a directory."));
    }
    for name in viewer::PACK_NAMES {
        let metadata = std::fs::symlink_metadata(resolved.join(name));
        if metadata.is_ok_and(|metadata| !metadata.file_type().is_file()) {
            return Err(format!(
                "{raw}: {name} is not a regular file; links are not followed."
            ));
        }
    }
    Ok(resolved)
}

/// Resolve the directories the user allows; each must exist.
pub fn allowed_directories(raw: &[PathBuf]) -> Result<Vec<PathBuf>> {
    raw.iter()
        .map(|path| {
            local(&path.to_string_lossy())?;
            let resolved = std::fs::canonicalize(path)
                .map_err(|error| format!("directory {}: {error}", path.display()))?;
            if resolved.is_dir() {
                Ok(resolved)
            } else {
                Err(format!("{} is not a directory.", path.display()))
            }
        })
        .collect()
}

/// Run one tool; `None` for a tool this server does not have.
#[expect(clippy::too_many_lines, reason = "one arm per tool")]
fn run_tool(name: &str, args: &Arguments) -> Option<Result<String>> {
    let pretty = |value: Result<Json>| value.map(|value| pyjson::pretty(&value));
    Some(match name {
        "view" => args.pack("pack_dir").and_then(|directory| {
            let (sheet, _) = viewer::render_review_sheet(&directory)?;
            Ok(format!("{sheet}\n{VIEW_NOTE}"))
        }),
        "queue" => (|| {
            let option = |key| {
                args.optional_text(key)
                    .map(|value| value.map(|text| PyStr::from(text.as_str())))
            };
            let summary = match args.values.get("summary") {
                None => false,
                Some(value) => value.as_bool().ok_or("summary must be true or false.")?,
            };
            let filters = queue::Filters {
                period: option("period")?,
                status: option("status")?,
                reviewer: option("reviewer")?,
                summary,
            };
            queue::render_review_queue(&args.paths("pack_dirs")?, &filters)
        })(),
        "compare" => pretty((|| {
            comparison::compare_packs(&comparison::CompareConfig {
                previous_pack: args.pack("previous_pack")?,
                current_pack: args.pack("current_pack")?,
                previous_tb: args.path("previous_tb")?,
                current_tb: args.path("current_tb")?,
                responses: args.optional_path("responses")?,
            })
        })()),
        "classify" => pretty((|| {
            classification::classify(
                &args.path("original_transactions")?,
                &args.path("current_transactions")?,
                &args.path("coding_evidence")?,
                &args.text("currency")?,
            )
        })()),
        "schedule" => pretty((|| {
            let kind = args.text("kind")?;
            if !["expenses", "migration", "interentity"].contains(&kind.as_str()) {
                return Err("kind must be expenses, migration or interentity.".into());
            }
            schedules::review_schedule(&args.path("input")?, &kind)
        })()),
        "jet" => pretty((|| {
            journal_entries::journal_entry_tests(
                &args.path("transactions")?,
                &args.text("currency")?,
                &args.text("period_end")?,
            )
        })()),
        _ => return None,
    })
}

fn server_info() -> Json {
    Json::object([
        ("name", Json::str("close-control")),
        ("version", Json::str(env!("CARGO_PKG_VERSION"))),
    ])
}

fn capabilities() -> Json {
    Json::object([("tools", Json::object([]))])
}

fn success(id: Json, mut result: Json) -> Json {
    result.set("resultType", Json::str("complete"));
    Json::object([
        ("jsonrpc", Json::str("2.0")),
        ("id", id),
        ("result", result),
    ])
}

fn failure(id: Json, code: i64, message: &str, data: Option<Json>) -> Json {
    let mut error = Json::object([("code", Json::int(code)), ("message", Json::str(message))]);
    if let Some(data) = data {
        error.set("data", data);
    }
    Json::object([("jsonrpc", Json::str("2.0")), ("id", id), ("error", error)])
}

fn initialize(params: Option<&Json>) -> Json {
    let requested = params
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Json::as_pystr)
        .and_then(PyStr::to_utf8)
        .filter(|version| VERSIONS[1..].contains(&version.as_str()));
    Json::object([
        (
            "protocolVersion",
            Json::str(requested.as_deref().unwrap_or(HANDSHAKE_VERSION)),
        ),
        ("capabilities", capabilities()),
        ("serverInfo", server_info()),
        ("instructions", Json::str(INSTRUCTIONS)),
    ])
}

fn discover() -> Json {
    Json::object([
        (
            "supportedVersions",
            Json::Array(VERSIONS.map(Json::str).to_vec()),
        ),
        ("capabilities", capabilities()),
        ("instructions", Json::str(INSTRUCTIONS)),
        ("ttlMs", Json::int(0)),
        ("cacheScope", Json::str("private")),
        (
            "_meta",
            Json::object([("io.modelcontextprotocol/serverInfo", server_info())]),
        ),
    ])
}

fn call(id: Json, params: Option<&Json>, allowed: &[PathBuf]) -> Json {
    let name = params
        .and_then(|params| params.get("name"))
        .and_then(Json::as_pystr)
        .and_then(PyStr::to_utf8)
        .unwrap_or_default();
    let empty = Json::object([]);
    let values = match params.and_then(|params| params.get("arguments")) {
        None => &empty,
        Some(values @ Json::Object(_)) => values,
        Some(_) => return failure(id, -32602, "arguments must be an object.", None),
    };
    let Some(outcome) = run_tool(&name, &Arguments { values, allowed }) else {
        return failure(id, -32602, &format!("Unknown tool: {name}"), None);
    };
    let (text, is_error) = match outcome {
        Ok(text) => (text, false),
        Err(text) => (text, true),
    };
    success(
        id,
        Json::object([
            (
                "content",
                Json::Array(vec![Json::object([
                    ("type", Json::str("text")),
                    ("text", Json::str(&text)),
                ])]),
            ),
            ("isError", Json::Bool(is_error)),
        ]),
    )
}

/// The error for a request whose `_meta` names a protocol version this
/// server does not support.
fn unsupported_version(id: &Json, params: Option<&Json>) -> Option<Json> {
    let version = params
        .and_then(|params| params.get("_meta"))
        .and_then(|meta| meta.get("io.modelcontextprotocol/protocolVersion"));
    if let Some(version) = version {
        let text = version
            .as_pystr()
            .and_then(PyStr::to_utf8)
            .unwrap_or_default();
        if !VERSIONS.contains(&text.as_str()) {
            let data = Json::object([
                ("supported", Json::Array(VERSIONS.map(Json::str).to_vec())),
                ("requested", Json::str(&text)),
            ]);
            return Some(failure(
                id.clone(),
                -32022,
                "Unsupported protocol version",
                Some(data),
            ));
        }
    }
    None
}

/// The reply to one line, or `None` for a notification or a response.
pub fn handle(line: &str, allowed: &[PathBuf]) -> Option<Json> {
    let Ok(message) = pyjson::loads(line) else {
        return Some(failure(Json::Null, -32700, "Parse error", None));
    };
    let id = message.get("id").cloned();
    let valid_id = |id: &Json| matches!(id, Json::Int(_)) || id.as_pystr().is_some();
    if !message
        .get("jsonrpc")
        .is_some_and(|version| version.is_str("2.0"))
        || id.as_ref().is_some_and(|id| !valid_id(id))
    {
        return Some(failure(Json::Null, -32600, "Invalid Request", None));
    }
    let method = message
        .get("method")
        .and_then(Json::as_pystr)
        .and_then(PyStr::to_utf8);
    let Some(method) = method else {
        // A response to a request this server never sends needs no reply.
        return (message.get("result").is_none() && message.get("error").is_none())
            .then(|| failure(id.unwrap_or(Json::Null), -32600, "Invalid Request", None));
    };
    let id = id?;
    let params = message.get("params");
    if params.is_some_and(|params| params.as_object().is_none()) {
        return Some(failure(id, -32602, "Invalid params", None));
    }
    if let Some(error) = unsupported_version(&id, params) {
        return Some(error);
    }
    Some(match method.as_str() {
        "initialize" => success(id, initialize(params)),
        "server/discover" => success(id, discover()),
        "ping" => success(id, Json::object([])),
        "tools/list" => success(id, Json::object([("tools", tools())])),
        "tools/call" => call(id, params, allowed),
        _ => failure(id, -32601, "Method not found", None),
    })
}

/// Answer each line of `input` on `output` until end of input.
pub fn serve(
    mut input: impl BufRead,
    output: &mut impl Write,
    allowed: &[PathBuf],
) -> std::io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        let reply = match std::str::from_utf8(&line) {
            Ok(text) if text.trim().is_empty() => continue,
            Ok(text) => handle(text, allowed),
            Err(_) => Some(failure(Json::Null, -32700, "Parse error", None)),
        };
        if let Some(reply) = reply {
            writeln!(output, "{}", pyjson::canonical(&reply))?;
            output.flush()?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn reply(line: &str, allowed: &[PathBuf]) -> Json {
        handle(line, allowed).unwrap()
    }

    fn error_code(reply: &Json) -> Option<num_bigint::BigInt> {
        reply.at("error").at("code").as_plain_int().cloned()
    }

    fn call_text(allowed: &[PathBuf], tool: &str, arguments: &str) -> (String, bool) {
        let line = format!(
            r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"{tool}","arguments":{arguments}}}}}"#
        );
        let result = reply(&line, allowed);
        let result = result.at("result");
        let text = result.at("content").as_array().unwrap()[0]
            .at("text")
            .as_pystr()
            .unwrap()
            .to_utf8()
            .unwrap();
        (text, result.at("isError").as_bool().unwrap())
    }

    /// JSON text for a path, with Windows separators escaped.
    fn quoted(path: &Path) -> String {
        pyjson::canonical(&Json::str(path.to_str().unwrap()))
    }

    #[test]
    fn malformed_messages_get_json_rpc_errors_and_others_no_reply() {
        assert_eq!(error_code(&reply("{", &[])), Some((-32700).into()));
        assert_eq!(
            error_code(&reply(r#"{"jsonrpc":"2.0","id":1}"#, &[])),
            Some((-32600).into())
        );
        assert!(matches!(reply("[1]", &[]).at("id"), Json::Null));
        assert!(handle(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#, &[]).is_none());
        assert!(handle(r#"{"jsonrpc":"2.0","id":1,"error":{}}"#, &[]).is_none());
        assert!(
            handle(
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                &[]
            )
            .is_none()
        );
        let unknown = reply(
            r#"{"jsonrpc":"2.0","id":"a","method":"resources/list"}"#,
            &[],
        );
        assert_eq!(error_code(&unknown), Some((-32601).into()));
        assert!(unknown.at("id").is_str("a"));
    }

    #[test]
    fn versions_are_negotiated_both_ways() {
        let init = |version: &str| {
            reply(
                &format!(
                    r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{version}"}}}}"#
                ),
                &[],
            )
        };
        let result = init("2025-06-18");
        let result = result.at("result");
        assert!(result.at("protocolVersion").is_str("2025-06-18"));
        assert!(result.at("serverInfo").at("name").is_str("close-control"));
        for version in ["2026-07-28", "1999-01-01"] {
            let result = init(version);
            assert!(
                result
                    .at("result")
                    .at("protocolVersion")
                    .is_str("2025-11-25")
            );
        }
        let bare = reply(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#, &[]);
        assert!(bare.at("result").at("protocolVersion").is_str("2025-11-25"));
        let discover = reply(
            r#"{"jsonrpc":"2.0","id":2,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}"#,
            &[],
        );
        let result = discover.at("result");
        assert!(result.at("resultType").is_str("complete"));
        assert_eq!(result.at("supportedVersions").as_array().unwrap().len(), 3);
        for version in [r#""1900-01-01""#, "3"] {
            let refused = reply(
                &format!(
                    r#"{{"jsonrpc":"2.0","id":3,"method":"ping","params":{{"_meta":{{"io.modelcontextprotocol/protocolVersion":{version}}}}}}}"#
                ),
                &[],
            );
            assert_eq!(error_code(&refused), Some((-32022).into()));
        }
        let ping = reply(r#"{"jsonrpc":"2.0","id":4,"method":"ping"}"#, &[]);
        assert!(ping.at("result").at("resultType").is_str("complete"));
    }

    #[test]
    fn every_tool_is_listed_read_only() {
        let listed = reply(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, &[]);
        let names: Vec<_> = listed
            .at("result")
            .at("tools")
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| {
                assert_eq!(
                    tool.at("annotations").at("readOnlyHint").as_bool(),
                    Some(true)
                );
                tool.at("name").as_pystr().unwrap().to_utf8().unwrap()
            })
            .collect();
        assert_eq!(
            names,
            ["view", "queue", "compare", "classify", "schedule", "jet"]
        );
    }

    #[test]
    fn calls_refuse_unknown_tools_and_bad_arguments() {
        let unknown = reply(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"review"}}"#,
            &[],
        );
        assert_eq!(error_code(&unknown), Some((-32602).into()));
        let nameless = reply(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call"}"#, &[]);
        assert_eq!(error_code(&nameless), Some((-32602).into()));
        let listed = reply(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"view","arguments":[]}}"#,
            &[],
        );
        assert_eq!(error_code(&listed), Some((-32602).into()));
        let (text, failed) = call_text(&[], "view", "{}");
        assert!(failed);
        assert_eq!(text, "pack_dir is required.");
        let (text, _) = call_text(&[], "view", r#"{"pack_dir":1}"#);
        assert_eq!(text, "pack_dir must be text.");
        let (text, _) = call_text(&[], "view", r#"{"pack_dir":"relative/pack"}"#);
        assert_eq!(text, "relative/pack is not an absolute path.");
        for arguments in [r#"{}"#, r#"{"pack_dirs":[]}"#, r#"{"pack_dirs":[1]}"#] {
            let (text, _) = call_text(&[], "queue", arguments);
            assert_eq!(text, "pack_dirs must be a non-empty list of paths.");
        }
        let (text, _) = call_text(&[], "queue", r#"{"pack_dirs":["x"],"summary":"yes"}"#);
        assert_eq!(text, "summary must be true or false.");
        let (text, _) = call_text(&[], "schedule", r#"{"kind":"budget","input":"x"}"#);
        assert_eq!(text, "kind must be expenses, migration or interentity.");
    }

    #[test]
    fn envelopes_ids_and_params_are_checked_before_dispatch() {
        let bad = reply(r#"{"jsonrpc":"1.0","id":[],"method":"ping"}"#, &[]);
        assert_eq!(error_code(&bad), Some((-32600).into()));
        assert!(matches!(bad.at("id"), Json::Null));
        for line in [
            r#"{"id":1,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
        ] {
            assert_eq!(
                error_code(&reply(line, &[])),
                Some((-32600).into()),
                "{line}"
            );
        }
        let scalar = reply(
            r#"{"jsonrpc":"2.0","id":"p","method":"ping","params":3}"#,
            &[],
        );
        assert_eq!(error_code(&scalar), Some((-32602).into()));
        assert!(scalar.at("id").is_str("p"));
        let notification = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":3}"#;
        assert!(handle(notification, &[]).is_none());
        let ping = reply(r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#, &[]);
        assert!(ping.get("result").is_some());
    }

    fn jet_arguments(path: &Path) -> String {
        format!(
            r#"{{"transactions":{},"currency":"AUD","period_end":"2026-07-31"}}"#,
            quoted(path)
        )
    }

    #[test]
    fn tools_read_only_regular_files_on_local_paths() {
        let temp = TempDir::new();
        let inside = temp.join("inside");
        let pack_dir = inside.join("pack");
        std::fs::create_dir_all(pack_dir.join(viewer::PACK_NAMES[0])).unwrap();
        let csv = inside.join("journals.csv");
        std::fs::write(&csv, "x").unwrap();
        let allowed = allowed_directories(std::slice::from_ref(&inside)).unwrap();
        let pack_argument = format!(r#"{{"pack_dir":{}}}"#, quoted(&pack_dir));
        let (text, failed) = call_text(&allowed, "view", &pack_argument);
        assert!(
            failed && text.ends_with("links are not followed."),
            "{text}"
        );
        let queue_argument = format!(r#"{{"pack_dirs":[{}]}}"#, quoted(&pack_dir));
        let (text, _) = call_text(&allowed, "queue", &queue_argument);
        assert!(text.ends_with("links are not followed."), "{text}");
        let (text, _) = call_text(
            &allowed,
            "view",
            &format!(r#"{{"pack_dir":{}}}"#, quoted(&csv)),
        );
        assert!(text.ends_with(" is not a directory."), "{text}");
        let (text, _) = call_text(&allowed, "jet", &jet_arguments(&pack_dir));
        assert!(text.ends_with(" is not a regular file."), "{text}");
        for raw in [
            r"\\server\share\journals.csv",
            "//server/share/journals.csv",
            r"\\?\UNC\server\share\journals.csv",
        ] {
            let (text, failed) = call_text(&allowed, "jet", &jet_arguments(Path::new(raw)));
            assert!(failed && text.ends_with("give a local path."), "{text}");
        }
        let remote = allowed_directories(&[PathBuf::from("//server/share")]).unwrap_err();
        assert!(remote.ends_with("give a local path."), "{remote}");
    }

    #[cfg(unix)]
    #[test]
    fn links_and_pipes_in_a_pack_are_refused_before_reading() {
        let temp = TempDir::new();
        let inside = temp.join("inside");
        let (linked, piped) = (inside.join("linked"), inside.join("piped"));
        std::fs::create_dir_all(&linked).unwrap();
        std::fs::create_dir_all(&piped).unwrap();
        let outside = temp.join("outside.json");
        std::fs::write(&outside, "{}").unwrap();
        std::os::unix::fs::symlink(&outside, linked.join(viewer::PACK_NAMES[0])).unwrap();
        let fifo = piped.join(viewer::PACK_NAMES[0]);
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let allowed = allowed_directories(std::slice::from_ref(&inside)).unwrap();
        for directory in [&linked, &piped] {
            let argument = format!(r#"{{"pack_dir":{}}}"#, quoted(directory));
            let (text, failed) = call_text(&allowed, "view", &argument);
            assert!(
                failed && text.ends_with("links are not followed."),
                "{text}"
            );
        }
        let (text, failed) = call_text(&allowed, "jet", &jet_arguments(&fifo));
        assert!(
            failed && text.ends_with(" is not a regular file."),
            "{text}"
        );
    }

    #[test]
    fn written_paths_are_checked_before_any_link_is_followed() {
        assert!(local(r"\\?\C:\client\pack").is_ok());
        assert!(local(r"\\?\UNC\server\share").is_err());
        assert!(local(r"\\.\pipe\name").is_err());
        let temp = TempDir::new();
        let inside = temp.join("inside");
        std::fs::create_dir(&inside).unwrap();
        let csv = inside.join("journals.csv");
        std::fs::write(&csv, "x").unwrap();
        let allowed = allowed_directories(std::slice::from_ref(&inside)).unwrap();
        let resolved = std::fs::canonicalize(&csv).unwrap();
        assert_eq!(file(resolved.to_str().unwrap(), &allowed), Ok(resolved));
        let escape = inside.join("..").join("inside").join("journals.csv");
        assert!(file(escape.to_str().unwrap(), &allowed).is_ok());
        let around = inside.join("..").join("journals.csv");
        std::fs::write(temp.join("journals.csv"), "x").unwrap();
        assert!(
            file(around.to_str().unwrap(), &allowed)
                .unwrap_err()
                .contains("outside")
        );
    }

    #[test]
    fn names_are_looked_up_as_written() {
        let temp = TempDir::new();
        let inside = std::fs::canonicalize(temp.join("."))
            .unwrap()
            .join("inside");
        let spaced = inside.join("hop ");
        std::fs::create_dir_all(&spaced).unwrap();
        std::fs::write(spaced.join("f.csv"), "x").unwrap();
        let allowed = allowed_directories(std::slice::from_ref(&inside)).unwrap();
        let written = spaced.join("f.csv");
        let written = written.to_str().unwrap();
        let written = written.strip_prefix("\\\\?\\").unwrap_or(written);
        let resolved = file(written, &allowed).unwrap();
        assert!(resolved.ends_with(Path::new("hop ").join("f.csv")));
        let pack = file(
            &format!("{}{}", inside.display(), std::path::MAIN_SEPARATOR),
            &allowed,
        );
        assert!(pack.unwrap_err().ends_with(" is not a regular file."));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_outside_the_directories_is_not_followed() {
        let temp = TempDir::new();
        let inside = temp.join("inside");
        std::fs::create_dir(&inside).unwrap();
        std::fs::write(inside.join("journals.csv"), "x").unwrap();
        let link = temp.join("elsewhere");
        std::os::unix::fs::symlink(&inside, &link).unwrap();
        let allowed = allowed_directories(std::slice::from_ref(&inside)).unwrap();
        let refused = |path: PathBuf| {
            let error = file(path.to_str().unwrap(), &allowed).unwrap_err();
            assert!(
                error.ends_with("outside the directories this server may read."),
                "{error}"
            );
        };
        refused(link.join("journals.csv"));
        // The detour through the outside link and back by name.
        refused(link.join("..").join("inside").join("journals.csv"));
        // A server directory given through a link does not admit its
        // neighbours.
        let sub = temp.join("u").join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("public.csv"), "x").unwrap();
        std::fs::write(temp.join("private.csv"), "x").unwrap();
        let alias = temp.join("alias");
        std::os::unix::fs::symlink(&sub, &alias).unwrap();
        let allowed = allowed_directories(&[alias.join(".")]).unwrap();
        let private = temp.join("private.csv");
        assert!(file(private.to_str().unwrap(), &allowed).is_err());
        let public = sub.join("public.csv");
        assert!(file(public.to_str().unwrap(), &allowed).is_ok());
        // A link inside a server directory is followed.
        let inner = sub.join("inner");
        std::os::unix::fs::symlink(sub.join("public.csv"), &inner).unwrap();
        assert!(file(inner.to_str().unwrap(), &allowed).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn dot_dot_follows_a_resolved_directory() {
        let temp = TempDir::new();
        let inside = temp.join("inside");
        let deep = inside.join("sub").join("deep");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(inside.join("g"), "lexical").unwrap();
        std::fs::write(inside.join("sub").join("g"), "physical").unwrap();
        std::os::unix::fs::symlink(&deep, inside.join("l")).unwrap();
        std::fs::write(inside.join("x"), "x").unwrap();
        let allowed = allowed_directories(std::slice::from_ref(&inside)).unwrap();
        let text = |path: &str| format!("{}/{path}", inside.display());
        let physical = std::fs::canonicalize(inside.join("sub").join("g")).unwrap();
        assert_eq!(file(&text("l/../g"), &allowed), Ok(physical));
        let error = file(&text("x/../g"), &allowed).unwrap_err();
        assert!(
            error.ends_with("a name before .. is not a directory."),
            "{error}"
        );
        for path in ["g/", "g/."] {
            let error = file(&text(path), &allowed).unwrap_err();
            assert!(error.ends_with(" is not a directory."), "{error}");
        }
    }

    #[test]
    fn paths_must_exist_inside_an_allowed_directory() {
        let temp = TempDir::new();
        let inside = temp.join("inside");
        let outside = temp.join("outside");
        std::fs::create_dir(&inside).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("journals.csv"), "x").unwrap();
        let allowed = allowed_directories(std::slice::from_ref(&inside)).unwrap();
        let file = outside.join("journals.csv");
        let arguments = format!(
            r#"{{"transactions":{},"currency":"AUD","period_end":"2026-07-31"}}"#,
            quoted(&file)
        );
        let (text, failed) = call_text(&allowed, "jet", &arguments);
        assert!(failed);
        assert!(
            text.ends_with(" is outside the directories this server may read."),
            "{text}"
        );
        let missing = inside.join("absent.csv");
        let (text, _) = call_text(
            &allowed,
            "view",
            &format!(r#"{{"pack_dir":{}}}"#, quoted(&missing)),
        );
        assert!(text.starts_with(missing.to_str().unwrap()), "{text}");
        assert!(
            allowed_directories(&[temp.join("absent")])
                .unwrap_err()
                .starts_with("directory ")
        );
        assert!(
            allowed_directories(&[file])
                .unwrap_err()
                .ends_with(" is not a directory.")
        );
    }

    #[test]
    fn serve_answers_each_line_and_skips_blank_ones() {
        let input = b"\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\r\n\xff\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n".to_vec();
        let mut output = Vec::new();
        serve(std::io::Cursor::new(input), &mut output, &[]).unwrap();
        let text = String::from_utf8(output).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 2, "{text}");
        assert_eq!(
            lines[0],
            r#"{"id":1,"jsonrpc":"2.0","result":{"resultType":"complete"}}"#
        );
        assert!(lines[1].contains("-32700"));
    }
}
