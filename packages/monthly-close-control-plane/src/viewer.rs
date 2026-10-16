//! Verify the saved artefacts before displaying any review result.
use crate::{
    Result,
    decimal::Decimal,
    equity, pycsv,
    pyjson::{self, Json, LoadError, PyStr},
    pypath, pytext,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub const PACK_NAMES: [&str; 4] = [
    "close-review-pack.json",
    "close-summary.md",
    "exceptions.csv",
    "client-queries.csv",
];
const J: &str = PACK_NAMES[0];
const S: &str = PACK_NAMES[1];
const C: &str = PACK_NAMES[2];
const Q: &str = PACK_NAMES[3];
const REQUIRED: [&str; 7] = [
    "acknowledgement",
    "current_report_dates",
    "exceptions",
    "overall_status",
    "prior_report_dates",
    "source_sha256",
    "thresholds",
];
const OPTIONAL: [&str; 4] = [
    "client_queries",
    "calculation_evidence",
    "controls_not_run",
    "equity_reconciliation",
];
const THRESHOLDS: [&str; 3] = [
    "absolute_variance",
    "percentage_variance",
    "reconciliation_tolerance",
];
const ACK_FIELDS: [&str; 4] = ["reviewer_initials", "reviewed_on", "comment", "effect"];
const EXCEPTION_FIELDS: [&str; 14] = [
    "control",
    "status",
    "tenant",
    "account_id",
    "account_code",
    "account_name",
    "review_group",
    "current_value",
    "prior_value",
    "difference",
    "threshold",
    "percentage_change",
    "reason",
    "reviewer_action",
];
const QUERY_FIELDS: [&str; 10] = [
    "query_id",
    "control",
    "tenant",
    "account_id",
    "account_code",
    "account_name",
    "review_group",
    "difference",
    "question",
    "evidence_requested",
];
const ACK_EFFECT: &str =
    "Acknowledgement is evidence of human review only; it does not approve or close a period.";
const STATUSES: [&str; 3] = ["PASS", "REVIEW", "BLOCKED"];
const BOUNDARY: &str = "This pack is a review aid. It does not approve a close, post a journal, make a payment, lodge a return, or lock a period.";
static NULL: Json = Json::Null;
fn get<'a>(value: &'a Json, key: &str) -> &'a Json {
    value.get(key).unwrap_or(&NULL)
}
fn text(value: &Json) -> String {
    // Encode JSON and summary text alike so lone surrogates cannot match literal private-use text.
    value.as_pystr().expect("validated string").os_text()
}
fn string_list<'a>(document: &'a Json, member: &str) -> Result<&'a [Json]> {
    document
        .at(member)
        .as_array()
        .filter(|v| v.iter().all(|s| s.as_pystr().is_some()))
        .ok_or_else(|| format!("{J}: {member} must be a list of strings"))
}
fn keys(value: &Json) -> Vec<PyStr> {
    let mut keys: Vec<_> = value.keys().unwrap().into_iter().cloned().collect();
    keys.sort();
    keys
}
fn key_repr(value: &Json) -> String {
    pyjson::repr(&Json::Array(
        keys(value).into_iter().map(Json::Str).collect(),
    ))
}
fn fields_repr(fields: &[&str]) -> String {
    let mut fields: Vec<_> = fields.iter().map(|s| PyStr::from(*s)).collect();
    fields.sort();
    pyjson::repr(&Json::Array(fields.into_iter().map(Json::Str).collect()))
}
fn is_status(value: &Json) -> bool {
    STATUSES.iter().any(|s| value.is_str(s))
}
fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
pub(crate) fn parse_json(payload: &[u8]) -> Result<Json> {
    let raw = std::str::from_utf8(payload).map_err(|_| format!("{J}: not valid UTF-8"))?;
    let document = pyjson::loads_detailed(raw).map_err(|(e, diagnostic)| match e {
        LoadError::DuplicateKey(key) => format!(
            "{J}: not readable ({J}: member {} appears more than once)",
            pytext::repr_code_points(&key.0)
        ),
        LoadError::Invalid => format!(
            "{J}: not valid JSON ({})",
            diagnostic.unwrap().rsplit_once(": line ").unwrap().0
        ),
        LoadError::Crash(reason) if reason.starts_with("maximum recursion") => {
            format!("{J}: nested too deeply to read")
        }
        LoadError::Crash(reason) => format!("{J}: not readable ({reason})"),
    })?;
    if !matches!(document, Json::Object(_)) {
        return Err(format!("{J}: top level must be a JSON object"));
    }
    let unknown: Vec<_> = keys(&document)
        .into_iter()
        .filter(|k| !REQUIRED.iter().chain(&OPTIONAL).any(|s| k.is(s)))
        .map(|k| k.os_text())
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "{J}: unknown top-level member(s): {}",
            unknown.join(", ")
        ));
    }
    let mut missing: Vec<_> = REQUIRED
        .iter()
        .filter(|k| document.get(k).is_none())
        .copied()
        .collect();
    missing.sort();
    if !missing.is_empty() {
        return Err(format!(
            "{J}: missing top-level member(s): {}",
            missing.join(", ")
        ));
    }
    Ok(document)
}
fn parse_threshold(value: &Json, key: &str) -> Result<Decimal> {
    let raw = value
        .as_pystr()
        .ok_or_else(|| format!("{J}: thresholds.{key} must be a string"))?
        .os_text();
    let number = Decimal::parse(raw.strip_suffix('%').unwrap_or(&raw)).map_err(|_| {
        format!(
            "{J}: thresholds.{key} is not a decimal: {}",
            pyjson::repr(value)
        )
    })?;
    if !number.is_finite() || number < Decimal::zero() {
        return Err(format!(
            "{J}: thresholds.{key} must be finite and non-negative"
        ));
    }
    Ok(number)
}
fn verify_schema(document: &Json) -> Result<()> {
    if !is_status(document.at("overall_status")) {
        return Err(format!(
            "{J}: overall_status must be one of PASS, REVIEW, BLOCKED; got {}",
            pyjson::repr(document.at("overall_status"))
        ));
    }
    string_list(document, "current_report_dates")?;
    string_list(document, "prior_report_dates")?;
    let thresholds = document.at("thresholds");
    if !thresholds.has_exact_keys(&THRESHOLDS) {
        return Err(format!(
            "{J}: thresholds must hold exactly {}",
            THRESHOLDS.join(", ")
        ));
    }
    for key in THRESHOLDS {
        parse_threshold(thresholds.at(key), key)?;
    }
    let sources = document.at("source_sha256");
    let Json::Object(pairs) = sources else {
        return Err(format!("{J}: source_sha256 must be a non-empty object"));
    };
    if pairs.is_empty() {
        return Err(format!("{J}: source_sha256 must be a non-empty object"));
    }
    for (label, digest) in pairs {
        if !digest.as_pystr().is_some_and(|s| is_digest(&s.os_text())) {
            return Err(format!(
                "{J}: source_sha256[{}] is not a lowercase SHA-256 digest",
                pytext::repr_code_points(&label.0)
            ));
        }
    }
    let exceptions = document
        .at("exceptions")
        .as_array()
        .ok_or_else(|| format!("{J}: exceptions must be a list"))?;
    for (index, item) in exceptions.iter().enumerate() {
        if !matches!(item, Json::Object(_)) {
            return Err(format!("{J}: exceptions[{index}] must be an object"));
        }
        if !item.has_exact_keys(&EXCEPTION_FIELDS) {
            return Err(format!(
                "{J}: exceptions[{index}] does not hold the fields the writer emits: holds {}, expected {}",
                key_repr(item),
                fields_repr(&EXCEPTION_FIELDS)
            ));
        }
        if !is_status(item.at("status")) {
            return Err(format!(
                "{J}: exceptions[{index}].status is not a pack status"
            ));
        }
    }
    if let Some(queries) = document.get("client_queries") {
        let queries = queries
            .as_array()
            .ok_or_else(|| format!("{J}: client_queries must be a list"))?;
        let mut seen = BTreeSet::new();
        for (index, item) in queries.iter().enumerate() {
            if !matches!(item, Json::Object(_)) {
                return Err(format!("{J}: client_queries[{index}] must be an object"));
            }
            if !item.has_exact_keys(&QUERY_FIELDS) {
                return Err(format!(
                    "{J}: client_queries[{index}] does not hold the fields the writer emits: holds {}, expected {}",
                    key_repr(item),
                    fields_repr(&QUERY_FIELDS)
                ));
            }
            for key in QUERY_FIELDS {
                if item.at(key).as_pystr().is_none() {
                    return Err(format!(
                        "{J}: client_queries[{index}].{key} must be a string"
                    ));
                }
            }
            let id = text(item.at("query_id"));
            if id.is_empty() {
                return Err(format!(
                    "{J}: client_queries[{index}].query_id must be a non-empty string"
                ));
            }
            if !seen.insert(id.clone()) {
                return Err(format!("{J}: client_queries holds {id} more than once"));
            }
        }
    }
    let acknowledgement = document.at("acknowledgement");
    if !matches!(acknowledgement, Json::Null) {
        if !matches!(acknowledgement, Json::Object(_)) {
            return Err(format!("{J}: acknowledgement must be null or an object"));
        }
        if !acknowledgement.has_exact_keys(&ACK_FIELDS) {
            return Err(format!(
                "{J}: acknowledgement does not hold the fields the writer emits: holds {}, expected {}",
                key_repr(acknowledgement),
                fields_repr(&ACK_FIELDS)
            ));
        }
        for key in ACK_FIELDS {
            if acknowledgement.at(key).as_pystr().is_none() {
                return Err(format!("{J}: acknowledgement.{key} must be a string"));
            }
        }
        if !acknowledgement.at("effect").is_str(ACK_EFFECT) {
            return Err(format!(
                "{J}: acknowledgement.effect is not the text the writer emits"
            ));
        }
    }
    Ok(())
}
fn read_csv(payload: &[u8], name: &str, fields: &[&str]) -> Result<Vec<Json>> {
    let raw = std::str::from_utf8(payload).map_err(|_| format!("{name}: not valid UTF-8"))?;
    let mut reader = pycsv::Reader::new(raw.strip_prefix('\u{feff}').unwrap_or(raw));
    let error = |e| format!("{name}: not readable as CSV ({e})");
    let header = reader.next().transpose().map_err(error)?;
    if header.as_deref() != Some(&fields.iter().map(|s| s.to_string()).collect::<Vec<_>>()) {
        return Err(format!(
            "{name}: header row does not match the written contract"
        ));
    }
    let mut rows = Vec::new();
    for (index, cells) in reader.enumerate() {
        let cells = cells.map_err(error)?;
        if cells.len() != fields.len() {
            return Err(format!(
                "{name}: row {} holds {} cells, but the header declares {}",
                index + 1,
                cells.len(),
                fields.len()
            ));
        }
        rows.push(Json::Object(
            fields
                .iter()
                .zip(cells)
                .map(|(k, v)| (PyStr::from(*k), Json::str(&v)))
                .collect(),
        ));
    }
    Ok(rows)
}
fn rows_match(
    name: &str,
    member: &str,
    noun: &str,
    json: &[Json],
    csv: &[Json],
    fields: &[&str],
    query: bool,
) -> Result<()> {
    if csv.len() != json.len() {
        return Err(format!(
            "{noun} counts disagree: {J} holds {}, {name} holds {} data rows",
            json.len(),
            csv.len()
        ));
    }
    for (index, (item, row)) in json.iter().zip(csv).enumerate() {
        for key in fields {
            if item.at(key).as_pystr().is_none() {
                return Err(format!("{J}: {member}[{index}].{key} must be a string"));
            }
            let mut expected = text(item.at(key));
            let actual = text(row.at(key));
            if ([
                "tenant",
                "account_id",
                "account_code",
                "account_name",
                "review_group",
            ]
            .contains(key)
                || query && ["question", "evidence_requested"].contains(key))
                && expected
                    .trim_start_matches(|c: char| pytext::is_space(c as u32))
                    .starts_with(['=', '+', '-', '@'])
            {
                expected.insert(0, '\'');
            }
            if actual != expected {
                return Err(format!(
                    "{key} disagrees on {noun} {}: {J} says {}, {name} says {}",
                    index + 1,
                    pytext::repr_os(&expected),
                    pytext::repr_os(&actual)
                ));
            }
        }
    }
    Ok(())
}

