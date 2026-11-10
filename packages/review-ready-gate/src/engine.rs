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
    collections::{BTreeMap, BTreeSet},
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
    fs_mapping: Option<Vec<FsMappingRow>>,
    prior_findings: Option<Vec<PriorFinding>>,
}

/// Keeps `rows` in `slot`, and says whether there were any.
fn held<T>(slot: &mut Option<Vec<T>>, rows: Vec<T>) -> bool {
    let any = !rows.is_empty();
    *slot = Some(rows);
    any
}

impl Loaded {
    /// Loads the file, and says whether it held anything for its control to read.
    fn load(&mut self, slot: &Slot, source: &SourceSnapshot, profile: &str) -> Result<bool> {
        Ok(match slot.kind {
            "trial_balance" if slot.name == "prior_trial_balance" => {
                held(&mut self.prior_trial_balance, load_canonical_tb(source)?)
            }
            "trial_balance" => held(&mut self.trial_balance, load_canonical_tb(source)?),
            "open_items" => held(&mut self.open_items, load_open_items(source)?),
            "self_review" => {
                self.self_review = Some(load_self_review(source, profile)?);
                true
            }
            "activity_statement" => held(
                &mut self.activity_statement,
                load_activity_statement(source)?,
            ),
            "gst_control_gl" => held(&mut self.gst_control_gl, load_gst_control(source)?),
            "bank_rec" => held(&mut self.bank_rec, load_bank_rec(source)?),
            "tie_out_matrix" => held(&mut self.tie_out_matrix, load_tie_out_matrix(source)?),
            "fs_mapping" => held(&mut self.fs_mapping, load_fs_mapping(source)?),
            "prior_findings" => held(&mut self.prior_findings, load_prior_findings(source)?),
            _ => return Err(format!("unknown slot kind {}.", pytext::repr(slot.kind))),
        })
    }
}

