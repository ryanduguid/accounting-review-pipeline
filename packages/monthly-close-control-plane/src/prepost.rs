// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Draft journals reviewed before posting: the checks `jet` runs on posted
//! listings, plus duplicates of posted entries and account pairs the ledger
//! has never posted, each with a stated risk score and a suggested fix. It
//! changes nothing; a person decides whether to post.
use crate::{
    Result,
    decimal::{self, Decimal},
    drivers::{self, Transaction},
    duplicates,
    isodate::Date,
    journal_entries::{has_narration, is_round, is_weekend},
    loader::SourceSnapshot,
    pyjson::Json,
    reconciliation,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const BOUNDARY: &str = "Review before posting only. The command posts, approves and changes nothing; a flag is a reason to look, not a finding of error, and a draft with no flag is not shown to be right.";

/// Each flag's code, its weight in the risk score and the fix it suggests, in
/// the order flags are listed.
pub const FLAGS: [(&str, i64, &str); 9] = [
    (
        "UNBALANCED",
        3,
        "Correct the lines so the debits equal the credits; a journal that does not balance cannot be posted.",
    ),
    (
        "OUTSIDE_PERIOD",
        3,
        "Date the entry in the period it belongs to, or post it in that period instead.",
    ),
    (
        "DUPLICATE_OF_POSTED",
        3,
        "Check that the entry is not already posted; discard the draft if it is.",
    ),
    ("DUPLICATE_DRAFT", 3, "Keep one of the drafts."),
    (
        "UNUSUAL_ACCOUNT_PAIR",
        2,
        "Confirm the accounts; a pair the ledger has never posted is often a coding error.",
    ),
    (
        "PERIOD_END",
        1,
        "Keep the evidence that the entry belongs to this period and not the next (cut-off).",
    ),
    (
        "NO_NARRATION",
        1,
        "Add a narration saying what the entry is for and where its evidence is.",
    ),
    (
        "ROUND_AMOUNT",
        1,
        "Attach the calculation or document the amount comes from.",
    ),
    ("WEEKEND", 1, "Confirm the date."),
];
/// The lowest score of each risk band above `none`.
const BANDS: [(i64, &str); 3] = [(6, "high"), (3, "medium"), (1, "low")];

struct Line {
    account: String,
    item: Transaction,
}

type Journals = BTreeMap<String, Vec<Line>>;
/// A journal's lines as sorted (account, signed amount) pairs.
type Signature = Vec<(String, String)>;

/// One tenant's journals, each line keyed by its journal.
fn journals(source: &SourceSnapshot, currency: &str) -> Result<(String, Journals)> {
    let mut selection = drivers::transactions(source, currency)?;
    let tenants: BTreeSet<_> = selection.order.iter().map(|key| key.0.clone()).collect();
    if tenants.len() > 1 {
        return Err(
            "Journal lines must belong to one tenant; review each tenant separately.".into(),
        );
    }
    let mut journals = Journals::new();
    for key in selection.order {
        for item in selection.by_account.remove(&key).unwrap() {
            let account = key.1.clone();
            journals
                .entry(item.identifier.clone())
                .or_default()
                .push(Line { account, item });
        }
    }
    Ok((tenants.into_iter().next().unwrap_or_default(), journals))
}

/// 2 journals with the same signature post the same amounts to the same
/// accounts.
fn signature(lines: &[Line]) -> Signature {
    let mut lines: Signature = lines
        .iter()
        .map(|line| {
            (
                line.account.clone(),
                reconciliation::fixed(&line.item.amount),
            )
        })
        .collect();
    lines.sort();
    lines
}

/// Every (debit account, credit account) pair a journal posts.
fn pairs(lines: &[Line]) -> BTreeSet<(String, String)> {
    let zero = Decimal::zero();
    let (debits, credits): (Vec<&Line>, Vec<&Line>) =
        lines.iter().partition(|line| line.item.amount > zero);
    let mut found = BTreeSet::new();
    for debit in &debits {
        for credit in &credits {
            found.insert((debit.account.clone(), credit.account.clone()));
        }
    }
    found
}

/// A journal's id, first date and first non-blank reference.
type Named = (String, Date, String);

fn named(id: &str, lines: &[Line]) -> Named {
    let date = lines.iter().map(|line| line.item.date).min().unwrap();
    let reference = lines
        .iter()
        .map(|line| {
            drivers::field(&line.item.row, "Reference")
                .trim()
                .to_owned()
        })
        .find(|reference| !reference.is_empty())
        .unwrap_or_default();
    (id.to_owned(), date, reference)
}

/// The journals in `others` that repeat `lines`: the same amounts to the same
/// accounts on the same date or under the same reference. A recurring entry
/// with a new date and reference is not a repeat.
fn repeats(id: &str, lines: &[Line], others: &BTreeMap<Signature, Vec<Named>>) -> Vec<String> {
    let (_, date, reference) = named(id, lines);
    let same = others.get(&signature(lines)).map_or(&[][..], Vec::as_slice);
    same.iter()
        .filter(|(other, day, named)| {
            other != id && (*day == date || (!reference.is_empty() && *named == reference))
        })
        .map(|(other, _, _)| other.clone())
        .collect()
}

/// What a draft is compared with: the posted journals and the drafts by
/// signature, and the account pairs the posted journals post.
struct Context {
    period: (Date, Date),
    posted: BTreeMap<Signature, Vec<Named>>,
    pairs: BTreeSet<(String, String)>,
    drafts: BTreeMap<Signature, Vec<Named>>,
}

impl Context {
    fn new(period: (Date, Date), posted: &Journals, drafts: &Journals) -> Self {
        let mut context = Self {
            period,
            posted: BTreeMap::new(),
            pairs: BTreeSet::new(),
            drafts: BTreeMap::new(),
        };
        for (id, lines) in posted {
            context
                .posted
                .entry(signature(lines))
                .or_default()
                .push(named(id, lines));
            context.pairs.extend(pairs(lines));
        }
        for (id, lines) in drafts {
            context
                .drafts
                .entry(signature(lines))
                .or_default()
                .push(named(id, lines));
        }
        context
    }
}

/// The reason for each flag a draft raises against the ledger history:
/// balance, period, duplicates and account pairs.
fn ledger_reasons(id: &str, lines: &[Line], context: &Context) -> [Option<String>; 5] {
    let (start, end) = context.period;
    let net = lines.iter().fold(Decimal::zero(), |sum, line| {
        sum.add(&line.item.amount).value
    });
    let posted = repeats(id, lines, &context.posted);
    let twins = repeats(id, lines, &context.drafts);
    let unusual: Vec<String> = pairs(lines)
        .into_iter()
        .filter(|pair| !context.pairs.contains(pair))
        .map(|(debit, credit)| format!("debit {debit} and credit {credit}"))
        .collect();
    let outside = lines
        .iter()
        .find(|line| line.item.date < start || line.item.date > end);
    [
        (!net.is_zero())
            .then(|| format!("Debits less credits is {}.", reconciliation::fixed(&net))),
        outside.map(|line| {
            format!(
                "The line to {} is dated {}, outside {} to {}.",
                line.account,
                line.item.date.iso(),
                start.iso(),
                end.iso()
            )
        }),
        (!posted.is_empty()).then(|| {
            format!(
                "Posts the same amounts to the same accounts, on the same date or under the same reference, as posted journal {}.",
                posted.join(", ")
            )
        }),
        (!twins.is_empty()).then(|| {
            format!(
                "Posts the same amounts to the same accounts, on the same date or under the same reference, as draft {}.",
                twins.join(", ")
            )
        }),
        (!unusual.is_empty()).then(|| format!("No posted journal pairs {}.", unusual.join("; "))),
    ]
}

/// The reason for each of the checks `jet` runs that a draft raises.
fn jet_reasons(lines: &[Line], end: Date) -> [Option<String>; 4] {
    let rows: Vec<&Json> = lines.iter().map(|line| &line.item.row).collect();
    [
        lines
            .iter()
            .any(|line| line.item.date == end)
            .then(|| "Dated on the last day of the period.".to_owned()),
        (!has_narration(&rows)).then(|| "No line carries a description or reference.".to_owned()),
        lines
            .iter()
            .find(|line| is_round(&line.item.amount))
            .map(|line| format!("The line to {} is a whole multiple of 1,000.", line.account)),
        lines
            .iter()
            .find(|line| is_weekend(line.item.date))
            .map(|line| format!("Dated {}, a Saturday or Sunday.", line.item.date.iso())),
    ]
}

fn band(score: i64) -> &'static str {
    BANDS
        .iter()
        .find(|(lowest, _)| score >= *lowest)
        .map_or("none", |(_, name)| name)
}