const HEADINGS: [&str; 8] = [
    "# Monthly Close Review Pack",
    "## Scope",
    "## Source evidence",
    "## Calculation evidence",
    "## Equity reconciliation",
    "## Exceptions",
    "## Client queries",
    "## Human acknowledgement",
];
const QUERY_HEADER: &str =
    "| Query | Control | Tenant | Account | Difference | Question | Evidence requested |";
const QUERY_DELIMITER: &str = "| --- | --- | --- | --- | ---: | --- | --- |";
const QUERY_PREAMBLE: [&str; 4] = [
    "These are draft questions for the preparer, derived from the exceptions above. Nothing has been sent to anyone. Read and edit them before they reach a client, and record the answers in the firm's own tracker: this pack is evidence of one run, and editing it breaks the check that its files still agree.",
    "",
    "Answering every query does not close the period, and a query nobody raised is not evidence that nothing needs asking.",
    "",
];
const EMPTY_QUERIES: &str = "No exception raised a question for the client. Every exception in this pack, if any, is one the firm settles from its own records.";
const EMPTY_EXCEPTIONS: &str =
    "No exceptions were raised. A human must still decide whether the close is appropriate.";
const NO_ACK: &str =
    "No reviewer acknowledgement was supplied. This does not create or imply an approval.";
