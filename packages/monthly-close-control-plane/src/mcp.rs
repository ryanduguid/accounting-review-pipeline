// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! A read-only Model Context Protocol server over stdio. Its tools run the
//! commands that only read local files, on paths beneath the directory the
//! user allowed when starting it; none writes a file or reaches a network.
use crate::{
    Result, bas,
    beneath::Root,
    classification, coding, comparison, duplicates, fixed_assets, journal_entries, overdue,
    payroll, prepost,
    pyjson::{self, Json, PyStr},
    queue, rules, schedules, scorecard, statement, viewer,
};
use std::{
    io::{BufRead, Write},
    path::PathBuf,
};

/// Newest first; `initialize` answers with the newest handshake version
/// when the client's is not listed.
pub const VERSIONS: [&str; 3] = ["2026-07-28", "2025-11-25", "2025-06-18"];
const HANDSHAKE_VERSION: &str = "2025-11-25";
const INSTRUCTIONS: &str = "Read-only close review tools over local files. Paths are relative to the directory the user allowed, with names separated by /. Results are review evidence for a person; they approve nothing.";
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
            vec![(
                "pack_dir",
                path("path, relative to the server's directory, of the pack directory"),
            )],
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
                            string_property(
                                "path, relative to the server's directory, of a pack directory",
                            ),
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
                (
                    "previous_pack",
                    path("path, relative to the server's directory, of the earlier pack"),
                ),
                (
                    "current_pack",
                    path("path, relative to the server's directory, of the later pack"),
                ),
                (
                    "previous_tb",
                    path(
                        "path, relative to the server's directory, of the earlier trial balance CSV",
                    ),
                ),
                (
                    "current_tb",
                    path(
                        "path, relative to the server's directory, of the later trial balance CSV",
                    ),
                ),
                (
                    "responses",
                    path("optional path, relative to the server's directory, of a responses file"),
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
                    path("path, relative to the server's directory, of the original CSV"),
                ),
                (
                    "current_transactions",
                    path("path, relative to the server's directory, of the current CSV"),
                ),
                (
                    "coding_evidence",
                    path("path, relative to the server's directory, of the coding evidence JSON"),
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
            "Review a canonical expense, migration, inter-entity, prepayment or roll-forward schedule.",
            vec![
                (
                    "kind",
                    Json::object([
                        ("type", Json::str("string")),
                        (
                            "enum",
                            Json::Array(schedules::KINDS.map(Json::str).to_vec()),
                        ),
                    ]),
                ),
                (
                    "input",
                    path("path, relative to the server's directory, of the schedule JSON"),
                ),
            ],
            &["kind", "input"],
        ),
        tool(
            "jet",
            "Journal-entry tests and a Benford first-digit test over one tenant's general-ledger listing.",
            vec![
                (
                    "transactions",
                    path(
                        "path, relative to the server's directory, of the canonical transaction CSV",
                    ),
                ),
                ("currency", currency()),
                (
                    "period_end",
                    string_property("last day of the period, YYYY-MM-DD"),
                ),
            ],
            &["transactions", "currency", "period_end"],
        ),
        tool(
            "duplicates",
            "Check a document register for possible duplicate invoices or payments in a period.",
            vec![
                (
                    "documents",
                    path("path, relative to the server's directory, of the document register CSV"),
                ),
                ("currency", currency()),
                (
                    "period_start",
                    string_property("first day of the period, YYYY-MM-DD"),
                ),
                (
                    "period_end",
                    string_property("last day of the period, YYYY-MM-DD"),
                ),
            ],
            &["documents", "currency", "period_start", "period_end"],
        ),
        tool(
            "fixed-assets",
            "Tie a fixed-asset register to a trial balance by asset and accumulated depreciation accounts.",
            vec![
                (
                    "register",
                    path(
                        "path, relative to the server's directory, of the fixed-asset register CSV",
                    ),
                ),
                (
                    "trial_balance",
                    path("path, relative to the server's directory, of the trial balance CSV"),
                ),
                ("currency", currency()),
            ],
            &["register", "trial_balance", "currency"],
        ),
        tool(
            "coding",
            "Compare a period's account and tax coding against prior postings by the same supplier.",
            vec![
                (
                    "lines",
                    path("path, relative to the server's directory, of the transaction lines CSV"),
                ),
                ("currency", currency()),
                (
                    "period_start",
                    string_property("first day of the period, YYYY-MM-DD"),
                ),
                (
                    "period_end",
                    string_property("last day of the period, YYYY-MM-DD"),
                ),
            ],
            &["lines", "currency", "period_start", "period_end"],
        ),
        tool(
            "scorecard",
            "Summarise saved check result JSON files for one tenant into a close scorecard.",
            vec![(
                "results",
                Json::object([
                    ("type", Json::str("array")),
                    (
                        "items",
                        string_property(
                            "path, relative to the server's directory, of a saved result JSON file",
                        ),
                    ),
                    ("minItems", Json::int(1)),
                ]),
            )],
            &["results"],
        ),
        tool(
            "payroll",
            "Reperform PAYG withholding and super guarantee on a payroll register.",
            vec![(
                "register",
                path("path, relative to the server's directory, of the payroll register CSV"),
            )],
            &["register"],
        ),
        tool(
            "statement",
            "Check that a bank statement's running balances and page totals tie out.",
            vec![
                (
                    "statement",
                    path("path, relative to the server's directory, of the bank statement CSV"),
                ),
                ("currency", currency()),
            ],
            &["statement", "currency"],
        ),
        tool(
            "bas",
            "Draft a BAS workpaper from a ledger, tax-code map and optional payroll register.",
            vec![
                (
                    "ledger",
                    path("path, relative to the server's directory, of the ledger CSV"),
                ),
                (
                    "tax_codes",
                    path("path, relative to the server's directory, of the tax-code map CSV"),
                ),
                (
                    "payroll",
                    path(
                        "optional path, relative to the server's directory, of the payroll register CSV",
                    ),
                ),
                ("currency", currency()),
                (
                    "period_start",
                    string_property("first day of the period, YYYY-MM-DD"),
                ),
                (
                    "period_end",
                    string_property("last day of the period, YYYY-MM-DD"),
                ),
                (
                    "basis",
                    Json::object([
                        ("type", Json::str("string")),
                        (
                            "enum",
                            Json::Array(["accruals", "cash"].map(Json::str).to_vec()),
                        ),
                    ]),
                ),
            ],
            &[
                "ledger",
                "tax_codes",
                "currency",
                "period_start",
                "period_end",
                "basis",
            ],
        ),
        tool(
            "prepost",
            "Review draft postings against prior posted transactions before posting.",
            vec![
                (
                    "drafts",
                    path("path, relative to the server's directory, of the draft postings CSV"),
                ),
                (
                    "posted",
                    path(
                        "path, relative to the server's directory, of the posted transactions CSV",
                    ),
                ),
                ("currency", currency()),
                (
                    "period_start",
                    string_property("first day of the period, YYYY-MM-DD"),
                ),
                (
                    "period_end",
                    string_property("last day of the period, YYYY-MM-DD"),
                ),
            ],
            &["drafts", "posted", "currency", "period_start", "period_end"],
        ),
        tool(
            "overdue",
            "Flag open invoices that are overdue relative to each customer's payment history.",
            vec![
                (
                    "invoices",
                    path("path, relative to the server's directory, of the invoices CSV"),
                ),
                ("currency", currency()),
                ("as_at", string_property("as-at date, YYYY-MM-DD")),
            ],
            &["invoices", "currency", "as_at"],
        ),
        tool(
            "rules",
            "List every close review control, when it runs, what it checks and its source.",
            vec![],
            &[],
        ),
    ])
}

