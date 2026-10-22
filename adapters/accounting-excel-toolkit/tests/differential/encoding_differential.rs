use accounting_excel_toolkit::check_bytes;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

struct Oracle(Child);
impl Drop for Oracle {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let root = std::env::temp_dir().canonicalize().unwrap();
        assert_eq!(self.0.parent(), Some(root.as_path()));
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn next(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn corpus(seed: u64) -> Vec<Vec<u8>> {
    let mut cases: Vec<_> = (0..=255).map(|byte| vec![byte]).collect();
    cases.extend([
        vec![],
        b"\r\n".to_vec(),
        b"\n\r".to_vec(),
        b"\xef\xbb\xbf".to_vec(),
        vec![0xff; 25],
        vec![b'\n'; 21],
        vec![b'\r'; 21],
    ]);
    let mut large = vec![b' '; 0x10000];
    large.push(0x80);
    cases.push(large);
    let mut state = seed.max(1);
    for _ in 0..5000 {
        let len = (next(&mut state) % 1025) as usize;
        cases.push((0..len).map(|_| next(&mut state) as u8).collect());
    }
    cases
}

fn compare_bytes(python: &Path, script: &Path, seed: u64) -> Result<usize, String> {
    let adapter =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/differential/encoding_oracle.py");
    let child = Command::new(python)
        .arg("-I")
        .arg("-B")
        .arg(adapter)
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut oracle = Oracle(child);
    let mut input = oracle.0.stdin.take().unwrap();
    let mut output = BufReader::new(oracle.0.stdout.take().unwrap());
    let cases = corpus(seed);
    for (number, bytes) in cases.iter().enumerate() {
        for byte in bytes {
            write!(input, "{byte:02x}").map_err(|e| e.to_string())?;
        }
        writeln!(input)
            .and_then(|()| input.flush())
            .map_err(|e| e.to_string())?;
        let mut result = String::new();
        if output.read_line(&mut result).map_err(|e| e.to_string())? == 0 {
            return Err(format!("oracle stopped at case {number}"));
        }
        let expected = result.trim_end_matches(['\r', '\n']);
        let actual = check_bytes(bytes).join("\t");
        if expected != actual {
            return Err(format!(
                "byte case {number}: Python {expected:?}; Rust {actual:?}"
            ));
        }
    }
    drop(input);
    if !oracle.0.wait().map_err(|e| e.to_string())?.success() {
        return Err("byte oracle failed".into());
    }
    Ok(cases.len())
}

fn scratch(seed: u64) -> Result<Scratch, String> {
    let root = std::env::temp_dir()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let path = root.join(format!(
        "excel-encoding-differential-{}-{seed}",
        std::process::id()
    ));
    fs::create_dir(&path).map_err(|e| e.to_string())?;
    Ok(Scratch(path))
}

fn compare_cli(python: &Path, script: &Path, binary: &Path, seed: u64) -> Result<usize, String> {
    let temp = scratch(seed)?;
    for (name, bytes) in [
        ("clean.bas", b"Option Explicit\r\n".as_slice()),
        ("dirty.frm", b"\xef\xbb\xbf\n"),
        ("--help", b"\r\n"),
        ("source.txt", b"\xff\r"),
        ("\u{e9}.CLS", b"\r\n"),
    ] {
        fs::write(temp.0.join(name), bytes).map_err(|e| e.to_string())?;
    }
    let cases = [
        vec!["clean.bas"],
        vec!["dirty.frm"],
        vec!["clean.bas", "dirty.frm"],
        vec!["--help"],
        vec!["source.txt"],
        vec!["\u{e9}.CLS"],
        vec!["clean.bas", "clean.bas"],
        vec!["z.missing", "a.missing"],
        vec!["././clean.bas"],
        vec![""],
        vec!["."],
    ];
    for (number, arguments) in cases.iter().enumerate() {
        let expected = Command::new(python)
            .args(["-I", "-B"])
            .arg(script)
            .args(arguments)
            .current_dir(&temp.0)
            .output()
            .map_err(|e| e.to_string())?;
        let actual = Command::new(binary)
            .args(arguments)
            .current_dir(&temp.0)
            .output()
            .map_err(|e| e.to_string())?;
        if expected.status.code() != actual.status.code()
            || expected.stdout != actual.stdout
            || expected.stderr != actual.stderr
        {
            return Err(format!(
                "CLI case {number}: Python {expected:?}; Rust {actual:?}"
            ));
        }
    }
    Ok(cases.len())
}

fn main() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.len() != 4 {
        return Err("usage: encoding-differential PYTHON FROZEN_SCRIPT NATIVE_BINARY SEED".into());
    }
    let seed = arguments[3]
        .to_str()
        .ok_or("non-Unicode seed")?
        .parse::<u64>()
        .map_err(|e| e.to_string())?;
    let python = Path::new(&arguments[0]);
    let script = Path::new(&arguments[1])
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let binary = Path::new(&arguments[2])
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let bytes = compare_bytes(python, &script, seed)?;
    let cli = compare_cli(python, &script, &binary, seed)?;
    println!("Seed {seed}: {bytes} byte cases, {cli} exact CLI comparisons, zero differences.");
    Ok(())
}