/// One draft's entry and the codes it raised.
fn reviewed(id: &str, lines: &[Line], context: &Context) -> (Json, Vec<&'static str>) {
    let reasons = ledger_reasons(id, lines, context)
        .into_iter()
        .chain(jet_reasons(lines, context.period.1));
    let mut codes = Vec::new();
    let mut flags = Vec::new();
    let mut score = 0;
    for ((code, weight, fix), reason) in FLAGS.iter().zip(reasons) {
        let Some(reason) = reason else { continue };
        codes.push(*code);
        score += weight;
        flags.push(Json::object([
            ("code", Json::str(code)),
            ("reason", Json::str(&reason)),
            ("suggested_fix", Json::str(fix)),
        ]));
    }
    let zero = Decimal::zero();
    let debits = lines
        .iter()
        .filter(|line| line.item.amount > zero)
        .fold(zero.clone(), |sum, line| sum.add(&line.item.amount).value);
    let first = lines.iter().map(|line| line.item.date).min().unwrap();
    let entry = Json::object([
        ("transaction_id", Json::str(id)),
        ("date", Json::str(&first.iso())),
        ("lines", Json::int(lines.len() as i64)),
        ("debits", Json::str(&reconciliation::fixed(&debits))),
        ("risk_score", Json::int(score)),
        ("risk", Json::str(band(score))),
        ("flags", Json::Array(flags)),
    ]);
    (entry, codes)
}

