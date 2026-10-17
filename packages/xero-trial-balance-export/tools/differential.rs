//! Seeded, offline comparison with the pinned CPython release.
use std::{
    env, fs,
    io::{self, BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use xero_trial_balance_export::{
    pyjson::{self, Json},
    report,
    tokens::TokenSession,
};

const LOCK_ORACLE: &str = r#"
import json, sys
sys.path.insert(0, sys.argv[1])
import xero_client
with open(sys.argv[2], 'r+b') as lock:
    for line in sys.stdin:
        action=json.loads(line)
        if action == 'release':
            xero_client._release_token_lock(lock)
            result=True
        else:
            result=xero_client._try_token_lock(lock)
            if result and action == 'try': xero_client._release_token_lock(lock)
        print(json.dumps(result), flush=True)
"#;

const CLI_MIGRATION: &str = r#"
import argparse, sys
original_add_argument=argparse.ArgumentParser.add_argument
def add_argument(self,*args,**kwargs):
    if '--token-file' in args:
        kwargs['help']=kwargs['help'].replace('module-adjacent','executable-adjacent')
    return original_add_argument(self,*args,**kwargs)
argparse.ArgumentParser.add_argument=add_argument
sys.path.insert(0,sys.argv.pop(1))
"#;

const ORACLE: &str = r#"
import base64, contextlib, datetime, decimal, io, json, os, pathlib, sys, tempfile
from unittest import mock
sys.path.insert(0, sys.argv[1])
import export_tb, xero_client, auth
def serial(value):
    if isinstance(value, decimal.Decimal): return export_tb.format_amount(value)
    if isinstance(value, (tuple, list)): return [serial(v) for v in value]
    if isinstance(value, dict): return {k:serial(v) for k,v in value.items()}
    return value
def run(request):
    op, args = request['op'], request.get('args', [])
    module = xero_client if op in ['retry_after_seconds', 'validate_rotated_response', 'validate_token_response'] else auth if op == 'callback_server_config' else export_tb
    kwargs = {}
    if op == 'format_amount': args = [decimal.Decimal(args[0])]
    if op in ['add_totals', 'check_balanced']: args = [tuple(decimal.Decimal(v) for v in group) for group in args]
    if op == 'check_balanced' and 'account_count' in request: kwargs['account_count'] = request['account_count']
    if op == 'retry_after_seconds': kwargs['now'] = datetime.datetime.fromtimestamp(request.get('now', 1760000000), datetime.timezone.utc)
    if op == 'validate_token_response':
        kwargs['label'] = args[1]
        kwargs['cached'] = request.get('cached', False)
        args = args[:1]
    out, err = io.StringIO(), io.StringIO()
    try:
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            if op == 'csv_artifact':
                rows, totals = export_tb.build_rows(*args)
                export_tb.check_balanced(totals, account_count=len(rows))
                class Clock:
                    @staticmethod
                    def now(zone): return datetime.datetime(2026,6,30,12,34,56,tzinfo=datetime.timezone.utc)
                with tempfile.TemporaryDirectory() as directory, mock.patch.object(export_tb, 'datetime', Clock):
                    path = os.path.join(directory, 'report.csv')
                    digest = export_tb.write_csv(rows, path)
                    export_tb.write_manifest(path, digest, args[1], args[2], 'accrual')
                    result = {'csv': base64.b64encode(pathlib.Path(path).read_bytes()).decode('ascii'), 'manifest': pathlib.Path(path + '.manifest.json').read_bytes().decode('utf-8')}
            else: result = getattr(module, op)(*args, **kwargs)
        result = serial(result)
        if op == 'validate_rotated_response': result = {'value': result, 'stderr': err.getvalue()}
        return {'ok': result}
    except BaseException as error:
        if isinstance(error, UnicodeError) and op == 'csv_artifact': return {'code': 1, 'stdout': out.getvalue(), 'stderr': 'error: the export contains an unpaired Unicode surrogate and cannot be encoded as UTF-8; nothing was written.\n'}
        if isinstance(error, SystemExit):
            code = error.code if isinstance(error.code, int) else 1
            message = '' if isinstance(error.code, (int, type(None))) else str(error.code) + '\n'
        else: code, message = 1, str(error) + '\n'
        return {'code': code, 'stdout': out.getvalue(), 'stderr': message}
for line in sys.stdin:
    try: response = run(json.loads(line))
    except BaseException as error: response = {'oracle_error': type(error).__name__ + ': ' + str(error)}
    print(json.dumps(response, ensure_ascii=True, separators=(',', ':')), flush=True)
"#;

struct Process {
    child: Child,
    input: Option<ChildStdin>,
    lines: mpsc::Receiver<io::Result<String>>,
    reader: Option<thread::JoinHandle<()>>,
}
impl Process {
    fn start(command: &mut Command) -> io::Result<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let input = child.stdin.take();
        let output = child.stdout.take().unwrap();
        let (send, lines) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            input,
            lines,
            reader: Some(reader),
        })
    }
    fn query(&mut self, value: &Json) -> Result<Json, Box<dyn std::error::Error>> {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{}", pyjson::canonical(value))?;
        input.flush()?;
        let line = self.lines.recv_timeout(Duration::from_secs(5))??;
        Ok(pyjson::loads(&line).map_err(|_| "invalid oracle response")?)
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.input.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn compare_locks(python: &Path, baseline: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let directory = env::temp_dir().join(format!("xero-differential-lock-{}", std::process::id()));
    fs::create_dir(&directory)?;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let path = directory.join("token.json");
    let session = TokenSession::new(&path).map_err(|e| e.stderr)?;
    let held = session.lock().map_err(|e| e.stderr)?;
    let lock_path = path.with_file_name("token.json.lock");
    let mut command = Command::new(python);
    command
        .args(["-I", "-B", "-u", "-c", LOCK_ORACLE])
        .arg(baseline)
        .arg(&lock_path)
        .env_remove("XERO_CLIENT_ID")
        .env_remove("XERO_CLIENT_SECRET")
        .env("XERO_TOKEN_FILE", &path);
    let mut oracle = Process::start(&mut command)?;
    if !matches!(oracle.query(&Json::str("try"))?, Json::Bool(false)) {
        return Err("Python acquired the native-held token lock".into());
    }
    drop(held);
    if !matches!(oracle.query(&Json::str("try"))?, Json::Bool(true)) {
        return Err("Python could not acquire the released native lock".into());
    }
    if !matches!(oracle.query(&Json::str("hold"))?, Json::Bool(true)) {
        return Err("Python could not hold the token lock".into());
    }
    let native = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)?;
    if !matches!(native.try_lock(), Err(fs::TryLockError::WouldBlock)) {
        return Err("Rust acquired the Python-held token lock".into());
    }
    if !matches!(oracle.query(&Json::str("release"))?, Json::Bool(true)) {
        return Err("Python did not release the token lock".into());
    }
    native.try_lock()?;
    drop(native);
    drop(oracle);
    if path.exists() {
        return Err("lock interoperability created a token cache".into());
    }
    println!("Token lock: Python and Rust exclude each other and release successfully");
    Ok(())
}

