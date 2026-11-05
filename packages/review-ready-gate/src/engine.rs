// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use crate::{
    Result,
    decimal::Decimal,
    loader::*,
    models::*,
    profiles::{self, Slot},
    pytext,
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

#[derive(Default)]
struct Loaded {
    trial_balance: Option<Vec<TrialBalanceRow>>,
    prior_trial_balance: Option<Vec<TrialBalanceRow>>,
    open_items: Option<Vec<OpenItem>>,
    self_review: Option<SelfReview>,
    activity_statement: Option<Vec<ActivityLabel>>,
    gst_control_gl: Option<Vec<GstControlRow>>,
    bank_rec: Option<Vec<BankRecRow>>,
    tie_out_matrix: Option<Vec<TieOutRow>>,
    prior_findings: Option<Vec<PriorFinding>>,
}

impl Loaded {
    fn load(&mut self, slot: &Slot, source: &SourceSnapshot, profile: &str) -> Result<()> {
        match slot.kind {
            "trial_balance" if slot.name == "prior_trial_balance" => {
                self.prior_trial_balance = Some(load_canonical_tb(source)?)
            }
            "trial_balance" => self.trial_balance = Some(load_canonical_tb(source)?),
            "open_items" => self.open_items = Some(load_open_items(source)?),
            "self_review" => self.self_review = Some(load_self_review(source, profile)?),
            "activity_statement" => {
                self.activity_statement = Some(load_activity_statement(source)?)
            }
            "gst_control_gl" => self.gst_control_gl = Some(load_gst_control(source)?),
            "bank_rec" => self.bank_rec = Some(load_bank_rec(source)?),
            "tie_out_matrix" => self.tie_out_matrix = Some(load_tie_out_matrix(source)?),
            "prior_findings" => self.prior_findings = Some(load_prior_findings(source)?),
            _ => return Err(format!("unknown slot kind {}.", pytext::repr(slot.kind))),
        }
        Ok(())
    }
}

pub fn overall(findings: &[Finding]) -> &'static str {
    if findings.iter().any(|f| f.status == "BLOCKED") {
        "BLOCKED"
    } else if findings.is_empty() {
        "READY"
    } else {
        "NOT_READY"
    }
}

const TB_PRECISION: &str = "trial balance totals exceed the supported 28-digit precision, so debits and credits cannot be compared exactly.";
const BAS_PRECISION: &str =
    "BAS amounts exceed the supported 28-digit precision, so net GST cannot be tied exactly.";

fn exact_sub(left: &Decimal, right: &Decimal, refusal: &str) -> Result<Decimal> {
    let result = left.sub(right);
    if result.inexact {
        Err(refusal.into())
    } else {
        Ok(result.value)
    }
}
fn exact_sum(values: impl Iterator<Item = Result<Decimal>>, refusal: &str) -> Result<Decimal> {
    let mut total = Decimal::parse("0.00").unwrap();
    for value in values {
        let result = total.add(&value?);
        if result.inexact {
            return Err(refusal.into());
        }
        total = result.value;
    }
    Ok(total)
}
pub fn tb_balanced(rows: &[TrialBalanceRow]) -> Result<(Decimal, Decimal)> {
    let movement = exact_sum(
        rows.iter()
            .map(|r| exact_sub(&r.debit, &r.credit, TB_PRECISION)),
        TB_PRECISION,
    )?;
    let ytd = exact_sum(
        rows.iter()
            .map(|r| exact_sub(&r.ytd_debit, &r.ytd_credit, TB_PRECISION)),
        TB_PRECISION,
    )?;
    Ok((movement, ytd))
}

pub fn review_pack(
    profile: &str,
    pack_dir: &Path,
    acknowledgement_path: Option<&Path>,
    tolerance: Decimal,
    document_paths: &[PathBuf],
) -> Result<ReadinessPack> {
    review_requiring(
        profile,
        pack_dir,
        acknowledgement_path,
        tolerance,
        document_paths,
        &[],
    )
}