const ACK_EFFECT_LINE: &str = "- Effect: acknowledgement records a human action only; it does not change the control status or approve a close.";
fn repr_lines(lines: &[&str]) -> String {
    pyjson::repr(&Json::Array(
        lines
            .iter()
            .map(|s| Json::Str(PyStr::from_os_text(s)))
            .collect(),
    ))
}
fn md_cell(value: &str) -> String {
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
fn normalise(value: &str) -> String {
    value
        .split(|c: char| pytext::is_space(c as u32))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}
fn absent(value: &str) -> &str {
    if value.is_empty() { "n/a" } else { value }
}
fn field(value: &Json, name: &str) -> String {
    text(value.at(name))
}
fn account(value: &Json) -> String {
    let code = field(value, "account_code");
    let name = field(value, "account_name");
    let pieces: Vec<_> = [code, name].into_iter().filter(|s| !s.is_empty()).collect();
    if pieces.is_empty() {
        absent(&field(value, "account_id")).into()
    } else {
        pieces.join(" / ")
    }
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
        lines.push(format!("  > {line}").trim_end().into());
    }
    lines
}
fn indented(line: &str) -> Option<&str> {
    let spaces = line.bytes().take_while(|&b| b == b' ').count();
    (spaces <= 3).then(|| &line[spaces..])
}
fn atx_heading(line: &str) -> bool {
    let Some(line) = indented(line) else {
        return false;
    };
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    (1..=6).contains(&hashes)
        && (hashes == line.len() || matches!(line.as_bytes()[hashes], b' ' | b'\t'))
}
fn setext(line: &str) -> bool {
    let Some(line) = indented(line) else {
        return false;
    };
    let Some(first) = line.bytes().next().filter(|&b| matches!(b, b'=' | b'-')) else {
        return false;
    };
    line.trim_end_matches([' ', '\t'])
        .bytes()
        .all(|b| b == first)
}
fn section_heading(line: &str) -> bool {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    (1..=2).contains(&hashes)
        && line[hashes..]
            .chars()
            .next()
            .is_some_and(|c| pytext::is_space(c as u32))
}
fn verify_headings(summary: &str, register: bool, evidence: bool, equity: bool) -> Result<()> {
    let lines = pytext::split_lines(summary);
    for (index, line) in lines.iter().enumerate() {
        if indented(line)
            .is_some_and(|s| s.starts_with("```") || s.starts_with("~~~") || s.starts_with('<'))
        {
            return Err(format!(
                "{S}: line {} opens a code fence or an HTML block, which the writer never emits: {}",
                index + 1,
                pytext::repr_os(line)
            ));
        }
    }
    let expected: Vec<_> = HEADINGS
        .into_iter()
        .filter(|&s| {
            (register || s != "## Client queries")
                && (evidence || s != "## Calculation evidence")
                && (equity || s != "## Equity reconciliation")
        })
        .collect();
    let found: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(i, line)| {
            atx_heading(line)
                || (!pytext::strip(line).is_empty() && lines.get(i + 1).is_some_and(|s| setext(s)))
        })
        .map(|(_, s)| *s)
        .collect();
    if found == expected {
        return Ok(());
    }
    let with_register: Vec<_> = HEADINGS
        .into_iter()
        .filter(|&s| {
            (evidence || s != "## Calculation evidence")
                && (equity || s != "## Equity reconciliation")
        })
        .collect();
    if !register && found == with_register {
        return Err(format!(
            "{S}: holds a client-query section while the pack carries no client-query register; the register is half removed, not absent"
        ));
    }
    Err(format!(
        "{S}: the headings are not the ones the writer emits: holds {}, expected {}",
        repr_lines(&found),
        repr_lines(&expected)
    ))
}
fn section<'a>(summary: &'a str, heading: &str) -> Result<Vec<&'a str>> {
    let lines = pytext::split_lines(summary);
    let indices: Vec<_> = lines
        .iter()
        .enumerate()
        .filter(|(_, s)| **s == heading)
        .map(|(i, _)| i)
        .collect();
    if indices.len() != 1 {
        return Err(format!(
            "{S}: expected exactly one {} heading, found {}",
            pytext::repr(heading),
            indices.len()
        ));
    }
    let start = indices[0] + 1;
    let end = (start..lines.len())
        .find(|&i| section_heading(lines[i]))
        .unwrap_or(lines.len());
    Ok(lines[start..end].to_vec())
}
fn rebuilt_section(
    summary: &str,
    heading: &str,
    expected: &[String],
    trailing: bool,
) -> Result<()> {
    let actual = section(summary, heading)?;
    let mut wanted = vec![""];
    wanted.extend(expected.iter().map(String::as_str));
    if trailing {
        wanted.push("");
    }
    if actual != wanted {
        return Err(format!(
            "{S}: the {} section disagrees with {J}: renders {}, expected {}",
            pytext::repr(heading.trim_start_matches(['#', ' '])),
            repr_lines(&actual),
            repr_lines(&wanted)
        ));
    }
    Ok(())
}
fn join_strings(values: &[Json], separator: &str) -> String {
    values.iter().map(text).collect::<Vec<_>>().join(separator)
}
fn expected_scope(document: &Json, register: bool) -> Result<Vec<String>> {
    let current = string_list(document, "current_report_dates")?;
    let prior = string_list(document, "prior_report_dates")?;
    let thresholds = document.at("thresholds");
    let exceptions = document.at("exceptions").as_array().unwrap();
    let blocked = exceptions
        .iter()
        .filter(|e| e.at("status").is_str("BLOCKED"))
        .count();
    let review = exceptions
        .iter()
        .filter(|e| e.at("status").is_str("REVIEW"))
        .count();
    let mut lines = vec![
        format!("- Current report date(s): {}", join_strings(current, ", ")),
        format!("- Prior report date(s): {}", join_strings(prior, ", ")),
        format!(
            "- Material variance thresholds: ${} and {}",
            field(thresholds, "absolute_variance"),
            field(thresholds, "percentage_variance")
        ),
        format!(
            "- Reconciliation tolerance: ${}",
            field(thresholds, "reconciliation_tolerance")
        ),
        format!(
            "- Exceptions: {} total; {blocked} blocked; {review} requiring review.",
            exceptions.len()
        ),
    ];
    if register {
        lines.push(format!(
            "- Client queries drafted: {}.",
            document.at("client_queries").as_array().unwrap().len()
        ));
    }
    if document.get("controls_not_run").is_some() {
        let skipped = join_strings(string_list(document, "controls_not_run")?, ", ");
        lines.push(format!(
            "- Controls not run: {}.",
            if skipped.is_empty() { "none" } else { &skipped }
        ));
    }
    Ok(lines)
}
fn exception_lines(exceptions: &[Json]) -> Vec<String> {
    if exceptions.is_empty() {
        return vec![EMPTY_EXCEPTIONS.into()];
    }
    let mut lines = vec![
        "| Status | Control | Tenant | Account | Difference | Reason |".into(),
        "| --- | --- | --- | --- | ---: | --- |".into(),
    ];
    for item in exceptions {
        lines.push(format!(
            "| {} | {} | {} | {} | {} | {} |",
            field(item, "status"),
            field(item, "control"),
            md_cell(absent(&field(item, "tenant"))),
            md_cell(&account(item)),
            absent(&field(item, "difference")),
            md_cell(&field(item, "reason"))
        ));
    }
    lines
}
fn acknowledgement_lines(value: &Json) -> Vec<String> {
    if matches!(value, Json::Null) {
        return vec![NO_ACK.into()];
    }
    let mut lines = vec![
        format!(
            "- Reviewer initials: {}",
            md_cell(&field(value, "reviewer_initials"))
        ),
        format!("- Reviewed on: {}", field(value, "reviewed_on")),
    ];
    lines.extend(note_lines(&field(value, "comment")));
    lines.push(ACK_EFFECT_LINE.into());
    lines
}
fn query_counts(summary: &str) -> Vec<String> {
    // The original regex uses LF for anchors and admits one trailing CR.
    summary
        .split('\n')
        .filter_map(|line| {
            line.strip_suffix('\r')
                .unwrap_or(line)
                .strip_prefix("- Client queries drafted: ")?
                .strip_suffix('.')
        })
        .filter(|s| !s.is_empty() && s.chars().all(|c| pytext::decimal_value(c as u32).is_some()))
        .map(str::to_owned)
        .collect()
}
fn verify_queries(summary: &str, queries: &[Json]) -> Result<()> {
    let section = section(summary, "## Client queries")?;
    if !normalise(&section.join(" ")).contains("These are draft questions for the preparer, derived from the exceptions above. Nothing has been sent to anyone.") {return Err(format!("{S}: the client-query boundary statement is missing or altered"));}
    let counts = query_counts(summary);
    if counts.len() != 1 {
        return Err(format!(
            "{S}: expected exactly one client-query count line, found {}",
            counts.len()
        ));
    }
    let count = counts[0]
        .chars()
        .map(|c| char::from_digit(pytext::decimal_value(c as u32).unwrap(), 10).unwrap())
        .collect::<String>();
    if count.parse::<num_bigint::BigUint>().unwrap() != num_bigint::BigUint::from(queries.len()) {
        return Err(format!(
            "client-query counts disagree: {J} holds {}, {S} states {}",
            queries.len(),
            counts[0]
        ));
    }
    let mut lead = vec![""];
    lead.extend(QUERY_PREAMBLE);
    if !section.starts_with(&lead) {
        return Err(format!(
            "{S}: the client-query section's preamble is missing or altered"
        ));
    }
    let body = &section[lead.len()..];
    if body.last() != Some(&"") {
        return Err(format!(
            "{S}: the client-query section does not end where the writer ends it"
        ));
    }
    let body = &body[..body.len() - 1];
    if queries.is_empty() {
        if body != [EMPTY_QUERIES] {
            return Err(format!(
                "{S}: holds a client-query table while {J} holds no queries"
            ));
        }
        return Ok(());
    }
    if body.first() != Some(&QUERY_HEADER) {
        return Err(format!(
            "{S}: the client-query table header is missing or altered"
        ));
    }
    if body.get(1) != Some(&QUERY_DELIMITER) {
        return Err(format!(
            "{S}: the client-query table delimiter is missing or altered; the register would not render as a table"
        ));
    }
    let rows = &body[2..];
    if rows.len() != queries.len() {
        return Err(format!(
            "client-query counts disagree: {J} holds {}, {S} renders {} table rows",
            queries.len(),
            rows.len()
        ));
    }
    for (index, (item, row)) in queries.iter().zip(rows).enumerate() {
        let expected = format!(
            "| {} | {} | {} | {} | {} | {} | {} |",
            field(item, "query_id"),
            field(item, "control"),
            md_cell(absent(&field(item, "tenant"))),
            md_cell(&account(item)),
            absent(&field(item, "difference")),
            md_cell(&field(item, "question")),
            md_cell(&field(item, "evidence_requested"))
        );
        if *row != expected {
            return Err(format!(
                "{S}: client-query row {} disagrees with {J}: renders {}, expected {}",
                index + 1,
                pytext::repr_os(row),
                pytext::repr_os(&expected)
            ));
        }
    }
    Ok(())
}
fn summary_sources(summary: &str) -> Result<BTreeMap<PyStr, Json>> {
    let mut found: BTreeMap<PyStr, Json> = BTreeMap::new();
    let mut remaining = summary;
    while let Some(start) = remaining.find('`') {
        let after = &remaining[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        let label = &after[..end];
        if !label.is_empty()
            && label.bytes().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b':' | b'-')
            })
            && let Some(value) = after[end..].strip_prefix("`: `")
            && value.len() >= 65
            && value.as_bytes()[64] == b'`'
            && is_digest(&value[..64])
        {
            let key = PyStr::from(label);
            let digest = Json::str(&value[..64]);
            if found
                .get(&key)
                .is_some_and(|old| !pyjson::py_eq(old, &digest, false))
            {
                return Err(format!(
                    "{S}: source-evidence label {} appears with two different digests",
                    pytext::repr(label)
                ));
            }
            found.insert(key, digest);
        }
        remaining = after;
    }
    if found.is_empty() {
        return Err(format!("{S}: no source-evidence digest lines found"));
    }
    Ok(found)
}

