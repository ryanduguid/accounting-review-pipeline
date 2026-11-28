// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Additional readiness controls: self-review contradictions, per-bank-account
//! reconciliation coverage, schedule-required evidence and pack file hygiene.
use super::{BANK_PRECISION, Loaded, exact_sub};
use crate::{Result, decimal::Decimal, loader::SourceSnapshot, models::*, profiles::Slot, pypath};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[cfg(test)]
mod tests;

const SCHEDULES: [(&str, &str); 5] = [
    ("fixed_assets", "fixed_asset_register.csv"),
    ("depreciation", "depreciation_schedule.csv"),
    ("loans", "loan_schedule.csv"),
    ("inventory", "stock_valuation.csv"),
    ("prepayments", "prepayments_schedule.csv"),
];

pub fn schedule_evidence_file(schedule: &str) -> Option<&'static str> {
    SCHEDULES
        .iter()
        .find_map(|&(name, file)| (name == schedule).then_some(file))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BankVerdict {
    Reconciled,
    Difference(Decimal),
    Unverified(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BankAccountCoverage {
    pub account_id: String,
    pub account_name: String,
    pub verdict: BankVerdict,
}

fn evidence_digest(evidence: &[SourceEvidence], filename: &str) -> Option<String> {
    evidence
        .iter()
        .find(|e| e.filename == filename)
        .map(|e| e.sha256.clone())
}

fn mapped_figures(loaded: &Loaded, covers: &[String]) -> Result<Vec<(String, Decimal)>> {
    let mut found = Vec::new();
    for code in covers {
        for row in loaded.activity_statement.iter().flatten() {
            if row.label == *code && !row.amount.is_zero() {
                found.push((code.clone(), row.amount.clone()));
            }
        }
        for row in loaded.trial_balance.iter().flatten() {
            if row.account_code == *code || row.account_id == *code {
                let ytd = exact_sub(&row.ytd_debit, &row.ytd_credit, BANK_PRECISION)?;
                let mov = exact_sub(&row.debit, &row.credit, BANK_PRECISION)?;
                if !ytd.is_zero() {
                    found.push((code.clone(), ytd));
                } else if !mov.is_zero() {
                    found.push((code.clone(), mov));
                }
            }
        }
    }
    Ok(found)
}

fn non_empty_pack_file(pack_dir: &Path, filename: &str) -> bool {
    fs::metadata(pack_dir.join(filename)).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}

fn contradiction_finding(reason: String, link: Option<&EvidenceLink>) -> Finding {
    let mut finding = Finding::new(
        "SELF_REVIEW_CONTRADICTION",
        "NOT_READY",
        "self_review",
        reason,
        "Return the pack. Reconcile the self-review answer against the posted figures and supporting artefacts.",
    );
    if let Some(ev) = link {
        finding = finding.with_evidence(ev.clone());
    }
    finding
}

fn check_self_review_item(
    item: &SelfReviewItem,
    loaded: &Loaded,
    pack_dir: &Path,
    digest: Option<&str>,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let figures = mapped_figures(loaded, &item.covers)?;
    let link = digest.map(|sha| EvidenceLink {
        file: "self_review.json".into(),
        sha256: sha.to_owned(),
        locator: format!("item:{}", item.name),
    });
    if !item.answer && !figures.is_empty() {
        let detail = figures
            .iter()
            .map(|(code, amt)| format!("{code}={}", amt.to_sci_string()))
            .collect::<Vec<_>>()
            .join(", ");
        findings.push(contradiction_finding(
            format!(
                "Self-review item '{}' is answered No, but posted figures exist on covered codes: {detail}.",
                item.name
            ),
            link.as_ref(),
        ));
    } else if item.answer && figures.is_empty() {
        findings.push(contradiction_finding(
            format!(
                "Self-review item '{}' is answered Yes, but no non-zero figure was posted on covered codes: {}.",
                item.name,
                item.covers.join(", ")
            ),
            link.as_ref(),
        ));
    }
    if item.answer
        && let Some(artefact) = &item.requires_artefact
        && !non_empty_pack_file(pack_dir, artefact)
    {
        findings.push(contradiction_finding(
            format!(
                "Self-review item '{}' is answered Yes, which requires supporting artefact '{artefact}', but it is missing or empty.",
                item.name
            ),
            link.as_ref(),
        ));
    }
    Ok(())
}

pub(super) fn apply_self_review_contradictions(
    loaded: &Loaded,
    pack_dir: &Path,
    evidence: &[SourceEvidence],
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let Some(review) = &loaded.self_review else {
        return Ok(());
    };
    let digest = evidence_digest(evidence, "self_review.json");
    for item in &review.items {
        check_self_review_item(item, loaded, pack_dir, digest.as_deref(), findings)?;
    }
    Ok(())
}

pub fn classify_bank_accounts(
    loaded: &Loaded,
    tolerance: &Decimal,
) -> Result<Vec<BankAccountCoverage>> {
    let Some(tb) = &loaded.trial_balance else {
        return Ok(Vec::new());
    };
    let period_end = loaded.self_review.as_ref().map(|s| s.period_end);
    let rec_by_id: BTreeMap<&str, &BankRecRow> = loaded
        .bank_rec
        .iter()
        .flatten()
        .map(|row| (row.account_id.as_str(), row))
        .collect();
    let mut coverage = Vec::new();
    for account in tb.iter().filter(|r| r.section.eq_ignore_ascii_case("Bank")) {
        let verdict = classify_one_bank(
            account,
            rec_by_id.get(account.account_id.as_str()).copied(),
            period_end,
            tolerance,
        )?;
        coverage.push(BankAccountCoverage {
            account_id: account.account_id.clone(),
            account_name: account.account_name.clone(),
            verdict,
        });
    }
    Ok(coverage)
}

fn classify_one_bank(
    account: &TrialBalanceRow,
    row: Option<&BankRecRow>,
    period_end: Option<crate::isodate::Date>,
    tolerance: &Decimal,
) -> Result<BankVerdict> {
    let Some(row) = row else {
        return Ok(BankVerdict::Unverified(
            "no bank reconciliation row was supplied for this Bank account".into(),
        ));
    };
    if row.unverified {
        return Ok(BankVerdict::Unverified(
            "bank reconciliation row is marked UNVERIFIED".into(),
        ));
    }
    if let (Some(as_at), Some(expected)) = (row.as_at, period_end)
        && as_at != expected
    {
        return Ok(BankVerdict::Unverified(format!(
            "bank reconciliation date {} does not match period_end {}",
            as_at.iso(),
            expected.iso()
        )));
    }
    let stmt_diff = exact_sub(&row.statement_balance, &row.gl_balance, BANK_PRECISION)?;
    if stmt_diff.abs() > *tolerance {
        return Ok(BankVerdict::Difference(stmt_diff));
    }
    let ledger = exact_sub(&account.ytd_debit, &account.ytd_credit, BANK_PRECISION)?;
    let gl_diff = exact_sub(&row.gl_balance, &ledger, BANK_PRECISION)?;
    if gl_diff.abs() > *tolerance {
        return Ok(BankVerdict::Difference(gl_diff));
    }
    Ok(BankVerdict::Reconciled)
}

fn unverified_bank_finding(
    entry: &BankAccountCoverage,
    reason: &str,
    searched: &[String],
    tb_sha: Option<&str>,
) -> Finding {
    let statement_note = if searched.is_empty() {
        " No evidence sources were declared searched, so no client statement request was raised."
            .into()
    } else {
        format!(
            " Client query: please supply the year-end bank statement for {} ({}); searched {}.",
            entry.account_name,
            entry.account_id,
            searched.join(", ")
        )
    };
    let mut finding = Finding::new(
        "BANK_REC_UNVERIFIED",
        "BLOCKED",
        "bank_rec",
        format!(
            "Bank account {} ({}) is unverified at period end: {reason}.{statement_note}",
            entry.account_id, entry.account_name
        ),
        "Do not finalise year end. Supply a verified period-end bank reconciliation for every Bank account.",
    );
    if let Some(sha) = tb_sha {
        finding = finding.with_evidence(EvidenceLink {
            file: "trial_balance.csv".into(),
            sha256: sha.to_owned(),
            locator: format!("account:{}", entry.account_id),
        });
    }
    finding
}

pub(super) fn apply_bank_coverage(
    profile: &str,
    loaded: &Loaded,
    tolerance: &Decimal,
    evidence: &[SourceEvidence],
    controls_not_run: &mut Vec<ControlNotRun>,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let coverage = classify_bank_accounts(loaded, tolerance)?;
    let searched = loaded
        .self_review
        .as_ref()
        .map_or(&[][..], |s| s.bank_evidence_searched.as_slice());
    let tb_sha = evidence_digest(evidence, "trial_balance.csv");
    for entry in &coverage {
        let BankVerdict::Unverified(reason) = &entry.verdict else {
            continue;
        };
        if profile == "year_end" {
            findings.push(unverified_bank_finding(
                entry,
                reason,
                searched,
                tb_sha.as_deref(),
            ));
        } else {
            controls_not_run.push(ControlNotRun {
                slot: format!("bank_rec:{}", entry.account_id),
                filename: "bank_rec.csv".into(),
                reason: format!(
                    "Bank account {} ({}): {reason}",
                    entry.account_id, entry.account_name
                ),
            });
        }
    }
    Ok(())
}

pub fn pdf_intact(bytes: &[u8]) -> bool {
    let tail_start = bytes.len().saturating_sub(1024);
    bytes.starts_with(b"%PDF-") && bytes[tail_start..].windows(5).any(|w| w == b"%%EOF")
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

pub fn xlsx_has_worksheets(bytes: &[u8]) -> bool {
    if bytes.len() < 22 || !bytes.starts_with(b"PK\x03\x04") {
        return false;
    }
    let min_eocd = bytes.len().saturating_sub(65_557);
    let Some(eocd) = (min_eocd..=bytes.len() - 22)
        .rev()
        .find(|&pos| &bytes[pos..pos + 4] == b"PK\x05\x06")
    else {
        return false;
    };
    let count = usize::from(read_u16_le(bytes, eocd + 10));
    let mut cursor = read_u32_le(bytes, eocd + 16) as usize;
    let (mut has_workbook, mut has_sheet) = (false, false);
    for _ in 0..count {
        if cursor + 46 > bytes.len() || &bytes[cursor..cursor + 4] != b"PK\x01\x02" {
            return false;
        }
        let name_len = usize::from(read_u16_le(bytes, cursor + 28));
        let extra_len = usize::from(read_u16_le(bytes, cursor + 30));
        let comment_len = usize::from(read_u16_le(bytes, cursor + 32));
        let name_end = cursor + 46 + name_len;
        let next = name_end + extra_len + comment_len;
        if next > bytes.len() {
            return false;
        }
        let name = String::from_utf8_lossy(&bytes[cursor + 46..name_end]).to_ascii_lowercase();
        has_workbook |= name == "xl/workbook.xml";
        has_sheet |= name.starts_with("xl/worksheets/") && name.ends_with(".xml");
        cursor = next;
    }
    has_workbook && has_sheet
}

fn apply_schedule_requirements(
    loaded: &Loaded,
    pack_dir: &Path,
    evidence: &[SourceEvidence],
    findings: &mut Vec<Finding>,
) -> BTreeSet<String> {
    let mut required_files = BTreeSet::new();
    let Some(review) = &loaded.self_review else {
        return required_files;
    };
    let self_sha = evidence_digest(evidence, "self_review.json");
    for schedule in &review.schedules {
        let file = schedule_evidence_file(schedule).expect("validated schedule name");
        required_files.insert(file.to_owned());
        let (code, reason) = match fs::metadata(pack_dir.join(file)) {
            Ok(meta) if meta.is_file() && meta.len() > 0 => continue,
            Ok(_) => (
                "EMPTY_ARTEFACT",
                format!("Schedule '{schedule}' requires evidence file '{file}', which is empty."),
            ),
            Err(_) => (
                "MISSING_ARTEFACT",
                format!(
                    "Schedule '{schedule}' requires evidence file '{file}', which is not in the pack directory."
                ),
            ),
        };
        let mut finding = Finding::new(
            code,
            "NOT_READY",
            schedule,
            reason,
            "Return the pack. Supply the supporting evidence required by the schedules present.",
        );
        if let Some(sha) = &self_sha {
            finding = finding.with_evidence(EvidenceLink {
                file: "self_review.json".into(),
                sha256: sha.clone(),
                locator: format!("schedule:{schedule}"),
            });
        }
        findings.push(finding);
    }
    required_files
}

fn referenced_pack_files(
    slots: &[Slot],
    loaded: &Loaded,
    schedule_files: &BTreeSet<String>,
    acknowledgement_path: Option<&Path>,
    pack_dir: &Path,
) -> BTreeSet<String> {
    let mut referenced: BTreeSet<String> =
        slots.iter().map(|slot| slot.filename.to_owned()).collect();
    referenced.extend(schedule_files.iter().cloned());
    for row in loaded.tie_out_matrix.iter().flatten() {
        if !row.source_file.is_empty() {
            referenced.insert(row.source_file.clone());
        }
    }
    for item in loaded.self_review.iter().flat_map(|s| &s.items) {
        if let Some(file) = &item.requires_artefact {
            referenced.insert(file.clone());
        }
    }
    if let Some(ack) = acknowledgement_path
        && ack.parent().is_some_and(|p| pypath::same_path(p, pack_dir))
        && let Some(name) = ack.file_name().and_then(|n| n.to_str())
    {
        referenced.insert(name.to_owned());
    }
    referenced
}

fn inspect_pack_file(
    pack_dir: &Path,
    name: &str,
    unreferenced: bool,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let lower = name.to_ascii_lowercase();
    let is_pdf = lower.ends_with(".pdf");
    let is_xlsx = lower.ends_with(".xlsx");
    if !is_pdf && !is_xlsx && !unreferenced {
        return Ok(());
    }
    let snapshot = SourceSnapshot::capture(&pack_dir.join(name), "pack evidence file")?;
    if is_pdf && !pdf_intact(&snapshot.content) {
        findings.push(
            Finding::new(
                "DAMAGED_ARTEFACT",
                "NOT_READY",
                "pack_evidence",
                format!("PDF artefact '{name}' is damaged: missing %PDF- header or %%EOF trailer."),
                "Return the pack. Replace the damaged PDF artefact with an intact copy.",
            )
            .with_evidence(EvidenceLink {
                file: name.to_owned(),
                sha256: snapshot.sha256.clone(),
                locator: "pdf:structure".into(),
            }),
        );
    } else if is_xlsx && !xlsx_has_worksheets(&snapshot.content) {
        findings.push(
            Finding::new(
                "DAMAGED_ARTEFACT",
                "NOT_READY",
                "pack_evidence",
                format!("XLSX artefact '{name}' is damaged or holds zero worksheets."),
                "Return the pack. Replace the damaged workbook with a readable copy containing at least one worksheet.",
            )
            .with_evidence(EvidenceLink {
                file: name.to_owned(),
                sha256: snapshot.sha256.clone(),
                locator: "xlsx:worksheets".into(),
            }),
        );
    }
    if unreferenced {
        findings.push(
            Finding::new(
                "UNREFERENCED_EVIDENCE",
                "NOT_READY",
                "pack_evidence",
                format!("Pack file '{name}' is not cited by any profile slot, tie-out, self-review item or schedule."),
                "Return the pack. Reference the file from the workpaper or remove stray files before review.",
            )
            .with_evidence(EvidenceLink {
                file: name.to_owned(),
                sha256: snapshot.sha256,
                locator: "file".into(),
            }),
        );
    }
    Ok(())
}

pub(super) fn apply_pack_evidence_controls(
    slots: &[Slot],
    loaded: &Loaded,
    pack_dir: &Path,
    acknowledgement_path: Option<&Path>,
    evidence: &[SourceEvidence],
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let schedule_files = apply_schedule_requirements(loaded, pack_dir, evidence, findings);
    let referenced = referenced_pack_files(
        slots,
        loaded,
        &schedule_files,
        acknowledgement_path,
        pack_dir,
    );
    let mut names: Vec<String> = fs::read_dir(pack_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|ft| ft.is_file()))
        .map(|entry| pypath::decode_os(&entry.file_name()))
        .collect();
    names.sort();
    for name in names {
        inspect_pack_file(pack_dir, &name, !referenced.contains(&name), findings)?;
    }
    Ok(())
}