struct Random(u64);
impl Random {
    fn next(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }
    fn leaf(&mut self) -> Json {
        let texts = [
            "",
            " ",
            "0",
            "-0.0000",
            "1_000.01",
            "٠.٢",
            "1e-31",
            "1e31",
            "NaN",
            "∞",
            "=SUM(A1)",
            "\t+1",
            "\u{a0}@x",
            "ßΣς",
            "e\u{301}",
            "200",
            "200 Account",
            "Revenue (200)",
            "synthetic",
        ];
        match self.next(5) {
            0 => Json::Null,
            1 => Json::Bool(self.next(2) == 1),
            2 => Json::int(self.next(2000) as i64 - 1000),
            _ => Json::str(texts[self.next(texts.len())]),
        }
    }
    fn mutate(&mut self, value: &mut Json, depth: usize) {
        if depth > 7 || self.next(4) == 0 {
            *value = self.leaf();
            return;
        }
        match value {
            Json::Array(items) if !items.is_empty() => {
                let i = self.next(items.len());
                if self.next(5) == 0 {
                    items.remove(i);
                } else {
                    self.mutate(&mut items[i], depth + 1);
                }
            }
            Json::Object(items) if !items.is_empty() => {
                let i = self.next(items.len());
                if self.next(5) == 0 {
                    items.remove(i);
                } else {
                    self.mutate(&mut items[i].1, depth + 1);
                }
            }
            _ => *value = self.leaf(),
        }
    }
}
fn request(op: &str, args: Vec<Json>) -> Json {
    Json::object([("op", Json::str(op)), ("args", Json::Array(args))])
}