const EVIDENCE_HEADING: &str = "## Calculation evidence";
const EVIDENCE_HEADER: &str =
    "| Calculation | Status | Period | Figures | Relied on | Entry digest |";
const EVIDENCE_DELIMITER: &str = "| --- | --- | --- | --- | --- | --- |";
const EVIDENCE_PREAMBLE: &str = "Read from files supplied to this run. This pack made no calculation and contacted no service. A figure here supports review and approves nothing.";
const NO_EVIDENCE: &str = "A calculation was required for this close and no evidence file was supplied for it. The exceptions below say which.";
const EVIDENCE_EFFECT: &str = "Evidence read from files. This pack made no calculation and contacted no service. A figure here supports review; it does not approve anything. `usable` is true only where the file hangs together, carries a figure and covers this close's period; read the exceptions for why one is false.";
const EVIDENCE_FIELDS: [&str; 14] = [
    "label",
    "schema",
    "provider",
    "calculator",
    "period",
    "status",
    "engine",
    "calculation_sha256",
    "file_sha256",
    "synthetic_input",
    "rate_tables",
    "advisory_notes",
    "values",
    "usable",
];
type EvidenceRows = BTreeMap<String, [String; 5]>;
fn slug(value: &str) -> bool {
    value.split('-').all(|s| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    })
}
fn evidence_rows(summary: &str) -> Result<(Vec<String>, EvidenceRows)> {
    let section = section(summary, EVIDENCE_HEADING)?;
    let lead = ["", EVIDENCE_PREAMBLE, ""];
    if !section.starts_with(&lead) {
        return Err(format!(
            "{S}: the calculation-evidence preamble is missing or altered"
        ));
    }
    let body = &section[lead.len()..];
    if body.last() != Some(&"") {
        return Err(format!(
            "{S}: the calculation-evidence section does not end where the writer ends it"
        ));
    }
    let body = &body[..body.len() - 1];
    if body.len() < 2 || !body[1].is_empty() {
        return Err(format!(
            "{S}: the calculation-evidence section's required line is missing or not where the writer puts it"
        ));
    }
    let required = body[0]
        .strip_prefix("Required: ")
        .and_then(|value| {
            if value == "none" {
                return Some(Vec::new());
            }
            value
                .split("; ")
                .map(|name| {
                    name.strip_prefix('`')?
                        .strip_suffix('`')
                        .filter(|s| slug(s))
                        .map(str::to_owned)
                })
                .collect::<Option<Vec<_>>>()
        })
        .ok_or_else(|| {
            format!(
                "{S}: the calculation-evidence required line is malformed: {}",
                pytext::repr_os(body[0])
            )
        })?;
    let body = &body[2..];
    if body == [NO_EVIDENCE] {
        return Ok((required, BTreeMap::new()));
    }
    if body.first() != Some(&EVIDENCE_HEADER) {
        return Err(format!(
            "{S}: the calculation-evidence table header is missing or altered"
        ));
    }
    if body.get(1) != Some(&EVIDENCE_DELIMITER) {
        return Err(format!(
            "{S}: the calculation-evidence table delimiter is missing or altered"
        ));
    }
    let mut rows = BTreeMap::new();
    for line in &body[2..] {
        if !line.starts_with("| ") || !line.ends_with(" |") {
            return Err(format!(
                "{S}: the calculation-evidence section holds a line the writer never emits: {}",
                pytext::repr_os(line)
            ));
        }
        let cells: Vec<_> = line
            .trim_matches('|')
            .split('|')
            .map(pytext::strip)
            .collect();
        if cells.len() != 6 {
            return Err(format!(
                "{S}: a calculation-evidence row holds {} cells, not 6",
                cells.len()
            ));
        }
        let label = cells[0];
        if rows.contains_key(label) {
            return Err(format!(
                "{S}: calculation {} appears twice in the table",
                pytext::repr_os(label)
            ));
        }
        if !is_digest(cells[5]) {
            return Err(format!(
                "{S}: calculation {} carries an entry digest that is not a lowercase SHA-256",
                pytext::repr_os(label)
            ));
        }
        rows.insert(label.into(), std::array::from_fn(|i| cells[i + 1].into()));
    }
    if rows.is_empty() {
        return Err(format!(
            "{S}: the calculation-evidence table has a header and no rows"
        ));
    }
    Ok((required, rows))
}
fn difference_keys(value: &Json, fields: &[&str]) -> (Vec<String>, Vec<String>) {
    let unknown = keys(value)
        .into_iter()
        .filter(|k| !fields.iter().any(|s| k.is(s)))
        .map(|k| k.os_text())
        .collect();
    let mut missing: Vec<_> = fields
        .iter()
        .filter(|k| value.get(k).is_none())
        .map(|s| s.to_string())
        .collect();
    missing.sort();
    (unknown, missing)
}
fn evidence_lists(block: &Json) -> Result<(&[Json], &[Json])> {
    if !matches!(block, Json::Object(_)) {
        return Err(format!("{J}: calculation_evidence must be an object"));
    }
    let (unknown, missing) = difference_keys(block, &["required", "supplied", "effect"]);
    if !unknown.is_empty() {
        return Err(format!(
            "{J}: calculation_evidence has unknown member(s): {}",
            unknown.join(", ")
        ));
    }
    if !missing.is_empty() {
        return Err(format!(
            "{J}: calculation_evidence is missing member(s): {}",
            missing.join(", ")
        ));
    }
    let supplied = block
        .at("supplied")
        .as_array()
        .ok_or_else(|| format!("{J}: calculation_evidence.supplied must be a list"))?;
    let required = block
        .at("required")
        .as_array()
        .filter(|s| s.iter().all(|v| v.as_pystr().is_some()))
        .ok_or_else(|| format!("{J}: calculation_evidence.required must be a list of strings"))?;
    if !block.at("effect").is_str(EVIDENCE_EFFECT) {
        return Err(format!(
            "{J}: calculation_evidence.effect is not the text the writer emits"
        ));
    }
    Ok((supplied, required))
}
fn encoded_str(value: &Json) -> String {
    match value {
        Json::Str(s) => s.os_text(),
        _ => PyStr::from(pyjson::repr(value).as_str()).os_text(),
    }
}
fn verify_evidence(block: &Json, summary: &str, sources: &Json) -> Result<()> {
    if matches!(block, Json::Null) {
        if pytext::split_lines(summary)
            .iter()
            .any(|line| pytext::strip(line) == EVIDENCE_HEADER)
        {
            return Err(format!(
                "{S}: a calculation-evidence table is present while {J} carries no calculation_evidence member"
            ));
        }
        let orphaned: Vec<_> = keys(sources)
            .iter()
            .map(PyStr::os_text)
            .filter(|k| k.starts_with("calculation_evidence:"))
            .collect();
        if !orphaned.is_empty() {
            return Err(format!(
                "{J}: source_sha256 records calculation evidence ({}) while the pack carries no calculation_evidence member; the evidence is half removed, not absent",
                orphaned.join(", ")
            ));
        }
        return Ok(());
    }
    let (supplied, required) = evidence_lists(block)?;
    let (summary_required, summary_rows) = evidence_rows(summary)?;
    if required.iter().map(text).collect::<Vec<_>>() != summary_required {
        return Err(format!(
            "calculation evidence disagrees: {J} requires {}, {S} states {}",
            pyjson::repr(&Json::Array(required.to_vec())),
            repr_lines(
                &summary_required
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            )
        ));
    }
    let mut json_rows: EvidenceRows = BTreeMap::new();
    for item in supplied {
        if !matches!(item, Json::Object(_)) {
            return Err(format!(
                "{J}: calculation_evidence.supplied holds a non-object entry"
            ));
        }
        let (unknown, missing) = difference_keys(item, &EVIDENCE_FIELDS);
        if !unknown.is_empty() || !missing.is_empty() {
            return Err(format!(
                "{J}: a calculation_evidence entry has the wrong members (unknown: {}; missing: {})",
                if unknown.is_empty() {
                    "none".into()
                } else {
                    unknown.join(", ")
                },
                if missing.is_empty() {
                    "none".into()
                } else {
                    missing.join(", ")
                }
            ));
        }
        let label = item.at("label");
        if label.as_pystr().is_some() && json_rows.contains_key(&md_cell(&text(label))) {
            return Err(format!(
                "{J}: calculation_evidence.supplied lists {} twice",
                pyjson::repr(label)
            ));
        }
        if label.as_pystr().is_none() || item.at("usable").as_bool().is_none() {
            return Err(format!(
                "{J}: a calculation_evidence entry has a non-string label or a non-boolean usable flag"
            ));
        }
        let values = item.at("values");
        let Json::Object(values_pairs) = values else {
            return Err(format!(
                "{J}: calculation_evidence[{}].values must map names to decimal strings",
                pyjson::repr(label)
            ));
        };
        if values_pairs.iter().any(|(_, v)| v.as_pystr().is_none()) {
            return Err(format!(
                "{J}: calculation_evidence[{}].values must map names to decimal strings",
                pyjson::repr(label)
            ));
        }
        let witnessed = get(sources, &format!("calculation_evidence:{}", text(label)));
        if !pyjson::py_eq(witnessed, item.at("file_sha256"), false) {
            return Err(format!(
                "{J}: calculation_evidence[{}] quotes file digest {}, which is not the digest source_sha256 records",
                pyjson::repr(label),
                pyjson::repr(item.at("file_sha256"))
            ));
        }
        let figures = keys(values)
            .iter()
            .map(|key| {
                let value = values_pairs.iter().find(|(k, _)| k == key).unwrap();
                format!("{} {}", key.os_text(), text(&value.1))
            })
            .collect::<Vec<_>>()
            .join("; ");
        let encoded = pyjson::canonical_utf8(item).map_err(str::to_owned)?;
        json_rows.insert(
            md_cell(&text(label)),
            [
                md_cell(&encoded_str(item.at("status"))),
                md_cell(absent(&encoded_str(item.at("period")))),
                md_cell(absent(&figures)),
                if item.at("usable").as_bool() == Some(true) {
                    "yes"
                } else {
                    "no"
                }
                .into(),
                crate::sha256(encoded.as_bytes()),
            ],
        );
    }
    if supplied.is_empty() && !summary_rows.is_empty() {
        return Err(format!(
            "{S}: holds a calculation-evidence table while {J} records no supplied evidence"
        ));
    }
    let witnessed: Vec<_> = keys(sources)
        .iter()
        .map(PyStr::os_text)
        .filter_map(|k| k.strip_prefix("calculation_evidence:").map(str::to_owned))
        .collect();
    let mut labels: Vec<_> = supplied
        .iter()
        .map(|item| encoded_str(item.at("label")))
        .collect();
    labels.sort_by_key(|s| PyStr::from_os_text(s));
    if witnessed != labels {
        return Err(format!(
            "{J}: source_sha256 witnesses calculation evidence for {} while calculation_evidence.supplied describes {}; the evidence is half removed, not absent",
            repr_lines(&witnessed.iter().map(String::as_str).collect::<Vec<_>>()),
            repr_lines(&labels.iter().map(String::as_str).collect::<Vec<_>>())
        ));
    }
    if json_rows != summary_rows {
        return Err(format!(
            "calculation evidence disagrees: {J} and {S} state different calculations, figures or relied-on flags"
        ));
    }
    Ok(())
}
fn verify_agreement(
    document: &Json,
    summary: &str,
    rows: &[Json],
    queries: Option<&[Json]>,
) -> Result<()> {
    let evidence = get(document, "calculation_evidence");
    verify_headings(
        summary,
        queries.is_some(),
        !matches!(evidence, Json::Null),
        document.get("equity_reconciliation").is_some(),
    )?;
    let status = field(document, "overall_status");
    let mut statuses = Vec::new();
    let mut rest = summary;
    while let Some(index) = rest.find("**Overall status: ") {
        let after = &rest[index + 18..];
        for s in STATUSES {
            if after.starts_with(&format!("{s}**")) {
                statuses.push(s);
                break;
            }
        }
        rest = after;
    }
    if statuses.len() != 1 {
        return Err(format!(
            "{S}: expected exactly one overall-status line, found {}",
            statuses.len()
        ));
    }
    if statuses[0] != status {
        return Err(format!(
            "overall status disagrees: {J} says {status}, {S} says {}",
            statuses[0]
        ));
    }
    if !summary.contains(BOUNDARY) {
        return Err(format!(
            "{S}: the review-boundary statement is missing or altered"
        ));
    }
    let sources = document.at("source_sha256");
    let summary_hashes = Json::Object(summary_sources(summary)?.into_iter().collect());
    if !pyjson::py_eq(&summary_hashes, sources, true) {
        return Err(format!(
            "source evidence disagrees: {S} and {J} list different source digests"
        ));
    }
    verify_evidence(evidence, summary, sources)?;
    let exceptions = document.at("exceptions").as_array().unwrap();
    if let Some(result) = document.get("equity_reconciliation") {
        equity::validate_result(result, sources, exceptions)?;
        if (!pyjson::py_eq(
            &Json::Array(vec![result.at("opening_date").clone()]),
            document.at("prior_report_dates"),
            true,
        ) || !pyjson::py_eq(
            &Json::Array(vec![result.at("closing_date").clone()]),
            document.at("current_report_dates"),
            true,
        )) && !result.at("status").is_str("BLOCKED")
        {
            return Err("Equity dates disagree with the pack.".into());
        }
        rebuilt_section(
            summary,
            "## Equity reconciliation",
            &equity::summary_lines(result),
            true,
        )?;
    } else if sources.get("equity_schedule").is_some() {
        return Err("Equity schedule source has no reconciliation evidence.".into());
    }
    rows_match(
        C,
        "exceptions",
        "exception",
        exceptions,
        rows,
        &EXCEPTION_FIELDS,
        false,
    )?;
    if let Some(queries) = queries {
        let json_queries = document.at("client_queries").as_array().unwrap();
        rows_match(
            Q,
            "client_queries",
            "client query",
            json_queries,
            queries,
            &QUERY_FIELDS,
            true,
        )?;
        verify_queries(summary, json_queries)?;
    } else {
        if pytext::split_lines(summary)
            .iter()
            .any(|line| pytext::strip(line) == QUERY_HEADER)
        {
            return Err(format!(
                "{S}: holds a client-query table while the pack carries no client-query register; the register is half removed, not absent"
            ));
        }
        if !query_counts(summary).is_empty() {
            return Err(format!(
                "{S}: states a client-query count while the pack carries no client-query register; the register is half removed, not absent"
            ));
        }
    }
    rebuilt_section(
        summary,
        "## Scope",
        &expected_scope(document, queries.is_some())?,
        true,
    )?;
    rebuilt_section(summary, "## Exceptions", &exception_lines(exceptions), true)?;
    rebuilt_section(
        summary,
        "## Human acknowledgement",
        &acknowledgement_lines(document.at("acknowledgement")),
        false,
    )
}

