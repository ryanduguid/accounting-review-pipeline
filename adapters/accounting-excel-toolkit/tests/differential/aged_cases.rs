use super::Oracle;
use accounting_excel_toolkit::{aged, transactions};
use serde_json::{Value, json};
const SOURCE: &str = include_str!("../../samples/sample-aged-receivables-review.csv");
const MANIFEST: &str = include_str!("../../samples/sample-aged-receivables-manifest.json");
const CONTROL: &str = include_str!("../../samples/sample-aged-receivables-control.json");
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}
pub fn review(source: &str) -> Value {
    let mut manifest: Value = serde_json::from_str(MANIFEST).unwrap();
    manifest["source_sha256"] = json!(aged::hash(source.as_bytes()));
    json!({"kind":"inspect","source":source,"manifest":manifest,"control":serde_json::from_str::<Value>(CONTROL).unwrap()})
}
fn directed_amounts(oracle: &mut Oracle) -> Result<(), String> {
    for text in [
        "",
        " ",
        "-0",
        "-0.0000",
        "+000.0000",
        "999999999999999.9999",
        "-999,999,999,999,999.9998",
        "1,00",
        "1.00001",
        "NaN",
        "inf",
        "1e3",
        "(25)",
        "--1",
        "1.",
        ".1",
        "\u{1c}-1.50\u{1f}",
    ] {
        oracle.compare(json!({"kind":"amount","text":text}))?;
    }
    for value in [json!(null), json!(true), json!(1), json!([]), json!({})] {
        oracle.compare(json!({"kind":"amount","text":value}))?;
    }
    for &zero in transactions::text::DECIMAL_ZEROS {
        let digit = |d| char::from_u32(zero + d).unwrap();
        for text in [
            format!(
                "-{}.{}{}{}{}",
                digit(0),
                digit(0),
                digit(0),
                digit(0),
                digit(0)
            ),
            format!(
                "{}{}{},{}{}{}.{}",
                digit(1),
                digit(2),
                digit(3),
                digit(4),
                digit(5),
                digit(6),
                digit(7)
            ),
        ] {
            oracle.compare(json!({"kind":"amount","text":text}))?;
        }
    }
    Ok(())
}
fn amounts(oracle: &mut Oracle, rng: &mut Rng) -> Result<(), String> {
    directed_amounts(oracle)?;
    for _ in 0..8000 {
        let whole = rng.next() % 1_000_000_000_000_000;
        let scale = (rng.next() % 5) as usize;
        let fraction = if scale == 0 {
            String::new()
        } else {
            format!(
                ".{:0>width$}",
                rng.next() % 10u64.pow(scale as u32),
                width = scale
            )
        };
        let sign = match rng.next() % 3 {
            0 => "-",
            1 => "+",
            _ => "",
        };
        oracle.compare(json!({"kind":"amount","text":format!("{sign}{whole}{fraction}")}))?;
    }
    Ok(())
}
fn dates(oracle: &mut Oracle, rng: &mut Rng) -> Result<(), String> {
    for day in [
        "2026-10-01",
        "20261001",
        "2026-W40-4",
        "2026W404",
        "2026-W40",
        "2026W40",
        "9999-12-31",
        "2026-02-29",
        "0001-01-01",
    ] {
        for time in [
            "12Z",
            "12:34Z",
            "123456+10",
            "12:34:56.123456789-00:30:12.1",
            "24Z",
            "24:00:00.0000001Z",
            "12:34",
            "12+24:00",
            "12+00:61",
            "12.5Z",
            "12:34.5Z",
            "12:34:56,Z",
            "12:34:56.0+00:00:00.1",
        ] {
            oracle.compare(json!({"kind":"generated","text":format!("{day}T{time}")}))?;
        }
    }
    for _ in 0..8000 {
        let y = rng.next() % 10010;
        let m = rng.next() % 15;
        let d = rng.next() % 34;
        let hour = rng.next() % 27;
        let min = rng.next() % 64;
        let sec = rng.next() % 64;
        let zone = rng.next() % 26;
        oracle.compare(json!({"kind":"generated","text":format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}+{zone:02}:00")}))?;
    }
    Ok(())
}
fn csv(oracle: &mut Oracle, rng: &mut Rng) -> Result<(), String> {
    for source in [
        "",
        "\r",
        "\n",
        "\r\n",
        "a,b\r\rc,d\n",
        "\"a\"x,b",
        "\"unfinished",
        "\"\"",
        "\"a\r\nb\",c\r",
        "a,\0b",
    ] {
        oracle.compare(json!({"kind":"csv","source":source}))?;
    }
    for length in [131071, 131072, 131073] {
        oracle.compare(json!({"kind":"csv","source":"x".repeat(length)}))?;
    }
    let alphabet = ['a', ',', '"', '\n', '\r', ' ', '\0', '\u{e9}'];
    for _ in 0..3000 {
        let len = rng.next() % 80;
        let source: String = (0..len)
            .map(|_| alphabet[(rng.next() % alphabet.len() as u64) as usize])
            .collect();
        oracle.compare(json!({"kind":"csv","source":source}))?;
    }
    Ok(())
}
fn reports(oracle: &mut Oracle, rng: &mut Rng) -> Result<(), String> {
    let mutations = [
        ("Synthetic Entity A", ""),
        (
            "Synthetic Entity A",
            "Synthetic Entity A\nSynthetic Entity A",
        ),
        ("Aged Receivables Summary", "Aged Payables Summary"),
        ("As at 30 September 2026", "As at 31 August 2026"),
        ("00123,100,,0,0,0,0,100", "Section,,,,,,,"),
        ("Total,100,-25,10,0,0,0,85", "Total,90,-25,10,0,0,10,85"),
        (
            "Ageing by due date",
            "Ageing by due date\nAgeing by due date",
        ),
        ("00123,100,,0,0,0,0,100", "\"multi\nline\",100,,0,0,0,0,100"),
    ];
    for source in
        std::iter::once(SOURCE.to_owned()).chain(mutations.map(|(a, b)| SOURCE.replace(a, b)))
    {
        let mut request = review(&source);
        for kind in ["inspect", "draft", "requests", "briefing", "pretty"] {
            request["kind"] = json!(kind);
            request["ascii"] = json!(true);
            oracle.compare(request.clone())?;
        }
    }
    metadata_mutations(oracle, rng)?;
    row_mutations(oracle, rng)?;
    oracle.compare(
        json!({"kind":"draft","source":SOURCE.lines().skip(5).collect::<Vec<_>>().join("\n")}),
    )?;
    for &(name, _, _) in aged::replay::CASES {
        oracle.compare(json!({"kind":"case","name":name}))?;
    }
    json_cases(oracle)
}
fn metadata_mutations(oracle: &mut Oracle, rng: &mut Rng) -> Result<(), String> {
    let keys = [
        "report_name",
        "entity",
        "generated_at",
        "as_at",
        "ageing_basis",
        "accounting_basis",
        "currency",
        "filters",
        "population",
        "source_format",
        "source_sha256",
        "footer_record",
        "on_screen_rows",
        "on_screen_total",
    ];
    let values = [
        json!(null),
        json!(true),
        json!(false),
        json!(0),
        json!(3),
        json!(10),
        json!(10.0),
        json!(-1),
        json!(999),
        json!(""),
        json!(" \u{1c}"),
        json!("changed"),
        json!([]),
        json!({}),
    ];
    for _ in 0..2500 {
        let mut request = review(SOURCE);
        let key = keys[(rng.next() % keys.len() as u64) as usize];
        let value = values[(rng.next() % values.len() as u64) as usize].clone();
        if rng.next().is_multiple_of(5) {
            request["manifest"].as_object_mut().unwrap().remove(key);
        } else {
            request["manifest"][key] = value;
        }
        if rng.next().is_multiple_of(5) {
            request["control"] = Value::Null;
        }
        oracle.compare(request.clone())?;
        request["kind"] = json!("briefing");
        oracle.compare(request)?;
    }
    Ok(())
}
fn json_cases(oracle: &mut Oracle) -> Result<(), String> {
    for text in [
        r#"{"x":1,"x":2}"#,
        r#"{"x":{"a":1,"a":2}}"#,
        r#"{"x":NaN,"y":Infinity,"n":-Infinity}"#,
        r#"{"z":-0.0,"x":1.000e-09,"y":90071992547409933333333333}"#,
        r#"{"s":"\ud800","u":"\ud83d\ude00\u007f"}"#,
        "[1,]",
        "{}",
    ] {
        for ascii in [true, false] {
            oracle.compare(json!({"kind":"json","text":text,"ascii":ascii}))?;
        }
    }
    Ok(())
}
fn row_mutations(oracle: &mut Oracle, rng: &mut Rng) -> Result<(), String> {
    let amounts = [
        "",
        "0",
        "-0.0000",
        "+0.01",
        "-25",
        "0.0049",
        "0.0050",
        "0.0051",
        "999999999999999.9999",
        "-999999999999999.9999",
        "1.2345",
        "1.00001",
        "NaN",
        "１２.５",
        "1,000",
    ];
    for _ in 0..1000 {
        let mut rows = aged::records(SOURCE.as_bytes())?;
        for row in &mut rows[6..] {
            for cell in &mut row[1..] {
                *cell = amounts[(rng.next() % amounts.len() as u64) as usize].replace(',', "");
            }
        }
        let remove_current = rng.next().is_multiple_of(2);
        let rotate = (rng.next() % 6) as usize;
        for row in &mut rows[5..] {
            if remove_current {
                row.remove(1);
            }
            row[1..].rotate_left(rotate);
        }
        let source = rows
            .iter()
            .map(|r| r.join(","))
            .collect::<Vec<_>>()
            .join("\r\n");
        let mut request = review(&source);
        for kind in ["inspect", "requests", "briefing", "pretty"] {
            request["kind"] = json!(kind);
            request["ascii"] = json!(true);
            oracle.compare(request.clone())?;
        }
    }
    Ok(())
}
pub fn run(oracle: &mut Oracle, seed: u64) -> Result<(), String> {
    let mut rng = Rng(seed.max(1));
    amounts(oracle, &mut rng)?;
    dates(oracle, &mut rng)?;
    csv(oracle, &mut rng)?;
    reports(oracle, &mut rng)
}