/// The named controls, sorted and without repeats, or a refusal: a mistyped
/// name would otherwise require nothing and pass quietly.
fn required_control_names(names: &[String]) -> Result<Vec<String>> {
    let known = profiles::control_names();
    if let Some(name) = names.iter().find(|name| !known.contains(name.as_str())) {
        return Err(format!(
            "--require-control {} is not a control. Name one of: {}.",
            pytext::repr(name),
            known.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(names
        .iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .cloned()
        .collect())
}

/// `review_pack`, with a `NOT_READY` finding for each named control that was
/// skipped or failed, including one the engagement profile does not have.
#[expect(
    clippy::too_many_lines,
    reason = "runs the readiness checks on one pack"
)]
pub fn review_requiring(
    profile: &str,
    pack_dir: &Path,
    acknowledgement_path: Option<&Path>,
    tolerance: Decimal,
    document_paths: &[PathBuf],
    required_controls: &[String],
) -> Result<ReadinessPack> {
    let required_names = required_control_names(required_controls)?;
    if !tolerance.is_finite() || tolerance < Decimal::zero() {
        return Err("tie-out tolerance must be a finite non-negative decimal.".into());
    }
    let slots = profiles::slots_for(profile)?;
    if !pack_dir.is_dir() {
        return Err(format!(
            "pack directory does not exist: {}.",
            crate::pypath::text(pack_dir)
        ));
    }
    let mut loaded = Loaded::default();
    let mut findings = Vec::new();
    let mut evidence = Vec::new();
    let mut controls = Vec::new();
    let mut ran = BTreeSet::new();
    let mut failed = BTreeSet::new();
    for slot in slots {
        let path = pack_dir.join(slot.filename);
        if !path.exists() {
            if slot.required {
                findings.push(Finding::new(
                    "MISSING_ARTEFACT",
                    "NOT_READY",
                    slot.name,
                    format!(
                        "Required artefact {} is not in the pack directory.",
                        slot.name
                    ),
                    "Return the pack to the preparer. Do not start technical review.",
                ));
            } else {
                controls.push(ControlNotRun {
                    slot: slot.name.into(),
                    filename: slot.filename.into(),
                    reason: "no file was supplied".into(),
                });
            }
            continue;
        }
        let source = SourceSnapshot::capture(&path, &format!("{} file", slot.name))?;
        evidence.push(SourceEvidence {
            slot: slot.name.into(),
            filename: slot.filename.into(),
            sha256: source.sha256.clone(),
        });
        if source.content.is_empty() {
            let scope = if slot.required {
                "Required"
            } else {
                "Optional"
            };
            let ran = if slot.required {
                ""
            } else {
                " Its control did not run."
            };
            failed.insert(slot.name);
            findings.push(Finding::new(
                "EMPTY_ARTEFACT",
                "NOT_READY",
                slot.name,
                format!(
                    "{scope} artefact {} exists but contains no bytes.{ran}",
                    slot.name
                ),
                "Return the pack to the preparer. An empty file is not a completed artefact.",
            ));
            if !slot.required {
                controls.push(ControlNotRun {
                    slot: slot.name.into(),
                    filename: slot.filename.into(),
                    reason: "the file supplied is empty".into(),
                });
            }
            continue;
        }
        loaded.load(slot, &source, profile)?;
        ran.insert(slot.name);
    }
    let mut required_controls = Vec::new();
    for name in required_names {
        let (state, code, outcome) = if ran.contains(name.as_str()) {
            ("ran", "", String::new())
        } else if failed.contains(name.as_str()) {
            (
                "failed",
                "REQUIRED_CONTROL_FAILED",
                "failed: its file is empty".to_owned(),
            )
        } else if slots.iter().any(|slot| slot.name == name) {
            (
                "skipped",
                "REQUIRED_CONTROL_SKIPPED",
                "was skipped: no file was supplied".to_owned(),
            )
        } else {
            (
                "skipped",
                "REQUIRED_CONTROL_SKIPPED",
                format!("was skipped: the {profile} profile does not run it"),
            )
        };
        if state != "ran" {
            findings.push(Finding::new(
                code,
                "NOT_READY",
                &name,
                format!("Required control {name} {outcome}, so this pack says nothing about it."),
                "Return the pack. Supply a usable artefact, or remove the requirement deliberately.",
            ));
        }
        required_controls.push(RequiredControl {
            name,
            state: state.into(),
        });
    }
    let period_end = loaded
        .self_review
        .as_ref()
        .map_or(String::new(), |s| s.period_end.iso());
    let preparer_initials = loaded
        .self_review
        .as_ref()
        .map_or(String::new(), |s| s.preparer_initials.clone());
    if let Some(s) = &loaded.self_review {
        let incomplete: Vec<_> = s
            .assertions
            .iter()
            .filter(|(_, value)| !value)
            .map(|(name, _)| name.as_str())
            .collect();
        if !incomplete.is_empty() {
            findings.push(Finding::new(
                "SELF_REVIEW_INCOMPLETE",
                "NOT_READY",
                "self_review",
                format!(
                    "Self-review assertions still false: {}.",
                    incomplete.join(", ")
                ),
                "Return the pack. The preparer has not certified it as review-ready.",
            ));
        }
    }
    apply_trial_balance(&loaded, &mut findings)?;
    apply_period_binding(&loaded, &mut findings);
    apply_open_items(&loaded, &mut findings);
    apply_bas(&loaded, &mut findings, &tolerance)?;
    apply_gst_period(&loaded, &mut findings);
    apply_bank_rec(&loaded, &mut findings, &tolerance);
    apply_year_end(&loaded, &mut findings);
    if !document_paths.is_empty() {
        let bundles = document_paths
            .iter()
            .map(|p| crate::documents::load_document(p))
            .collect::<Result<Vec<_>>>()?;
        let entity = loaded
            .trial_balance
            .as_ref()
            .and_then(|rows| rows.first())
            .map(|r| r.tenant.as_str());
        let (extra, sources) = crate::documents::document_controls(
            &bundles,
            entity,
            (!period_end.is_empty()).then_some(period_end.as_str()),
        );
        findings.extend(extra);
        evidence.extend(sources);
    }
    if let Some(prior) = &loaded.prior_findings {
        let open: BTreeSet<_> = prior
            .iter()
            .filter(|p| p.status == "OPEN")
            .map(|p| (&p.finding_code, &p.slot))
            .collect();
        for finding in &mut findings {
            if open.contains(&(&finding.code, &finding.slot)) && !finding.repeat {
                finding.repeat = true;
                finding
                    .reason
                    .push_str(" This finding was OPEN on the prior pack.");
                finding
                    .reviewer_action
                    .push_str(" Treat as a repeated preparer or vendor failure.");
            }
        }
    }
    let acknowledgement = load_reviewer_acknowledgement(acknowledgement_path)?;
    if let (Some(ack), Some(s)) = (&acknowledgement, &loaded.self_review) {
        if pytext::casefold(pytext::strip(&ack.reviewer_initials))
            == pytext::casefold(pytext::strip(&s.preparer_initials))
        {
            return Err("reviewer initials must differ from the preparer initials.".into());
        }
        if ack.reviewed_on < s.period_end {
            return Err("review note reviewed_on is earlier than the pack period_end; a note cannot review a period that has not ended.".into());
        }
    }
    Ok(ReadinessPack {
        status: overall(&findings).into(),
        engagement_type: profile.into(),
        period_end,
        preparer_initials,
        findings,
        source_evidence: evidence,
        tieout_tolerance: tolerance,
        acknowledgement,
        controls_not_run: Some(controls),
        required_controls,
        producer: None,
    })
}

fn apply_trial_balance(loaded: &Loaded, findings: &mut Vec<Finding>) -> Result<()> {
    if let Some(current) = &loaded.trial_balance {
        let (movement, ytd) = tb_balanced(current)?;
        if !movement.is_zero() || !ytd.is_zero() {
            findings.push(Finding::new("TB_UNBALANCED", "BLOCKED", "trial_balance", format!("Trial balance does not balance. Movement net {} and YTD net {} must both be zero.", movement.to_sci_string(), ytd.to_sci_string()), "Do not review judgement. Return the pack until the trial balance balances."));
        }
    }
    if let Some(prior) = &loaded.prior_trial_balance {
        let (movement, ytd) = tb_balanced(prior)?;
        if !movement.is_zero() || !ytd.is_zero() {
            findings.push(Finding::new("TB_UNBALANCED", "BLOCKED", "prior_trial_balance", format!("Prior trial balance does not balance. Movement net {} and YTD net {} must both be zero.", movement.to_sci_string(), ytd.to_sci_string()), "Return the pack. A prior period that does not balance cannot support comparative review."));
        }
        if let Some(current) = &loaded.trial_balance {
            let current = &current[0];
            let prior = &prior[0];
            if current.tenant != prior.tenant {
                findings.push(Finding::new(
                    "PERIOD_ORDER",
                    "BLOCKED",
                    "prior_trial_balance",
                    format!(
                        "Current tenant {} does not match prior tenant {}.",
                        pytext::repr(&current.tenant),
                        pytext::repr(&prior.tenant)
                    ),
                    "Return the pack. Comparative files must name the same entity.",
                ));
            } else if prior.report_date >= current.report_date {
                findings.push(Finding::new(
                    "PERIOD_ORDER",
                    "BLOCKED",
                    "prior_trial_balance",
                    format!(
                        "Prior ReportDate {} is not earlier than current ReportDate {}.",
                        prior.report_date.iso(),
                        current.report_date.iso()
                    ),
                    "Return the pack. The prior file must be an earlier period.",
                ));
            }
        }
    }
    Ok(())
}

fn apply_period_binding(loaded: &Loaded, findings: &mut Vec<Finding>) {
    if let (Some(s), Some(current)) = (&loaded.self_review, &loaded.trial_balance)
        && current[0].report_date != s.period_end
    {
        findings.push(Finding::new("PERIOD_ORDER", "BLOCKED", "trial_balance", format!("Trial balance ReportDate {} is not the declared period_end {}.", current[0].report_date.iso(), s.period_end.iso()), "Return the pack. The trial balance must be the export for the period the self-review declares."));
    }
}
fn apply_open_items(loaded: &Loaded, findings: &mut Vec<Finding>) {
    for row in loaded.open_items.iter().flatten() {
        if row.status == "OPEN" && row.severity == "BLOCKING" && row.resolution.is_empty() {
            findings.push(Finding::new(
                "OPEN_ITEM_BLOCKING",
                "NOT_READY",
                "open_items",
                format!(
                    "Blocking open item {} still OPEN with no resolution: {}",
                    row.item_id, row.description
                ),
                "Return the pack. Blocking items must be cleared or resolved before review.",
            ));
        }
        if row.status == "CLEARED" && row.resolution.is_empty() {
            findings.push(Finding::new(
                "OPEN_ITEM_INCOMPLETE",
                "NOT_READY",
                "open_items",
                format!(
                    "Open item {} is CLEARED but has no resolution text.",
                    row.item_id
                ),
                "Return the pack. A cleared item needs a written resolution.",
            ));
        }
    }
}
fn apply_bas(loaded: &Loaded, findings: &mut Vec<Finding>, tolerance: &Decimal) -> Result<()> {
    let (Some(activity), Some(gst)) = (&loaded.activity_statement, &loaded.gst_control_gl) else {
        return Ok(());
    };
    let labels = |name| activity.iter().find(|r| r.label == name).map(|r| &r.amount);
    let (Some(a), Some(b)) = (labels("1A"), labels("1B")) else {
        findings.push(Finding::new(
            "TIEOUT_BREAK",
            "NOT_READY",
            "activity_statement",
            "Activity statement is missing label 1A or 1B, so net GST cannot be tied.",
            "Return the pack. BAS readiness requires 1A and 1B.",
        ));
        return Ok(());
    };
    let statement = exact_sub(a, b, BAS_PRECISION)?;
    let gl = exact_sum(
        gst.iter()
            .map(|r| exact_sub(&r.credit, &r.debit, BAS_PRECISION)),
        BAS_PRECISION,
    )?;
    let difference = exact_sub(&statement, &gl, BAS_PRECISION)?;
    let whole = a.is_integral() && b.is_integral();
    let abs = difference.abs();
    if abs > *tolerance && !(whole && abs < Decimal::parse("1").unwrap()) {
        let allowance = if whole {
            " or the 99-cent allowance for whole-dollar 1A and 1B"
        } else {
            ""
        };
        findings.push(Finding::new("TIEOUT_BREAK", "NOT_READY", "gst_control_gl", format!("Net GST on the activity statement ({}) does not tie to GST control movement ({}); difference {} exceeds tolerance {}{allowance}.", statement.to_sci_string(), gl.to_sci_string(), difference.to_sci_string(), tolerance.to_sci_string()), "Return the pack. Complete the GST control tie-out before review."));
    }
    Ok(())
}
fn apply_gst_period(loaded: &Loaded, findings: &mut Vec<Finding>) {
    let Some(s) = &loaded.self_review else {
        return;
    };
    let late: BTreeSet<_> = loaded
        .gst_control_gl
        .iter()
        .flatten()
        .filter(|r| r.posted_on > s.period_end)
        .map(|r| r.posted_on)
        .collect();
    if late.is_empty() {
        return;
    }
    let shown = late
        .iter()
        .take(3)
        .map(|d| d.iso())
        .collect::<Vec<_>>()
        .join(", ");
    let more = if late.len() > 3 {
        format!(" and {} more", late.len() - 3)
    } else {
        String::new()
    };
    findings.push(Finding::new(
        "PERIOD_ORDER",
        "NOT_READY",
        "gst_control_gl",
        format!(
            "GST control postings are dated after the declared period_end {}: {shown}{more}.",
            s.period_end.iso()
        ),
        "Return the pack. The GST control export must be for the period the self-review declares.",
    ));
}
fn apply_bank_rec(loaded: &Loaded, findings: &mut Vec<Finding>, tolerance: &Decimal) {
    for row in loaded.bank_rec.iter().flatten() {
        // The existing bank control rounds subtraction in the fixed 28-digit context.
        let difference = row.statement_balance.sub(&row.gl_balance).value;
        if difference.abs() > *tolerance {
            findings.push(Finding::new(
                "BANK_REC_BREAK",
                "NOT_READY",
                "bank_rec",
                format!(
                    "Account {} statement {} differs from GL {} by {}.",
                    row.account_id,
                    row.statement_balance.to_sci_string(),
                    row.gl_balance.to_sci_string(),
                    difference.to_sci_string()
                ),
                "Return the pack. Finish the bank reconciliation before review.",
            ));
        }
    }
}
fn apply_year_end(loaded: &Loaded, findings: &mut Vec<Finding>) {
    for row in loaded.tie_out_matrix.iter().flatten() {
        let line = pytext::repr(&row.statement_line);
        match row.status.as_str() {
            "UNSUPPORTED" => findings.push(Finding::new("TIEOUT_UNSUPPORTED", "NOT_READY", "tie_out_matrix", format!("Statement line {line} is UNSUPPORTED; there is no source for the figure."), "Return the pack. Unsupported lines are not review-ready.")),
            "EXCEPTION" => findings.push(Finding::new("TIEOUT_BREAK", "NOT_READY", "tie_out_matrix", format!("Statement line {line} has an unresolved tie-out exception."), "Return the pack. Resolve the tie-out exception before review.")),
            "TIED" if row.workpaper_ref.is_empty() || row.source_file.is_empty() => findings.push(Finding::new("TIEOUT_UNSUPPORTED", "NOT_READY", "tie_out_matrix", format!("Statement line {line} is marked TIED without a workpaper reference and source file."), "Return the pack. A tied line must name its evidence.")),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{self, TempDir};
    use std::fs;
    fn review(profile: &str, dir: &Path) -> Result<ReadinessPack> {
        review_pack(profile, dir, None, Decimal::parse("0.01").unwrap(), &[])
    }
    #[test]
    fn examples_preserve_profiles_statuses_and_repeat_findings() {
        for (example, profile, status) in [
            ("bas-ready", "bas", "READY"),
            ("bas-not-ready", "bas", "NOT_READY"),
            ("bas-blocked", "bas", "BLOCKED"),
            ("month-end-ready", "month_end", "READY"),
            ("year-end-ready", "year_end", "READY"),
        ] {
            let pack = review(profile, &test_support::examples().join(example)).unwrap();
            assert_eq!(pack.status, status);
            assert_eq!(
                pack.period_end,
                if profile == "bas" {
                    "2026-03-31"
                } else {
                    "2026-06-30"
                }
            );
            assert_eq!(
                pack.preparer_initials,
                match profile {
                    "bas" => "AB",
                    "month_end" => "CD",
                    _ => "EF",
                }
            );
            if example == "bas-not-ready" {
                assert!(
                    pack.findings
                        .iter()
                        .any(|f| f.slot == "gst_control_gl" && f.repeat)
                );
            }
        }
    }
    #[test]
    fn bas_refuses_gst_postings_after_the_declared_period() {
        for (posted_on, status) in [
            ("2026-04-01", "NOT_READY"),
            ("2026-03-31", "READY"),
            ("2026-01-01", "READY"),
        ] {
            let tmp = TempDir::new();
            let pack_dir = tmp.join("pack");
            test_support::copy_example("bas-ready", &pack_dir);
            let source = pack_dir.join("gst_control_gl.csv");
            let text = fs::read_to_string(&source).unwrap();
            let mut lines = text.lines();
            let mut rewritten = format!("{}\n", lines.next().unwrap());
            for line in lines {
                let (_, rest) = line.split_once(',').unwrap();
                rewritten.push_str(&format!("{posted_on},{rest}\n"));
            }
            fs::write(&source, rewritten).unwrap();
            let pack = review("bas", &pack_dir).unwrap();
            assert_eq!(pack.status, status, "{posted_on}");
            assert_eq!(
                pack.findings
                    .iter()
                    .any(|f| f.code == "PERIOD_ORDER" && f.slot == "gst_control_gl"),
                status == "NOT_READY",
                "{posted_on}"
            );
        }
    }
    #[test]
    fn empty_optional_evidence_is_a_finding_and_names_the_unrun_control() {
        let tmp = TempDir::new();
        let dir = tmp.join("pack");
        test_support::copy_example("month-end-ready", &dir);
        fs::write(dir.join("prior_findings.csv"), "").unwrap();
        let pack = review("month_end", &dir).unwrap();
        assert_eq!(pack.status, "NOT_READY");
        assert!(
            pack.findings
                .iter()
                .any(|f| f.code == "EMPTY_ARTEFACT" && f.slot == "prior_findings")
        );
        assert!(
            pack.controls_not_run
                .as_ref()
                .unwrap()
                .iter()
                .any(|c| c.slot == "prior_findings" && c.reason == "the file supplied is empty")
        );
        fs::remove_file(dir.join("prior_findings.csv")).unwrap();
        let pack = review("month_end", &dir).unwrap();
        assert_eq!(pack.status, "READY");
        assert!(
            pack.controls_not_run
                .as_ref()
                .unwrap()
                .iter()
                .any(|c| c.slot == "prior_findings" && c.reason == "no file was supplied")
        );
    }
    #[test]
    fn month_end_without_its_bank_reconciliation_is_not_ready() {
        let tmp = TempDir::new();
        let dir = tmp.join("pack");
        test_support::copy_example("month-end-ready", &dir);
        fs::remove_file(dir.join("bank_rec.csv")).unwrap();
        let pack = review("month_end", &dir).unwrap();
        assert_eq!(pack.status, "NOT_READY");
        assert!(
            pack.findings
                .iter()
                .any(|f| f.code == "MISSING_ARTEFACT" && f.slot == "bank_rec")
        );
        assert!(
            !pack
                .controls_not_run
                .as_ref()
                .unwrap()
                .iter()
                .any(|c| c.slot == "bank_rec")
        );
    }
    #[test]
    fn exact_totals_refuse_lost_digits_without_an_input_digit_limit() {
        let mut row = test_support::ready_pack();
        let source = SourceSnapshot::capture(
            &test_support::examples().join("bas-ready/trial_balance.csv"),
            "tb",
        )
        .unwrap();
        let mut rows = load_canonical_tb(&source).unwrap();
        let huge = Decimal::parse("100000000000000000000000000000.00").unwrap();
        rows[0].debit = huge.clone();
        rows[0].credit = huge;
        rows[0].ytd_debit = Decimal::zero();
        rows[0].ytd_credit = Decimal::zero();
        rows.truncate(1);
        assert_eq!(tb_balanced(&rows).unwrap().0.to_sci_string(), "0.00");
        rows[0].credit = Decimal::parse("0.01").unwrap();
        assert_eq!(tb_balanced(&rows).unwrap_err(), TB_PRECISION);
        row.findings.push(Finding::new(
            "test",
            "NOT_READY",
            "slot",
            "reason",
            "action",
        ));
        assert_eq!(overall(&row.findings), "NOT_READY");
        row.findings
            .push(Finding::new("test", "BLOCKED", "slot", "reason", "action"));
        assert_eq!(overall(&row.findings), "BLOCKED");
    }
    #[test]
    fn bas_allowance_is_strictly_under_one_dollar_and_rounded_bank_control_is_preserved() {
        let mut loaded = Loaded {
            activity_statement: Some(vec![
                ActivityLabel {
                    label: "1A".into(),
                    amount: Decimal::parse("100").unwrap(),
                },
                ActivityLabel {
                    label: "1B".into(),
                    amount: Decimal::zero(),
                },
            ]),
            gst_control_gl: Some(vec![GstControlRow {
                posted_on: crate::isodate::Date::new(2026, 3, 31).unwrap(),
                account_id: "1".into(),
                account_name: "GST".into(),
                debit: Decimal::zero(),
                credit: Decimal::parse("99.01").unwrap(),
                description: "".into(),
            }]),
            ..Loaded::default()
        };
        let tolerance = Decimal::parse("0.01").unwrap();
        let mut findings = Vec::new();
        apply_bas(&loaded, &mut findings, &tolerance).unwrap();
        assert!(findings.is_empty());
        loaded.gst_control_gl.as_mut().unwrap()[0].credit = Decimal::parse("99.00").unwrap();
        apply_bas(&loaded, &mut findings, &tolerance).unwrap();
        assert_eq!(findings.len(), 1);
        loaded.bank_rec = Some(vec![BankRecRow {
            account_id: "bank".into(),
            statement_balance: Decimal::parse("1000000000000000000000000000.01").unwrap(),
            gl_balance: Decimal::zero(),
        }]);
        findings.clear();
        apply_bank_rec(&loaded, &mut findings, &tolerance);
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0]
                .reason
                .ends_with("by 1000000000000000000000000000.")
        );
    }
    fn amount(text: &str) -> Decimal {
        Decimal::parse(text).unwrap()
    }
    fn codes(findings: &[Finding]) -> Vec<&str> {
        findings.iter().map(|f| f.code.as_str()).collect()
    }
    #[test]
    fn tolerance_may_be_zero_but_not_negative_or_non_finite() {
        let pack = test_support::examples().join("bas-ready");
        let run = |tolerance: &str| review_pack("bas", &pack, None, amount(tolerance), &[]);
        assert_eq!(run("0").unwrap().status, "READY");
        for refused in ["-0.01", "NaN", "Infinity"] {
            assert_eq!(
                run(refused).unwrap_err(),
                "tie-out tolerance must be a finite non-negative decimal.",
                "{refused}"
            );
        }
    }
    #[test]
    fn bas_and_bank_differences_equal_to_the_tolerance_tie() {
        let tolerance = amount("0.01");
        let mut loaded = Loaded {
            activity_statement: Some(vec![
                ActivityLabel {
                    label: "1A".into(),
                    amount: amount("100.50"),
                },
                ActivityLabel {
                    label: "1B".into(),
                    amount: Decimal::zero(),
                },
            ]),
            gst_control_gl: Some(vec![GstControlRow {
                posted_on: crate::isodate::Date::new(2026, 3, 31).unwrap(),
                account_id: "1".into(),
                account_name: "GST".into(),
                debit: Decimal::zero(),
                credit: amount("100.49"),
                description: "".into(),
            }]),
            bank_rec: Some(vec![BankRecRow {
                account_id: "bank".into(),
                statement_balance: amount("100.01"),
                gl_balance: amount("100.00"),
            }]),
            ..Loaded::default()
        };
        let mut findings = Vec::new();
        apply_bas(&loaded, &mut findings, &tolerance).unwrap();
        apply_bank_rec(&loaded, &mut findings, &tolerance);
        assert!(findings.is_empty(), "{:?}", codes(&findings));
        loaded.gst_control_gl.as_mut().unwrap()[0].credit = amount("100.48");
        loaded.bank_rec.as_mut().unwrap()[0].statement_balance = amount("100.02");
        apply_bas(&loaded, &mut findings, &tolerance).unwrap();
        apply_bank_rec(&loaded, &mut findings, &tolerance);
        assert_eq!(codes(&findings), ["TIEOUT_BREAK", "BANK_REC_BREAK"]);
    }
    #[test]
    fn bas_allowance_needs_both_labels_in_whole_dollars() {
        // 1A whole and 1B in cents: the 50-cent difference is not the allowance.
        let loaded = Loaded {
            activity_statement: Some(vec![
                ActivityLabel {
                    label: "1A".into(),
                    amount: amount("100"),
                },
                ActivityLabel {
                    label: "1B".into(),
                    amount: amount("0.50"),
                },
            ]),
            gst_control_gl: Some(vec![GstControlRow {
                posted_on: crate::isodate::Date::new(2026, 3, 31).unwrap(),
                account_id: "1".into(),
                account_name: "GST".into(),
                debit: Decimal::zero(),
                credit: amount("99.00"),
                description: "".into(),
            }]),
            ..Loaded::default()
        };
        let mut findings = Vec::new();
        apply_bas(&loaded, &mut findings, &amount("0.01")).unwrap();
        assert_eq!(codes(&findings), ["TIEOUT_BREAK"]);
    }
    #[test]
    fn prior_trial_balance_must_balance_on_movement_and_ytd_alike() {
        let source = SourceSnapshot::capture(
            &test_support::examples().join("month-end-ready/prior_trial_balance.csv"),
            "tb",
        )
        .unwrap();
        let balanced = load_canonical_tb(&source).unwrap();
        let mut findings = Vec::new();
        let mut loaded = Loaded {
            prior_trial_balance: Some(balanced.clone()),
            ..Loaded::default()
        };
        apply_trial_balance(&loaded, &mut findings).unwrap();
        assert!(findings.is_empty(), "{:?}", codes(&findings));
        for movement_only in [true, false] {
            let mut rows = balanced.clone();
            if movement_only {
                rows[0].debit = rows[0].debit.add(&amount("1")).value;
            } else {
                rows[0].ytd_debit = rows[0].ytd_debit.add(&amount("1")).value;
            }
            loaded.prior_trial_balance = Some(rows);
            findings.clear();
            apply_trial_balance(&loaded, &mut findings).unwrap();
            assert_eq!(codes(&findings), ["TB_UNBALANCED"], "{movement_only}");
        }
    }
    #[test]
    fn only_unresolved_blocking_open_items_block() {
        let item = |severity: &str, status: &str, resolution: &str| OpenItem {
            item_id: format!("{severity}-{status}-{resolution}"),
            severity: severity.into(),
            owner: "AB".into(),
            due_date: crate::isodate::Date::new(2026, 4, 5).unwrap(),
            status: status.into(),
            description: "fabricated".into(),
            resolution: resolution.into(),
        };
        for (row, expected) in [
            (item("BLOCKING", "OPEN", ""), vec!["OPEN_ITEM_BLOCKING"]),
            (item("EXPLAIN", "OPEN", ""), vec![]),
            (item("BLOCKING", "OPEN", "Resolved by AB"), vec![]),
            (
                item("BLOCKING", "CLEARED", ""),
                vec!["OPEN_ITEM_INCOMPLETE"],
            ),
            (item("BLOCKING", "CLEARED", "Filed"), vec![]),
        ] {
            let loaded = Loaded {
                open_items: Some(vec![row.clone()]),
                ..Loaded::default()
            };
            let mut findings = Vec::new();
            apply_open_items(&loaded, &mut findings);
            assert_eq!(codes(&findings), expected, "{}", row.item_id);
        }
    }
    #[test]
    fn late_gst_postings_show_three_dates_then_a_count() {
        for (late, suffix) in [(3, "2026-04-03."), (4, "2026-04-03 and 1 more.")] {
            let tmp = TempDir::new();
            let pack_dir = tmp.join("pack");
            test_support::copy_example("bas-ready", &pack_dir);
            let mut rows = String::from("Date,AccountID,AccountName,Debit,Credit,Description\n");
            for day in 1..=late {
                rows.push_str(&format!("2026-04-0{day},820,GST Payable,0.00,0.00,late\n"));
            }
            fs::write(pack_dir.join("gst_control_gl.csv"), rows).unwrap();
            let pack = review("bas", &pack_dir).unwrap();
            let finding = pack
                .findings
                .iter()
                .find(|f| f.code == "PERIOD_ORDER" && f.slot == "gst_control_gl")
                .unwrap();
            assert!(
                finding
                    .reason
                    .contains("2026-04-01, 2026-04-02, 2026-04-03")
                    && finding.reason.contains(suffix),
                "{late}: {}",
                finding.reason
            );
        }
    }
    #[test]
    fn year_end_lines_need_a_tie_with_its_workpaper_and_source() {
        let row = |status: &str, workpaper_ref: &str, source_file: &str| TieOutRow {
            statement_line: "Cash".into(),
            workpaper_ref: workpaper_ref.into(),
            source_file: source_file.into(),
            status: status.into(),
        };
        for (line, expected) in [
            (row("TIED", "WP-100", "trial_balance.csv"), vec![]),
            (
                row("TIED", "", "trial_balance.csv"),
                vec!["TIEOUT_UNSUPPORTED"],
            ),
            (row("TIED", "WP-100", ""), vec!["TIEOUT_UNSUPPORTED"]),
            (
                row("UNSUPPORTED", "WP-100", "trial_balance.csv"),
                vec!["TIEOUT_UNSUPPORTED"],
            ),
            (
                row("EXCEPTION", "WP-100", "trial_balance.csv"),
                vec!["TIEOUT_BREAK"],
            ),
        ] {
            let loaded = Loaded {
                tie_out_matrix: Some(vec![line.clone()]),
                ..Loaded::default()
            };
            let mut findings = Vec::new();
            apply_year_end(&loaded, &mut findings);
            assert_eq!(codes(&findings), expected, "{line:?}");
        }
    }
    #[test]
    fn only_findings_open_on_the_prior_pack_are_repeats() {
        let pack = review("bas", &test_support::examples().join("bas-not-ready")).unwrap();
        assert!(pack.findings.iter().any(|f| !f.repeat));
        for finding in &pack.findings {
            assert_eq!(
                finding.repeat,
                finding.code == "MISSING_ARTEFACT" && finding.slot == "gst_control_gl",
                "{} {}",
                finding.code,
                finding.slot
            );
        }
    }
    #[test]
    fn a_review_note_may_be_dated_on_the_period_end_but_not_before() {
        let tmp = TempDir::new();
        let pack = test_support::examples().join("bas-ready");
        for (reviewed_on, accepted) in [("2026-03-31", true), ("2026-03-30", false)] {
            let note = tmp.join(format!("note-{reviewed_on}.json"));
            fs::write(
                &note,
                format!(r#"{{"reviewer_initials": "RD", "reviewed_on": "{reviewed_on}", "comment": "fabricated"}}"#),
            )
            .unwrap();
            let result = review_pack("bas", &pack, Some(&note), amount("0.01"), &[]);
            assert_eq!(result.is_ok(), accepted, "{reviewed_on}: {result:?}");
        }
    }
    #[test]
    fn documents_are_checked_against_the_pack_entity_and_period() {
        let pack = test_support::examples().join("bas-ready");
        let invoice = test_support::examples().join("document-intake/invoice.txt");
        for (period, mismatch) in [("2026-03-31", false), ("2026-06-30", true)] {
            let tmp = TempDir::new();
            let bundle = crate::documents::create_intake(crate::documents::IntakeOptions {
                source: &invoice,
                text: &invoice,
                output: &tmp.join("bundle"),
                entity: "Cedar and Pine Consulting Pty Ltd",
                period,
                origin: "manual",
                version: "",
            })
            .unwrap();
            let result = review_pack("bas", &pack, None, amount("0.01"), &[bundle]).unwrap();
            assert!(
                result.source_evidence.len() > test_support::ready_pack().source_evidence.len(),
                "{period}"
            );
            assert_eq!(
                result
                    .findings
                    .iter()
                    .any(|f| f.code == "DOCUMENT_CONTEXT_MISMATCH"),
                mismatch,
                "{period}"
            );
        }
    }
}
