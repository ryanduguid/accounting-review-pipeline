//! Capture fabricated Python baseline calls as portable Rust regression cases.
//! CPython is required as the behaviour oracle; it is not a binary dependency.
use std::{env, process::Command};

const CAPTURE: &str = r#"
import contextlib, datetime, decimal, functools, io, json, os, pathlib, sys, unittest
root, output = sys.argv[1:]
os.chdir(root)
sys.path.insert(0, root)
import export_tb, xero_client, auth
modules = [export_tb, xero_client, auth]
format_amount = export_tb.format_amount
active = ''
depth = 0
records = []
covered = set()
class Tee:
    def __init__(self, original):
        self.original, self.buffer = original, io.StringIO()
    def write(self, text):
        self.buffer.write(text)
        return self.original.write(text)
    def flush(self): self.original.flush()
    def __getattr__(self, key): return getattr(self.original, key)
def serial(value):
    if isinstance(value, decimal.Decimal): return format_amount(value)
    if isinstance(value, datetime.datetime): return value.timestamp()
    if isinstance(value, (list, tuple)): return [serial(v) for v in value]
    if isinstance(value, dict): return {k: serial(v) for k, v in value.items()}
    if value is None or isinstance(value, (str, int, float, bool)): return value
    raise TypeError(type(value).__name__)
def capture(module, name):
    original = getattr(module, name)
    @functools.wraps(original)
    def wrapper(*args, **kwargs):
        global depth
        if name == 'retry_after_seconds' and kwargs.get('now') is None:
            kwargs = dict(kwargs, now=xero_client.datetime.now(xero_client.timezone.utc))
        outer = depth == 0
        depth += 1
        stdout, stderr = Tee(sys.stdout), Tee(sys.stderr)
        outcome = None
        try:
            with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
                result = original(*args, **kwargs)
            value = serial(result)
            if name == 'validate_rotated_response': value = {'value': value, 'stderr': stderr.buffer.getvalue()}
            outcome = {'ok': value}
            return result
        except BaseException as error:
            if isinstance(error, SystemExit):
                code = error.code if isinstance(error.code, int) else 1
                message = '' if isinstance(error.code, (int, type(None))) else str(error.code) + '\n'
            elif isinstance(error, (ValueError, TypeError)):
                code, message = 1, str(error) + '\n'
            else:
                raise
            outcome = {'code': code, 'stdout': stdout.buffer.getvalue(), 'stderr': message}
            raise
        finally:
            depth -= 1
            if outer and active and outcome is not None:
                try:
                    inputs = serial(args)
                    if name == 'is_account_code' and (not inputs or not isinstance(inputs[0], str)): raise TypeError()
                    if name == 'format_amount': inputs = [serial(args[0])]
                    if name == 'validate_token_response': inputs = [inputs[0], kwargs.get('label', 'token.json')]
                    request = {'op': name, 'args': inputs}
                    if 'now' in kwargs and kwargs['now'] is not None: request['now'] = serial(kwargs['now'])
                    if 'cached' in kwargs: request['cached'] = kwargs['cached']
                    if name == 'check_balanced' and kwargs.get('account_count') is not None: request['account_count'] = kwargs['account_count']
                    records.append({'test': active, 'request': request, 'expected': outcome})
                    covered.add(active)
                except (TypeError, UnicodeError): pass
    for target in modules:
        for key, value in list(vars(target).items()):
            if value is original: setattr(target, key, wrapper)
for name in ['to_number', 'format_amount', 'is_account_code', 'iso_date', 'excel_safe', 'validate_report_identity', 'flatten_report', 'validated_connections', 'select_tenant', 'default_output_filename', 'row_amounts', 'add_totals', 'build_rows', 'check_balanced']:
    capture(export_tb, name)
for name in ['retry_after_seconds', 'validate_rotated_response', 'validate_token_response']:
    capture(xero_client, name)
capture(auth, 'callback_server_config')
class Result(unittest.TextTestResult):
    def startTest(self, test):
        global active
        active = test.id()
        super().startTest(test)
    def stopTest(self, test):
        global active
        active = ''
        super().stopTest(test)
suite = unittest.defaultTestLoader.discover('tests')
result = unittest.TextTestRunner(verbosity=1, resultclass=Result).run(suite)
if not result.wasSuccessful(): raise SystemExit('baseline tests failed during capture')
payload = {'source_revision': '1af8a855578afaec97bd2e56836a8aa8fb15e729', 'tests_run': result.testsRun, 'covered_tests': sorted(covered), 'cases': records}
pathlib.Path(output).write_text(json.dumps(payload, ensure_ascii=True, separators=(',', ':')) + '\n', encoding='utf-8')
print(f'captured {len(records)} calls from {len(covered)} test methods')
"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.len() != 3 { return Err("usage: record-python-tests PYTHON BASELINE_COMPONENT OUTPUT".into()); }
    let token = env::temp_dir().join("xero-rust-capture-synthetic").join("token.json");
    let status = Command::new(&args[0]).args(["-B", "-c", CAPTURE]).args(&args[1..])
        .env_remove("XERO_CLIENT_ID").env_remove("XERO_CLIENT_SECRET").env("XERO_TOKEN_FILE", token).status()?;
    if !status.success() { return Err("Python baseline capture failed".into()); }
    Ok(())
}
