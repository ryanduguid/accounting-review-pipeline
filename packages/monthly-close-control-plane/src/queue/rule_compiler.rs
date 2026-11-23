// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Deterministic override-to-rule compiler with backtesting and zero-model replay.

use crate::{
    Result, isodate,
    pyjson::{self, Json, PyStr},
    pytext, sha256,
};
use std::collections::{BTreeMap, BTreeSet};

pub const RULESET_SCHEMA: &str = "close-override-rules-v1";
pub const REPLAY_SCHEMA: &str = "close-rule-replay-v1";
pub const BOUNDARY: &str = "Compiled rules classify recurring review patterns deterministically; they do not approve a close, post a journal, or replace human sign-off.";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OverrideDisposition {
    AcceptKnownTiming,
    ExpectedContra,
    ApprovedVarianceNote,
    Reclassify { target_account_id: String },
}

impl OverrideDisposition {
    pub fn as_code(&self) -> String {
        match self {
            Self::AcceptKnownTiming => "accept_known_timing".into(),
            Self::ExpectedContra => "expected_contra".into(),
            Self::ApprovedVarianceNote => "approved_variance_note".into(),
            Self::Reclassify { target_account_id } => format!("reclassify:{target_account_id}"),
        }
    }

    pub fn parse(raw: &str) -> Result<Self> {
        match raw {
            "accept_known_timing" => Ok(Self::AcceptKnownTiming),
            "expected_contra" => Ok(Self::ExpectedContra),
            "approved_variance_note" => Ok(Self::ApprovedVarianceNote),
            other => {
                if let Some(target) = other.strip_prefix("reclassify:")
                    && !target.trim().is_empty()
                {
                    return Ok(Self::Reclassify {
                        target_account_id: target.trim().to_owned(),
                    });
                }
                Err(format!("Unsupported override disposition: '{raw}'."))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverrideRecord {
    pub period: String,
    pub tenant: String,
    pub control: String,
    pub account_id: String,
    pub pattern: String,
    pub disposition: OverrideDisposition,
    pub reviewer_initials: String,
    pub reviewed_on: String,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompilePolicy {
    pub min_agreements: usize,
    pub require_distinct_periods: bool,
}

impl Default for CompilePolicy {
    fn default() -> Self {
        Self {
            min_agreements: 2,
            require_distinct_periods: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledRule {
    pub rule_id: String,
    pub version: u32,
    pub tenant: String,
    pub control: String,
    pub account_id: String,
    pub match_pattern: String,
    pub disposition: OverrideDisposition,
    pub support_count: usize,
    pub supporting_periods: Vec<String>,
    pub rule_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedCandidate {
    pub tenant: String,
    pub control: String,
    pub account_id: String,
    pub match_pattern: String,
    pub reason: String,
    pub observation_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledRuleSet {
    pub rules: Vec<CompiledRule>,
    pub rejected: Vec<RejectedCandidate>,
    pub ruleset_sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BacktestResult {
    pub total_records: usize,
    pub matched_records: usize,
    pub agreed_records: usize,
    pub conflicted_records: usize,
    pub demoted_rule_ids: Vec<String>,
    pub verified_rules: Vec<CompiledRule>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenException {
    pub item_id: String,
    pub tenant: String,
    pub control: String,
    pub account_id: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleClearedItem {
    pub item_id: String,
    pub rule_id: String,
    pub rule_sha256: String,
    pub disposition: OverrideDisposition,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayReport {
    pub ruleset_sha256: String,
    pub rule_cleared: Vec<RuleClearedItem>,
    pub exceptions_remaining: Vec<String>,
    pub ambiguous_items: Vec<String>,
}

fn normalise_pattern(raw: &str) -> String {
    pytext::casefold(raw)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn validate_date(raw: &str, field: &str) -> Result<()> {
    if isodate::date_fromisoformat(raw).is_none() {
        return Err(format!("Field '{field}' must be a valid YYYY-MM-DD date."));
    }
    Ok(())
}

fn validate_record(record: &OverrideRecord) -> Result<String> {
    validate_date(&record.period, "period")?;
    validate_date(&record.reviewed_on, "reviewed_on")?;
    for (name, val) in [
        ("tenant", record.tenant.as_str()),
        ("control", record.control.as_str()),
        ("account_id", record.account_id.as_str()),
        ("reviewer_initials", record.reviewer_initials.as_str()),
        ("note", record.note.as_str()),
    ] {
        if val.trim().is_empty() {
            return Err(format!("Override record field '{name}' must not be blank."));
        }
    }
    let pattern = normalise_pattern(&record.pattern);
    if pattern.is_empty() {
        return Err("Override record field 'pattern' must not be blank.".into());
    }
    Ok(pattern)
}

fn rule_payload(rule: &CompiledRule) -> Json {
    Json::object([
        ("rule_id", Json::str(&rule.rule_id)),
        ("version", Json::int(i64::from(rule.version))),
        ("tenant", Json::str(&rule.tenant)),
        ("control", Json::str(&rule.control)),
        ("account_id", Json::str(&rule.account_id)),
        ("match_pattern", Json::str(&rule.match_pattern)),
        ("disposition", Json::str(&rule.disposition.as_code())),
        ("support_count", Json::int(rule.support_count as i64)),
        (
            "supporting_periods",
            Json::Array(
                rule.supporting_periods
                    .iter()
                    .map(|p| Json::str(p))
                    .collect(),
            ),
        ),
    ])
}

fn rule_to_json(rule: &CompiledRule) -> Json {
    let mut obj = rule_payload(rule);
    obj.set("rule_sha256", Json::str(&rule.rule_sha256));
    obj
}

fn ruleset_digest(rules: &[CompiledRule]) -> String {
    let array = Json::Array(rules.iter().map(rule_to_json).collect());
    sha256(pyjson::canonical(&array).as_bytes())
}

fn toml_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c => escaped.push(c),
        }
    }
    escaped.push('"');
    escaped
}

impl CompiledRuleSet {
    pub fn to_json(&self) -> Json {
        let rules = self.rules.iter().map(rule_to_json).collect();
        let rejected = self
            .rejected
            .iter()
            .map(|item| {
                Json::object([
                    ("tenant", Json::str(&item.tenant)),
                    ("control", Json::str(&item.control)),
                    ("account_id", Json::str(&item.account_id)),
                    ("match_pattern", Json::str(&item.match_pattern)),
                    ("reason", Json::str(&item.reason)),
                    (
                        "observation_count",
                        Json::int(item.observation_count as i64),
                    ),
                ])
            })
            .collect();
        Json::object([
            ("schema", Json::str(RULESET_SCHEMA)),
            ("boundary", Json::str(BOUNDARY)),
            ("ruleset_sha256", Json::str(&self.ruleset_sha256)),
            ("rules", Json::Array(rules)),
            ("rejected", Json::Array(rejected)),
        ])
    }

    pub fn to_toml(&self) -> String {
        let mut out = format!(
            "schema = {}\nboundary = {}\nruleset_sha256 = {}\n",
            toml_string(RULESET_SCHEMA),
            toml_string(BOUNDARY),
            toml_string(&self.ruleset_sha256)
        );
        for rule in &self.rules {
            let periods = rule
                .supporting_periods
                .iter()
                .map(|p| toml_string(p))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "\n[[rules]]\nrule_id = {}\nversion = {}\ntenant = {}\ncontrol = {}\naccount_id = {}\nmatch_pattern = {}\ndisposition = {}\nsupport_count = {}\nsupporting_periods = [{periods}]\nrule_sha256 = {}\n",
                toml_string(&rule.rule_id),
                rule.version,
                toml_string(&rule.tenant),
                toml_string(&rule.control),
                toml_string(&rule.account_id),
                toml_string(&rule.match_pattern),
                toml_string(&rule.disposition.as_code()),
                rule.support_count,
                toml_string(&rule.rule_sha256),
            ));
        }
        out
    }

    pub fn from_json(value: &Json) -> Result<Self> {
        if !value
            .get("schema")
            .is_some_and(|s| s.is_str(RULESET_SCHEMA))
        {
            return Err("Rule set JSON must declare schema 'close-override-rules-v1'.".into());
        }
        let recorded_digest = read_text(value, "ruleset_sha256")?;
        let raw_rules = value
            .get("rules")
            .and_then(Json::as_array)
            .ok_or_else(|| "Rule set JSON must contain a 'rules' array.".to_owned())?;
        let mut rules = Vec::with_capacity(raw_rules.len());
        for item in raw_rules {
            rules.push(parse_rule(item)?);
        }
        if ruleset_digest(&rules) != recorded_digest {
            return Err("Rule set ruleset_sha256 verification failed.".into());
        }
        Ok(Self {
            rules,
            rejected: Vec::new(),
            ruleset_sha256: recorded_digest,
        })
    }
}

fn read_text(obj: &Json, key: &str) -> Result<String> {
    obj.get(key)
        .and_then(Json::as_pystr)
        .and_then(PyStr::to_utf8)
        .ok_or_else(|| format!("Rule set field '{key}' must be UTF-8 text."))
}

fn read_periods(item: &Json) -> Result<Vec<String>> {
    let raw = item
        .get("supporting_periods")
        .and_then(Json::as_array)
        .ok_or_else(|| "Compiled rule supporting_periods must be an array.".to_owned())?;
    let mut periods = Vec::with_capacity(raw.len());
    for entry in raw {
        let text = entry
            .as_pystr()
            .and_then(PyStr::to_utf8)
            .ok_or_else(|| "supporting_periods entry must be UTF-8 text.".to_owned())?;
        periods.push(text);
    }
    Ok(periods)
}

fn parse_rule(item: &Json) -> Result<CompiledRule> {
    let version = item
        .get("version")
        .and_then(Json::as_plain_int)
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| "Compiled rule version must be a positive integer.".to_owned())?;
    let support_count = item
        .get("support_count")
        .and_then(Json::as_plain_int)
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| "Compiled rule support_count must be an integer.".to_owned())?;
    let recorded_sha = read_text(item, "rule_sha256")?;
    let rule = CompiledRule {
        rule_id: read_text(item, "rule_id")?,
        version,
        tenant: read_text(item, "tenant")?,
        control: read_text(item, "control")?,
        account_id: read_text(item, "account_id")?,
        match_pattern: read_text(item, "match_pattern")?,
        disposition: OverrideDisposition::parse(&read_text(item, "disposition")?)?,
        support_count,
        supporting_periods: read_periods(item)?,
        rule_sha256: recorded_sha.clone(),
    };
    if sha256(pyjson::canonical(&rule_payload(&rule)).as_bytes()) != recorded_sha {
        return Err(format!(
            "Compiled rule '{}' failed SHA-256 verification.",
            rule.rule_id
        ));
    }
    Ok(rule)
}

fn make_rejected(
    key: (String, String, String, String),
    reason: &str,
    observation_count: usize,
) -> RejectedCandidate {
    let (tenant, control, account_id, match_pattern) = key;
    RejectedCandidate {
        tenant,
        control,
        account_id,
        match_pattern,
        reason: reason.into(),
        observation_count,
    }
}

fn build_rule(
    key: (String, String, String, String),
    disposition: OverrideDisposition,
    support_count: usize,
    supporting_periods: Vec<String>,
) -> CompiledRule {
    let (tenant, control, account_id, match_pattern) = key;
    let seed = format!(
        "{tenant}\0{control}\0{account_id}\0{match_pattern}\0{}",
        disposition.as_code()
    );
    let mut rule = CompiledRule {
        rule_id: format!("R-{}", &sha256(seed.as_bytes())[..12]),
        version: 1,
        tenant,
        control,
        account_id,
        match_pattern,
        disposition,
        support_count,
        supporting_periods,
        rule_sha256: String::new(),
    };
    rule.rule_sha256 = sha256(pyjson::canonical(&rule_payload(&rule)).as_bytes());
    rule
}

fn promote_group(
    key: (String, String, String, String),
    bucket: &[&OverrideRecord],
    policy: &CompilePolicy,
    rules: &mut Vec<CompiledRule>,
    rejected: &mut Vec<RejectedCandidate>,
) {
    let dispositions: BTreeSet<&OverrideDisposition> =
        bucket.iter().map(|r| &r.disposition).collect();
    if dispositions.len() > 1 {
        rejected.push(make_rejected(key, "conflicting_dispositions", bucket.len()));
        return;
    }
    let periods: BTreeSet<String> = bucket.iter().map(|r| r.period.clone()).collect();
    let support = if policy.require_distinct_periods {
        periods.len()
    } else {
        bucket.len()
    };
    if support < policy.min_agreements {
        rejected.push(make_rejected(key, "insufficient_agreements", bucket.len()));
        return;
    }
    rules.push(build_rule(
        key,
        bucket[0].disposition.clone(),
        bucket.len(),
        periods.into_iter().collect(),
    ));
}

pub fn compile_rules(
    records: &[OverrideRecord],
    policy: &CompilePolicy,
) -> Result<CompiledRuleSet> {
    if policy.min_agreements < 2 {
        return Err("min_agreements must be at least 2.".into());
    }
    type GroupKey = (String, String, String, String);
    let mut groups: BTreeMap<GroupKey, Vec<&OverrideRecord>> = BTreeMap::new();
    for record in records {
        let pattern = validate_record(record)?;
        groups
            .entry((
                record.tenant.trim().to_owned(),
                record.control.trim().to_owned(),
                record.account_id.trim().to_owned(),
                pattern,
            ))
            .or_default()
            .push(record);
    }
    let mut rules = Vec::new();
    let mut rejected = Vec::new();
    for (key, bucket) in groups {
        promote_group(key, &bucket, policy, &mut rules, &mut rejected);
    }
    let ruleset_sha256 = ruleset_digest(&rules);
    Ok(CompiledRuleSet {
        rules,
        rejected,
        ruleset_sha256,
    })
}

fn rule_matches(
    rule: &CompiledRule,
    tenant: &str,
    control: &str,
    account_id: &str,
    text: &str,
) -> bool {
    rule.tenant == tenant.trim()
        && rule.control == control.trim()
        && rule.account_id == account_id.trim()
        && normalise_pattern(text).contains(&rule.match_pattern)
}

fn check_backtest_record(
    rules: &[CompiledRule],
    record: &OverrideRecord,
    demoted: &mut BTreeSet<String>,
) -> Option<bool> {
    let matching: Vec<&CompiledRule> = rules
        .iter()
        .filter(|r| {
            rule_matches(
                r,
                &record.tenant,
                &record.control,
                &record.account_id,
                &record.pattern,
            )
        })
        .collect();
    if matching.is_empty() {
        return None;
    }
    let mut conflict = false;
    for rule in matching {
        if rule.disposition != record.disposition {
            demoted.insert(rule.rule_id.clone());
            conflict = true;
        }
    }
    Some(conflict)
}

pub fn backtest_rules(
    ruleset: &CompiledRuleSet,
    held_out: &[OverrideRecord],
) -> Result<BacktestResult> {
    let (mut matched_records, mut agreed_records, mut conflicted_records) = (0, 0, 0);
    let mut demoted = BTreeSet::new();
    for record in held_out {
        validate_record(record)?;
        if let Some(conflict) = check_backtest_record(&ruleset.rules, record, &mut demoted) {
            matched_records += 1;
            if conflict {
                conflicted_records += 1;
            } else {
                agreed_records += 1;
            }
        }
    }
    let verified_rules = ruleset
        .rules
        .iter()
        .filter(|r| !demoted.contains(&r.rule_id))
        .cloned()
        .collect();
    Ok(BacktestResult {
        total_records: held_out.len(),
        matched_records,
        agreed_records,
        conflicted_records,
        demoted_rule_ids: demoted.into_iter().collect(),
        verified_rules,
    })
}

pub fn replay_rules(
    ruleset: &CompiledRuleSet,
    open_items: &[OpenException],
) -> Result<ReplayReport> {
    if ruleset_digest(&ruleset.rules) != ruleset.ruleset_sha256 {
        return Err(
            "Refusing to replay a rule set whose ruleset_sha256 does not match its rules.".into(),
        );
    }
    let (mut rule_cleared, mut exceptions_remaining, mut ambiguous_items) =
        (Vec::new(), Vec::new(), Vec::new());
    for item in open_items {
        let mut matching: Vec<&CompiledRule> = ruleset
            .rules
            .iter()
            .filter(|r| rule_matches(r, &item.tenant, &item.control, &item.account_id, &item.text))
            .collect();
        if matching.is_empty() {
            exceptions_remaining.push(item.item_id.clone());
            continue;
        }
        let distinct: BTreeSet<&OverrideDisposition> =
            matching.iter().map(|r| &r.disposition).collect();
        if distinct.len() > 1 {
            ambiguous_items.push(item.item_id.clone());
            exceptions_remaining.push(item.item_id.clone());
            continue;
        }
        matching.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));
        let chosen = matching[0];
        rule_cleared.push(RuleClearedItem {
            item_id: item.item_id.clone(),
            rule_id: chosen.rule_id.clone(),
            rule_sha256: chosen.rule_sha256.clone(),
            disposition: chosen.disposition.clone(),
        });
    }
    Ok(ReplayReport {
        ruleset_sha256: ruleset.ruleset_sha256.clone(),
        rule_cleared,
        exceptions_remaining,
        ambiguous_items,
    })
}

impl ReplayReport {
    pub fn to_json(&self) -> Json {
        let cleared = self
            .rule_cleared
            .iter()
            .map(|item| {
                Json::object([
                    ("item_id", Json::str(&item.item_id)),
                    ("rule_id", Json::str(&item.rule_id)),
                    ("rule_sha256", Json::str(&item.rule_sha256)),
                    ("disposition", Json::str(&item.disposition.as_code())),
                ])
            })
            .collect();
        Json::object([
            ("schema", Json::str(REPLAY_SCHEMA)),
            ("boundary", Json::str(BOUNDARY)),
            ("ruleset_sha256", Json::str(&self.ruleset_sha256)),
            (
                "rule_cleared_count",
                Json::int(self.rule_cleared.len() as i64),
            ),
            (
                "exceptions_remaining_count",
                Json::int(self.exceptions_remaining.len() as i64),
            ),
            ("rule_cleared", Json::Array(cleared)),
            (
                "exceptions_remaining",
                Json::Array(
                    self.exceptions_remaining
                        .iter()
                        .map(|id| Json::str(id))
                        .collect(),
                ),
            ),
            (
                "ambiguous_items",
                Json::Array(
                    self.ambiguous_items
                        .iter()
                        .map(|id| Json::str(id))
                        .collect(),
                ),
            ),
        ])
    }
}
