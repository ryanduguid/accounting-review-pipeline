use super::{Oracle, SAMPLE};
use accounting_excel_toolkit::transactions::{self, text::DECIMAL_ZEROS};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn amounts(oracle: &mut Oracle, state: &mut u64) -> Result<(), String> {
    for text in [
        "",
        "0",
        "00.00",
        "1.",
        ".1",
        "-1",
        "+1",
        "1e0",
        "NaN",
        "Infinity",
        "1,234.56",
        "12,34.56",
        "1,234,567,890,123.12",
        "999999999999999.99",
        "1000000000000000",
        "1.001",
        " 1",
        "1\n",
        "1_000",
        "\u{b2}",
    ] {
        oracle.compare(json!({"kind": "amount", "text": text}))?;
    }
    for &zero in DECIMAL_ZEROS {
        for digit in 0..10 {
            let ch = char::from_u32(zero + digit).unwrap();
            for text in [
                ch.to_string(),
                format!("{ch}{ch}.{ch}{ch}"),
                format!("{ch},{ch}{ch}{ch}.{ch}"),
            ] {
                oracle.compare(json!({"kind": "amount", "text": text}))?;
            }
        }
    }
    for _ in 0..5000 {
        let whole = next(state) % 1_000_000_000_000_001;
        let fraction = next(state) % 10000;
        let text = match next(state) % 5 {
            0 => whole.to_string(),
            1 => format!("{whole}.{:02}", fraction % 100),
            2 => format!("{whole}.{fraction:04}"),
            3 => format!("{whole},123"),
            _ => format!("{whole}.{:01}", fraction % 10),
        };
        oracle.compare(json!({"kind": "amount", "text": text}))?;
    }
    Ok(())
}

fn dates(oracle: &mut Oracle) -> Result<(), String> {
    let months = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    for year in [0, 1, 4, 99, 100, 400, 1900, 2000, 2024, 2026, 9999, 10000] {
        for month in 0..=13 {
            for day in [0, 1, 9, 28, 29, 30, 31, 32] {
                oracle.compare(json!({"kind":"date", "text":format!("{day:02}/{month:02}/{year:04}"), "full":false}))?;
                if (1..=12).contains(&month) {
                    let name = months[month - 1];
                    oracle.compare(json!({"kind":"date", "text":format!("{day} {name} {year:04}"), "full":true}))?;
                    oracle.compare(json!({"kind":"date", "text":format!("{day} {} {year:04}", &name[..3]), "full":false}))?;
                }
            }
        }
    }
    for text in [
        " 1 Sep 2026",
        " 01 Sep 2026",
        "1 Sep 2026 ",
        "1\tSeP\u{a0}2026",
        "1\u{1c}Sep 2026",
        "1\u{200b}Sep 2026",
        "1 Sep 0001",
        "1 Sep 02026",
        "\u{661}/09/2026",
        "0\u{661}/09/2026",
        "\u{660}\u{661}/09/2026",
        "01/\u{660}\u{669}/2026",
        "01/09/\u{662}\u{660}\u{662}\u{666}",
        "1 Apr\u{130}l 2026",
        "1 \u{17f}ep 2026",
    ] {
        for full in [true, false] {
            oracle.compare(json!({"kind":"date", "text":text, "full":full}))?;
        }
    }
    Ok(())
}

fn csv(oracle: &mut Oracle, state: &mut u64) -> Result<(), String> {
    for text in [
        "",
        "\n",
        ",",
        "a\rb",
        "a\r\n",
        "\"a\r\nb\",c\n",
        "\"a\"x,b",
        "\"unterminated",
        "\0,x",
        "\u{feff}a,b\n",
    ] {
        oracle.compare(json!({"kind":"csv", "source":text}))?;
    }
    for length in [131071, 131072, 131073] {
        for (left, right) in [("", ""), ("\"", "\"")] {
            oracle.compare(
                json!({"kind":"csv", "source":format!("{left}{}{right}", "a".repeat(length))}),
            )?;
        }
    }
    let alphabet = ['a', 'b', ',', '"', '\r', '\n', '\0', '\t', '\u{e9}'];
    for _ in 0..3000 {
        let length = next(state) % 101;
        let text: String = (0..length)
            .map(|_| alphabet[next(state) as usize % alphabet.len()])
            .collect();
        oracle.compare(json!({"kind":"csv", "source":text}))?;
    }
    Ok(())
}

