use crate::{
    Result,
    decimal::Decimal,
    models::*,
    pycsv,
    pyjson::{Json, PyStr},
    pypath, pytext,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const REVIEW_BOUNDARY: &str = "This pack is a review aid. It does not approve a file, lodge a return, or replace professional judgement.";
pub const ACKNOWLEDGEMENT_EFFECT: &str = "Acknowledgement is evidence of human review only; it does not change readiness or approve a file.";
pub const PACK_NAMES: [&str; 3] = [
    "readiness-pack.json",
    "readiness-summary.md",
    "findings.csv",
];
pub const COMPONENT: &str = "review-ready-gate";
/// The pack layout a `producer` block declares. Packs without the block are contract 1.
pub const PACK_CONTRACT: &str = "2";
/// The commit recorded in `producer`: the full SHA given in
/// `ACCOUNTING_REVIEW_SOURCE_COMMIT` at build time, otherwise `unknown`.
pub const SOURCE_COMMIT: &str = match option_env!("ACCOUNTING_REVIEW_SOURCE_COMMIT") {
    Some(commit) => commit,
    None => "unknown",
};
const _: () = assert!(
    option_env!("ACCOUNTING_REVIEW_SOURCE_COMMIT").is_none() || is_commit(SOURCE_COMMIT),
    "ACCOUNTING_REVIEW_SOURCE_COMMIT must be a 40-character lowercase hexadecimal commit"
);
pub const fn is_commit(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if !matches!(bytes[index], b'0'..=b'9' | b'a'..=b'f') {
            return false;
        }
        index += 1;
    }
    bytes.len() == 40
}
/// Identifies the build that wrote a pack. It holds no time, host or path, so
/// the same build and inputs still write byte-identical packs.
pub fn producer() -> Json {
    Json::object([
        ("commit", Json::str(SOURCE_COMMIT)),
        ("component", Json::str(COMPONENT)),
        ("pack_contract", Json::str(PACK_CONTRACT)),
        ("version", Json::str(env!("CARGO_PKG_VERSION"))),
    ])
}
fn producer_line(producer: &Json) -> String {
    let field = |key| producer.at(key).as_pystr().unwrap().os_text();
    format!(
        "- Produced by: {} {}, commit {}, pack contract {}.",
        field("component"),
        field("version"),
        field("commit"),
        field("pack_contract")
    )
}
pub const CSV_FIELDS: [&str; 6] = [
    "code",
    "status",
    "slot",
    "repeat",
    "reason",
    "reviewer_action",
];