/// Review the draft journals in `drafts` against the posted listing `posted`
/// for the period `start` to `end`.
pub fn prepost_review(
    drafts: &Path,
    posted: &Path,
    currency: &str,
    start: &str,
    end: &str,
) -> Result<Json> {
    let period = duplicates::period_options(currency, start, end)?;
    let drafts = SourceSnapshot::capture(drafts, "Draft journals")?;
    let posted = SourceSnapshot::capture(posted, "Posted journals")?;
    decimal::with_precision(40, || review(&drafts, &posted, currency, period))
}

fn review(
    drafts_source: &SourceSnapshot,
    posted_source: &SourceSnapshot,
    currency: &str,
    period: (Date, Date),
) -> Result<Json> {
    let (tenant, drafts) = journals(drafts_source, currency)?;
    let (posted_tenant, posted) = journals(posted_source, currency)?;
    if drafts.is_empty() {
        return Err("No draft journals were supplied.".into());
    }
    if !posted.is_empty() && posted_tenant != tenant {
        return Err("The posted listing belongs to another tenant than the drafts.".into());
    }
    let context = Context::new(period, &posted, &drafts);
    let mut entries = Vec::new();
    let mut raised = BTreeSet::new();
    for (id, lines) in &drafts {
        let (entry, codes) = reviewed(id, lines, &context);
        entries.push(entry);
        raised.extend(codes);
    }
    let status = if raised.contains("UNBALANCED") {
        "BLOCKED"
    } else if raised.is_empty() {
        "PASS"
    } else {
        "REVIEW"
    };
    let weights = FLAGS
        .iter()
        .map(|(code, weight, _)| {
            Json::object([("code", Json::str(code)), ("weight", Json::int(*weight))])
        })
        .collect();
    let mut result = duplicates::period_result(false, &tenant, currency, period, BOUNDARY);
    for (key, value) in [
        ("status", Json::str(status)),
        (
            "source_sha256",
            Json::object([
                ("drafts", Json::str(&drafts_source.sha256)),
                ("posted", Json::str(&posted_source.sha256)),
            ]),
        ),
        ("draft_count", Json::int(drafts.len() as i64)),
        ("posted_count", Json::int(posted.len() as i64)),
        ("weights", Json::Array(weights)),
        ("journals", Json::Array(entries)),
    ] {
        result.set(key, value);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{band, review};
    use crate::{
        isodate::Date,
        loader::SourceSnapshot,
        pyjson::{self, Json},
    };
    use std::path::Path;

    const HEAD: &str =
        "Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit\n";
    const RENT: &str = "T,610,AUD,J-1,2026-07-06,RENT-07,Rent,2450.75,0\nT,090,AUD,J-1,2026-07-06,RENT-07,Rent,0,2450.75\n";

    fn csv(name: &str, rows: &str) -> SourceSnapshot {
        SourceSnapshot::new(Path::new(name), format!("{HEAD}{rows}").into_bytes())
    }

    fn period() -> (Date, Date) {
        (
            Date::new(2026, 8, 1).unwrap(),
            Date::new(2026, 8, 31).unwrap(),
        )
    }

    fn run(drafts: &str, posted: &str) -> crate::Result<Json> {
        review(
            &csv("d.csv", drafts),
            &csv("p.csv", posted),
            "AUD",
            period(),
        )
    }

    fn text(value: &Json) -> String {
        value.as_pystr().unwrap().to_utf8().unwrap()
    }

    /// Each draft's id and flag codes.
    fn flags(result: &Json) -> Vec<String> {
        let journals = result.at("journals").as_array().unwrap();
        journals
            .iter()
            .map(|entry| {
                let codes: Vec<String> = entry
                    .at("flags")
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|flag| text(flag.at("code")))
                    .collect();
                format!("{} {}", text(entry.at("transaction_id")), codes.join(","))
            })
            .collect()
    }

    #[test]
    fn a_recurring_entry_is_not_a_duplicate_but_a_reused_reference_is() {
        let drafts = "T,610,AUD,D-1,2026-08-06,RENT-08,Rent,2450.75,0\nT,090,AUD,D-1,2026-08-06,RENT-08,Rent,0,2450.75\n\
                      T,610,AUD,D-2,2026-08-14,RENT-07,Rent,2450.75,0\nT,090,AUD,D-2,2026-08-14,RENT-07,Rent,0,2450.75\n";
        let result = run(drafts, RENT).unwrap();
        assert_eq!(flags(&result), ["D-1 ", "D-2 DUPLICATE_OF_POSTED"]);
        assert!(result.at("status").is_str("REVIEW"));
    }

    #[test]
    fn a_clean_draft_passes_with_no_score() {
        let drafts = "T,610,AUD,D-1,2026-08-06,RENT-08,Rent,2450.75,0\nT,090,AUD,D-1,2026-08-06,RENT-08,Rent,0,2450.75\n";
        let result = run(drafts, RENT).unwrap();
        assert!(result.at("status").is_str("PASS"));
        let entry = &result.at("journals").as_array().unwrap()[0];
        assert!(entry.at("risk").is_str("none"));
        assert_eq!(text(entry.at("debits")), "2450.75");
    }

    #[test]
    fn with_no_posted_history_every_pair_is_unusual() {
        let drafts = "T,610,AUD,D-1,2026-08-06,RENT-08,Rent,2450.75,0\nT,090,AUD,D-1,2026-08-06,RENT-08,Rent,0,2450.75\n";
        let result = run(drafts, "").unwrap();
        assert_eq!(flags(&result), ["D-1 UNUSUAL_ACCOUNT_PAIR"]);
        assert_eq!(pyjson::canonical(result.at("posted_count")), "0");
    }

    #[test]
    fn an_unbalanced_draft_outside_the_period_blocks_at_high_risk() {
        let drafts =
            "T,610,AUD,D-1,2026-09-02,X,Rent,100.00,0\nT,090,AUD,D-1,2026-09-02,X,Rent,0,90.00\n";
        let result = run(drafts, RENT).unwrap();
        assert!(result.at("status").is_str("BLOCKED"));
        let entry = &result.at("journals").as_array().unwrap()[0];
        assert_eq!(pyjson::canonical(entry.at("risk_score")), "6");
        assert!(entry.at("risk").is_str("high"));
        let reason = text(entry.at("flags").as_array().unwrap()[0].at("reason"));
        assert_eq!(reason, "Debits less credits is 10.00.");
    }

    #[test]
    fn bands_follow_the_stated_thresholds() {
        let named: Vec<&str> = [0, 1, 2, 3, 5, 6, 9].into_iter().map(band).collect();
        assert_eq!(
            named,
            ["none", "low", "low", "medium", "medium", "high", "high"]
        );
    }

    #[test]
    fn bad_inputs_are_refused() {
        let other = RENT.replace("T,", "U,");
        let one =
            "T,610,AUD,D-1,2026-08-06,R,Rent,1.00,0\nT,090,AUD,D-1,2026-08-06,R,Rent,0,1.00\n";
        let error = run(one, &other).unwrap_err().to_string();
        assert!(error.contains("another tenant"), "{error}");
        let error = run("", RENT).unwrap_err().to_string();
        assert!(error.contains("No draft journals"), "{error}");
        let error = run(&format!("{one}{other}"), RENT).unwrap_err().to_string();
        assert!(error.contains("one tenant"), "{error}");
    }
}
