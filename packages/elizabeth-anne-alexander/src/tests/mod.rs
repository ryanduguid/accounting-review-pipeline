//! Rust ports of the Python package's test suite. Each test keeps the Python
//! test's name; a comment marks the few whose subject no longer exists.

mod cli_tests;
mod docs;
mod gateway_tests;
mod source_gates;
mod util;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::cli;
use crate::gateway::{self, Evaluation};
use crate::pyjson::{self, Json, PyStr};
use crate::util::{Bundle, Env, Failure};

pub const CONTEXT: &str = "samples/contexts/sample-monthly-variance.context.json";
pub const REQUEST: &str = "samples/requests/sample-revenue-variance.request.json";
pub const POLICY: &str = "policy/demo-policy-v1.json";
pub const DECISION: &str = "samples/decisions/sample-review-decision.json";
pub const CURRENT_CSV: &str = "samples/inputs/sample-tb-2026-06-30.csv";
pub const CURRENT_MANIFEST: &str = "samples/manifests/sample-tb-2026-06-30.manifest.json";
pub const PRIOR_MANIFEST: &str = "samples/manifests/sample-tb-2026-05-31.manifest.json";
pub const EVALUATE: [&str; 7] = [
    "evaluate",
    "--context",
    CONTEXT,
    "--request",
    REQUEST,
    "--policy",
    POLICY,
];

/// A private directory removed on drop, resolved so paths compare as the gateway sees them.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> TempDir {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "eaa-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(crate::pypath::resolve(&path, true).unwrap())
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An environment whose working directory is `cwd` and whose bundled data is the shipped data.
pub fn env(cwd: &Path) -> Env {
    Env {
        bundle: Bundle::shipped(),
        cwd: cwd.to_path_buf(),
    }
}

pub fn shipped(rel: &str) -> String {
    String::from_utf8(Bundle::shipped().files[rel].clone()).unwrap()
}

pub fn parse(text: &str) -> Json {
    pyjson::loads(text).unwrap()
}

pub fn shipped_json(rel: &str) -> Json {
    parse(&shipped(rel))
}

/// `json.dumps(payload, indent=2) + "\n"`, enough for test inputs.
pub fn dumps(value: &Json) -> String {
    pyjson::pretty(value) + "\n"
}

/// The value at a path of object keys or list indices.
pub fn get<'a>(value: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(value, |node, key| match node {
        Json::Array(items) => &items[key.parse::<usize>().unwrap()],
        _ => node.get(key).unwrap_or_else(|| panic!("missing {key}")),
    })
}

fn get_mut<'a>(value: &'a mut Json, key: &str) -> &'a mut Json {
    match value {
        Json::Object(pairs) => {
            &mut pairs
                .iter_mut()
                .find(|(k, _)| k.is(key))
                .unwrap_or_else(|| panic!("missing {key}"))
                .1
        }
        Json::Array(items) => &mut items[key.parse::<usize>().unwrap()],
        _ => panic!("not a container at {key}"),
    }
}

/// Set (or add) the value at a path of object keys or list indices.
pub fn set(value: &mut Json, path: &[&str], new: Json) {
    let (last, parents) = path.split_last().unwrap();
    let target = parents.iter().fold(value, |node, key| get_mut(node, key));
    match target {
        Json::Object(pairs) => match pairs.iter_mut().find(|(k, _)| k.is(last)) {
            Some(slot) => slot.1 = new,
            None => pairs.push((PyStr::from(*last), new)),
        },
        Json::Array(items) => items[last.parse::<usize>().unwrap()] = new,
        _ => panic!("not a container"),
    }
}

pub fn remove(value: &mut Json, path: &[&str]) {
    let (last, parents) = path.split_last().unwrap();
    let target = parents.iter().fold(value, |node, key| get_mut(node, key));
    let Json::Object(pairs) = target else {
        panic!("not an object")
    };
    pairs.retain(|(k, _)| !k.is(last));
}

pub fn list_mut<'a>(value: &'a mut Json, path: &[&str]) -> &'a mut Vec<Json> {
    match path.iter().fold(value, |node, key| get_mut(node, key)) {
        Json::Array(items) => items,
        _ => panic!("not a list"),
    }
}

pub fn text(value: &Json) -> String {
    value
        .as_pystr()
        .and_then(PyStr::to_utf8)
        .unwrap_or_else(|| panic!("not text: {value:?}"))
}

pub fn evaluate(env: &Env) -> Evaluation {
    gateway::evaluate(env, Path::new(CONTEXT), Path::new(REQUEST), Path::new(POLICY)).unwrap()
}

/// The refusal message, failing the test for success or a crash.
pub fn refusal<T: std::fmt::Debug>(result: Result<T, Failure>) -> String {
    match result {
        Err(Failure::Gateway(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[track_caller]
pub fn assert_refused<T: std::fmt::Debug>(result: Result<T, Failure>, fragment: &str) {
    let message = refusal(result);
    assert!(message.contains(fragment), "{message:?} does not contain {fragment:?}");
}

pub struct Run {
    pub code: i32,
    pub out: String,
    pub err: String,
}

pub fn run(env: &Env, args: &[&str]) -> Run {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = cli::main(&args, env, &mut out, &mut err);
    Run {
        code,
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
    }
}

pub fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

pub fn read_json(path: &Path) -> Json {
    parse(&std::fs::read_to_string(path).unwrap())
}

/// `json.dumps(payload, indent=2, sort_keys=True) + "\n"`, as the gateway writes it.
pub fn write_json(path: &Path, value: &Json) {
    write(path, &dumps(value));
}