pub fn money(value: &Decimal) -> Result<String> {
    value
        .format_places(2.max(-value.exponent()))
        .ok_or_else(|| "decimal fixed-point output exceeds available memory".into())
}
pub fn csv_safe(value: &str) -> String {
    if value
        .trim_start_matches(|c: char| pytext::is_space(c as u32))
        .starts_with(['=', '+', '-', '@'])
    {
        format!("'{value}")
    } else {
        value.into()
    }
}
pub fn md_cell(value: &str) -> String {
    value
        .split(|c: char| pytext::is_space(c as u32))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('*', "\\*")
        .replace('`', "\\`")
}
fn note_lines(comment: &str) -> Vec<String> {
    if !comment.contains(['\r', '\n']) {
        return vec![format!("- Comment: {}", md_cell(comment))];
    }
    let mut lines = vec!["- Comment:".into()];
    for raw in pytext::split_lines(comment) {
        let mut line = md_cell(raw);
        if line.starts_with('#') {
            line.insert(0, '\\');
        }
        lines.push(format!("  > {line}").trim_end().to_owned());
    }
    lines
}
pub fn finding_json(f: &Finding) -> Json {
    Json::object([
        ("code", Json::str(&f.code)),
        ("status", Json::str(&f.status)),
        ("slot", Json::str(&f.slot)),
        ("repeat", Json::str(if f.repeat { "true" } else { "false" })),
        ("reason", Json::str(&f.reason)),
        ("reviewer_action", Json::str(&f.reviewer_action)),
    ])
}
pub fn as_json(pack: &ReadinessPack) -> Result<Json> {
    let acknowledgement = pack.acknowledgement.as_ref().map_or(Json::Null, |a| {
        Json::object([
            ("reviewer_initials", Json::str(&a.reviewer_initials)),
            ("reviewed_on", Json::str(&a.reviewed_on.iso())),
            ("comment", Json::str(&a.comment)),
            ("effect", Json::str(ACKNOWLEDGEMENT_EFFECT)),
        ])
    });
    let mut pairs = vec![
        (PyStr::from("acknowledgement"), acknowledgement),
        (
            PyStr::from("engagement_type"),
            Json::str(&pack.engagement_type),
        ),
        (
            PyStr::from("findings"),
            Json::Array(pack.findings.iter().map(finding_json).collect()),
        ),
        (PyStr::from("overall_status"), Json::str(&pack.status)),
        (PyStr::from("period_end"), Json::str(&pack.period_end)),
        (
            PyStr::from("preparer_initials"),
            Json::str(&pack.preparer_initials),
        ),
        (PyStr::from("review_boundary"), Json::str(REVIEW_BOUNDARY)),
        (
            PyStr::from("source_sha256"),
            Json::Object(
                pack.source_evidence
                    .iter()
                    .map(|e| {
                        (
                            PyStr::from(e.slot.as_str()),
                            Json::object([
                                ("filename", Json::str(&e.filename)),
                                ("sha256", Json::str(&e.sha256)),
                            ]),
                        )
                    })
                    .collect(),
            ),
        ),
        (
            PyStr::from("thresholds"),
            Json::object([(
                "tieout_tolerance",
                Json::str(&money(&pack.tieout_tolerance)?),
            )]),
        ),
    ];
    if let Some(controls) = &pack.controls_not_run {
        pairs.push((
            PyStr::from("controls_not_run"),
            Json::Array(
                controls
                    .iter()
                    .map(|c| {
                        Json::object([
                            ("slot", Json::str(&c.slot)),
                            ("filename", Json::str(&c.filename)),
                            ("reason", Json::str(&c.reason)),
                        ])
                    })
                    .collect(),
            ),
        ));
    }
    if let Some(producer) = &pack.producer {
        pairs.push((PyStr::from("producer"), producer.clone()));
    }
    Ok(Json::Object(pairs))
}
pub fn as_markdown(pack: &ReadinessPack) -> Result<String> {
    let absent = |s: &str| {
        if s.is_empty() {
            "n/a".to_owned()
        } else {
            s.to_owned()
        }
    };
    let blocked = pack
        .findings
        .iter()
        .filter(|f| f.status == "BLOCKED")
        .count();
    let not_ready = pack
        .findings
        .iter()
        .filter(|f| f.status == "NOT_READY")
        .count();
    let repeats = pack.findings.iter().filter(|f| f.repeat).count();
    let mut lines = vec![
        "# Review-Ready Pack".into(),
        "".into(),
        format!("**Overall status: {}**", pack.status),
        "".into(),
        REVIEW_BOUNDARY.into(),
        "".into(),
        "## Scope".into(),
        "".into(),
        format!("- Engagement type: {}", pack.engagement_type),
        format!("- Period end: {}", absent(&pack.period_end)),
        format!(
            "- Preparer initials: {}",
            if pack.preparer_initials.is_empty() {
                "n/a".into()
            } else {
                md_cell(&pack.preparer_initials)
            }
        ),
        format!("- Tie-out tolerance: ${}", money(&pack.tieout_tolerance)?),
        format!(
            "- Findings: {} total; {blocked} blocked; {not_ready} not ready; {repeats} repeats.",
            pack.findings.len()
        ),
    ];
    if let Some(controls) = &pack.controls_not_run {
        lines.push(format!("- Controls not run: {}.", controls.len()));
    }
    lines.extend(pack.producer.as_ref().map(producer_line));
    lines.extend(["".into(), "## Source evidence".into(), "".into()]);
    if pack.source_evidence.is_empty() {
        lines.push("No source files were present in the pack directory.".into());
    } else {
        for e in &pack.source_evidence {
            lines.push(format!("- `{}` (`{}`): `{}`", e.slot, e.filename, e.sha256));
        }
    }
    if let Some(controls) = &pack.controls_not_run {
        lines.extend(["".into(), "## Controls not run".into(), "".into()]);
        if controls.is_empty() {
            lines.push("Every control this engagement profile configures ran.".into());
        } else {
            lines.push("The status above says nothing about these controls. They had no usable input, so they reached no verdict:".into());
            for c in controls {
                lines.push(format!("- `{}` (`{}`): {}", c.slot, c.filename, c.reason));
            }
        }
    }
    lines.extend(["".into(), "## Findings".into(), "".into()]);
    if pack.findings.is_empty() {
        lines.push("No readiness findings were raised. A human must still decide whether the file is acceptable.".into());
    } else {
        lines.extend([
            "| Status | Code | Slot | Repeat | Reason |".into(),
            "| --- | --- | --- | --- | --- |".into(),
        ]);
        for f in &pack.findings {
            lines.push(format!(
                "| {} | {} | {} | {} | {} |",
                f.status,
                f.code,
                md_cell(&f.slot),
                if f.repeat { "yes" } else { "no" },
                md_cell(&f.reason)
            ));
        }
    }
    lines.extend(["".into(), "## Human acknowledgement".into(), "".into()]);
    if let Some(a) = &pack.acknowledgement {
        lines.extend([
            format!("- Reviewer initials: {}", md_cell(&a.reviewer_initials)),
            format!("- Reviewed on: {}", a.reviewed_on.iso()),
        ]);
        lines.extend(note_lines(&a.comment));
        lines.push("- Effect: acknowledgement records a human action only; it does not change readiness or approve a file.".into());
    } else {
        lines.push(
            "No reviewer acknowledgement was supplied. This does not create or imply an approval."
                .into(),
        );
    }
    lines.push(String::new());
    Ok(lines.join("\n"))
}
pub fn as_csv(pack: &ReadinessPack) -> String {
    let mut out = pycsv::write_record(&CSV_FIELDS);
    for f in &pack.findings {
        out.push_str(&pycsv::write_record(&[
            &f.code,
            &f.status,
            &csv_safe(&f.slot),
            if f.repeat { "true" } else { "false" },
            &csv_safe(&f.reason),
            &csv_safe(&f.reviewer_action),
        ]));
    }
    out
}

