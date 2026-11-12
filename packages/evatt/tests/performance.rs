// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! The original timing constructions, asserting bounded growth rather than
//! absolute times. Warm compiled regexes, then time the 2 sizes in
//! alternating rounds, so load during a check reaches both sizes.
use evatt::{patterns, redact};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Every CPU-heavy test here runs inside `growth`, so holding this lock there
/// stops parallel test threads loading one another's timed rounds.
static TIMING: Mutex<()> = Mutex::new(());

fn time(run: &mut dyn FnMut()) -> Duration {
    let start = Instant::now();
    run();
    start.elapsed()
}
/// Keeps each size's minimum and passes once, after at least 3 rounds, the
/// large minimum is under 8 times the small one plus 10 ms. Linear work grows
/// 4 times and quadratic 16, so quadratic growth fails unless the small run
/// takes under 1.25 ms. Later rounds only filter out noise.
fn bounded(rounds: impl IntoIterator<Item = (Duration, Duration)>) -> Result<(), (Duration, Duration)> {
    let (mut a, mut b) = (Duration::MAX, Duration::MAX);
    for (round, (small, large)) in rounds.into_iter().enumerate() {
        (a, b) = (a.min(small), b.min(large));
        if round >= 2 && b < a * 8 + Duration::from_millis(10) {
            return Ok(());
        }
    }
    Err((a, b))
}
fn growth(case: &str, mut build: impl FnMut(usize) -> Box<dyn FnMut()>, small: usize, large: usize) {
    // Debug keeps the construction and 4x ratio at one tenth the size;
    // release runs the original sizes. VM instrumentation in debug otherwise
    // spends most of a CI job revisiting millions of bounded candidates.
    let divisor = if cfg!(debug_assertions) { 10 } else { 1 };
    let (n, m) = (small / divisor, large / divisor);
    let (mut run_small, mut run_large) = (build(n), build(m));
    let _timing = TIMING.lock().unwrap_or_else(PoisonError::into_inner);
    run_small();
    run_large();
    if let Err((a, b)) = bounded((0..12).map(|_| (time(&mut *run_small), time(&mut *run_large)))) {
        panic!("{case}: size {n}: {a:?}; size {m}: {b:?}");
    }
}

/// A Windows CI run timed all 3 large samples at 41.9 ms or more against a
/// 2.8 ms small one; clean rounds after such samples must still pass.
#[test]
fn growth_check_outlasts_slow_rounds_but_not_super_linear_growth() {
    let ms = Duration::from_millis;
    let slow = (ms(3), ms(42));
    assert!(bounded([slow, slow, slow, (ms(3), ms(12))]).is_ok());
    assert!(bounded([slow; 3]).is_err());
    assert!(bounded([(ms(3), ms(12)); 2]).is_err());
    assert!(bounded([(ms(3), ms(12)), (ms(3), ms(12)), slow]).is_ok());
    assert!(bounded([slow, slow, (ms(30), ms(42))]).is_err());
    assert!(bounded([(ms(10), ms(160)); 12]).is_err());
}

#[test]
fn labelled_whitespace_and_markup_keep_bounded_growth() {
    for (label, name) in [
        ("TFN", "TFN_LABELLED"),
        ("ABN", "ABN_LABELLED"),
        ("ACN", "ACN_LABELLED"),
        ("Medicare", "MEDICARE_LABELLED"),
    ] {
        growth(
            &format!("{name} spaces"),
            |n| {
                let text = format!("{label}{}", " ".repeat(n));
                Box::new(move || assert!(!patterns::has(name, &text).unwrap()))
            },
            20_000,
            80_000,
        );
        for padding in ["", " \t"] {
            growth(
                &format!("{name} markup {padding:?}"),
                |n| {
                    let text = format!(
                        "{label}{}{}:{}x",
                        format!("*_{padding}`").repeat(n),
                        " ".repeat(n),
                        format!("`{padding}_*").repeat(n)
                    );
                    Box::new(move || assert!(!patterns::has(name, &text).unwrap()))
                },
                5_000,
                20_000,
            );
        }
    }
}

#[test]
fn new_label_separator_families_keep_bounded_growth() {
    for unit in [
        "tax file no. (TFN): is ",
        "Australian business number (ABN): of ",
        "Australian company number (ACN): was ",
        "Medicare: is ",
        "BSB: was ",
    ] {
        for run in [" ", "\t", "-", ".,", "\u{2013}", "*_` "] {
            growth(
                &format!("{unit:?} then {run:?}"),
                |n| {
                    let text = format!("{unit}{}x", run.repeat(n));
                    Box::new(move || assert!(patterns::structured_spans(&text).unwrap().is_empty()))
                },
                2_000,
                8_000,
            );
        }
    }
}

