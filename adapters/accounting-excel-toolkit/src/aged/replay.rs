use super::{Json, hash, json, records};
use crate::transactions::csv;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub const CASES: &[(&str, u8, &str)] = &[
    ("clean", 0, ""),
    ("wrong-hash", 2, "exact source bytes"),
    ("row-total", 2, "record 9 buckets"),
    ("column-offset", 2, "contact column Current"),
    ("wrong-period", 2, "control tie is unsupported"),
    ("missing-control", 2, "control evidence is missing"),
    ("missing-footer", 2, "footer has not been identified"),
    ("short-record", 1, "field count"),
    ("extra-record", 1, "field count"),
    ("bad-amount", 1, "finite signed amount"),
    ("unterminated-quote", 1, "unexpected end"),
    ("after-footer", 1, "Unexpected record"),
];
const SOURCE: &[u8] = include_bytes!("../../samples/sample-aged-receivables-review.csv");
const MANIFEST: &str = include_str!("../../samples/sample-aged-receivables-manifest.json");
const CONTROL: &str = include_str!("../../samples/sample-aged-receivables-control.json");

pub struct Scratch {
    pub root: PathBuf,
    parent: PathBuf,
}
impl Scratch {
    pub fn new() -> Result<Self, String> {
        let parent = fs::canonicalize(std::env::temp_dir()).map_err(|e| e.to_string())?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let root = parent.join(format!(
            "fabricated-aged-replay-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir(&root).map_err(|e| e.to_string())?;
        Ok(Self { root, parent })
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        if self.root.parent() == Some(&self.parent)
            && fs::symlink_metadata(&self.root).is_ok_and(|m| !m.file_type().is_symlink())
        {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

pub fn case(name: &str) -> Result<(Vec<u8>, Json, Option<Json>), String> {
    if !CASES.iter().any(|(n, _, _)| *n == name) {
        return Err("Unknown replay case.".into());
    }
    let mut rows = records(SOURCE)?;
    let mut manifest = json::loads(MANIFEST).map_err(|e| format!("{e:?}"))?;
    let mut control = Some(json::loads(CONTROL).map_err(|e| format!("{e:?}"))?);
    match name {
        "row-total" => *rows[8].last_mut().unwrap() = "11".into(),
        "column-offset" => {
            let row = rows.last_mut().unwrap();
            row[1] = "90".into();
            let last = row.len() - 2;
            row[last] = "10".into();
        }
        "wrong-period" => control
            .as_mut()
            .unwrap()
            .set("as_at", Json::str("2026-08-31")),
        "missing-control" => control = None,
        "missing-footer" => manifest.set("footer_record", Json::Null),
        "short-record" => {
            rows[6].pop();
        }
        "extra-record" => rows[6].push("extra".into()),
        "bad-amount" => rows[6][1] = "TBC".into(),
        "after-footer" => rows.push(
            ["Fabricated", "0", "0", "0", "0", "0", "0", "0"]
                .map(str::to_owned)
                .to_vec(),
        ),
        _ => {}
    }
    let mut content = encode(&rows);
    if name == "unterminated-quote" {
        content.push_str("\"unfinished");
    }
    manifest.set(
        "source_sha256",
        Json::str(&if name == "wrong-hash" {
            "0".repeat(64)
        } else {
            hash(content.as_bytes())
        }),
    );
    Ok((content.into_bytes(), manifest, control))
}

fn encode(rows: &[Vec<String>]) -> String {
    let mut content = String::new();
    for row in rows {
        let refs: Vec<_> = row.iter().map(String::as_str).collect();
        let encoded = csv::write_record(&refs);
        content.push_str(encoded.strip_suffix('\n').unwrap());
        content.push_str("\r\n");
    }
    content
}

pub fn pack(selected: &str, verifier: &Path) -> Result<Json, String> {
    if selected != "all" && !CASES.iter().any(|(n, _, _)| *n == selected) {
        return Err("Unknown replay case.".into());
    }
    let scratch = Scratch::new()?;
    let (source, manifest, control) = (
        scratch.root.join("source.csv"),
        scratch.root.join("manifest.json"),
        scratch.root.join("control.json"),
    );
    let mut outcomes = Vec::new();
    for &(name, expected, message) in CASES
        .iter()
        .filter(|(n, _, _)| selected == "all" || *n == selected)
    {
        let (content, evidence, independent) = case(name)?;
        fs::write(&source, &content).map_err(|e| e.to_string())?;
        fs::write(&manifest, json::dumps(&evidence, false)).map_err(|e| e.to_string())?;
        let mut command = Command::new(verifier);
        command.arg(&source).arg("--manifest").arg(&manifest);
        if let Some(value) = independent {
            fs::write(&control, json::dumps(&value, false)).map_err(|e| e.to_string())?;
            command.arg("--control").arg(&control);
        }
        let actual = command.output().map_err(|e| e.to_string())?;
        let body = if actual.status.code() == Some(1) {
            &actual.stderr
        } else {
            &actual.stdout
        };
        let body = std::str::from_utf8(body).map_err(|e| e.to_string())?;
        let response = json::loads(body).map_err(|e| format!("Replay {name}: {e:?}"))?;
        validate(
            name,
            expected,
            message,
            actual.status.code(),
            body,
            &response,
        )?;
        outcomes.push(Json::object([
            ("case", Json::str(name)),
            ("expected_exit", Json::int(i64::from(expected))),
            ("actual_exit", Json::int(i64::from(expected))),
            ("source_sha256", Json::str(&hash(&content))),
            ("response", response),
        ]));
    }
    Ok(pack_result(outcomes))
}

fn pack_result(outcomes: Vec<Json>) -> Json {
    Json::object([
        ("base_sha256", Json::str(&hash(SOURCE))),
        ("cases", Json::Array(outcomes)),
        (
            "boundary",
            Json::str("Fabricated failure demonstrations; no accounting decision or assurance."),
        ),
    ])
}

fn validate(
    name: &str,
    expected: u8,
    message: &str,
    actual: Option<i32>,
    body: &str,
    response: &Json,
) -> Result<(), String> {
    let status = match expected {
        0 => "PASS",
        2 => "REVIEW",
        _ => "INVALID_INPUT",
    };
    if actual != Some(i32::from(expected))
        || !response
            .get("summary_status")
            .is_some_and(|v| v.is_str(status))
        || (!message.is_empty() && !body.to_lowercase().contains(&message.to_lowercase()))
        || (expected != 1
            && !response
                .get("debtor_decisions")
                .is_some_and(|v| v.is_str("REVIEW")))
    {
        return Err(format!(
            "Replay {name} failed its output contract: {}: {body}",
            actual.map_or_else(|| "signal".to_owned(), |n| n.to_string())
        ));
    }
    Ok(())
}
