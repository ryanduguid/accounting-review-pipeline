use accounting_excel_toolkit::transactions;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, Command, Stdio},
};

mod transactions_cases;
mod transactions_cli;

const SAMPLE: &str = include_str!("../../samples/xero-account-transactions.csv");
const AMOUNT_ERROR: &str = "Expected an explicit non-negative amount with at most 2 places: ";

fn outcome(result: Result<Value, String>) -> Value {
    match result {
        Ok(value) => json!({"ok": value}),
        Err(error) => {
            if let Some(value) = error.strip_prefix(AMOUNT_ERROR) {
                json!({"amount_error": serde_json::from_str::<Value>(value).expect("quoted invalid amount")})
            } else {
                json!({"error": error})
            }
        }
    }
}

fn native(request: &Value) -> Value {
    let text = |key| request[key].as_str().expect("request string");
    outcome(match text("kind") {
        "amount" => transactions::amount(text("text")).map(|value| json!(value.to_string())),
        "date" => Ok(
            transactions::parse_date(text("text"), request["full"].as_bool().unwrap())
                .map_or(Value::Null, |date| json!(date.iso())),
        ),
        "csv" => transactions::records(text("source").as_bytes()).map(|rows| json!(rows)),
        "inspect" => transactions::inspect_export(
            text("source").as_bytes(),
            text("tenant"),
            text("account_name"),
        ),
        "convert" => transactions::convert(
            text("source").as_bytes(),
            &request["mapping"],
            text("tenant"),
            text("account_name"),
            text("account_id"),
            text("currency"),
        )
        .map(|rows| json!(rows)),
        _ => panic!("unknown comparison"),
    })
}

struct Oracle {
    child: Child,
    input: std::process::ChildStdin,
    output: BufReader<std::process::ChildStdout>,
    counts: BTreeMap<String, usize>,
}

impl Oracle {
    fn new(python: &Path, script: &Path) -> Result<Self, String> {
        let adapter =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/differential/transactions_oracle.py");
        let mut child = Command::new(python)
            .args(["-I", "-B"])
            .arg(adapter)
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        Ok(Self {
            child,
            input,
            output,
            counts: BTreeMap::new(),
        })
    }

    fn compare(&mut self, request: Value) -> Result<(), String> {
        writeln!(self.input, "{request}")
            .and_then(|()| self.input.flush())
            .map_err(|e| e.to_string())?;
        let mut line = String::new();
        if self
            .output
            .read_line(&mut line)
            .map_err(|e| e.to_string())?
            == 0
        {
            return Err("frozen oracle stopped early".into());
        }
        let expected: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        let actual = native(&request);
        if expected != actual {
            return Err(format!(
                "Request {request}: Python {expected}; Rust {actual}"
            ));
        }
        *self
            .counts
            .entry(request["kind"].as_str().unwrap().to_owned())
            .or_default() += 1;
        Ok(())
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn main() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.len() != 4 {
        return Err(
            "usage: transactions-differential PYTHON FROZEN_SCRIPT NATIVE_BINARY SEED".into(),
        );
    }
    let python = Path::new(&arguments[0]);
    let script = Path::new(&arguments[1])
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let binary = Path::new(&arguments[2])
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let seed = arguments[3]
        .to_str()
        .ok_or("Unicode seed required")?
        .parse::<u64>()
        .map_err(|e| e.to_string())?;
    let mut oracle = Oracle::new(python, &script)?;
    transactions_cases::compare(&mut oracle, seed)?;
    let cli = transactions_cli::compare(python, &script, &binary, seed)?;
    let total: usize = oracle.counts.values().sum();
    println!(
        "Seed {seed}: {total} core cases {:?}; {cli} CLI checks; zero unexplained differences.",
        oracle.counts
    );
    Ok(())
}