#[derive(Clone, Debug)]
pub struct VerifiedPack {
    pub document: Json,
    pub summary_text: String,
    pub exceptions: Vec<Json>,
    pub queries: Option<Vec<Json>>,
    pub artefact_sha256: BTreeMap<String, String>,
}
pub fn verify_payloads(payloads: &BTreeMap<String, Vec<u8>>) -> Result<VerifiedPack> {
    let document = parse_json(&payloads[J])?;
    verify_schema(&document)?;
    let has_file = payloads.contains_key(Q);
    let has_member = document.get("client_queries").is_some();
    if has_file != has_member {
        let member = format!("{J} member client_queries");
        let (present, absent) = if has_file {
            (Q, &*member)
        } else {
            (&*member, Q)
        };
        return Err(format!(
            "client-query register is half present: {present} exists but {absent} does not"
        ));
    }
    let summary = std::str::from_utf8(&payloads[S]).map_err(|_| format!("{S}: not valid UTF-8"))?;
    let rows = read_csv(&payloads[C], C, &EXCEPTION_FIELDS)?;
    let queries = payloads
        .get(Q)
        .map(|bytes| read_csv(bytes, Q, &QUERY_FIELDS))
        .transpose()?;
    verify_agreement(
        &document,
        &PyStr::from(summary).os_text(),
        &rows,
        queries.as_deref(),
    )?;
    Ok(VerifiedPack {
        document,
        summary_text: summary.into(),
        exceptions: rows,
        queries,
        artefact_sha256: payloads
            .iter()
            .map(|(k, v)| (k.clone(), crate::sha256(v)))
            .collect(),
    })
}
fn read_artefact(directory: &Path, name: &str) -> Result<Vec<u8>> {
    let path = directory.join(name);
    fs::read(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::IsADirectory {
            format!("{name}: expected a file, found a directory")
        } else if e.kind() == std::io::ErrorKind::NotFound {
            format!("{name}: not found in {}", pypath::text(directory))
        } else {
            format!(
                "{name}: could not be read from {} ({e})",
                pypath::text(directory)
            )
        }
    })
}
pub fn verify_pack(directory: &Path) -> Result<VerifiedPack> {
    let mut payloads = BTreeMap::new();
    for name in &PACK_NAMES[..3] {
        payloads.insert(name.to_string(), read_artefact(directory, name)?);
    }
    match fs::read(directory.join(Q)) {
        Ok(bytes) => {
            payloads.insert(Q.into(), bytes);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            read_artefact(directory, Q)?;
            return Err(format!("{Q}: could not be read"));
        }
    }
    verify_payloads(&payloads)
}

