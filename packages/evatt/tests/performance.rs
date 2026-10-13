//! The original timing constructions, asserting bounded growth rather than
//! absolute times. Warm compiled regexes before measuring three-run minima.
use evatt::{patterns, redact};
use std::time::{Duration, Instant};

fn best(mut run: impl FnMut()) -> Duration {
    run();
    (0..3)
        .map(|_| {
            let start = Instant::now();
            run();
            start.elapsed()
        })
        .min()
        .unwrap()
}
fn growth(mut build: impl FnMut(usize) -> Box<dyn FnMut()>, small: usize, large: usize) {
    // Debug keeps the construction and 4x ratio at one tenth the size;
    // release runs the original sizes. VM instrumentation in debug otherwise
    // spends most of a CI job revisiting millions of bounded candidates.
    let divisor = if cfg!(debug_assertions) { 10 } else { 1 };
    let (small, large) = (small / divisor, large / divisor);
    let a = best(build(small));
    let b = best(build(large));
    assert!(b < a * 8 + Duration::from_millis(10), "small {a:?}; large {b:?}");
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
            |n| {
                let text = format!("{label}{}", " ".repeat(n));
                Box::new(move || assert!(!patterns::has(name, &text).unwrap()))
            },
            20_000,
            80_000,
        );
        for padding in ["", " \t"] {
            growth(
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
fn rejected_name_retries_keep_bounded_growth() {
    for unit in ["Tax Court ", "Tax Zzz ", "Income Tax\nJohn Smith\n"] {
        growth(
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
    for unit in [
        "TFN: 1234567\n82. next\n".into(),
        "2123\n456701) next\n".into(),
        "ABN: 51 824\n753556. next\n".into(),
        "TFN: 1234567\n82. 3 next\n".into(),
        "TFN: 1234567\n8200. next\n".into(),
        format!("TFN: 1234567\n{}. next\n", "8".repeat(1000)),
        format!("TFN: 1234567\n8200.{}next\n", " ".repeat(1000)),
        format!("TFN: {}1234567\n82. next\n", " ".repeat(100)),
    ] {
        growth(
            |n| {
                let text = unit.repeat(n);
                Box::new(move || assert!(patterns::structured_spans(&text).unwrap().is_empty()))
            },
            500,
            2000,
        );
        growth(
            |n| {
                let text = unit.repeat(n);
                Box::new(move || assert!(!redact::residual(&text).unwrap().is_empty()))
            },
            500,
            2000,
        );
    }
}