#[test]
fn split_identifier_chains_keep_bounded_growth() {
    // One unbroken chain, and many short ones, of groups no token joins.
    for unit in ["123   ", "12 | ", "1,   ", "123<br>", "123&nbsp;"] {
        growth(
            &format!("{unit:?}"),
            |n| {
                let text = unit.repeat(n);
                Box::new(move || {
                    redact::numbers(&text).unwrap();
                })
            },
            2_000,
            8_000,
        );
    }
}

#[test]
fn unterminated_tags_keep_bounded_growth() {
    for unit in ["<a ", "</", "<a 1 "] {
        growth(
            &format!("{unit:?}"),
            |n| {
                let text = unit.repeat(n);
                Box::new(move || {
                    redact::numbers(&text).unwrap();
                })
            },
            2_000,
            8_000,
        );
    }
}

/// Every unterminated tag on a line once searched the rest of it.
#[cfg(feature = "probe")]
#[test]
fn unterminated_tags_are_searched_once_a_line() {
    for unit in ["<a ", "</", "<a 1 "] {
        let examined = |n: usize| redact::numbers_counted(&unit.repeat(n)).unwrap().1.characters_examined;
        assert!(examined(4_000) <= examined(1_000) * 5, "{unit:?}");
    }
}

/// One start per token, and a start for each 9, 10 and 11 digit window.
#[cfg(feature = "probe")]
#[test]
fn a_split_chain_counts_each_window_once() {
    let stats = redact::numbers_counted(&"123   ".repeat(100)).unwrap().1;
    assert_eq!(stats.candidate_starts, 100 + 292 + 291 + 290);
}

#[test]
fn rejected_name_retries_keep_bounded_growth() {
    for unit in ["Tax Court ", "Tax Zzz ", "Income Tax\nJohn Smith\n"] {
        growth(
            &format!("{unit:?}"),
            |n| {
                let text = unit.repeat(n);
                Box::new(move || {
                    patterns::person_names(&text).unwrap();
                })
            },
            2_000,
            8_000,
        );
    }
}

#[test]
fn atomic_joiners_keep_bounded_growth() {
    for (value, prefix, unit) in [
        ("A * B", "A", "*"),
        ("A B", "A ", "<br\n>"),
        ("A <br B", "A ", "<br "),
        ("A B", "A", "\n> "),
        ("A B", "A", "\u{200B}_*"),
    ] {
        let regex = patterns::value_pattern(value).unwrap();
        growth(
            &format!("{value:?} joined by {unit:?}"),
            |n| {
                let text = format!("{prefix}{}X", unit.repeat(n));
                let re = regex.clone();
                Box::new(move || assert!(!re.is_match(&text).unwrap()))
            },
            5_000,
            20_000,
        );
    }
}

#[test]
fn number_and_ambiguity_sweeps_keep_bounded_growth() {
    for unit in ["9", "9-", "9.,", "9\t ", "-", " "] {
        growth(
            &format!("residual sweep {unit:?}"),
            |n| {
                let text = unit.repeat(n);
                Box::new(move || {
                    let found = redact::residual(&text).unwrap();
                    assert_eq!(found.len(), usize::from(unit.starts_with('9')));
                })
            },
            5_000,
            20_000,
        );
    }
    for (i, unit) in [
        "TFN: 1234567\n82. next\n".into(),
        "2123\n456701) next\n".into(),
        "ABN: 51 824\n753556. next\n".into(),
        "TFN: 1234567\n82. 3 next\n".into(),
        "TFN: 1234567\n8200. next\n".into(),
        format!("TFN: 1234567\n{}. next\n", "8".repeat(1000)),
        format!("TFN: 1234567\n8200.{}next\n", " ".repeat(1000)),
        format!("TFN: {}1234567\n82. next\n", " ".repeat(100)),
    ]
    .into_iter()
    .enumerate()
    {
        growth(
            &format!("ambiguity case {i} spans"),
            |n| {
                let text = unit.repeat(n);
                Box::new(move || assert!(patterns::structured_spans(&text).unwrap().is_empty()))
            },
            500,
            2000,
        );
        growth(
            &format!("ambiguity case {i} residual"),
            |n| {
                let text = unit.repeat(n);
                Box::new(move || assert!(!redact::residual(&text).unwrap().is_empty()))
            },
            500,
            2000,
        );
    }
}