pub fn render(rows: &[Vec<String>]) -> String {
    rows.iter()
        .map(|row| {
            row.iter()
                .map(|field| {
                    if field.contains([',', '"', '\r', '\n']) {
                        format!("\"{}\"", field.replace('"', "\"\""))
                    } else {
                        field.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
                + "\n"
        })
        .collect()
}

fn request(source: &str, kind: &str) -> Value {
    json!({"kind":kind, "source":source, "tenant":"Fabricated Services", "account_name":"Clearing", "account_id":"090", "currency":"AUD"})
}

pub fn mapping(source: &str) -> Value {
    json!({"source_sha256":Sha256::digest(source.as_bytes()).iter().map(|b| format!("{b:02x}")).collect::<String>(), "lines":{"9":"source-line-1", "10":"source-line-2"}})
}

fn reports(oracle: &mut Oracle, state: &mut u64) -> Result<(), String> {
    let rows = transactions::records(SAMPLE.as_bytes())?;
    for source in [
        SAMPLE.to_owned(),
        format!("\u{feff}{SAMPLE}"),
        SAMPLE.replace('\n', "\r\n"),
        SAMPLE
            .replace("01/09/2026", "1 Sep 2026")
            .replace("03/09/2026", "3 Sep 2026"),
    ] {
        let mut request = request(&source, "convert");
        request["mapping"] = mapping(&source);
        oracle.compare(request)?;
        oracle.compare(self::request(&source, "inspect"))?;
    }
    let candidates = [
        "",
        "0",
        "0.00",
        "1",
        "110.00",
        "100.00",
        "999999999999999.99",
        "1,000.00",
        "-1",
        "1.001",
        "\u{661}\u{662}.00",
        "01/10/2026",
        "29/02/2024",
        "3 Sep 2026",
        "\"quoted\",line\r\nnext",
        "\u{1c}=SUM(1)",
        "\u{1f}@x",
        "\u{e9}\u{1f600}",
    ];
    for _ in 0..2000 {
        let mut changed = rows.clone();
        let row = 1 + next(state) as usize % (changed.len() - 1);
        if !changed[row].is_empty() {
            let col = next(state) as usize % changed[row].len();
            changed[row][col] = candidates[next(state) as usize % candidates.len()].to_owned();
        }
        let source = render(&changed);
        oracle.compare(request(&source, "inspect"))?;
        let mut conversion = request(&source, "convert");
        conversion["mapping"] = mapping(&source);
        oracle.compare(conversion)?;
    }
    invalid_mappings(oracle)
}

fn invalid_mappings(oracle: &mut Oracle) -> Result<(), String> {
    for bad in [
        Value::Null,
        json!({}),
        json!({"lines":{}}),
        json!({"source_sha256":"wrong","lines":{}}),
        json!({"source_sha256":mapping(SAMPLE)["source_sha256"],"lines":{"9":"same","10":"same"}}),
    ] {
        let mut conversion = request(SAMPLE, "convert");
        conversion["mapping"] = bad;
        oracle.compare(conversion)?;
    }
    Ok(())
}

pub fn compare(oracle: &mut Oracle, seed: u64) -> Result<(), String> {
    let mut state = seed.max(1);
    amounts(oracle, &mut state)?;
    dates(oracle)?;
    csv(oracle, &mut state)?;
    reports(oracle, &mut state)
}
