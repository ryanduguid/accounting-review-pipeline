use accounting_excel_toolkit::aged::{self, Json, json};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, Command, Stdio},
};
mod aged_cases;
mod aged_cli;

fn native(request: &Value) -> Json {
    let parsed = json::loads(&request.to_string()).unwrap();
    let text = |key| request[key].as_str().unwrap();
    let result = match text("kind") {
        "amount" => {
            if let Some(text) = request["text"].as_str() {
                aged::amount::parse(text).map(|a| Json::str(&a.to_string()))
            } else {
                Err("Amounts must be supplied as decimal text.".into())
            }
        }
        "csv" => aged::records(text("source").as_bytes()).map(|rows| {
            Json::Array(
                rows.iter()
                    .map(|r| Json::Array(r.iter().map(|s| Json::str(s)).collect()))
                    .collect(),
            )
        }),
        "generated" => Ok(Json::Bool(aged::date::generated(text("text")))),
        "draft" => aged::evidence::draft(text("source").as_bytes()),
        "json" => parse_json(text("text"), request["ascii"].as_bool().unwrap()),
        "case" => aged::replay::case(text("name")).map(|(content, manifest, control)| {
            Json::object([
                ("content", Json::str(std::str::from_utf8(&content).unwrap())),
                ("manifest", manifest),
                ("control", control.unwrap_or(Json::Null)),
            ])
        }),
        kind => aged::inspect_export(
            text("source").as_bytes(),
            parsed.at("manifest"),
            parsed.get("control").filter(|v| !matches!(v, Json::Null)),
        )
        .and_then(|result| match kind {
            "requests" => Ok(aged::evidence::requests(&result)),
            "briefing" => Ok(Json::str(&aged::evidence::briefing(&result))),
            "pretty" => {
                if request["ascii"].as_bool().unwrap() {
                    Ok(Json::str(&json::pretty_ordered(&result)))
                } else {
                    json::pretty_utf8(&result).map(|s| Json::str(&s))
                }
            }
            _ => Ok(result),
        }),
    };
    match result {
        Ok(v) => Json::object([("ok", v)]),
        Err(e) => Json::object([("error", Json::str(&e))]),
    }
}

fn parse_json(text: &str, ascii: bool) -> Result<Json, String> {
    json::loads(text)
        .map_err(|e| match e {
            json::LoadError::DuplicateKey(_) => "Duplicate JSON field.".into(),
            _ => "JSON parse error".into(),
        })
        .and_then(|v| {
            if ascii {
                Ok(Json::str(&json::pretty_ordered(&v)))
            } else {
                json::pretty_utf8(&v).map(|s| Json::str(&s))
            }
        })
}

struct Oracle {
    child: Child,
    input: std::process::ChildStdin,
    output: BufReader<std::process::ChildStdout>,
    counts: BTreeMap<String, usize>,
}
impl Oracle {
    fn new(python: &Path, tools: &Path) -> Result<Self, String> {
        let adapter =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/differential/aged_oracle.py");
        let mut child = Command::new(python)
            .args(["-I", "-B"])
            .arg(adapter)
            .arg(tools)
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
            return Err("Frozen aged oracle stopped early".into());
        }
        let expected = json::loads(&line).map_err(|e| format!("Oracle JSON: {e:?}"))?;
        let actual = native(&request);
        if json::dumps(&expected, false) != json::dumps(&actual, false) {
            return Err(format!(
                "Request {request}: Python {}; Rust {}",
                json::dumps(&expected, false),
                json::dumps(&actual, false)
            ));
        }
        *self
            .counts
            .entry(request["kind"].as_str().unwrap().into())
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
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 4 {
        return Err("usage: aged-differential PYTHON FROZEN_TOOLS NATIVE_BIN_DIR SEED".into());
    }
    let seed = args[3]
        .to_str()
        .unwrap()
        .parse::<u64>()
        .map_err(|e| e.to_string())?;
    let mut oracle = Oracle::new(Path::new(&args[0]), Path::new(&args[1]))?;
    aged_cases::run(&mut oracle, seed)?;
    let cli = aged_cli::run(
        Path::new(&args[0]),
        Path::new(&args[1]),
        Path::new(&args[2]),
        seed,
    )?;
    println!(
        "Seed {seed}: {} core cases {:?}; {cli} CLI checks; zero unexplained differences.",
        oracle.counts.values().sum::<usize>(),
        oracle.counts
    );
    Ok(())
}