/// The finding code for a required control's state, if it needs a finding.
pub fn required_control_code(state: &str) -> Option<&'static str> {
    match state {
        "skipped" => Some("REQUIRED_CONTROL_SKIPPED"),
        "failed" => Some("REQUIRED_CONTROL_FAILED"),
        _ => None,
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
const STATEMENT_PRECISION: &str = "statement and trial balance amounts exceed the supported 28-digit precision, so a statement line cannot be computed exactly.";
const BANK_PRECISION: &str = "bank reconciliation and trial balance amounts exceed the supported 28-digit precision, so a GL balance cannot be tied to the trial balance exactly.";

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
        if loaded.load(slot, &source, profile)? {
            ran.insert(slot.name);
        } else {
            failed.insert(slot.name);
        }
    }
    let mut required_controls = Vec::new();
    for name in required_names {
        let (state, outcome) = if ran.contains(name.as_str()) {
            ("ran", String::new())
        } else if failed.contains(name.as_str()) {
            ("failed", "failed: its file holds no rows".to_owned())
        } else if slots.iter().any(|slot| slot.name == name) {
            ("skipped", "was skipped: no file was supplied".to_owned())
        } else {
            let outcome = format!("was skipped: the {profile} profile does not run it");
            ("skipped", outcome)
        };
        if let Some(code) = required_control_code(state) {
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
    apply_gst_roll_forward(&loaded, &mut findings, &tolerance)?;
    apply_gst_period(&loaded, &mut findings);
    apply_bank_rec(&loaded, &mut findings, &tolerance)?;
    apply_year_end(&loaded, &mut findings);
    apply_statement_mapping(&loaded, &mut findings, &tolerance)?;
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
    let gl = gst_movement(gst)?;
    let difference = exact_sub(&statement, &gl, BAS_PRECISION)?;
    let whole = a.is_integral() && b.is_integral();
    let abs = difference.abs();
    if abs > *tolerance && !(whole && abs < Decimal::parse("1").unwrap()) {
        let allowance = if whole {
            " or the 99-cent allowance for whole-dollar 1A and 1B"
        } else {
            ""
        };
        findings.push(Finding::new("TIEOUT_BREAK", "NOT_READY", "gst_control_gl", format!("Net GST on the activity statement ({}) does not tie to GST control movement ({}); difference {} exceeds tolerance {}{allowance}.{}", statement.to_sci_string(), gl.to_sci_string(), difference.to_sci_string(), tolerance.to_sci_string(), difference_hint(&difference, gst)), "Return the pack. Complete the GST control tie-out before review."));
    }
    Ok(())
}
/// The GST control file's net movement, credits positive.
fn gst_movement(gst: &[GstControlRow]) -> Result<Decimal> {
    exact_sum(
        gst.iter()
            .map(|r| exact_sub(&r.credit, &r.debit, BAS_PRECISION)),
        BAS_PRECISION,
    )
}
/// The activity-statement label that declares the balance the GST control
/// accounts carry in from before the period and still hold at `period_end`:
/// the period-start balance less any settlement of it posted in the period,
/// which the GST control file does not hold. Credit positive, like the
/// closing balance.
const GST_OPENING: &str = "GST_OPENING";
/// The GST control accounts' closing balance in the trial balance,
/// `YTDCredit - YTDDebit`, against the balance they carry in plus the
/// period's movement in the GST control file. The balance carried in is the
/// activity statement's `GST_OPENING`, or nil when it declares none.
fn apply_gst_roll_forward(
    loaded: &Loaded,
    findings: &mut Vec<Finding>,
    tolerance: &Decimal,
) -> Result<()> {
    let (Some(gst), Some(tb)) = (&loaded.gst_control_gl, &loaded.trial_balance) else {
        return Ok(());
    };
    let accounts: BTreeSet<_> = gst.iter().map(|r| r.account_id.as_str()).collect();
    if let Some(finding) = gst_accounts_missing(&accounts, tb) {
        findings.push(finding);
        return Ok(());
    }
    let closing = exact_sum(
        tb.iter()
            .filter(|r| accounts.contains(r.account_id.as_str()))
            .map(|r| exact_sub(&r.ytd_credit, &r.ytd_debit, BAS_PRECISION)),
        BAS_PRECISION,
    )?;
    let opening = loaded
        .activity_statement
        .iter()
        .flatten()
        .find(|r| r.label == GST_OPENING)
        .map(|r| &r.amount);
    let movement = gst_movement(gst)?;
    let carried = match opening {
        Some(opening) => exact_sub(&closing, opening, BAS_PRECISION)?,
        None => closing.clone(),
    };
    let difference = exact_sub(&carried, &movement, BAS_PRECISION)?;
    if difference.abs() > *tolerance {
        let figures = [&closing, &movement, &difference, tolerance].map(Decimal::to_sci_string);
        findings.push(Finding::new(
            "TIEOUT_BREAK",
            "NOT_READY",
            "gst_control_gl",
            roll_forward_break(&figures, opening) + &difference_hint(&difference, gst),
            "Return the pack. Complete the GST control file, or declare the balance carried in as GST_OPENING on the activity statement, before review.",
        ));
    }
    Ok(())
}
/// The roll-forward break, naming the balance carried in that it used.
fn roll_forward_break(figures: &[String; 4], opening: Option<&Decimal>) -> String {
    let [closing, movement, difference, tolerance] = figures;
    match opening {
        Some(opening) => format!(
            "GST control accounts close at {closing} in the trial balance, but carry in {} from before the period, the activity statement's GST_OPENING, and the GST control file moves them by {movement}; difference {difference} exceeds tolerance {tolerance}. The file may be missing postings, or GST_OPENING may be wrong.",
            opening.to_sci_string()
        ),
        None => format!(
            "GST control accounts close at {closing} in the trial balance, but the GST control file moves them by {movement} and, with no GST_OPENING on the activity statement, they are taken to carry in nil; difference {difference} exceeds tolerance {tolerance}. The file may be missing postings, or the accounts carried a balance in, such as an unpaid earlier BAS or the cents the ATO drops from 1A and 1B, which GST_OPENING declares.",
        ),
    }
}
fn gst_accounts_missing(accounts: &BTreeSet<&str>, tb: &[TrialBalanceRow]) -> Option<Finding> {
    let missing: Vec<_> = accounts
        .iter()
        .filter(|id| !tb.iter().any(|r| r.account_id == **id))
        .map(|id| (*id).to_owned())
        .collect();
    (!missing.is_empty()).then(|| Finding::new(
        "TIEOUT_BREAK",
        "NOT_READY",
        "gst_control_gl",
        format!(
            "GST control file accounts are not in the trial balance: {}. The control accounts cannot be rolled forward to the ledger.",
            first_three(&missing)
        ),
        "Return the pack. Export the trial balance and the GST control file from the same ledger.",
    ))
}
/// A pointer to the likeliest cause of a GST difference, the other figure less
/// the file's: a file posting the other figure lacks, so the difference is minus
/// the posting; one it holds on the other side, minus twice the posting; or
/// transposed digits. A posting of the right size in the wrong direction
/// explains nothing, so it is not named.
fn difference_hint(difference: &Decimal, gst: &[GstControlRow]) -> String {
    let size = difference.copy_abs();
    let posting = |r: &GstControlRow| {
        let net = r.credit.sub(&r.debit).value;
        let side = if net.is_signed() { "debit" } else { "credit" };
        let description = if r.description.is_empty() {
            String::new()
        } else {
            format!(" {}", pytext::repr(&r.description))
        };
        let label = format!(
            "the {} posting{description} ({side} {})",
            r.posted_on.iso(),
            net.copy_abs().to_sci_string()
        );
        (net, label)
    };
    let postings: Vec<_> = gst
        .iter()
        .map(posting)
        .filter(|(net, _)| !net.is_zero())
        .collect();
    let lacks = |net: &Decimal| net.add(difference).value.is_zero();
    if let Some((_, label)) = postings.iter().find(|(net, _)| lacks(net)) {
        return format!(" Hint: the difference is the size of {label}; check that posting first.");
    }
    if let Some((_, label)) = postings.iter().find(|(net, _)| lacks(&net.add(net).value)) {
        return format!(
            " Hint: the difference is twice {label}, so that posting may be on the wrong side."
        );
    }
    let cents = size.mul(&Decimal::parse("100").unwrap());
    let nine = Decimal::parse("9").unwrap();
    let ninths = cents.div(&nine);
    if cents.is_integral() && ninths.is_integral() && ninths.mul(&nine) == cents {
        return " Hint: the difference is a whole number of cents divisible by 9, which can mean 2 digits were transposed.".into();
    }
    String::new()
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
    let late: Vec<_> = late.iter().map(|d| d.iso()).collect();
    findings.push(Finding::new(
        "PERIOD_ORDER",
        "NOT_READY",
        "gst_control_gl",
        format!(
            "GST control postings are dated after the declared period_end {}: {}.",
            s.period_end.iso(),
            first_three(&late)
        ),
        "Return the pack. The GST control export must be for the period the self-review declares.",
    ));
}
/// Each reconciled account's GL balance against the trial balance's
/// `YTDDebit - YTDCredit` for the same `AccountID`: debit balances positive,
/// credit balances negative, as Monthly Close Controls reads `SubledgerBalance`.
fn apply_bank_rec(loaded: &Loaded, findings: &mut Vec<Finding>, tolerance: &Decimal) -> Result<()> {
    apply_bank_statement(loaded, findings, tolerance);
    let (Some(bank), Some(tb)) = (&loaded.bank_rec, &loaded.trial_balance) else {
        return Ok(());
    };
    for row in bank {
        let Some(account) = tb.iter().find(|r| r.account_id == row.account_id) else {
            findings.push(Finding::new(
                "BANK_REC_BREAK",
                "NOT_READY",
                "bank_rec",
                format!(
                    "Account {} is not in the trial balance, so its GL balance {} cannot be tied to the ledger.",
                    row.account_id,
                    row.gl_balance.to_sci_string()
                ),
                "Return the pack. Reconcile an account the trial balance holds, or correct its AccountID.",
            ));
            continue;
        };
        let ledger = exact_sub(&account.ytd_debit, &account.ytd_credit, BANK_PRECISION)?;
        let difference = exact_sub(&row.gl_balance, &ledger, BANK_PRECISION)?;
        if difference.abs() > *tolerance {
            findings.push(Finding::new(
                "BANK_REC_BREAK",
                "NOT_READY",
                "bank_rec",
                format!(
                    "Account {} GL balance {} differs from the trial balance's balance {} by {}.",
                    row.account_id,
                    row.gl_balance.to_sci_string(),
                    ledger.to_sci_string(),
                    difference.to_sci_string()
                ),
                "Return the pack. Reconcile the bank account to the ledger this trial balance reports.",
            ));
        }
    }
    Ok(())
}
fn apply_bank_statement(loaded: &Loaded, findings: &mut Vec<Finding>, tolerance: &Decimal) {
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
/// Each trial-balance account's `YTDDebit - YTDCredit`, by `AccountID`.
type Balances<'a> = BTreeMap<&'a str, (Decimal, &'a TrialBalanceRow)>;
/// Each `TIED` statement line against the trial-balance accounts `fs_mapping`
/// places on it, and every account with a balance on some line. The computed
/// amount is the sum of `YTDDebit - YTDCredit`, negated on a credit-normal line.
fn apply_statement_mapping(
    loaded: &Loaded,
    findings: &mut Vec<Finding>,
    tolerance: &Decimal,
) -> Result<()> {
    let (Some(lines), Some(mapping), Some(tb)) = (
        &loaded.tie_out_matrix,
        &loaded.fs_mapping,
        &loaded.trial_balance,
    ) else {
        return Ok(());
    };
    let balances = tb
        .iter()
        .map(|r| {
            let net = exact_sub(&r.ytd_debit, &r.ytd_credit, STATEMENT_PRECISION)?;
            Ok((r.account_id.as_str(), (net, r)))
        })
        .collect::<Result<Balances>>()?;
    mapping_coverage(lines, mapping, &balances, findings);
    computed_lines(lines, mapping, &balances, tolerance, findings)
}
fn mapping_finding(code: &str, reason: String) -> Finding {
    let action = if code == "TIEOUT_UNSUPPORTED" {
        "Return the pack. Map the trial-balance accounts behind every tied line."
    } else {
        "Return the pack. Correct the statement, the mapping or the ledger until each tied line computes from the trial balance."
    };
    Finding::new(code, "NOT_READY", "fs_mapping", reason, action)
}
/// Mapping rows that name a line or account the pack does not have, and
/// trial-balance accounts with a balance that no row places on a line.
fn mapping_coverage(
    lines: &[TieOutRow],
    mapping: &[FsMappingRow],
    balances: &Balances,
    findings: &mut Vec<Finding>,
) {
    for row in mapping {
        let line = pytext::repr(&row.statement_line);
        if !lines.iter().any(|l| l.statement_line == row.statement_line) {
            findings.push(mapping_finding(
                "TIEOUT_BREAK",
                format!(
                    "fs_mapping places account {} on statement line {line}, which the tie-out matrix does not have.",
                    row.account_id
                ),
            ));
        }
        if !balances.contains_key(row.account_id.as_str()) {
            findings.push(mapping_finding(
                "TIEOUT_BREAK",
                format!(
                    "fs_mapping places account {} on statement line {line}, but the trial balance does not hold that account.",
                    row.account_id
                ),
            ));
        }
    }
    let unmapped: Vec<_> = balances
        .iter()
        .filter(|(id, (net, _))| !net.is_zero() && !mapping.iter().any(|m| m.account_id == **id))
        .map(|(id, (_, r))| format!("{id} ({})", r.account_name))
        .collect();
    if !unmapped.is_empty() {
        findings.push(mapping_finding(
            "TIEOUT_BREAK",
            format!(
                "Trial-balance accounts with a balance are on no statement line: {}.",
                first_three(&unmapped)
            ),
        ));
    }
}
/// The sum of the accounts' `YTDDebit - YTDCredit`, negated on a credit-normal
/// line, or `None` when the trial balance lacks one of them.
fn line_amount(
    accounts: &[&FsMappingRow],
    credit: bool,
    balances: &Balances,
) -> Result<Option<Decimal>> {
    let Some(nets) = accounts
        .iter()
        .map(|m| {
            balances
                .get(m.account_id.as_str())
                .map(|(net, _)| Ok(net.clone()))
        })
        .collect::<Option<Vec<_>>>()
    else {
        return Ok(None);
    };
    let debit_net = exact_sum(nets.into_iter(), STATEMENT_PRECISION)?;
    Ok(Some(if credit {
        exact_sub(&Decimal::zero(), &debit_net, STATEMENT_PRECISION)?
    } else {
        debit_net
    }))
}
/// Each `TIED` line's `StatementAmount` against the sum of its mapped accounts.
fn computed_lines(
    lines: &[TieOutRow],
    mapping: &[FsMappingRow],
    balances: &Balances,
    tolerance: &Decimal,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    for row in lines.iter().filter(|l| l.status == "TIED") {
        let line = pytext::repr(&row.statement_line);
        let accounts: Vec<_> = mapping
            .iter()
            .filter(|m| m.statement_line == row.statement_line)
            .collect();
        let Some(first) = accounts.first() else {
            findings.push(mapping_finding(
                "TIEOUT_UNSUPPORTED",
                format!(
                    "Statement line {line} is TIED, but fs_mapping places no trial-balance account on it, so its amount was not computed."
                ),
            ));
            continue;
        };
        // A mapped account the trial balance lacks is already a finding.
        let Some(computed) = line_amount(&accounts, first.credit, balances)? else {
            continue;
        };
        let difference = exact_sub(&row.statement_amount, &computed, STATEMENT_PRECISION)?;
        if difference.abs() > *tolerance {
            findings.push(mapping_finding(
                "TIEOUT_BREAK",
                format!(
                    "Statement line {line} shows {}, but its mapped trial-balance accounts give {}; difference {} exceeds tolerance {}.",
                    row.statement_amount.to_sci_string(),
                    computed.to_sci_string(),
                    difference.to_sci_string(),
                    tolerance.to_sci_string()
                ),
            ));
        }
    }
    Ok(())
}
/// The first 3 items, then a count of the rest.
fn first_three(items: &[String]) -> String {
    let shown = items.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
    if items.len() > 3 {
        format!("{shown} and {} more", items.len() - 3)
    } else {
        shown
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
    /// The bas-ready pack with a reconciliation of its operating bank account,
    /// which its trial balance holds at 127500.00.
    fn bas_with_bank_rec(tmp: &TempDir, statement: &str) -> PathBuf {
        let dir = tmp.join("pack");
        test_support::copy_example("bas-ready", &dir);
        fs::write(
            dir.join("bank_rec.csv"),
            format!(
                "AccountID,StatementBalance,GLBalance
100,{statement},127500.00
"
            ),
        )
        .unwrap();
        dir
    }
    fn bank_rec_not_run(pack: &ReadinessPack) -> bool {
        let controls = pack.controls_not_run.as_ref().unwrap();
        controls.iter().any(|c| c.slot == "bank_rec")
    }
    #[test]
    fn bas_runs_a_bank_reconciliation_when_the_pack_holds_one() {
        // Without the file the pack is still READY, and says the control did not run.
        let pack = review("bas", &test_support::examples().join("bas-ready")).unwrap();
        assert_eq!(pack.status, "READY");
        assert!(bank_rec_not_run(&pack));
        for (statement, status, expected) in [
            ("127500.00", "READY", vec![]),
            ("120000.00", "NOT_READY", vec!["BANK_REC_BREAK"]),
        ] {
            let tmp = TempDir::new();
            let pack = review("bas", &bas_with_bank_rec(&tmp, statement)).unwrap();
            assert_eq!(pack.status, status, "{statement}");
            assert_eq!(codes(&pack.findings), expected, "{statement}");
            assert!(!bank_rec_not_run(&pack), "{statement}");
            assert!(pack.source_evidence.iter().any(|e| e.slot == "bank_rec"));
        }
    }
    #[test]
    fn a_bas_can_be_held_for_its_bank_reconciliation() {
        let require = |dir: &Path| {
            let required = ["bank_rec".to_owned()];
            review_requiring("bas", dir, None, amount("0.01"), &[], &required).unwrap()
        };
        // No file: skipped for want of one, not because bas has no such control.
        let pack = require(&test_support::examples().join("bas-ready"));
        assert_eq!(pack.status, "NOT_READY");
        assert_eq!(codes(&pack.findings), ["REQUIRED_CONTROL_SKIPPED"]);
        assert!(
            pack.findings[0].reason.contains("no file was supplied"),
            "{}",
            pack.findings[0].reason
        );
        for (statement, status, expected) in [
            ("127500.00", "READY", vec![]),
            ("120000.00", "NOT_READY", vec!["BANK_REC_BREAK"]),
        ] {
            let tmp = TempDir::new();
            let pack = require(&bas_with_bank_rec(&tmp, statement));
            assert_eq!(pack.status, status, "{statement}");
            assert_eq!(codes(&pack.findings), expected, "{statement}");
            assert_eq!(pack.required_controls[0].state, "ran", "{statement}");
        }
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
        apply_bank_rec(&loaded, &mut findings, &tolerance).unwrap();
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
        apply_bank_rec(&loaded, &mut findings, &tolerance).unwrap();
        assert!(findings.is_empty(), "{:?}", codes(&findings));
        loaded.gst_control_gl.as_mut().unwrap()[0].credit = amount("100.48");
        loaded.bank_rec.as_mut().unwrap()[0].statement_balance = amount("100.02");
        apply_bas(&loaded, &mut findings, &tolerance).unwrap();
        apply_bank_rec(&loaded, &mut findings, &tolerance).unwrap();
        assert_eq!(codes(&findings), ["TIEOUT_BREAK", "BANK_REC_BREAK"]);
    }
    /// month-end-ready with its bank reconciliation replaced. With `overdraft`,
    /// the trial balance also holds account 101, a $5,000 credit balance, and
    /// account 100 carries the matching $5,000 so both pairs still balance.
    fn month_end_bank(tmp: &TempDir, overdraft: bool, bank_rec: &str) -> PathBuf {
        let dir = tmp.join("pack");
        test_support::copy_example("month-end-ready", &dir);
        if overdraft {
            let path = dir.join("trial_balance.csv");
            let text = fs::read_to_string(&path).unwrap();
            let bank = ",100,Operating Bank,1000,15000.00,0.00,120000.00,0.00";
            assert!(text.contains(bank));
            let text = text.replace(
                bank,
                ",100,Operating Bank,1000,15000.00,0.00,125000.00,0.00",
            );
            fs::write(&path, format!("{text}2026-06-30,Varrock Ventures Pty Ltd,Liabilities,101,Business Overdraft,1010,0.00,0.00,0.00,5000.00\n")).unwrap();
        }
        fs::write(
            dir.join("bank_rec.csv"),
            format!("AccountID,StatementBalance,GLBalance\n{bank_rec}"),
        )
        .unwrap();
        dir
    }
    fn bank_breaks(pack: &ReadinessPack) -> Vec<&str> {
        pack.findings
            .iter()
            .filter(|f| f.code == "BANK_REC_BREAK")
            .map(|f| f.reason.as_str())
            .collect()
    }
    #[test]
    fn a_bank_rec_that_agrees_with_itself_but_not_the_trial_balance_is_not_ready() {
        // The statement equals the reconciliation's own GL balance, so the
        // statement check passes; the trial balance holds 120,000.00.
        let tmp = TempDir::new();
        let pack = review(
            "month_end",
            &month_end_bank(&tmp, false, "100,119000.00,119000.00\n"),
        )
        .unwrap();
        assert_eq!(pack.status, "NOT_READY");
        assert_eq!(
            bank_breaks(&pack),
            [
                "Account 100 GL balance 119000.00 differs from the trial balance's balance 120000.00 by -1000.00."
            ]
        );
    }
    #[test]
    fn a_reconciled_account_the_trial_balance_does_not_hold_is_not_ready() {
        let tmp = TempDir::new();
        let pack = review(
            "month_end",
            &month_end_bank(&tmp, false, "100,120000.00,120000.00\n999,50.00,50.00\n"),
        )
        .unwrap();
        assert_eq!(pack.status, "NOT_READY");
        assert_eq!(
            bank_breaks(&pack),
            [
                "Account 999 is not in the trial balance, so its GL balance 50.00 cannot be tied to the ledger."
            ]
        );
    }
    #[test]
    fn bank_gl_balances_tie_to_the_trial_balance_debit_positive_in_both_signs() {
        for (rows, breaks) in [
            ("100,125000.00,125000.00\n101,-5000.00,-5000.00\n", 0),
            // A credit balance entered as positive is a sign error, not a tie.
            ("100,125000.00,125000.00\n101,5000.00,5000.00\n", 1),
            ("100,-125000.00,-125000.00\n101,-5000.00,-5000.00\n", 1),
        ] {
            let tmp = TempDir::new();
            let pack = review("month_end", &month_end_bank(&tmp, true, rows)).unwrap();
            assert_eq!(bank_breaks(&pack).len(), breaks, "{rows}");
            assert_eq!(pack.status == "READY", breaks == 0, "{rows}");
        }
    }
    #[test]
    fn bank_gl_to_trial_balance_differences_equal_to_the_tolerance_tie() {
        for (gl, ties) in [
            ("120000.00", true),
            ("120000.009", true),
            ("120000.01", true),
            ("119999.99", true),
            ("120000.011", false),
            ("119999.989", false),
            ("120000.02", false),
        ] {
            let tmp = TempDir::new();
            let pack = review(
                "month_end",
                &month_end_bank(&tmp, false, &format!("100,{gl},{gl}\n")),
            )
            .unwrap();
            assert_eq!(bank_breaks(&pack).is_empty(), ties, "{gl}");
        }
    }
    /// Every account in year-end-ready's trial balance on a statement line.
    /// Loss for the year is 80,000 + 90,000 - 130,000 = 40,000, debit-normal.
    const MATRIX: &str = "Cash,120000.00,WP-100,trial_balance.csv,TIED
Trade debtors,85000.00,WP-110,trial_balance.csv,TIED
Trade creditors,65000.00,WP-200,trial_balance.csv,TIED
Loss for the year,40000.00,WP-300,trial_balance.csv,TIED
Retained earnings,180000.00,WP-900,trial_balance.csv,TIED
";
    const MAPPING: &str = "Cash,100,DEBIT
Trade debtors,110,DEBIT
Trade creditors,200,CREDIT
Loss for the year,300,DEBIT
Loss for the year,400,DEBIT
Loss for the year,500,DEBIT
Retained earnings,900,CREDIT
";
    fn year_end_mapped(tmp: &TempDir, matrix: &str, mapping: &str) -> PathBuf {
        let dir = tmp.join("pack");
        test_support::copy_example("year-end-ready", &dir);
        fs::write(
            dir.join("tie_out_matrix.csv"),
            format!("StatementLine,StatementAmount,WorkpaperRef,SourceFile,Status\n{matrix}"),
        )
        .unwrap();
        fs::write(
            dir.join("fs_mapping.csv"),
            format!("StatementLine,AccountID,NormalBalance\n{mapping}"),
        )
        .unwrap();
        dir
    }
    fn mapping_codes(pack: &ReadinessPack) -> Vec<&str> {
        pack.findings
            .iter()
            .filter(|f| f.slot == "fs_mapping")
            .map(|f| f.code.as_str())
            .collect()
    }
    #[test]
    fn year_end_without_a_statement_mapping_names_it_under_controls_not_run() {
        let dir = test_support::examples().join("year-end-ready");
        let pack = review("year_end", &dir).unwrap();
        assert_eq!(pack.status, "READY");
        assert_eq!(
            pack.controls_not_run.unwrap(),
            [
                ControlNotRun {
                    slot: "fs_mapping".into(),
                    filename: "fs_mapping.csv".into(),
                    reason: "no file was supplied".into(),
                },
                ControlNotRun {
                    slot: "prior_findings".into(),
                    filename: "prior_findings.csv".into(),
                    reason: "no file was supplied".into(),
                },
            ]
        );
        let required = review_requiring(
            "year_end",
            &dir,
            None,
            amount("0.01"),
            &[],
            &["fs_mapping".into()],
        )
        .unwrap();
        assert_eq!(codes(&required.findings), ["REQUIRED_CONTROL_SKIPPED"]);
    }
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one case per statement mapping shape"
    )]
    fn statement_lines_are_computed_from_the_mapped_trial_balance_accounts() {
        let profit = MATRIX.replace(
            "Loss for the year,40000.00",
            "Profit for the year,-40000.00",
        );
        let profit_mapping = MAPPING.replace("Loss for the year,", "Profit for the year,");
        let profit_mapping = profit_mapping.replace(",DEBIT\nProfit", ",CREDIT\nProfit");
        let profit_mapping = profit_mapping.replace("500,DEBIT", "500,CREDIT");
        let retained = "Retained earnings,180000.00,WP-900,trial_balance.csv,TIED\n";
        for (case, matrix, mapping, expected, asserted) in [
            (
                "one-to-one and several to one",
                MATRIX.to_owned(),
                MAPPING.to_owned(),
                vec![],
                vec![],
            ),
            (
                "a negative credit-normal line",
                profit,
                profit_mapping,
                vec![],
                vec![],
            ),
            (
                "an unmatched line",
                format!("{MATRIX}Inventory,0.00,WP-130,trial_balance.csv,TIED\n"),
                MAPPING.to_owned(),
                vec!["TIEOUT_UNSUPPORTED"],
                vec![],
            ),
            (
                "an unmapped account",
                MATRIX.replace(retained, ""),
                MAPPING.replace("Retained earnings,900,CREDIT\n", ""),
                vec!["TIEOUT_BREAK"],
                vec![],
            ),
            (
                "a mapped account the trial balance lacks",
                MATRIX.to_owned(),
                format!("{MAPPING}Cash,105,DEBIT\n"),
                vec!["TIEOUT_BREAK"],
                vec![],
            ),
            (
                "a mapped line the matrix lacks",
                MATRIX.to_owned(),
                format!("{MAPPING}Inventory,100,DEBIT\n"),
                vec!["TIEOUT_BREAK"],
                vec![],
            ),
            (
                "a credit balance on a debit-normal line",
                MATRIX.to_owned(),
                MAPPING.replace("200,CREDIT", "200,DEBIT"),
                vec!["TIEOUT_BREAK"],
                vec![],
            ),
            (
                "an asserted exception",
                MATRIX.replace(
                    "Cash,120000.00,WP-100,trial_balance.csv,TIED",
                    "Cash,1.00,WP-100,trial_balance.csv,EXCEPTION",
                ),
                MAPPING.to_owned(),
                vec![],
                vec!["TIEOUT_BREAK"],
            ),
            (
                "an asserted unsupported line",
                MATRIX.replace(
                    "Cash,120000.00,WP-100,trial_balance.csv,TIED",
                    "Cash,1.00,,,UNSUPPORTED",
                ),
                MAPPING.to_owned(),
                vec![],
                vec!["TIEOUT_UNSUPPORTED"],
            ),
        ] {
            let tmp = TempDir::new();
            let pack = review("year_end", &year_end_mapped(&tmp, &matrix, &mapping)).unwrap();
            assert_eq!(mapping_codes(&pack), expected, "{case}");
            let matrix_codes: Vec<_> = pack
                .findings
                .iter()
                .filter(|f| f.slot == "tie_out_matrix")
                .map(|f| f.code.as_str())
                .collect();
            assert_eq!(matrix_codes, asserted, "{case}");
            assert_eq!(
                pack.status == "READY",
                expected.is_empty() && asserted.is_empty(),
                "{case}"
            );
            assert!(
                !pack
                    .controls_not_run
                    .unwrap()
                    .iter()
                    .any(|c| c.slot == "fs_mapping"),
                "{case}"
            );
        }
    }
    #[test]
    fn computed_statement_differences_equal_to_the_tolerance_tie() {
        for (cash, ties) in [
            ("120000.009", true),
            ("120000.01", true),
            ("119999.99", true),
            ("120000.011", false),
            ("119999.989", false),
        ] {
            let tmp = TempDir::new();
            let matrix = MATRIX.replace("Cash,120000.00,", &format!("Cash,{cash},"));
            let pack = review("year_end", &year_end_mapped(&tmp, &matrix, MAPPING)).unwrap();
            assert_eq!(mapping_codes(&pack).is_empty(), ties, "{cash}");
        }
    }
    #[test]
    fn a_statement_mapping_refuses_repeats_mixed_sides_and_no_rows() {
        for (mapping, refusal) in [
            ("Cash,100,DEBIT\nCash,100,DEBIT\n", "duplicate mapping"),
            (
                "Loss for the year,300,DEBIT\nLoss for the year,400,CREDIT\n",
                "both DEBIT and CREDIT",
            ),
            ("Cash,100,debit\n", "NormalBalance must be DEBIT or CREDIT"),
            ("", "no statement-mapping rows"),
        ] {
            let tmp = TempDir::new();
            let error = review("year_end", &year_end_mapped(&tmp, MATRIX, mapping)).unwrap_err();
            assert!(error.contains(refusal), "{mapping}: {error}");
        }
    }
    /// bas-ready with account 820's closing `YTDDebit`/`YTDCredit` in the trial
    /// balance set to `gst`, or the account left out; the bank's YTD debit
    /// absorbs the change so both pairs still balance. Optional replacement
    /// GST control and activity statement rows follow their headers.
    fn bas_gst(
        tmp: &TempDir,
        gst: Option<(&str, &str)>,
        control: Option<&str>,
        activity: Option<&str>,
    ) -> PathBuf {
        let dir = tmp.join("pack");
        test_support::copy_example("bas-ready", &dir);
        let path = dir.join("trial_balance.csv");
        let text = fs::read_to_string(&path).unwrap();
        let mut rows: Vec<_> = text.lines().filter(|l| !l.contains(",820,")).collect();
        assert_eq!(rows.len(), text.lines().count() - 1);
        let (debit, credit) = gst.unwrap_or(("0.00", "0.00"));
        let bank = amount("120000.00")
            .add(&amount(credit))
            .value
            .sub(&amount(debit))
            .value
            .to_sci_string();
        let bank = format!(
            "2026-03-31,Cedar and Pine Consulting Pty Ltd,Assets,100,Operating Bank,1000,15000.00,0.00,{bank},0.00"
        );
        let gst_row = format!(
            "2026-03-31,Cedar and Pine Consulting Pty Ltd,Liabilities,820,GST Payable,8200,0.00,0.00,{debit},{credit}"
        );
        rows[1] = &bank;
        if gst.is_some() {
            rows.push(&gst_row);
        }
        fs::write(&path, rows.join("\n") + "\n").unwrap();
        if let Some(control) = control {
            fs::write(
                dir.join("gst_control_gl.csv"),
                format!("Date,AccountID,AccountName,Debit,Credit,Description\n{control}"),
            )
            .unwrap();
        }
        if let Some(activity) = activity {
            fs::write(
                dir.join("activity_statement.csv"),
                format!("Label,Amount\n{activity}"),
            )
            .unwrap();
        }
        dir
    }
    fn gst_breaks(pack: &ReadinessPack) -> Vec<&str> {
        pack.findings
            .iter()
            .filter(|f| f.code == "TIEOUT_BREAK" && f.slot == "gst_control_gl")
            .map(|f| f.reason.as_str())
            .collect()
    }
    #[test]
    fn a_gst_control_account_the_trial_balance_lacks_is_not_ready() {
        let tmp = TempDir::new();
        let pack = review("bas", &bas_gst(&tmp, None, None, None)).unwrap();
        assert_eq!(pack.status, "NOT_READY");
        assert_eq!(
            gst_breaks(&pack),
            [
                "GST control file accounts are not in the trial balance: 820. The control accounts cannot be rolled forward to the ledger."
            ]
        );
    }
    #[test]
    fn the_gst_control_rolls_forward_to_the_trial_balance_within_tolerance() {
        for (closing, ties) in [
            ("7500.00", true),
            ("7500.009", true),
            ("7500.01", true),
            ("7499.99", true),
            ("7500.011", false),
            ("7499.989", false),
            ("9500.00", false),
        ] {
            let tmp = TempDir::new();
            let pack = review("bas", &bas_gst(&tmp, Some(("0.00", closing)), None, None)).unwrap();
            assert_eq!(gst_breaks(&pack).is_empty(), ties, "{closing}");
            assert_eq!(pack.status == "READY", ties, "{closing}");
        }
    }
    #[test]
    fn a_gst_refund_position_rolls_forward_as_a_debit_balance() {
        // Credits of 1,000 less debits of 1,500: a 500 debit balance, refundable.
        let control = "2026-01-15,820,GST Payable,0.00,1000.00,GST on sales
2026-02-20,820,GST Payable,1500.00,0.00,GST on purchases
";
        let activity = "1A,1000.00\n1B,1500.00\n";
        for (gst, ties) in [(("500.00", "0.00"), true), (("0.00", "500.00"), false)] {
            let tmp = TempDir::new();
            let pack = review(
                "bas",
                &bas_gst(&tmp, Some(gst), Some(control), Some(activity)),
            )
            .unwrap();
            assert_eq!(gst_breaks(&pack).is_empty(), ties, "{gst:?}");
        }
    }
    #[test]
    fn a_declared_opening_gst_balance_rolls_forward_to_the_trial_balance() {
        let declared = |opening: &str| format!("1A,10000.00\n1B,2500.00\nGST_OPENING,{opening}\n");
        for (closing, opening) in [
            // An earlier BAS of 500 still unpaid at the period end.
            ("8000.00", "500.00"),
            // Cents the ATO dropped from whole-dollar 1A and 1B, carried in.
            ("7500.73", "0.73"),
            // A refund of 200 still owed: a debit balance carried in.
            ("7300.00", "-200.00"),
        ] {
            let tmp = TempDir::new();
            let pack = review(
                "bas",
                &bas_gst(
                    &tmp,
                    Some(("0.00", closing)),
                    None,
                    Some(&declared(opening)),
                ),
            )
            .unwrap();
            assert_eq!(pack.status, "READY", "{opening}: {:?}", gst_breaks(&pack));
        }
        let tmp = TempDir::new();
        let without = review("bas", &bas_gst(&tmp, Some(("0.00", "8000.00")), None, None)).unwrap();
        assert_eq!(
            gst_breaks(&without),
            [
                "GST control accounts close at 8000.00 in the trial balance, but the GST control file moves them by 7500.00 and, with no GST_OPENING on the activity statement, they are taken to carry in nil; difference 500.00 exceeds tolerance 0.01. The file may be missing postings, or the accounts carried a balance in, such as an unpaid earlier BAS or the cents the ATO drops from 1A and 1B, which GST_OPENING declares."
            ]
        );
        let tmp = TempDir::new();
        let wrong = bas_gst(
            &tmp,
            Some(("0.00", "8000.00")),
            None,
            Some(&declared("400.00")),
        );
        assert_eq!(
            gst_breaks(&review("bas", &wrong).unwrap()),
            [
                "GST control accounts close at 8000.00 in the trial balance, but carry in 400.00 from before the period, the activity statement's GST_OPENING, and the GST control file moves them by 7500.00; difference 100.00 exceeds tolerance 0.01. The file may be missing postings, or GST_OPENING may be wrong."
            ]
        );
    }
    #[test]
    fn a_malformed_declared_opening_gst_balance_is_refused() {
        let declared = |opening: &str| format!("1A,10000.00\n1B,2500.00\nGST_OPENING,{opening}\n");
        for (opening, refusal) in [
            ("abc", "has invalid Amount: 'abc'"),
            ("", "has invalid Amount: ''"),
            (
                "500.00\nGST_OPENING,500.00",
                "duplicate Label 'GST_OPENING'",
            ),
        ] {
            let tmp = TempDir::new();
            let pack = bas_gst(
                &tmp,
                Some(("0.00", "8000.00")),
                None,
                Some(&declared(opening)),
            );
            let error = review("bas", &pack).unwrap_err();
            assert!(error.contains(refusal), "{opening}: {error}");
        }
    }
    #[test]
    fn gst_breaks_carry_a_difference_hint() {
        for (closing, hint) in [
            (
                "10000.00",
                " Hint: the difference is the size of the 2026-02-20 posting 'GST on purchases' (debit 2500.00); check that posting first.",
            ),
            (
                "12500.00",
                " Hint: the difference is twice the 2026-02-20 posting 'GST on purchases' (debit 2500.00), so that posting may be on the wrong side.",
            ),
            (
                "7500.90",
                " Hint: the difference is a whole number of cents divisible by 9, which can mean 2 digits were transposed.",
            ),
            ("7501.00", ""),
            // The purchases posting's size, and twice it, in the other direction:
            // the ledger is 2500 or 5000 lower, which that debit cannot explain.
            ("5000.00", ""),
            ("2500.00", ""),
        ] {
            let tmp = TempDir::new();
            let pack = review("bas", &bas_gst(&tmp, Some(("0.00", closing)), None, None)).unwrap();
            let reasons = gst_breaks(&pack);
            assert_eq!(reasons.len(), 1, "{closing}");
            assert!(
                reasons[0].ends_with(&format!("which GST_OPENING declares.{hint}")),
                "{closing}: {}",
                reasons[0]
            );
        }
        // The 1A less 1B tie-out carries one too: a purchase posted twice.
        let control = "2026-01-15,820,GST Payable,0.00,10000.00,GST on sales
2026-02-20,820,GST Payable,2500.00,0.00,GST on purchases
2026-02-21,820,GST Payable,2500.00,0.00,GST on purchases
";
        let tmp = TempDir::new();
        let pack = review(
            "bas",
            &bas_gst(&tmp, Some(("0.00", "5000.00")), Some(control), None),
        )
        .unwrap();
        assert_eq!(
            gst_breaks(&pack),
            [
                "Net GST on the activity statement (7500.00) does not tie to GST control movement (5000.00); difference 2500.00 exceeds tolerance 0.01 or the 99-cent allowance for whole-dollar 1A and 1B. Hint: the difference is the size of the 2026-02-20 posting 'GST on purchases' (debit 2500.00); check that posting first."
            ]
        );
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
            statement_amount: amount("120000.00"),
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
