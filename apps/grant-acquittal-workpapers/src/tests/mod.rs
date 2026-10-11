//! Rust ports of tests/test_workpaper.py and tests/test_contributions.py.
//! Each test keeps the Python test's name.

mod contributions;
mod workpaper_tests;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::pycsv::{self, DictReader};
use crate::pyjson::{self, Json, PyStr};
use crate::workpaper::{self, Failure, Record};

const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/two-grants");

pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> TempDir {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "grant-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
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

/// The Python tests' setUp: a fresh copy of the example at `<root>/inputs`.
pub struct Fixture {
    pub temp: TempDir,
}

pub const SOURCE: &str = "inputs";

impl Fixture {
    pub fn new() -> Fixture {
        let fixture = Fixture { temp: TempDir::new() };
        fixture.reset();
        fixture
    }

    pub fn root(&self) -> &Path {
        self.temp.path()
    }

    pub fn source(&self) -> PathBuf {
        self.root().join(SOURCE)
    }

    /// `shutil.rmtree(source); shutil.copytree(EXAMPLE, source)`.
    pub fn reset(&self) {
        let _ = std::fs::remove_dir_all(self.source());
        std::fs::create_dir(self.source()).unwrap();
        for entry in std::fs::read_dir(EXAMPLE).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), self.source().join(entry.file_name())).unwrap();
        }
    }

    pub fn review(&self) -> workpaper::Result<Json> {
        workpaper::review(SOURCE, self.root())
    }

    pub fn write(&self, result: &Json, output: &str) -> workpaper::Result<()> {
        workpaper::write(result, output, self.root())
    }

    /// `main(["--input", source, "--output", output])`: the status and stderr.
    pub fn main(&self, output: &str) -> (i32, String) {
        let args: Vec<String> = ["--input", SOURCE, "--output", output].map(String::from).to_vec();
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let status = crate::cli::main("grant_workpaper.py", &args, self.root(), &mut stdout, &mut stderr);
        (status, String::from_utf8(stderr).unwrap())
    }

    /// Rewrite one CSV through `csv.DictReader` and `csv.DictWriter`.
    pub fn edit_csv(&self, name: &str, change: impl FnOnce(&mut Vec<Record>)) {
        let path = self.source().join(name);
        let text = std::fs::read_to_string(&path).unwrap();
        let mut reader = DictReader::new(&text).unwrap();
        let fields = reader.fieldnames.clone().unwrap();
        let mut records: Vec<Record> = reader
            .by_ref()
            .map(|row| {
                fields
                    .iter()
                    .cloned()
                    .zip(row.unwrap().values.into_iter().map(Option::unwrap))
                    .collect()
            })
            .collect();
        change(&mut records);
        let mut out = pycsv::write_row(&fields);
        for record in &records {
            let cells: Vec<String> = fields.iter().map(|f| get(record, f).to_string()).collect();
            out.push_str(&pycsv::write_row(&cells));
        }
        std::fs::write(&path, out).unwrap();
    }

    pub fn agreements(&self) -> Json {
        pyjson::loads_bytes(&std::fs::read(self.source().join("agreements.json")).unwrap()).unwrap()
    }

    pub fn save_agreements(&self, document: &Json) {
        std::fs::write(self.source().join("agreements.json"), pyjson::dumps_indent2(document)).unwrap();
    }

    /// `dict.update` on the agreement with this grant ID.
    pub fn edit_agreement(&self, grant_id: &str, changes: &[(&str, Json)]) {
        let mut document = self.agreements();
        let Json::Array(agreements) = field_mut(&mut document, "agreements") else {
            panic!("a list")
        };
        let agreement = agreements
            .iter_mut()
            .find(|a| a.at("grant_id").is_str(grant_id))
            .unwrap();
        for (key, value) in changes {
            set(agreement, key, value.clone());
        }
        self.save_agreements(&document);
    }

    /// The finding codes for one reference.
    pub fn codes(&self, reference: &str) -> Vec<String> {
        findings(&self.review().unwrap())
            .into_iter()
            .filter(|(_, r)| r == reference)
            .map(|(c, _)| c)
            .collect()
    }

    pub fn temporary_directories(&self) -> Vec<String> {
        std::fs::read_dir(self.root())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".grant-workpaper-"))
            .collect()
    }
}

pub fn get<'a>(record: &'a Record, key: &str) -> &'a str {
    &record.iter().find(|(k, _)| k == key).unwrap().1
}

pub fn put(record: &mut Record, key: &str, value: &str) {
    record.iter_mut().find(|(k, _)| k == key).unwrap().1 = value.to_string();
}

pub fn field_mut<'a>(value: &'a mut Json, key: &str) -> &'a mut Json {
    let Json::Object(pairs) = value else {
        panic!("an object")
    };
    &mut pairs.iter_mut().find(|(k, _)| k.is(key)).unwrap().1
}

/// `obj[key] = value`: replace in place, or append a new key.
pub fn set(value: &mut Json, key: &str, item: Json) {
    let Json::Object(pairs) = value else {
        panic!("an object")
    };
    match pairs.iter_mut().find(|(k, _)| k.is(key)) {
        Some(pair) => pair.1 = item,
        None => pairs.push((PyStr::from(key), item)),
    }
}

pub fn text(value: &Json) -> String {
    match value {
        Json::Str(s) => s.lossy(),
        other => pyjson::repr(other),
    }
}

/// `{(row["code"], row["reference"]) for row in result["findings"]}`, in order.
pub fn findings(result: &Json) -> Vec<(String, String)> {
    let Json::Array(rows) = result.at("findings") else {
        panic!("a list")
    };
    rows.iter()
        .map(|row| (text(row.at("code")), text(row.at("reference"))))
        .collect()
}

pub fn rows<'a>(result: &'a Json, table: &str) -> &'a [Json] {
    let Json::Array(rows) = result.at(table) else {
        panic!("a list")
    };
    rows
}

/// A refused input: the message the CLI would print.
pub fn refused<T: std::fmt::Debug>(outcome: workpaper::Result<T>) -> String {
    match outcome {
        Err(Failure::Error(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

pub fn read_csv(path: &Path) -> Vec<Record> {
    let text = std::fs::read_to_string(path).unwrap();
    let mut reader = DictReader::new(&text).unwrap();
    let fields = reader.fieldnames.clone().unwrap();
    reader
        .by_ref()
        .map(|row| {
            fields
                .iter()
                .cloned()
                .zip(row.unwrap().values.into_iter().map(Option::unwrap))
                .collect()
        })
        .collect()
}

/// A file symlink, or `None` where the host does not permit one (the Python tests skip).
pub fn symlink_file(target: &Path, link: &Path) -> Option<()> {
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_file(target, link);
    match made {
        Ok(()) => Some(()),
        Err(error) => {
            eprintln!("skipped: this host does not permit file symlinks: {error}");
            None
        }
    }
}