/// Arguments of one tool call, with every path beneath the root.
struct Arguments<'a> {
    values: &'a Json,
    root: &'a Root,
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
        file(&self.text(key)?, self.root)
    }
    fn optional_path(&self, key: &str) -> Result<Option<PathBuf>> {
        self.optional_text(key)?
            .map(|raw| file(&raw, self.root))
            .transpose()
    }
    fn pack(&self, key: &str) -> Result<PathBuf> {
        pack(&self.text(key)?, self.root)
    }
    fn paths_with(
        &self,
        key: &str,
        open: fn(&str, &Root) -> Result<PathBuf>,
    ) -> Result<Vec<PathBuf>> {
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
                open(&raw, self.root)
            })
            .collect()
    }
    fn paths(&self, key: &str) -> Result<Vec<PathBuf>> {
        self.paths_with(key, pack)
    }
    fn files(&self, key: &str) -> Result<Vec<PathBuf>> {
        self.paths_with(key, file)
    }
}

/// A regular file beneath the root.
fn file(raw: &str, root: &Root) -> Result<PathBuf> {
    let path = root.join(raw)?;
    root.file(&path)
        .map_err(|error| format!("{raw}: {error}"))?;
    Ok(path)
}

/// A review-pack directory beneath the root. Its pack files are read
/// through the root's handle, which refuses a link, pipe or directory.
fn pack(raw: &str, root: &Root) -> Result<PathBuf> {
    let path = root.join(raw)?;
    root.directory(&path)
        .map_err(|error| format!("{raw}: {error}"))?;
    Ok(path)
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
            if !schedules::KINDS.contains(&kind.as_str()) {
                let (last, rest) = schedules::KINDS.split_last().unwrap();
                return Err(format!("kind must be {} or {last}.", rest.join(", ")));
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
        "duplicates" => pretty((|| {
            duplicates::duplicate_documents(
                &args.path("documents")?,
                &args.text("currency")?,
                &args.text("period_start")?,
                &args.text("period_end")?,
            )
        })()),
        "fixed-assets" => pretty((|| {
            fixed_assets::fixed_assets(
                &args.path("register")?,
                &args.path("trial_balance")?,
                &args.text("currency")?,
            )
        })()),
        "coding" => pretty((|| {
            coding::coding_review(
                &args.path("lines")?,
                &args.text("currency")?,
                &args.text("period_start")?,
                &args.text("period_end")?,
            )
        })()),
        "scorecard" => pretty((|| scorecard::scorecard(&args.files("results")?))()),
        "payroll" => pretty((|| payroll::payroll_reperformance(&args.path("register")?))()),
        "statement" => pretty((|| {
            statement::statement_tie_out(&args.path("statement")?, &args.text("currency")?)
        })()),
        "bas" => pretty((|| {
            let ledger = args.path("ledger")?;
            let tax_codes = args.path("tax_codes")?;
            let payroll = args.optional_path("payroll")?;
            let currency = args.text("currency")?;
            let period_start = args.text("period_start")?;
            let period_end = args.text("period_end")?;
            let basis = args.text("basis")?;
            bas::bas_workpaper(&bas::BasConfig {
                ledger: &ledger,
                tax_codes: &tax_codes,
                payroll: payroll.as_deref(),
                currency: &currency,
                period_start: &period_start,
                period_end: &period_end,
                basis: &basis,
            })
        })()),
        "prepost" => pretty((|| {
            prepost::prepost_review(
                &args.path("drafts")?,
                &args.path("posted")?,
                &args.text("currency")?,
                &args.text("period_start")?,
                &args.text("period_end")?,
            )
        })()),
        "overdue" => pretty((|| {
            overdue::overdue_against_history(
                &args.path("invoices")?,
                &args.text("currency")?,
                &args.text("as_at")?,
            )
        })()),
        "rules" => Ok(rules::render()),
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

fn call(id: Json, params: Option<&Json>, root: &Root) -> Json {
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
    let Some(outcome) = root.within(|| run_tool(&name, &Arguments { values, root })) else {
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
pub fn handle(line: &str, root: &Root) -> Option<Json> {
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
        "tools/call" => call(id, params, root),
        _ => failure(id, -32601, "Method not found", None),
    })
}

/// Answer each line of `input` on `output` until end of input.
pub fn serve(mut input: impl BufRead, output: &mut impl Write, root: &Root) -> std::io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            return Ok(());
        }
        let reply = match std::str::from_utf8(&line) {
            Ok(text) if text.trim().is_empty() => continue,
            Ok(text) => handle(text, root),
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
    use std::path::Path;

    fn reply(line: &str, root: &Root) -> Json {
        handle(line, root).unwrap()
    }

    /// A root for messages that read no file.
    fn nowhere() -> Root {
        Root::open(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    fn error_code(reply: &Json) -> Option<num_bigint::BigInt> {
        reply.at("error").at("code").as_plain_int().cloned()
    }

    fn call_text(root: &Root, tool: &str, arguments: &str) -> (String, bool) {
        let line = format!(
            r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"{tool}","arguments":{arguments}}}}}"#
        );
        let result = reply(&line, root);
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
        assert_eq!(error_code(&reply("{", &nowhere())), Some((-32700).into()));
        assert_eq!(
            error_code(&reply(r#"{"jsonrpc":"2.0","id":1}"#, &nowhere())),
            Some((-32600).into())
        );
        assert!(matches!(reply("[1]", &nowhere()).at("id"), Json::Null));
        assert!(handle(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#, &nowhere()).is_none());
        assert!(handle(r#"{"jsonrpc":"2.0","id":1,"error":{}}"#, &nowhere()).is_none());
        assert!(
            handle(
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                &nowhere()
            )
            .is_none()
        );
        let unknown = reply(
            r#"{"jsonrpc":"2.0","id":"a","method":"resources/list"}"#,
            &nowhere(),
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
                &nowhere(),
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
        let bare = reply(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            &nowhere(),
        );
        assert!(bare.at("result").at("protocolVersion").is_str("2025-11-25"));
        let discover = reply(
            r#"{"jsonrpc":"2.0","id":2,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}"#,
            &nowhere(),
        );
        let result = discover.at("result");
        assert!(result.at("resultType").is_str("complete"));
        assert_eq!(result.at("supportedVersions").as_array().unwrap().len(), 3);
        for version in [r#""1900-01-01""#, "3"] {
            let refused = reply(
                &format!(
                    r#"{{"jsonrpc":"2.0","id":3,"method":"ping","params":{{"_meta":{{"io.modelcontextprotocol/protocolVersion":{version}}}}}}}"#
                ),
                &nowhere(),
            );
            assert_eq!(error_code(&refused), Some((-32022).into()));
        }
        let ping = reply(r#"{"jsonrpc":"2.0","id":4,"method":"ping"}"#, &nowhere());
        assert!(ping.at("result").at("resultType").is_str("complete"));
    }

    #[test]
    fn every_tool_is_listed_read_only() {
        let listed = reply(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            &nowhere(),
        );
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
            [
                "view",
                "queue",
                "compare",
                "classify",
                "schedule",
                "jet",
                "duplicates",
                "fixed-assets",
                "coding",
                "scorecard",
                "payroll",
                "statement",
                "bas",
                "prepost",
                "overdue",
                "rules",
            ]
        );
    }

    #[test]
    fn calls_refuse_unknown_tools_and_bad_arguments() {
        let unknown = reply(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"review"}}"#,
            &nowhere(),
        );
        assert_eq!(error_code(&unknown), Some((-32602).into()));
        let nameless = reply(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call"}"#,
            &nowhere(),
        );
        assert_eq!(error_code(&nameless), Some((-32602).into()));
        let listed = reply(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"view","arguments":[]}}"#,
            &nowhere(),
        );
        assert_eq!(error_code(&listed), Some((-32602).into()));
        let (text, failed) = call_text(&nowhere(), "view", "{}");
        assert!(failed);
        assert_eq!(text, "pack_dir is required.");
        let (text, _) = call_text(&nowhere(), "view", r#"{"pack_dir":1}"#);
        assert_eq!(text, "pack_dir must be text.");
        let (text, _) = call_text(&nowhere(), "view", r#"{"pack_dir":"/pack"}"#);
        assert_eq!(
            text,
            "/pack: give a path relative to the server's directory, with names separated by / and no ., .., \\ or :."
        );
        for arguments in [r#"{}"#, r#"{"pack_dirs":[]}"#, r#"{"pack_dirs":[1]}"#] {
            let (text, _) = call_text(&nowhere(), "queue", arguments);
            assert_eq!(text, "pack_dirs must be a non-empty list of paths.");
        }
        for arguments in [r#"{}"#, r#"{"results":[]}"#, r#"{"results":[1]}"#] {
            let (text, _) = call_text(&nowhere(), "scorecard", arguments);
            assert_eq!(text, "results must be a non-empty list of paths.");
        }
        let (text, _) = call_text(
            &nowhere(),
            "queue",
            r#"{"pack_dirs":["x"],"summary":"yes"}"#,
        );
        assert_eq!(text, "summary must be true or false.");
        let (text, _) = call_text(&nowhere(), "schedule", r#"{"kind":"budget","input":"x"}"#);
        assert_eq!(
            text,
            "kind must be expenses, migration, interentity, prepayments or rollforward."
        );
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "table of missing required arguments for each added tool"
    )]
    fn added_tools_refuse_missing_arguments() {
        let temp = TempDir::new();
        let dir = temp.join("args");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.csv"), "h\n").unwrap();
        std::fs::write(dir.join("b.csv"), "h\n").unwrap();
        let root = Root::open(&dir).unwrap();
        for (tool, arguments, expected) in [
            ("duplicates", r#"{}"#, "documents is required."),
            (
                "duplicates",
                r#"{"documents":"a.csv"}"#,
                "currency is required.",
            ),
            (
                "duplicates",
                r#"{"documents":"a.csv","currency":"AUD"}"#,
                "period_start is required.",
            ),
            (
                "duplicates",
                r#"{"documents":"a.csv","currency":"AUD","period_start":"2026-09-01"}"#,
                "period_end is required.",
            ),
            ("fixed-assets", r#"{}"#, "register is required."),
            (
                "fixed-assets",
                r#"{"register":"a.csv"}"#,
                "trial_balance is required.",
            ),
            (
                "fixed-assets",
                r#"{"register":"a.csv","trial_balance":"b.csv"}"#,
                "currency is required.",
            ),
            ("coding", r#"{}"#, "lines is required."),
            ("coding", r#"{"lines":"a.csv"}"#, "currency is required."),
            (
                "coding",
                r#"{"lines":"a.csv","currency":"AUD"}"#,
                "period_start is required.",
            ),
            (
                "coding",
                r#"{"lines":"a.csv","currency":"AUD","period_start":"2026-09-01"}"#,
                "period_end is required.",
            ),
            ("payroll", r#"{}"#, "register is required."),
            ("statement", r#"{}"#, "statement is required."),
            (
                "statement",
                r#"{"statement":"a.csv"}"#,
                "currency is required.",
            ),
            ("bas", r#"{}"#, "ledger is required."),
            ("bas", r#"{"ledger":"a.csv"}"#, "tax_codes is required."),
            (
                "bas",
                r#"{"ledger":"a.csv","tax_codes":"b.csv","payroll":"/bad"}"#,
                "/bad: give a path relative",
            ),
            (
                "bas",
                r#"{"ledger":"a.csv","tax_codes":"b.csv"}"#,
                "currency is required.",
            ),
            (
                "bas",
                r#"{"ledger":"a.csv","tax_codes":"b.csv","currency":"AUD"}"#,
                "period_start is required.",
            ),
            (
                "bas",
                r#"{"ledger":"a.csv","tax_codes":"b.csv","currency":"AUD","period_start":"2026-07-01"}"#,
                "period_end is required.",
            ),
            (
                "bas",
                r#"{"ledger":"a.csv","tax_codes":"b.csv","currency":"AUD","period_start":"2026-07-01","period_end":"2026-09-30"}"#,
                "basis is required.",
            ),
            ("prepost", r#"{}"#, "drafts is required."),
            ("prepost", r#"{"drafts":"a.csv"}"#, "posted is required."),
            (
                "prepost",
                r#"{"drafts":"a.csv","posted":"b.csv"}"#,
                "currency is required.",
            ),
            (
                "prepost",
                r#"{"drafts":"a.csv","posted":"b.csv","currency":"AUD"}"#,
                "period_start is required.",
            ),
            (
                "prepost",
                r#"{"drafts":"a.csv","posted":"b.csv","currency":"AUD","period_start":"2026-08-01"}"#,
                "period_end is required.",
            ),
            ("overdue", r#"{}"#, "invoices is required."),
            (
                "overdue",
                r#"{"invoices":"a.csv"}"#,
                "currency is required.",
            ),
            (
                "overdue",
                r#"{"invoices":"a.csv","currency":"AUD"}"#,
                "as_at is required.",
            ),
        ] {
            let (text, failed) = call_text(&root, tool, arguments);
            assert!(failed, "{tool} {arguments}: {text}");
            assert!(text.starts_with(expected), "{tool} {arguments}: {text}");
        }
    }

    #[test]
    fn envelopes_ids_and_params_are_checked_before_dispatch() {
        let bad = reply(r#"{"jsonrpc":"1.0","id":[],"method":"ping"}"#, &nowhere());
        assert_eq!(error_code(&bad), Some((-32600).into()));
        assert!(matches!(bad.at("id"), Json::Null));
        for line in [
            r#"{"id":1,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
        ] {
            assert_eq!(
                error_code(&reply(line, &nowhere())),
                Some((-32600).into()),
                "{line}"
            );
        }
        let scalar = reply(
            r#"{"jsonrpc":"2.0","id":"p","method":"ping","params":3}"#,
            &nowhere(),
        );
        assert_eq!(error_code(&scalar), Some((-32602).into()));
        assert!(scalar.at("id").is_str("p"));
        let notification = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":3}"#;
        assert!(handle(notification, &nowhere()).is_none());
        let ping = reply(r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#, &nowhere());
        assert!(ping.get("result").is_some());
    }

    fn jet_arguments(raw: &str) -> String {
        format!(
            r#"{{"transactions":{},"currency":"AUD","period_end":"2026-07-31"}}"#,
            quoted(Path::new(raw))
        )
    }

    fn refused(root: &Root, raw: &str) -> String {
        let (text, failed) = call_text(root, "jet", &jet_arguments(raw));
        assert!(failed, "{raw}: {text}");
        text
    }

    fn journals() -> PathBuf {
        crate::test_support::examples().join("journal_entries.csv")
    }

    /// A folder `inside` holding `sub/f.csv`, with `outside.csv` beside it.
    fn client(temp: &TempDir) -> (PathBuf, Root) {
        let inside = temp.join("inside");
        std::fs::create_dir_all(inside.join("sub")).unwrap();
        for file in [inside.join("sub").join("f.csv"), temp.join("outside.csv")] {
            std::fs::copy(journals(), file).unwrap();
        }
        let root = Root::open(&inside).unwrap();
        (inside, root)
    }

    // S3, S5, S7, S9, S10 and S11 came from comparing or normalising
    // spellings of absolute paths; a path is now plain names under the root.
    #[test]
    fn paths_are_plain_names_beneath_the_root() {
        let temp = TempDir::new();
        let (inside, root) = client(&temp);
        let absolute = inside.join("sub").join("f.csv");
        for raw in [
            "",
            "/sub/f.csv",
            absolute.to_str().unwrap(),
            r"\\server\share\f.csv",
            "//server/share/f.csv",
            r"\\?\UNC\server\share\f.csv",
            r"\\?\C:\sub\f.csv",
            r"\\.\pipe\name",
            "C:/sub/f.csv",
            "C:f.csv",
            "../outside.csv",
            "sub/../../outside.csv",
            "sub/../sub/f.csv",
            "./sub/f.csv",
            "sub/",
            "sub/.",
            "sub//f.csv",
            r"sub\f.csv",
            r"sub\..\..\outside.csv",
            "sub/f.csv:stream",
            "sub/f.csv\0",
        ] {
            let text = refused(&root, raw);
            assert!(text.ends_with("no ., .., \\ or :."), "{raw}: {text}");
        }
        let (_, failed) = call_text(&root, "jet", &jet_arguments("sub/f.csv"));
        assert!(!failed);
    }

    // S7 and S8: Windows trims trailing dots and spaces from names in a
    // full path; names opened through a handle are taken as written.
    #[test]
    fn names_are_looked_up_as_written() {
        let temp = TempDir::new();
        let (inside, root) = client(&temp);
        // Windows' verbatim form keeps the trailing space when creating it.
        let spaced = std::fs::canonicalize(&inside).unwrap().join("hop ");
        std::fs::create_dir(&spaced).unwrap();
        std::fs::copy(journals(), spaced.join("f.csv")).unwrap();
        let (_, failed) = call_text(&root, "jet", &jet_arguments("hop /f.csv"));
        assert!(!failed);
        for raw in ["hop/f.csv", "sub./f.csv", "sub /f.csv", "sub/f.csv."] {
            refused(&root, raw);
        }
    }

    // S1, S3 and S6: a link is refused at every step, wherever it points.
    #[cfg(unix)]
    #[test]
    fn links_are_refused_at_every_step() {
        use std::os::unix::fs::symlink;
        let temp = TempDir::new();
        let (inside, root) = client(&temp);
        symlink(temp.join("outside.csv"), inside.join("out.csv")).unwrap();
        symlink(inside.join("sub").join("f.csv"), inside.join("in.csv")).unwrap();
        symlink(&temp.0, inside.join("up")).unwrap();
        symlink(inside.join("sub"), inside.join("alias")).unwrap();
        let pack = inside.join("pack");
        std::fs::create_dir(&pack).unwrap();
        symlink(temp.join("outside.csv"), pack.join(viewer::PACK_NAMES[0])).unwrap();
        for raw in ["out.csv", "in.csv", "up/outside.csv", "alias/f.csv"] {
            let text = refused(&root, raw);
            assert!(text.ends_with("links are not followed"), "{raw}: {text}");
        }
        let (text, failed) = call_text(&root, "view", r#"{"pack_dir":"pack"}"#);
        assert!(failed && text.contains("links are not followed"), "{text}");
        let (text, failed) = call_text(&root, "view", r#"{"pack_dir":"alias"}"#);
        assert!(failed && text.ends_with("links are not followed"), "{text}");
    }

    // S8: a junction is a reparse point and is refused like a link.
    #[cfg(windows)]
    #[test]
    fn junctions_are_refused() {
        let temp = TempDir::new();
        let (inside, root) = client(&temp);
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(inside.join("hop"))
            .arg(&temp.0)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
        let text = refused(&root, "hop/outside.csv");
        assert!(text.ends_with("links are not followed"), "{text}");
        // The name is taken as written, so 'hop ' is not the junction.
        let text = refused(&root, "hop /outside.csv");
        assert!(!text.contains("links"), "{text}");
    }

    // S4: pipes are refused without waiting for a writer.
    #[cfg(unix)]
    #[test]
    fn pipes_are_refused_without_waiting() {
        let temp = TempDir::new();
        let (inside, root) = client(&temp);
        let pack = inside.join("pack");
        std::fs::create_dir(&pack).unwrap();
        for fifo in [inside.join("p.csv"), pack.join(viewer::PACK_NAMES[0])] {
            let made = std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap();
            assert!(made.success());
        }
        let text = refused(&root, "p.csv");
        assert!(text.ends_with("is not a regular file"), "{text}");
        let (text, failed) = call_text(&root, "view", r#"{"pack_dir":"pack"}"#);
        assert!(failed && text.contains("is not a regular file"), "{text}");
    }

    // S6: the root is opened once, so renaming it or reaching it through an
    // alias admits nothing else.
    #[cfg(unix)]
    #[test]
    fn the_root_is_opened_once() {
        let temp = TempDir::new();
        let (inside, _) = client(&temp);
        let alias = temp.join("alias");
        std::os::unix::fs::symlink(inside.join("sub"), &alias).unwrap();
        let root = Root::open(&alias).unwrap();
        let (_, failed) = call_text(&root, "jet", &jet_arguments("f.csv"));
        assert!(!failed);
        refused(&root, "../f.csv");
        // classify, like queue, resolves each path before reading it.
        for name in ["transactions.csv", "coding-evidence.json"] {
            let example = crate::test_support::examples().join("classification");
            std::fs::copy(example.join(name), inside.join("sub").join(name)).unwrap();
        }
        let (text, failed) = call_text(
            &root,
            "classify",
            r#"{"original_transactions":"transactions.csv","current_transactions":"transactions.csv","coding_evidence":"coding-evidence.json","currency":"AUD"}"#,
        );
        assert!(!failed, "{text}");
        let root = Root::open(&inside).unwrap();
        std::fs::rename(&inside, temp.join("moved")).unwrap();
        std::fs::create_dir_all(inside.join("sub")).unwrap();
        std::fs::copy(journals(), inside.join("sub").join("g.csv")).unwrap();
        let (_, failed) = call_text(&root, "jet", &jet_arguments("sub/f.csv"));
        assert!(!failed);
        refused(&root, "sub/g.csv");
    }

    #[test]
    fn tools_read_only_regular_files() {
        let temp = TempDir::new();
        let (inside, root) = client(&temp);
        std::fs::create_dir_all(inside.join("pack").join(viewer::PACK_NAMES[0])).unwrap();
        let (text, failed) = call_text(&root, "view", r#"{"pack_dir":"pack"}"#);
        assert!(failed && text.contains("found a directory"), "{text}");
        let (text, _) = call_text(&root, "queue", r#"{"pack_dirs":["pack"]}"#);
        assert!(text.contains("found a directory"), "{text}");
        let (text, _) = call_text(&root, "view", r#"{"pack_dir":"sub/f.csv"}"#);
        assert!(text.starts_with("sub/f.csv: "), "{text}");
        let text = refused(&root, "sub");
        assert!(text.ends_with("is not a regular file"), "{text}");
        let text = refused(&root, "absent.csv");
        assert!(text.starts_with("absent.csv: "), "{text}");
        let (text, _) = call_text(&root, "view", r#"{"pack_dir":"absent"}"#);
        assert!(text.starts_with("absent: "), "{text}");
        assert!(
            Root::open(&temp.join("absent"))
                .unwrap_err()
                .starts_with("directory ")
        );
        assert!(Root::open(&temp.join("outside.csv")).is_err());
    }

    #[test]
    fn reads_outside_a_tool_call_are_not_confined() {
        let temp = TempDir::new();
        let (inside, root) = client(&temp);
        let outside = temp.join("outside.csv");
        assert!(crate::beneath::read(&outside).is_ok());
        // The root itself and a path that climbs back out are not files
        // beneath it, however they are reached.
        for path in [&outside, &inside, &inside.join("..").join("outside.csv")] {
            assert!(root.within(|| crate::beneath::read(path)).is_err());
        }
        assert!(crate::beneath::read(&outside).is_ok());
        assert!(Root::open(Path::new("")).is_err());
    }

    #[test]
    fn serve_answers_each_line_and_skips_blank_ones() {
        let input = b"\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\r\n\xff\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n".to_vec();
        let mut output = Vec::new();
        serve(std::io::Cursor::new(input), &mut output, &nowhere()).unwrap();
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