pub fn render_verified(pack: &VerifiedPack) -> Result<String> {
    let document = &pack.document;
    let thresholds = document.at("thresholds");
    let exceptions = document.at("exceptions").as_array().unwrap();
    let blocked = exceptions
        .iter()
        .filter(|e| e.at("status").is_str("BLOCKED"))
        .count();
    let review = exceptions
        .iter()
        .filter(|e| e.at("status").is_str("REVIEW"))
        .count();
    let mut lines=vec!["Close Review Sheet".into(),String::new(),format!("Overall status: {}",field(document,"overall_status")),String::new(),
        "Review states only. This sheet does not approve a close, post a journal, make a payment, lodge a return, or lock a period.".into(),String::new(),"Scope".into(),String::new(),
        format!("- Current report date(s): {}",absent(&join_strings(document.at("current_report_dates").as_array().unwrap(),", "))),format!("- Prior report date(s): {}",absent(&join_strings(document.at("prior_report_dates").as_array().unwrap(),", "))),
        format!("- Material variance thresholds: {} and {}",field(thresholds,"absolute_variance"),field(thresholds,"percentage_variance")),format!("- Reconciliation tolerance: {}",field(thresholds,"reconciliation_tolerance")),
        format!("- Exceptions: {} total; {blocked} blocked; {review} requiring review.",exceptions.len()),
    ];
    if let Some(queries) = document.get("client_queries") {
        lines.push(format!(
            "- Client queries drafted: {}.",
            queries.as_array().unwrap().len()
        ));
    }
    if document.get("controls_not_run").is_some() {
        let skipped = join_strings(string_list(document, "controls_not_run")?, ", ");
        lines.push(format!(
            "- Controls not run: {}.",
            if skipped.is_empty() { "none" } else { &skipped }
        ));
    }
    lines.extend([String::new(), "Source evidence".into(), String::new()]);
    let sources = document.at("source_sha256");
    for key in keys(sources) {
        lines.push(format!(
            "- {}: {}",
            key.os_text(),
            field(sources, &key.to_utf8().unwrap())
        ));
    }
    let evidence = get(document, "calculation_evidence");
    if !matches!(evidence, Json::Null) {
        lines.extend([String::new(), "Calculation evidence".into(), String::new()]);
        let required = join_strings(evidence.at("required").as_array().unwrap(), ", ");
        lines.push(format!(
            "- Required: {}",
            if required.is_empty() {
                "none"
            } else {
                &required
            }
        ));
        let supplied = evidence.at("supplied").as_array().unwrap();
        if supplied.is_empty() {
            lines.push("- Supplied: none. The exceptions say which calculation is missing.".into());
        }
        for entry in supplied {
            let values = entry.at("values");
            let figures = keys(values)
                .iter()
                .map(|key| {
                    format!(
                        "{} {}",
                        key.os_text(),
                        field(values, &key.to_utf8().unwrap())
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            let period = encoded_str(entry.at("period"));
            lines.push(format!(
                "- {}: {}, period {}, {}, relied on: {}",
                field(entry, "label"),
                encoded_str(entry.at("status")),
                absent(&period),
                if figures.is_empty() {
                    "no figure"
                } else {
                    &figures
                },
                if entry.at("usable").as_bool() == Some(true) {
                    "yes"
                } else {
                    "no"
                }
            ));
        }
        lines.push("- A figure here supports review. It approves nothing, and the calculation-evidence exceptions say why one is not relied on.".into());
    }
    if let Some(equity) = document.get("equity_reconciliation") {
        lines.extend([String::new(), "Equity reconciliation".into(), String::new()]);
        lines.extend(equity::summary_lines(equity));
    }
    lines.extend([String::new(), "Exceptions".into(), String::new()]);
    if exceptions.is_empty() {
        lines.push(EMPTY_EXCEPTIONS.into());
    } else {
        let width = exceptions.len().to_string().len();
        for (index, item) in exceptions.iter().enumerate() {
            lines.push(format!(
                "[{:>width$}] {} {} | {} | {} | difference {}",
                index + 1,
                field(item, "status"),
                field(item, "control"),
                absent(&field(item, "tenant")),
                account(item),
                absent(&field(item, "difference"))
            ));
            lines.push(format!("     reason: {}", field(item, "reason")));
            lines.push(format!("     action: {}", field(item, "reviewer_action")));
        }
    }
    lines.extend([String::new(),"Client queries".into(),String::new(),"Draft questions for the preparer. Nothing has been sent. Edit them before they reach a client and record answers in the firm's own tracker.".into(),String::new()]);
    match document.get("client_queries") {
        None=>lines.push("This pack was written before the client-query register existed, so it carries none. Its exceptions are unchanged.".into()),
        Some(value)=>{let queries=value.as_array().unwrap();if queries.is_empty(){lines.push("No exception raised a question for the client.".into());}else{let width=queries.len().to_string().len();for (index,item) in queries.iter().enumerate(){lines.push(format!("[{:>width$}] {} {} | {} | {} | difference {}",index+1,field(item,"query_id"),field(item,"control"),absent(&field(item,"tenant")),account(item),absent(&field(item,"difference"))));lines.push(format!("     ask: {}",field(item,"question")));lines.push(format!("     evidence: {}",field(item,"evidence_requested")));}}}
    }
    lines.extend([String::new(), "Human acknowledgement".into(), String::new()]);
    let a = document.at("acknowledgement");
    if matches!(a, Json::Null) {
        lines.push(NO_ACK.into());
    } else {
        lines.push(format!(
            "- Reviewer initials: {}",
            field(a, "reviewer_initials")
        ));
        lines.push(format!("- Reviewed on: {}", field(a, "reviewed_on")));
        let comment = field(a, "comment");
        if !comment.is_empty() {
            lines.push(format!("- Comment: {comment}"));
        }
        lines.push(ACK_EFFECT_LINE.into());
    }
    lines.extend([String::new(), "Artefacts verified".into(), String::new()]);
    for name in PACK_NAMES {
        if let Some(hash) = pack.artefact_sha256.get(name) {
            lines.push(format!("- {name}: sha256 {hash}"));
        }
    }
    lines.push(String::new());
    Ok(lines.join("\n"))
}
pub fn render_review_sheet(directory: &Path) -> Result<(String, BTreeMap<String, String>)> {
    let pack = verify_pack(directory)?;
    Ok((render_verified(&pack)?, pack.artefact_sha256))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        engine::{self, ReviewConfig},
        isodate::Date,
        loader::SourceSnapshot,
        models::ReviewerAcknowledgement,
        report,
        test_support::TempDir,
    };
    fn fixture(tenant: &str, comment: Option<&str>) -> BTreeMap<String, Vec<u8>> {
        let tb = |date| {
            format!(
                "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n{date},{tenant},Assets,A,Asset,100,0,0,100,0\n{date},{tenant},Equity,E,Capital,900,0,0,0,100\n"
            )
        };
        let config = ReviewConfig::new("current.csv".into(), "prior.csv".into());
        let mut pack = engine::review_with(&config, &mut |path, _| {
            Ok(SourceSnapshot::new(
                path,
                tb(if path == Path::new("current.csv") {
                    "2026-09-30"
                } else {
                    "2026-08-31"
                })
                .into_bytes(),
            ))
        })
        .unwrap();
        if let Some(comment) = comment {
            pack.acknowledgement = Some(ReviewerAcknowledgement {
                reviewer_initials: "RD".into(),
                reviewed_on: Date::new(2026, 9, 30).unwrap(),
                comment: comment.into(),
            });
        }
        PACK_NAMES
            .into_iter()
            .map(str::to_owned)
            .zip(report::rendered(&pack).unwrap())
            .collect()
    }
    #[test]
    fn reads_current_and_fully_archived_packs() {
        let mut payloads = fixture("Synthetic", None);
        assert!(verify_payloads(&payloads).unwrap().queries.is_some());
        let mut document = parse_json(&payloads[J]).unwrap();
        let Json::Object(pairs) = &mut document else {
            unreachable!()
        };
        pairs.retain(|(key, _)| !key.is("client_queries") && !key.is("controls_not_run"));
        payloads.insert(J.into(), pyjson::pretty(&document).into_bytes());
        payloads.remove(Q);
        let summary = String::from_utf8(payloads[S].clone())
            .unwrap()
            .replace("\r\n", "\n");
        let start = summary.find("\n## Client queries\n").unwrap();
        let end = summary.find("\n## Human acknowledgement\n").unwrap();
        let old = format!("{}{}", &summary[..start], &summary[end..]);
        let old = old
            .lines()
            .filter(|line| {
                !line.starts_with("- Client queries drafted:")
                    && !line.starts_with("- Controls not run:")
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        payloads.insert(S.into(), old.into_bytes());
        let pack = verify_payloads(&payloads).unwrap();
        assert!(pack.queries.is_none());
        assert!(
            render_verified(&pack)
                .unwrap()
                .contains("before the client-query register existed")
        );
    }
    #[test]
    fn refuses_extra_csv_cells_blank_rows_and_hidden_markdown_sections() {
        for suffix in [b"\r\n".as_slice(), b"=unwitnessed\r\n"] {
            let mut payloads = fixture("Synthetic", None);
            payloads.get_mut(C).unwrap().extend(suffix);
            assert!(
                verify_payloads(&payloads)
                    .unwrap_err()
                    .contains("cells, but the header declares")
            );
        }
        for prefix in ["```\n", "   ~~~\n", "<div>\n"] {
            let mut payloads = fixture("Synthetic", None);
            let mut changed = prefix.as_bytes().to_vec();
            changed.extend(&payloads[S]);
            payloads.insert(S.into(), changed);
            assert!(
                verify_payloads(&payloads)
                    .unwrap_err()
                    .contains("opens a code fence or an HTML block")
            );
        }
    }
    #[test]
    fn literal_private_use_text_cannot_witness_a_lone_surrogate() {
        let literal = "Literal \u{10f7ff}\u{10f800}";
        let mut payloads = fixture(literal, Some(literal));
        let pack = verify_payloads(&payloads).unwrap();
        let sheet = PyStr::from_os_text(&render_verified(&pack).unwrap())
            .to_utf8()
            .unwrap();
        assert!(sheet.contains(literal));
        let mut document = parse_json(&payloads[J]).unwrap();
        let mut acknowledgement = document.at("acknowledgement").clone();
        acknowledgement.set("comment", Json::Str(PyStr(vec![0xd800])));
        document.set("acknowledgement", acknowledgement);
        payloads.insert(J.into(), pyjson::pretty(&document).into_bytes());
        assert!(
            verify_payloads(&payloads)
                .unwrap_err()
                .contains("Human acknowledgement' section disagrees")
        );
    }
    #[test]
    fn physical_verification_and_display_preserve_file_bytes_and_mtimes() {
        let temp = TempDir::new();
        let payloads = fixture("Synthetic", Some("Synthetic review"));
        for (name, bytes) in &payloads {
            fs::write(temp.0.join(name), bytes).unwrap();
        }
        let before: Vec<_> = PACK_NAMES
            .iter()
            .map(|name| {
                let path = temp.0.join(name);
                (
                    fs::read(&path).unwrap(),
                    fs::metadata(path).unwrap().modified().unwrap(),
                )
            })
            .collect();
        verify_pack(&temp.0).unwrap();
        render_review_sheet(&temp.0).unwrap();
        for (index, name) in PACK_NAMES.iter().enumerate() {
            let path = temp.0.join(name);
            assert_eq!(fs::read(&path).unwrap(), before[index].0);
            assert_eq!(
                fs::metadata(path).unwrap().modified().unwrap(),
                before[index].1
            );
        }
        assert_eq!(fs::read_dir(&temp.0).unwrap().count(), 4);
    }
    #[test]
    fn unreadable_required_files_and_query_register_are_named() {
        for name in PACK_NAMES {
            let temp = TempDir::new();
            for (filename, bytes) in fixture("Synthetic", None) {
                if filename == name {
                    fs::create_dir(temp.0.join(filename)).unwrap();
                } else {
                    fs::write(temp.0.join(filename), bytes).unwrap();
                }
            }
            let error = verify_pack(&temp.0).unwrap_err();
            assert!(error.starts_with(name), "{name}: {error}");
            assert!(!error.contains("before the client-query"));
        }
    }
}