type CapturedOutput = (i32, Vec<u8>, Vec<u8>);
fn command_output(command: &mut Command) -> Result<CapturedOutput, Box<dyn std::error::Error>> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()?;
    let read = |pipe: Box<dyn Read + Send>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut pipe = pipe;
            pipe.read_to_end(&mut bytes).map(|_| bytes)
        })
    };
    let stdout = read(Box::new(child.stdout.take().unwrap()));
    let stderr = read(Box::new(child.stderr.take().unwrap()));
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout.join();
            let _ = stderr.join();
            return Err("CLI exceeded ten-second deadline".into());
        }
        thread::sleep(Duration::from_millis(5));
    };
    Ok((
        status.code().unwrap_or(-1),
        stdout.join().map_err(|_| "stdout reader failed")??,
        stderr.join().map_err(|_| "stderr reader failed")??,
    ))
}

fn compare_cli(
    python: &std::ffi::OsStr,
    baseline: &Path,
    probe: &Path,
) -> Result<usize, Box<dyn std::error::Error>> {
    let directory = env::temp_dir().join(format!("xero-differential-cli-{}", std::process::id()));
    fs::create_dir(&directory)?;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let cases: &[&[&str]] = &[
        &[],
        &["--help"],
        &["-h"],
        &["--help=x"],
        &["-hfoo"],
        &["--he"],
        &["--unknown"],
        &["--unknown", "--help"],
        &["--date"],
        &["--date", "bad"],
        &["--date=2024-W01-1"],
        &["--date", "20240101"],
        &["--out"],
        &["--out", "-1"],
        &["--out", "x.txt"],
        &["--out", "../outside.csv"],
        &["--q=x"],
        &["--t"],
        &["--quiet", "--no-manifest"],
        &["--", "--help"],
        &["--token-file", "wrong.json"],
        &["--token-file", "token.json"],
        &["-"],
        &["-hfoo=1"],
        &["--quiet=1"],
        &["--pay"],
        &["--date", "--help"],
    ];
    let mut differences = 0;
    for (module, binary) in [("export_tb", "export-tb"), ("auth", "xero-tb-auth")] {
        for columns in [20, 40, 80, 120] {
            for flags in cases {
                let script = format!(
                    "{CLI_MIGRATION}\nsys.argv[0]='{binary}';import {module};{module}.{}()",
                    if module == "auth" { "main" } else { "run" }
                );
                let mut source = Command::new(python);
                source
                    .args(["-I", "-B", "-c", &script])
                    .arg(baseline)
                    .args(*flags);
                let executable =
                    probe.with_file_name(format!("{binary}{}", env::consts::EXE_SUFFIX));
                let mut rust = Command::new(executable);
                rust.args(*flags);
                for command in [&mut source, &mut rust] {
                    command
                        .current_dir(&directory)
                        .env_remove("XERO_CLIENT_ID")
                        .env_remove("XERO_CLIENT_SECRET")
                        .env("XERO_TOKEN_FILE", directory.join("synthetic/token.json"))
                        .env("COLUMNS", columns.to_string())
                        .env("TERM", "dumb")
                        .env("NO_COLOR", "1")
                        .env("PYTHONIOENCODING", "utf-8");
                }
                let expected = command_output(&mut source)?;
                let actual = command_output(&mut rust)?;
                if expected != actual {
                    differences += 1;
                    if differences <= 20 {
                        println!(
                            "CLI {binary} columns={columns} {flags:?}\nPython {:?}\nRust {:?}",
                            (
                                expected.0,
                                String::from_utf8_lossy(&expected.1),
                                String::from_utf8_lossy(&expected.2)
                            ),
                            (
                                actual.0,
                                String::from_utf8_lossy(&actual.1),
                                String::from_utf8_lossy(&actual.2)
                            )
                        );
                    }
                }
            }
        }
    }
    println!("CLI: {} cases, {differences} differences", cases.len() * 8);
    Ok(differences)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 5 {
        return Err(
            "usage: xero-tb-differential PYTHON BASELINE_COMPONENT PROBE SEED COUNT".into(),
        );
    }
    let seed: u64 = args[3].to_string_lossy().parse()?;
    let count: usize = args[4].to_string_lossy().parse()?;
    let baseline = fs::canonicalize(&args[1])?;
    // Keep the venv symlink: resolving it selects the base interpreter's packages.
    let python_path = std::path::absolute(&args[0])?;
    let probe_path = fs::canonicalize(&args[2])?;
    let mut python_cmd = Command::new(&python_path);
    python_cmd
        .args(["-I", "-B", "-u", "-c", ORACLE])
        .arg(&baseline)
        .env_remove("XERO_CLIENT_ID")
        .env_remove("XERO_CLIENT_SECRET")
        .env(
            "XERO_TOKEN_FILE",
            env::temp_dir().join("xero-differential-synthetic/token.json"),
        );
    let mut python = Process::start(&mut python_cmd)?;
    let mut native = Process::start(&mut Command::new(&probe_path))?;
    let fixture = pyjson::loads(include_str!("../tests/fixtures/python-value-cases.json"))
        .map_err(|_| "invalid fixture")?;
    let Json::Array(cases) = fixture.at("cases") else {
        return Err("missing cases".into());
    };
    let seeds: Vec<_> = cases
        .iter()
        .filter_map(|case| {
            let value = case.at("request");
            let op = report::text(value.at("op"));
            (matches!(
                op.as_str(),
                "to_number"
                    | "excel_safe"
                    | "flatten_report"
                    | "validate_report_identity"
                    | "validated_connections"
                    | "validate_token_response"
                    | "validate_rotated_response"
            ) && pyjson::canonical(value).len() < 16000)
                .then_some(value)
        })
        .collect();
    let mut random = Random(seed.max(1));
    let mut inputs = Vec::new();
    for case in cases {
        let original = case.at("request");
        if report::text(original.at("op")) == "build_rows"
            && let Some(Json::Array(args)) = original.get("args")
        {
            let mut args = args.clone();
            if let Some(Json::Object(tenant)) = args.get_mut(1)
                && !tenant.iter().any(|(key, _)| key.is("tenantId"))
            {
                tenant.push(("tenantId".into(), Json::str("synthetic-tenant-id")));
            }
            inputs.push(request("csv_artifact", args));
        }
    }
    for points in [
        vec![0xd800],
        vec![0x10f7ff, 0x10f800],
        vec![61, 0xd800],
        vec![49, 0xdcff],
        vec![0, 49, 0x2028],
    ] {
        for operation in ["to_number", "excel_safe"] {
            inputs.push(request(
                operation,
                vec![Json::Str(pyjson::PyStr(points.clone()))],
            ));
        }
    }
    for _ in 0..count {
        let mut value = (*seeds[random.next(seeds.len())]).clone();
        let op = report::text(value.at("op"));
        if let Json::Object(fields) = &mut value
            && let Some((_, Json::Array(params))) = fields.iter_mut().find(|(k, _)| k.is("args"))
            && !params.is_empty()
        {
            if matches!(op.as_str(), "flatten_report" | "validate_report_identity") {
                if let Json::Object(items) = &mut params[0]
                    && !items.is_empty()
                {
                    let i = random.next(items.len());
                    random.mutate(&mut items[i].1, 1);
                }
            } else {
                random.mutate(&mut params[0], 0);
            }
        }
        inputs.push(value);
    }
    for exponent in -40..=40 {
        for coefficient in [
            "0",
            "-0",
            "1",
            "-1",
            "1.000000000000000000000000000001",
            "999999999999999999999999999999999999",
        ] {
            inputs.push(request(
                "to_number",
                vec![Json::str(&format!("{coefficient}e{exponent}"))],
            ));
        }
    }
    for text in [
        "2024-01-01",
        "20240101",
        "2024-W01",
        "2024W011",
        "2024-W01-1suffix",
        "2024-001",
        "0000-01-01",
        "10000-01-01",
        "1",
        "",
        "2024-02-30",
    ] {
        inputs.push(request("iso_date", vec![Json::str(text)]));
    }
    for uri in [
        "http:/callback",
        "http:///callback",
        "http://",
        "http://localhost?next=/callback",
        "http://localhost:bad/a",
        "HTTP://LOCALHOST:8400/../callback",
        "http://localhost:65536/a",
        "http://localhost:0/a",
        "http://user@localhost/a",
        "http://localhost/a%2Fb",
        "http://localhost/callback;parameters",
        " http://localhost/callback",
        "http://local\thost/callback",
        "http://%6cocalhost/callback",
        "http://localhost\\other/callback",
        "http://localhost./callback",
        "http://[::1]/callback",
    ] {
        inputs.push(request("callback_server_config", vec![Json::str(uri)]));
    }
    for year in [1, 2000, 2024, 9999] {
        for week in [0, 1, 52, 53, 99] {
            for day in [0, 1, 7, 8] {
                inputs.push(request(
                    "iso_date",
                    vec![Json::str(&format!("{year:04}-W{week:02}-{day}"))],
                ));
            }
        }
    }
    for value in [
        "Sun, 06 Nov 1994 08:49:37 GMT",
        "Sunday, 06-Nov-94 08:49:37 GMT",
        "Sun Nov  6 08:49:37 1994",
        "06 Nov 1994 08:49:37",
        "Fri, 09 Oct 2026 01:02:03 +1000",
        "000000000000000000000003",
        "+3",
        "1_0",
    ] {
        let mut value = request("retry_after_seconds", vec![Json::str(value)]);
        if let Json::Object(fields) = &mut value {
            fields.push(("now".into(), Json::int(784111770)));
        }
        inputs.push(value);
    }
    let mut differences = 0;
    for (index, value) in inputs.iter().enumerate() {
        let mut expected = python.query(value)?;
        xero_trial_balance_export::compat_probe::migrate_guidance(&mut expected);
        let actual = native.query(value)?;
        if !pyjson::py_eq(&expected, &actual, true) {
            differences += 1;
            if differences <= 20 {
                println!(
                    "{}",
                    pyjson::canonical(&Json::object([
                        ("index", Json::int(index as i64)),
                        ("request", value.clone()),
                        ("python", expected),
                        ("rust", actual)
                    ]))
                );
            }
        }
    }
    println!(
        "seed {seed}: {} cases, {differences} differences; baseline {}",
        inputs.len(),
        Path::new(&baseline).display()
    );
    differences += compare_cli(python_path.as_os_str(), &baseline, &probe_path)?;
    compare_locks(&python_path, &baseline)?;
    if differences > 0 {
        return Err("unexplained behaviour differences".into());
    }
    Ok(())
}