fn enclosing_repository(directory: &Path) -> std::io::Result<Option<(PathBuf, String)>> {
    for candidate in directory.ancestors() {
        for marker in [".git", ".hg", ".svn", ".bzr"] {
            match fs::symlink_metadata(candidate.join(marker)) {
                Ok(_) => {
                    return Ok(Some((
                        candidate.to_owned(),
                        format!("which holds {marker}"),
                    )));
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) => {}
                Err(e) => return Err(e),
            }
        }
    }
    // This is the single non-secret Git setting the original output guard reads.
    if let Some(value) = std::env::var_os("GIT_WORK_TREE").filter(|s| !s.is_empty()) {
        let work_tree = pypath::resolve(Path::new(&value), false)
            .map_err(|_| std::io::Error::other("cannot resolve GIT_WORK_TREE"))?;
        for candidate in directory.ancestors() {
            if pypath::same_path(candidate, &work_tree) {
                return Ok(Some((
                    work_tree,
                    "which the GIT_WORK_TREE environment variable names".into(),
                )));
            }
            match (pypath::file_id(candidate), pypath::file_id(&work_tree)) {
                (Ok(a), Ok(b)) if a == b => {
                    return Ok(Some((
                        work_tree,
                        "which the GIT_WORK_TREE environment variable names".into(),
                    )));
                }
                (Err(e), _) | (_, Err(e))
                    if !matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                    ) =>
                {
                    return Err(e);
                }
                _ => {}
            }
        }
    }
    Ok(None)
}
pub fn require_output_outside_repository(output: &Path) -> Result<PathBuf> {
    let inspect = || -> std::io::Result<(PathBuf, Option<(PathBuf, String)>)> {
        let resolved = pypath::resolve(output, false)
            .map_err(|_| std::io::Error::other("cannot resolve output path"))?;
        for candidate in resolved.ancestors() {
            match fs::metadata(candidate) {
                Ok(_) => break,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            }
        }
        let checkout = enclosing_repository(&resolved)?;
        Ok((resolved, checkout))
    };
    let (resolved, checkout) = inspect().map_err(|e| format!("{} cannot be examined for an enclosing version-control checkout: {e}. An output this run cannot inspect is refused rather than written to, because a readiness pack names a client's file and its open findings. Point --output somewhere readable.", pypath::text(output)))?;
    if let Some((repo, reason)) = checkout {
        return Err(format!(
            "{} is inside the version-control checkout at {}, {reason}. A readiness pack names a client's file and its open findings, so it belongs in an access-controlled directory outside version control. Point --output somewhere outside that checkout.",
            pypath::text(output),
            pypath::text(&repo)
        ));
    }
    Ok(resolved)
}

pub fn rendered(pack: &ReadinessPack) -> Result<[Vec<u8>; 3]> {
    let native = |text: String| {
        if cfg!(windows) {
            text.replace('\n', "\r\n").into_bytes()
        } else {
            text.into_bytes()
        }
    };
    let json = crate::pyjson::pretty(&as_json(pack)?) + "\n";
    let mut csv = b"\xef\xbb\xbf".to_vec();
    csv.extend(as_csv(pack).as_bytes());
    Ok([native(json), native(as_markdown(pack)?), csv])
}
pub fn write_review_pack(pack: &ReadinessPack, output: &Path) -> Result<[PathBuf; 3]> {
    let output = require_output_outside_repository(output)?;
    fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    crate::publication::publish(&output, &rendered(pack)?).map_err(|e| e.to_string())?;
    Ok(PACK_NAMES.map(|name| output.join(name)))
}
