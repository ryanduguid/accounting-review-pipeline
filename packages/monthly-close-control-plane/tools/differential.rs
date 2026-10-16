//! Compare fabricated inputs with the frozen CPython implementation.
use monthly_close_control_plane::probe::{self, Json, PyStr};
use monthly_close_control_plane::pyjson;
use std::{
    env, fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const ORACLE: &str = r#"
import contextlib, dataclasses, datetime, decimal, hashlib, io, json, pathlib, sys, unicodedata, tempfile
from unittest.mock import patch
if sys.version_info[:2] != (3,15) or unicodedata.unidata_version != '17.0.0':
    raise SystemExit('the oracle requires CPython 3.15 with Unicode 17.0.0')
sys.path.insert(0, sys.argv[1])
from closecontrol import loader, engine, queries, calculation_evidence, equity, report, viewer, schedules, reconciliation, reconciliation_report, drivers, classification, comparison, queue, cli
sys.argv[0]='close-control'
def converted(value):
    if dataclasses.is_dataclass(value):
        return {f.name:converted(getattr(value,f.name)) for f in dataclasses.fields(value)}
    if isinstance(value,decimal.Decimal): return str(value)
    if isinstance(value,datetime.date): return value.isoformat()
    if isinstance(value,pathlib.Path): return str(value)
    if isinstance(value,dict): return {k:converted(v) for k,v in value.items()}
    if isinstance(value,(set,frozenset)): return sorted(value)
    if isinstance(value,(tuple,list)): return [converted(v) for v in value]
    return value
def snapshot(r):
    data=bytes.fromhex(r['hex'])
    return loader.SourceSnapshot(pathlib.Path(r['path']),data,hashlib.sha256(data).hexdigest())
def operation(r):
    op=r['op']
    if op=='cli':
        with tempfile.TemporaryDirectory(prefix='closecontrol-oracle-cli-') as directory:
            root=pathlib.Path(directory)
            for name,content in r.get('files',{}).items():
                path=root/name;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(bytes.fromhex(content))
            for name in r.get('directories',[]): (root/name).mkdir(parents=True,exist_ok=True)
            args=[value.replace('$TMP',directory) for value in r['args']]
            out,err=io.StringIO(),io.StringIO()
            with contextlib.redirect_stdout(out),contextlib.redirect_stderr(err):
                try: code=cli.main(args)
                except SystemExit as exited: code=exited.code
            def normalise(text):
                for raw in (directory,directory.replace('\\','/')):
                    variants=[raw]
                    for _ in range(3): variants.append(json.dumps(variants[-1])[1:-1])
                    for variant in reversed(variants): text=text.replace(variant,'$TMP')
                return text
            def file_bytes(path):
                raw=path.read_bytes()
                try: return normalise(raw.decode('utf-8')).encode('utf-8').hex()
                except UnicodeDecodeError: return raw.hex()
            files={path.relative_to(root).as_posix():file_bytes(path) for path in root.rglob('*') if path.is_file()}
            return {'code':code,'stdout':normalise(out.getvalue()),'stderr':normalise(err.getvalue()),'files':files}
    if op=='money': return loader.parse_money(r['value'],field='Amount',row_number=2,path=pathlib.Path('input.csv'))
    if op=='date': return loader.parse_iso_date(r['value'],field='Date',row_number=2,path=pathlib.Path('input.csv'))
    if op=='csv':
        functions={'trial_balance':loader.load_canonical_tb,'mapping':loader.load_mapping,'mapping_policy':loader.load_mapping_policy,'balance_policy':loader.load_balance_policy,'subledger':loader.load_subledger}
        result=functions[r['kind']](snapshot(r))
        if r['kind']=='subledger': return [[tenant,key,str(value)] for (tenant,key),value in sorted(result.items())]
        return result
    if op=='review_note': return loader.load_reviewer_acknowledgement(snapshot(r))
    if op=='schedule':
        with patch.object(loader.SourceSnapshot,'capture',lambda path,*,label:snapshot(r)):
            return schedules.review_schedule(pathlib.Path(r['path']),kind=r['kind'])
    if op=='reconciliation_date': return reconciliation._date(r['value'])
    if op=='classify':
        sources=[snapshot(r[key]) for key in ('original','current','coding_evidence')]
        inputs={source.path.resolve():dataclasses.replace(source,path=source.path.resolve()) for source in reversed(sources)}
        with patch.object(loader.SourceSnapshot,'capture',lambda path,*,label:inputs[path]):
            return classification.classify(*(source.path for source in sources),currency=r['currency'])
    if op=='compare':
        packs={pathlib.Path(name+'-pack'):r[name] for name in ('previous','current')}
        inputs={}
        for name in ('previous','current'):
            source=snapshot(r[name]['trial_balance']);inputs[source.path]=source
        response=snapshot(r['responses']) if r.get('responses') is not None else None
        if response is not None: inputs[response.path]=response
        def verified(path):
            payloads={key:bytes.fromhex(value) for key,value in packs[path]['payloads'].items()}
            with patch.object(viewer,'_load_artefact_bytes',lambda directory:payloads): return viewer.verify_pack(path)
        with patch.object(comparison,'verify_pack',verified),patch.object(loader.SourceSnapshot,'capture',lambda path,*,label:inputs[path]):
            return comparison.compare_packs(previous_pack=pathlib.Path('previous-pack'),current_pack=pathlib.Path('current-pack'),previous_tb=pathlib.Path(r['previous']['trial_balance']['path']),current_tb=pathlib.Path(r['current']['trial_balance']['path']),responses=response.path if response is not None else None)
    if op=='queue':
        def verified(path):
            pack=next(pack for pack in r['packs'] if pathlib.Path(pack['path']).resolve()==path)
            if 'error' in pack:
                from closecontrol.errors import ControlInputError
                raise ControlInputError(pack['error'])
            payloads={key:bytes.fromhex(value) for key,value in pack['payloads'].items()}
            with patch.object(viewer,'_load_artefact_bytes',lambda directory:payloads): return viewer.verify_pack(path)
        with patch.object(queue,'verify_pack',verified):
            return queue.render_review_queue([pathlib.Path(pack['path']) for pack in r['packs']],**{key:r[key] for key in ('period','status','reviewer') if key in r})
    if op in ('drivers','drivers_render'):
        names=(*viewer._REQUIRED_PACK_FILE_NAMES,viewer._QUERY_CSV_NAME)
        payloads={key:bytes.fromhex(r['payloads'][key]) for key in names if key in r['payloads']}
        source=snapshot(r['transactions'])
        with patch.object(viewer,'_load_artefact_bytes',lambda directory:payloads),patch.object(loader.SourceSnapshot,'capture',lambda path,*,label:source):
            result=drivers.variance_drivers(pathlib.Path('.'),source.path,currency=r['currency'],top=r['top'])
        if op=='drivers': return result
        with tempfile.TemporaryDirectory(prefix='closecontrol-oracle-') as directory:
            output=pathlib.Path(directory)/'pack'
            drivers.write_drivers(result,output)
            return {p.name:p.read_bytes().hex() for p in output.iterdir()}
    if op=='reconciliation_money':
        with decimal.localcontext(decimal.Context(prec=40)): return reconciliation._money(r['value'])
    if op in ('reconcile','reconcile_render'):
        inputs={}
        source=snapshot(r['transactions']);inputs[source.path]=source
        config={key:r[key] for key in ('tenant','account_id','currency','period_start','period_end','opening_balance','closing_balance')}
        for key in ('opening_items','decisions'):
            if r.get(key) is not None:
                src=snapshot(r[key]);inputs[src.path]=src;config[key]=src.path
        config['decisions_escaped']=r.get('decisions_escaped',False)
        with patch.object(loader.SourceSnapshot,'capture',lambda path,*,label:inputs[path]):
            result=reconciliation.reconcile(source.path,**config)
        if op=='reconcile': return result
        with tempfile.TemporaryDirectory(prefix='closecontrol-oracle-') as directory:
            output=pathlib.Path(directory)/'pack'
            reconciliation_report.write_reconciliation(result,output)
            return {p.name:p.read_bytes().hex() for p in output.iterdir()}
    if op=='verify':
        payloads={key:bytes.fromhex(value) for key,value in r['payloads'].items()}
        with patch.object(viewer,'_load_artefact_bytes',lambda directory:payloads):
            return {'verified':viewer.verify_pack(pathlib.Path('.')),'sheet':viewer.render_review_sheet(pathlib.Path('.'))}
    if op in ('review','render'):
        inputs={}
        config={}
        for key in ('current','prior','mapping','mapping_policy','balance_policy','subledger','acknowledgement','equity_schedule'):
            if r.get(key) is not None:
                src=snapshot(r[key]);inputs[src.path]=src;config[key+'_path']=src.path
        for key,field in [('absolute','absolute_threshold'),('percentage','percentage_threshold'),('tolerance','reconciliation_tolerance')]:
            if key in r: config[field]=decimal.Decimal(r[key])
        for key,field in [('required','required_calculations'),('currency','equity_currency'),('currency_evidence','equity_currency_evidence')]:
            if key in r: config[field]=r[key]
        if 'evidence' in r:
            config['calculation_evidence_paths']=[]
            for record in r['evidence']:
                src=snapshot(record);inputs[src.path]=src;config['calculation_evidence_paths'].append(src.path)
        def capture(path,*,label):
            if path not in inputs: raise loader.ControlInputError(f'{label} does not exist: {path}.')
            return inputs[path]
        with patch.object(loader.SourceSnapshot,'capture',capture): pack=engine.review_close(**config)
        if op=='review': return pack
        native=lambda text:text.replace('\n','\r\n').encode('utf-8') if sys.platform=='win32' else text.encode('utf-8')
        return [native(json.dumps(report._as_json(pack),indent=2,sort_keys=True)+'\n').hex(),native(report._as_markdown(pack)).hex(),report._as_csv(pack).encode('utf-8-sig').hex(),report._as_query_csv(pack).encode('utf-8-sig').hex()]
    if op=='integrity': return engine._integrity_exceptions(loader.load_canonical_tb(snapshot(r)),'Current')
    if op=='compare_core':
        current=loader.load_canonical_tb(snapshot(r['current']))
        prior=loader.load_canonical_tb(snapshot(r['prior']))
        findings=engine._financial_year_reset_exceptions(current[0].report_date,prior[0].report_date)
        findings+=engine._period_comparison_exceptions({row.key:row for row in current},{row.key:row for row in prior},decimal.Decimal(r['absolute']),decimal.Decimal(r['percentage']))
        return {'exceptions':findings,'queries':queries.derive_client_queries(findings)}
    if op=='calculation_evidence': return calculation_evidence.load(snapshot(r))
    if op=='equity_load': return equity.load_schedule(snapshot(r))
    if op=='equity':
        old, new = snapshot(r['prior']), snapshot(r['current'])
        source=snapshot(r)
        result,findings=equity.reconcile_equity(source,prior=loader.load_canonical_tb(old),current=loader.load_canonical_tb(new),prior_sha256=old.sha256,current_sha256=new.sha256,currency=r['currency'],currency_evidence=r['currency_evidence'],tolerance=decimal.Decimal(r['tolerance']))
        equity.validate_result(result,{'equity_schedule':source.sha256},converted(findings))
        return {'result':result,'exceptions':findings,'summary':equity.summary_lines(result)}
    if op=='equity_validate': return equity.validate_result(r['result'],r['sources'],r['exceptions'])
    if op=='covers_period': return calculation_evidence.covers_period(calculation_evidence.load(snapshot(r)),datetime.date.fromisoformat(r['date']))
    if op=='canonical_utf8': return calculation_evidence._canonical(r['value']).decode('utf-8')
    if op=='arithmetic':
        with decimal.localcontext(decimal.Context(prec=int(r['precision']),rounding=decimal.ROUND_HALF_EVEN)) as context:
            a,b=decimal.Decimal(r['a']),decimal.Decimal(r['b'])
            value={'add':lambda:a+b,'sub':lambda:a-b,'mul':lambda:a*b,'div':lambda:a/b}[r['kind']]()
            if r['kind'] in ('add','sub'): return {'value':str(value),'inexact':bool(context.flags[decimal.Inexact])}
            return str(value)
    raise ValueError('unsupported oracle operation')
for line in sys.stdin:
    try: result={'ok':converted(operation(json.loads(line)))}
    except Exception as exc: result={'error':str(exc)}
    print(json.dumps(result,sort_keys=True,separators=(',',':'),ensure_ascii=True),flush=True)
"#;
struct Corpus(Vec<Json>);
impl Corpus {
    fn push(&mut self, name: impl AsRef<str>, op: &str, pairs: Vec<(&str, Json)>) {
        let mut record = vec![
            (PyStr::from("name"), Json::str(name.as_ref())),
            (PyStr::from("op"), Json::str(op)),
        ];
        record.extend(pairs.into_iter().map(|(k, v)| (PyStr::from(k), v)));
        self.0.push(Json::Object(record));
    }
    fn scalar(&mut self, name: &str, op: &str, value: &str) {
        self.push(name, op, vec![("value", Json::str(value))]);
    }
    fn csv(&mut self, name: &str, kind: &str, text: &[u8]) {
        self.push(
            name,
            "csv",
            vec![
                ("path", Json::str("input.csv")),
                ("kind", Json::str(kind)),
                ("hex", Json::str(&hex(text))),
            ],
        );
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn money(c: &mut Corpus, seed: u64) {
    for (i, value) in [
        "",
        "0",
        "-0.00",
        "+$1,234.5600",
        "$0.01",
        "0.1234567890123456789",
        "123456789012345678.01",
        "9007199254740993.00",
        "NaN",
        "Infinity",
        "1e2",
        "1_000",
        "=1+1",
        "-",
        "1,23",
        "1,234,567.89",
        "1 000",
        "1.",
        ".1",
        "\u{1c}1\u{1f}",
        "1\u{200b}",
        "00,001",
        "0000,000",
        "1,234.00\n",
        "-+$1",
        "１２.３４",
    ]
    .iter()
    .enumerate()
    {
        c.scalar(&format!("money-fixed-{i}"), "money", value);
    }
    for (i, ch) in probe::decimal_digits().iter().enumerate() {
        c.scalar(
            &format!("money-unicode-{i}"),
            "money",
            &format!("{ch}{ch}.{ch}{ch}"),
        );
    }
    let mut rng = seed;
    for i in 0..4000 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let count = (rng % 64 + 1) as usize;
        let digits = format!("{}{}", rng % 9 + 1, "0123456789".repeat(7));
        let digits = &digits[..count];
        let decimal = (rng.rotate_left(17) % (count as u64 + 1)) as usize;
        let raw = if decimal == 0 {
            digits.into()
        } else {
            format!(
                "{}.{}",
                if decimal == count {
                    "0"
                } else {
                    &digits[..count - decimal]
                },
                &digits[count - decimal..]
            )
        };
        let raw = if rng & 4 != 0 { format!("-{raw}") } else { raw };
        c.scalar(&format!("money-generated-{i}"), "money", &raw);
    }
}
fn dates(c: &mut Corpus) {
    for year in [0, 1, 4, 100, 400, 1900, 2000, 2024, 2026, 9999, 10000] {
        for month in 0..=13 {
            for day in [0, 1, 28, 29, 30, 31, 32] {
                let value = format!("{year:04}-{month:02}-{day:02}");
                c.scalar(&format!("date-{value}"), "date", &value);
            }
        }
    }
    for value in [
        "20260331",
        "2026-W01-1",
        "2026-3-31",
        "２０２６-03-31",
        "2026-03-31\n",
        "2026-03-31T00:00:00",
        "",
    ] {
        c.scalar(&format!("date-other-{value:?}"), "date", value);
    }
}

fn exchange(mut command: Command, corpus: &str) -> Result<Vec<String>> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    let data = corpus.as_bytes().to_vec();
    let writer = thread::spawn(move || stdin.write_all(&data));
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let reader = thread::spawn(move || {
        use std::io::Read;
        let mut out = out;
        let mut bytes = Vec::new();
        out.read_to_end(&mut bytes).map(|_| bytes)
    });
    let errors = thread::spawn(move || {
        use std::io::Read;
        let mut err = err;
        let mut bytes = Vec::new();
        err.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(120);
    let status: Result<_> = (|| {
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err("comparison process exceeded its 120-second deadline".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
    })();
    if status.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let written = writer.join().map_err(|_| "oracle input thread panicked");
    let bytes = reader.join().map_err(|_| "oracle output thread panicked");
    let errors = errors.join().map_err(|_| "oracle error thread panicked");
    let status = status?;
    let bytes = bytes??;
    let errors = errors??;
    if !status.success() {
        return Err(format!(
            "comparison process failed: {}",
            String::from_utf8_lossy(&errors)
        )
        .into());
    }
    written??;
    Ok(String::from_utf8(bytes)?
        .lines()
        .map(str::to_owned)
        .collect())
}

fn source(path: &str, bytes: &[u8]) -> Json {
    Json::object([("path", Json::str(path)), ("hex", Json::str(&hex(bytes)))])
}
fn put(object: &mut Json, key: &str, value: Json) {
    let Json::Object(pairs) = object else {
        panic!("fixture is an object")
    };
    if let Some((_, old)) = pairs.iter_mut().find(|(name, _)| name.is(key)) {
        *old = value;
    } else {
        pairs.push((PyStr::from(key), value));
    }
}
fn csvs(c: &mut Corpus) {
    let templates = [
        (
            "trial_balance",
            "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit",
            "2026-03-31,Demo,Assets,A1,Cash,090,0.00,0.00,0.00,0.00",
        ),
        ("mapping", "AccountID,ReviewGroup", "A1,Cash"),
        ("mapping_policy", "Section,ReviewGroup", "Assets,Cash"),
        (
            "balance_policy",
            "AccountID,ExpectedBalance,ExpectMovement",
            "A1,debit,yes",
        ),
        (
            "subledger",
            "Tenant,AccountID,SubledgerBalance",
            "Demo,A1,0.00",
        ),
    ];
    for (kind, header, row) in templates {
        let valid = format!("{header}\n{row}\n");
        for (index, text) in [
            String::new(),
            format!("{header}\n"),
            valid.clone(),
            format!("{valid}{row}\n"),
            format!("{header},Extra\n{row}\n"),
            format!("{header}\n{row},extra\n"),
            format!("\u{feff}{valid}"),
            valid.replace('\n', "\r\n"),
            valid.replace('\n', "\r"),
        ]
        .iter()
        .enumerate()
        {
            c.csv(&format!("{kind}-shape-{index}"), kind, text.as_bytes());
        }
        let headings: Vec<_> = header.split(',').collect();
        let cells: Vec<_> = row.split(',').collect();
        for at in 0..cells.len() {
            let mut missing = headings.clone();
            missing.remove(at);
            c.csv(
                &format!("{kind}-missing-{at}"),
                kind,
                format!("{}\n{row}\n", missing.join(",")).as_bytes(),
            );
            let mut duplicate = headings.clone();
            duplicate[at] = headings[(at + 1) % headings.len()];
            c.csv(
                &format!("{kind}-duplicate-{at}"),
                kind,
                format!("{}\n{row}\n", duplicate.join(",")).as_bytes(),
            );
            for (index, replacement) in [
                "",
                " ",
                "=1+1",
                "\u{200b}",
                "x\t",
                "\u{202e}",
                "bad",
                "'1",
                "NaN",
                "0.1234567890123456789",
                "\"quoted,cell\"",
                "\"multi\nline\"",
                "\"ab\"stray",
                "\u{10f7ff}\u{10f800}",
            ]
            .iter()
            .enumerate()
            {
                let mut changed = cells.clone();
                changed[at] = replacement;
                c.csv(
                    &format!("{kind}-value-{at}-{index}"),
                    kind,
                    format!("{header}\n{}\n", changed.join(",")).as_bytes(),
                );
            }
            let mut short = cells.clone();
            short.truncate(at);
            c.csv(
                &format!("{kind}-short-{at}"),
                kind,
                format!("{header}\n{}\n", short.join(",")).as_bytes(),
            );
        }
        c.csv(&format!("{kind}-invalid-utf8"), kind, &[0xff]);
    }
    let h = templates[0].1;
    c.csv(
        "precedence-before-long-later-row",
        "trial_balance",
        format!(
            "{h}\n2026-03-31,Demo,Assets,A1,Cash,090,invalid,0,0,0\n{}",
            "x".repeat(131_073)
        )
        .as_bytes(),
    );
}
fn arithmetic(c: &mut Corpus) {
    let values = [
        "0",
        "-0.00",
        "1.00",
        "-1",
        "3",
        "1234567890123456789.0123456789",
        "1E+60",
        "1E-60",
        "0E-999999999999999999",
    ];
    for precision in ["28", "40", "80"] {
        for (i, a) in values.iter().enumerate() {
            for (j, b) in values.iter().enumerate() {
                for kind in ["add", "sub", "mul", "div"] {
                    if kind == "div" && (j == 0 || j == 1 || j == 8) {
                        continue;
                    }
                    c.push(
                        format!("arithmetic-{precision}-{i}-{j}-{kind}"),
                        "arithmetic",
                        vec![
                            ("precision", Json::str(precision)),
                            ("kind", Json::str(kind)),
                            ("a", Json::str(a)),
                            ("b", Json::str(b)),
                        ],
                    );
                }
            }
        }
    }
}
fn tb(date: &str, amount: &str, id: &str, name: &str) -> Vec<u8> {
    format!("ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n{date},Demo,Assets,{id},{name},090,{amount},0,{amount},0\n").into_bytes()
}
fn core(c: &mut Corpus, seed: u64) {
    for amount in [
        "0",
        "-0.00",
        "0.01",
        "1000.001",
        "123456789012345678901234567890.01",
    ] {
        let bytes = tb("2026-03-31", amount, "A1", "Cash");
        c.push(
            format!("integrity-{amount}"),
            "integrity",
            vec![
                ("path", Json::str("input.csv")),
                ("hex", Json::str(&hex(&bytes))),
            ],
        );
    }
    let mut rng = seed;
    for index in 0..300 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let before = format!("{}.{:02}", rng % 200_000, rng.rotate_left(13) % 100);
        let after = if index % 7 == 0 {
            "0.00".into()
        } else {
            format!(
                "{}{}.{:02}",
                if index % 3 == 0 { "-" } else { "" },
                rng.rotate_left(7) % 200_000,
                rng % 100
            )
        };
        let current = tb(
            if index % 4 == 0 {
                "2026-07-31"
            } else {
                "2026-03-31"
            },
            &after,
            "A1",
            if index % 5 == 0 { "Bank" } else { "Cash" },
        );
        let prior = tb(
            "2026-02-28",
            &before,
            if index % 11 == 0 { "A2" } else { "A1" },
            "Cash",
        );
        for (absolute, percentage) in [
            ("1000", "0.10"),
            ("0", "0"),
            ("0.01", "1"),
            ("0.00000000000000001", "0.0000000000000000001"),
        ] {
            c.push(
                format!("core-{index}-{absolute}-{percentage}"),
                "compare_core",
                vec![
                    ("current", source("current.csv", &current)),
                    ("prior", source("prior.csv", &prior)),
                    ("absolute", Json::str(absolute)),
                    ("percentage", Json::str(percentage)),
                ],
            );
        }
    }
    let header = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n";
    let prior = format!(
        "{header}\
         2026-06-30,Synthetic,Assets,100,Operating Bank,1000,0.00,0.00,100000.00,0.00\n\
         2026-06-30,Synthetic,Assets,777,New Clearing,1770,0.00,0.00,0.00,0.00\n\
         2026-06-30,Synthetic,Assets,888,Old Clearing,1880,0.00,0.00,50000.00,0.00\n\
         2026-06-30,Synthetic,Equity,900,Retained Earnings,3000,0.00,0.00,0.00,150000.00\n"
    );
    let current = format!(
        "{header}\
         2026-07-31,Synthetic,Assets,100,Operating Bank,1000,0.00,0.00,100000.00,0.00\n\
         2026-07-31,Synthetic,Assets,777,New Clearing,1770,900000.00,0.00,900000.00,0.00\n\
         2026-07-31,Synthetic,Assets,888,Old Clearing,1880,0.00,50000.00,0.00,0.00\n\
         2026-07-31,Synthetic,Equity,900,Retained Earnings,3000,0.00,850000.00,0.00,1000000.00\n"
    );
    let pair = |date: &str, amount: &str| {
        format!(
            "{header}\
             {date},Synthetic,Assets,110,Clearing,110,0.00,0.00,{amount},0.00\n\
             {date},Synthetic,Equity,900,Retained Earnings,900,0.00,0.00,0.00,{amount}\n"
        )
    };
    let mut cases = vec![
        ("first-funding", current, prior),
        (
            "nil-prior-ytd",
            pair("2026-07-31", "50000.00"),
            pair("2026-06-30", "0.00"),
        ),
    ];
    for (label, prior, current) in [
        ("april-may", "2026-04-30", "2026-05-31"),
        ("may-june", "2026-05-31", "2026-06-30"),
        ("december-january", "2025-12-31", "2026-01-31"),
    ] {
        cases.push((label, pair(current, "0.00"), pair(prior, "0.00")));
    }
    for (label, current, prior) in cases {
        c.push(
            format!("core-legacy-{label}"),
            "compare_core",
            vec![
                ("current", source("current.csv", current.as_bytes())),
                ("prior", source("prior.csv", prior.as_bytes())),
                ("absolute", Json::str("10000")),
                ("percentage", Json::str("0.10")),
            ],
        );
    }
}
fn notes(c: &mut Corpus) {
    let valid = probe::decode(
        r#"{"reviewer_initials":"RD","reviewed_on":"2026-03-31","comment":"Reviewed"}"#,
    )
    .unwrap();
    let replacements = [
        Json::Null,
        Json::Bool(false),
        Json::int(1),
        Json::str(""),
        Json::str(" "),
        Json::str("\u{202e}"),
        Json::Str(PyStr(vec![0xd800])),
        Json::str("Literal \u{10f7ff}\u{10f800}"),
        Json::str("longer-than-twelve-characters"),
        Json::str("line\nreview"),
        Json::str("20260331"),
    ];
    for key in ["reviewer_initials", "reviewed_on", "comment"] {
        for (index, value) in replacements.iter().enumerate() {
            let mut record = valid.clone();
            put(&mut record, key, value.clone());
            let bytes = probe::encode(&record);
            c.push(
                format!("note-{key}-{index}"),
                "review_note",
                vec![
                    ("path", Json::str("input.json")),
                    ("hex", Json::str(&hex(bytes.as_bytes()))),
                ],
            );
        }
    }
    for (index,value) in ["[]","null","{}","{",r#"{"x":1,"x":2}"#,r#"{"reviewer_initials":"RD","reviewed_on":"2026-03-31","comment":"first","comment":"second"}"#].iter().enumerate() {
        c.push(format!("note-structure-{index}"),"review_note",vec![("path",Json::str("input.json")),("hex",Json::str(&hex(value.as_bytes())))]);
    }
}
fn evidence(c: &mut Corpus, root: &Path) -> Result<()> {
    let mut bits = 0x3e60_0000_0000_0000u64;
    for index in 0..12000 {
        let value = f64::from_bits(bits);
        if value.is_finite() {
            c.push(
                format!("canonical-float-{index}"),
                "canonical_utf8",
                vec![("value", Json::Float(value))],
            );
        }
        bits = bits.wrapping_mul(6364136223846793005).wrapping_add(1);
    }
    for power in 0..=2046u64 {
        for sign in [0u64, 1u64 << 63] {
            let bits = (power << 52) | sign;
            for neighbour in [bits.saturating_sub(1), bits, bits + 1] {
                let value = f64::from_bits(neighbour);
                if value.is_finite() {
                    c.push(
                        format!("canonical-power-{neighbour}"),
                        "canonical_utf8",
                        vec![("value", Json::Float(value))],
                    );
                }
            }
        }
    }
    for file in [
        "calculation-evidence-coal-lsl-levy.json",
        "calculation-evidence-refused.json",
    ] {
        let base = probe::decode(&fs::read_to_string(root.join("examples").join(file))?)?;
        let path = "input.json";
        let original = probe::encode(&base);
        for value in [
            f64::from_bits(0x3e60_0000_0000_0000),
            f64::from_bits(0xbe60_0000_0000_0000),
            f64::from_bits(0x3e70_0000_0000_0000),
            f64::from_bits(0xbe70_0000_0000_0000),
        ] {
            let mut record = base.clone();
            let mut calculation = record.at("calculation").clone();
            calculation.set("metadata_float", Json::Float(value));
            let bytes = pyjson::canonical_utf8(&calculation)?;
            record.set("calculation", calculation);
            record.set(
                "calculation_sha256",
                Json::str(&monthly_close_control_plane::sha256(bytes.as_bytes())),
            );
            c.push(
                format!("evidence-float-{file}-{value}"),
                "calculation_evidence",
                vec![
                    ("path", Json::str(path)),
                    ("hex", Json::str(&hex(probe::encode(&record).as_bytes()))),
                ],
            );
        }
        c.push(
            format!("evidence-original-{file}"),
            "calculation_evidence",
            vec![
                ("path", Json::str(path)),
                ("hex", Json::str(&hex(original.as_bytes()))),
            ],
        );
        for (index, field) in [
            "label",
            "synthetic_input",
            "call",
            "provider",
            "engine",
            "normalised",
            "validation",
            "upstream",
        ]
        .iter()
        .enumerate()
        {
            for (j, value) in [
                Json::Null,
                Json::Bool(true),
                Json::int(1),
                Json::str(""),
                Json::str("bad\u{202e}"),
                Json::Array(vec![]),
            ]
            .iter()
            .enumerate()
            {
                let mut calculation = base.at("calculation").clone();
                put(&mut calculation, field, value.clone());
                let mut record = base.clone();
                put(&mut record, "calculation", calculation.clone());
                if let Ok(bytes) = monthly_close_control_plane::pyjson::canonical_utf8(&calculation)
                {
                    put(
                        &mut record,
                        "calculation_sha256",
                        Json::str(&monthly_close_control_plane::sha256(bytes.as_bytes())),
                    );
                }
                let encoded = probe::encode(&record);
                c.push(
                    format!("evidence-{file}-{index}-{j}"),
                    "calculation_evidence",
                    vec![
                        ("path", Json::str(path)),
                        ("hex", Json::str(&hex(encoded.as_bytes()))),
                    ],
                );
            }
        }
        for period in [
            "x:2026-03",
            "x:2026-02",
            "x:fy2026",
            "fbt:fy2026",
            "fy0000",
            "fy0001",
            "fy9999",
            "x:2026-13",
            "",
            "unknown",
        ] {
            let mut record = base.clone();
            let mut calculation = base.at("calculation").clone();
            let mut call = calculation.at("call").clone();
            put(&mut call, "period", Json::str(period));
            put(&mut calculation, "call", call);
            let bytes = monthly_close_control_plane::pyjson::canonical_utf8(&calculation).unwrap();
            put(
                &mut record,
                "calculation_sha256",
                Json::str(&monthly_close_control_plane::sha256(bytes.as_bytes())),
            );
            put(&mut record, "calculation", calculation);
            let encoded = probe::encode(&record);
            for date in [
                "2025-04-01",
                "2025-07-01",
                "2026-03-31",
                "2026-04-01",
                "2026-06-30",
                "2026-07-01",
            ] {
                c.push(
                    format!("period-{file}-{period}-{date}"),
                    "covers_period",
                    vec![
                        ("path", Json::str(path)),
                        ("hex", Json::str(&hex(encoded.as_bytes()))),
                        ("date", Json::str(date)),
                    ],
                );
            }
        }
    }
    for raw in [
        r#"{"text":"é𐐀\u007f\n","value":1.0}"#,
        r#"{"text":"\u10ff","a":[true,false,null,-0.0,1e-7]}"#,
    ] {
        c.push(
            format!("canonical-{raw}"),
            "canonical_utf8",
            vec![("value", probe::decode(raw)?)],
        );
    }
    Ok(())
}
fn equity(c: &mut Corpus, seed: u64) -> Result<()> {
    use monthly_close_control_plane::sha256;
    fn source(name: &str, bytes: &[u8]) -> Json {
        Json::object([("path", Json::str(name)), ("hex", Json::str(&hex(bytes)))])
    }
    fn tb(date: &str, amount: &str, section: &str) -> String {
        format!(
            "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n{date},Synthetic,{section},E,Equity,E,0,0,0,{amount}\n"
        )
    }
    let prior = tb("2026-08-31", "100", "Equity");
    let current = tb("2026-09-30", "125", "Equity");
    let movement = probe::decode(
        r#"{"movement_id":"m","date":"2026-09-01","account_id":"E","debit":"0","credit":"25","description":"Synthetic contribution","evidence_reference":"Fabricated record"}"#,
    )?;
    let mut base = probe::decode(
        r#"{"schema_version":1,"tenant":"Synthetic","opening_date":"2026-08-31","closing_date":"2026-09-30","currency":"AUD","basis":"ledger_equity","prior_source_sha256":"","current_source_sha256":"","equity_account_ids":["E"],"complete":true,"movements":[]}"#,
    )?;
    base.set("prior_source_sha256", Json::str(&sha256(prior.as_bytes())));
    base.set(
        "current_source_sha256",
        Json::str(&sha256(current.as_bytes())),
    );
    base.set("movements", Json::Array(vec![movement.clone()]));
    let push = |c: &mut Corpus,
                name: &str,
                op: &str,
                document: &Json,
                old: &str,
                new: &str,
                currency: Json,
                evidence: Json,
                tolerance: &str| {
        c.push(
            name,
            op,
            vec![
                ("path", Json::str("equity.json")),
                ("hex", Json::str(&hex(probe::encode(document).as_bytes()))),
                ("prior", source("prior.csv", old.as_bytes())),
                ("current", source("current.csv", new.as_bytes())),
                ("currency", currency),
                ("currency_evidence", evidence),
                ("tolerance", Json::str(tolerance)),
            ],
        );
    };
    let good_currency = || Json::str("AUD");
    let good_evidence = || Json::str("Synthetic report metadata");
    for (index, raw) in [
        "",
        " ",
        "{",
        "[",
        "}",
        "[1,]",
        "{\"a\":1,}",
        "{a:1}",
        "{\"a\" 1}",
        "{\"a\":}",
        "{\"a\":1 \"b\":2}",
        "[1 2]",
        "\"abc",
        "\"a\\",
        "\"a\n\"",
        "\"a\\x\"",
        "\"\\uZZZZ\"",
        "\"\\u12\"",
        "\"\\ud800\\uZZZZ\"",
        "{\n \"é\": [1,],\n}",
        "{} {}",
        "01",
        "-",
        "1e",
        "\u{feff}{}",
        "\u{feff}\u{feff}{}",
        "{\"x\":1,\"x\":2}",
        "{\"x\":{\"a\":1,\"a\":2}}",
    ]
    .iter()
    .enumerate()
    {
        c.push(
            format!("equity-json-error-{index}"),
            "equity_load",
            vec![
                ("path", Json::str("equity.json")),
                ("hex", Json::str(&hex(raw.as_bytes()))),
            ],
        );
    }
    push(
        c,
        "equity-balanced",
        "equity",
        &base,
        &prior,
        &current,
        good_currency(),
        good_evidence(),
        "0.01",
    );
    let values = [
        "null",
        "true",
        "false",
        "0",
        "1",
        "1.0",
        "[]",
        "{}",
        r#""""#,
        r#"" ""#,
        r#""text""#,
        r#""\u0000""#,
        r#""\ud800""#,
    ];
    for key in base.keys().unwrap() {
        let key = key.to_utf8().unwrap();
        for (index, raw) in values.iter().enumerate() {
            let mut document = base.clone();
            document.set(&key, probe::decode(raw)?);
            push(
                c,
                &format!("equity-field-{key}-{index}"),
                "equity_load",
                &document,
                &prior,
                &current,
                good_currency(),
                good_evidence(),
                "0.01",
            );
        }
    }
    for key in movement.keys().unwrap() {
        let key = key.to_utf8().unwrap();
        for (index, raw) in values.iter().enumerate() {
            let mut row = movement.clone();
            row.set(&key, probe::decode(raw)?);
            let mut document = base.clone();
            document.set("movements", Json::Array(vec![row]));
            push(
                c,
                &format!("equity-movement-{key}-{index}"),
                "equity_load",
                &document,
                &prior,
                &current,
                good_currency(),
                good_evidence(),
                "0.01",
            );
        }
    }
    for (index, date) in [
        "2026-08-31",
        "2026-10-01",
        "2026-09-31",
        "20260901",
        "2026W361",
        "2026-W36-1",
        "2026W36",
        "2026-W36",
        "2026W361xx",
        "2026W361éx",
        "2026-W36-9",
        "2026-9-01",
        "2020-W53-7",
        "2021-W53-7",
        "0001-W01-1",
        "9999-W52-7",
        "2026-09-01Z",
    ]
    .iter()
    .enumerate()
    {
        let mut row = movement.clone();
        row.set("date", Json::str(date));
        let mut document = base.clone();
        document.set("movements", Json::Array(vec![row]));
        push(
            c,
            &format!("equity-date-{index}"),
            "equity_load",
            &document,
            &prior,
            &current,
            good_currency(),
            good_evidence(),
            "0.01",
        );
    }
    for (index, value) in [
        "0",
        "0.0",
        "00.100",
        "-0",
        "+1",
        "1.",
        ".1",
        "1e2",
        "NaN",
        "Infinity",
        "１",
        "1_0",
        "1,000",
        "\n1",
        "99999999999999999999999999999.1",
        "99999999999999999999999999999999999999999999999999999999999999999999",
    ]
    .iter()
    .enumerate()
    {
        let mut row = movement.clone();
        row.set("credit", Json::str(value));
        let mut document = base.clone();
        document.set("movements", Json::Array(vec![row]));
        push(
            c,
            &format!("equity-money-{index}"),
            "equity",
            &document,
            &prior,
            &current,
            good_currency(),
            good_evidence(),
            "0.01",
        );
    }
    for (index, value) in [
        Json::Null,
        Json::str(""),
        Json::str("usd"),
        Json::str("USD"),
        Json::str("AUD"),
        Json::str("AUD\n"),
    ]
    .into_iter()
    .enumerate()
    {
        push(
            c,
            &format!("equity-currency-{index}"),
            "equity",
            &base,
            &prior,
            &current,
            value,
            good_evidence(),
            "0.01",
        );
    }
    for (index, value) in [
        Json::Null,
        Json::str(""),
        Json::str(" \u{1c}"),
        Json::str("a\nb"),
        Json::str("<b>& |*`\\'\"é"),
    ]
    .into_iter()
    .enumerate()
    {
        push(
            c,
            &format!("equity-currency-evidence-{index}"),
            "equity",
            &base,
            &prior,
            &current,
            good_currency(),
            value,
            "0.01",
        );
    }
    let mut rng = seed;
    for index in 0..200 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let opening = format!("{}.{:04}", rng % 100000, rng.rotate_left(19) % 10000);
        let closing = format!(
            "{}.{:04}",
            rng.rotate_left(31) % 100000,
            rng.rotate_left(7) % 10000
        );
        let old = tb("2026-08-31", &opening, "Equity");
        let new = tb(
            "2026-09-30",
            &closing,
            if index % 13 == 0 { "Assets" } else { "Equity" },
        );
        let mut document = base.clone();
        document.set("prior_source_sha256", Json::str(&sha256(old.as_bytes())));
        document.set("current_source_sha256", Json::str(&sha256(new.as_bytes())));
        if index % 11 == 0 {
            document.set("complete", Json::Bool(false));
        }
        push(
            c,
            &format!("equity-arithmetic-{index}"),
            "equity",
            &document,
            &old,
            &new,
            good_currency(),
            good_evidence(),
            if index % 3 == 0 { "1000" } else { "0.0001" },
        );
    }
    let mut result = probe::decode(
        r#"{"schema_version":1,"basis":"ledger_equity","tenant":"Synthetic","opening_date":"2026-08-31","closing_date":"2026-09-30","currency":"AUD","currency_evidence":"Synthetic report","schedule_sha256":"test","status":"PASS","issues":[],"accounts":[{"account_id":"E","movement_ids":[],"opening":"100","movement":"0","expected_close":"100","actual_close":"100","unexplained":"0"}],"total":{"opening":"100","movement":"0","expected_close":"100","actual_close":"100","unexplained":"0"},"movements":[],"tolerance":"0.01","complete":true}"#,
    )?;
    let sources = Json::object([("equity_schedule", Json::str("test"))]);
    let mut validate = |name: &str, value: Json| {
        c.push(
            name,
            "equity_validate",
            vec![
                ("result", value),
                ("sources", sources.clone()),
                ("exceptions", Json::Array(Vec::new())),
            ],
        )
    };
    validate("equity-validate-valid", result.clone());
    for key in result.keys().unwrap() {
        let key = key.to_utf8().unwrap();
        for (index, raw) in values.iter().enumerate() {
            let mut changed = result.clone();
            changed.set(&key, probe::decode(raw)?);
            validate(&format!("equity-validate-{key}-{index}"), changed);
        }
    }
    let account = result.at("accounts").as_array().unwrap()[0].clone();
    for key in account.keys().unwrap() {
        let key = key.to_utf8().unwrap();
        for (index, raw) in values
            .iter()
            .chain(
                [
                    r#""-0.0""#,
                    r#""101""#,
                    r#""1e2""#,
                    r#""99999999999999999999999999999.1""#,
                ]
                .iter(),
            )
            .enumerate()
        {
            let mut row = account.clone();
            row.set(&key, probe::decode(raw)?);
            let mut changed = result.clone();
            changed.set("accounts", Json::Array(vec![row]));
            validate(&format!("equity-validate-account-{key}-{index}"), changed);
        }
    }
    result.set("status", Json::str("BLOCKED"));
    result.set("issues", Json::Array(vec![Json::str("missing")]));
    result.set("accounts", Json::Array(Vec::new()));
    result.set("total", Json::Null);
    c.push(
        "equity-validate-blocked",
        "equity_validate",
        vec![
            ("result", result),
            ("sources", sources),
            (
                "exceptions",
                Json::Array(vec![Json::object([
                    ("control", Json::str("equity_reconciliation")),
                    ("status", Json::str("BLOCKED")),
                ])]),
            ),
        ],
    );
    Ok(())
}

fn reviews(c: &mut Corpus, root: &Path, seed: u64) -> Result<()> {
    fn source(name: &str, bytes: &[u8]) -> Json {
        Json::object([("path", Json::str(name)), ("hex", Json::str(&hex(bytes)))])
    }
    fn tb(date: &str, tenant: &str, a: &str, b: &str) -> String {
        format!(
            "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n{date},{tenant},Assets,A,Asset,100,0,0,{a},0\n{date},{tenant},Equity,E,Capital,900,0,0,0,{b}\n"
        )
    }
    let current = tb("2026-09-30", "Synthetic", "1500", "1500");
    let prior = tb("2026-08-31", "Synthetic", "1000", "1000");
    let base = vec![
        ("current", source("current.csv", current.as_bytes())),
        ("prior", source("prior.csv", prior.as_bytes())),
    ];
    let equity_tb = |date: &str, first: &str, second: &str, total: &str| {
        format!(
            "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n\
             {date},Synthetic,Equity,E,Account E,E,0,{first},0,{first}\n\
             {date},Synthetic,Equity,F,Account F,F,0,{second},0,{second}\n\
             {date},Synthetic,Assets,A,Account A,A,{total},0,{total},0\n"
        )
    };
    for (label, first, second, total, tolerance) in [
        ("nil-movement", "100", "100", "200", "0.01"),
        ("opposite-errors", "101", "99", "200", "0.01"),
        (
            "aggregate-tolerance",
            "100.009",
            "100.009",
            "200.018",
            "0.01",
        ),
        (
            "positive-at-tolerance",
            "100.001",
            "100",
            "200.001",
            "0.001",
        ),
        ("negative-at-tolerance", "99.999", "100", "199.999", "0.001"),
        (
            "positive-over-tolerance",
            "100.0011",
            "100",
            "200.0011",
            "0.001",
        ),
        (
            "negative-over-tolerance",
            "99.9989",
            "100",
            "199.9989",
            "0.001",
        ),
    ] {
        let old = equity_tb("2026-08-31", "100", "100", "200");
        let new = equity_tb("2026-09-30", first, second, total);
        let mut schedule = probe::decode(
            r#"{"schema_version":1,"tenant":"Synthetic","opening_date":"2026-08-31","closing_date":"2026-09-30","currency":"AUD","basis":"ledger_equity","equity_account_ids":["E","F"],"complete":true,"movements":[]}"#,
        )?;
        schedule.set(
            "prior_source_sha256",
            Json::str(&monthly_close_control_plane::sha256(old.as_bytes())),
        );
        schedule.set(
            "current_source_sha256",
            Json::str(&monthly_close_control_plane::sha256(new.as_bytes())),
        );
        c.push(
            format!("review-equity-legacy-{label}"),
            "review",
            vec![
                ("current", source("current.csv", new.as_bytes())),
                ("prior", source("prior.csv", old.as_bytes())),
                (
                    "equity_schedule",
                    source("equity.json", probe::encode(&schedule).as_bytes()),
                ),
                ("currency", Json::str("AUD")),
                (
                    "currency_evidence",
                    Json::str("Both fabricated source reports, metadata"),
                ),
                ("absolute", Json::str("999999999")),
                ("tolerance", Json::str(tolerance)),
            ],
        );
    }
    let debit_tb = |date: &str, amount: &str| {
        format!(
            "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit\n\
             {date},Synthetic,Equity,E,Debit Equity,E,{amount},0,{amount},0\n\
             {date},Synthetic,Assets,A,Asset,A,0,{amount},0,{amount}\n"
        )
    };
    let old = debit_tb("2026-08-31", "100");
    let new = debit_tb("2026-09-30", "125");
    let mut schedule = probe::decode(
        r#"{"schema_version":1,"tenant":"Synthetic","opening_date":"2026-08-31","closing_date":"2026-09-30","currency":"AUD","basis":"ledger_equity","equity_account_ids":["E"],"complete":true,"movements":[{"movement_id":"loss","date":"2026-09-30","account_id":"E","debit":"25","credit":"0","description":"Loss transfer","evidence_reference":"Fabricated independent closing record"}]}"#,
    )?;
    schedule.set(
        "prior_source_sha256",
        Json::str(&monthly_close_control_plane::sha256(old.as_bytes())),
    );
    schedule.set(
        "current_source_sha256",
        Json::str(&monthly_close_control_plane::sha256(new.as_bytes())),
    );
    c.push(
        "review-equity-legacy-debit-loss",
        "review",
        vec![
            ("current", source("current.csv", new.as_bytes())),
            ("prior", source("prior.csv", old.as_bytes())),
            (
                "equity_schedule",
                source("equity.json", probe::encode(&schedule).as_bytes()),
            ),
            ("currency", Json::str("AUD")),
            ("currency_evidence", Json::str("Fabricated source metadata")),
            ("absolute", Json::str("999999999")),
        ],
    );
    for (label, date, prior_date, tenant, account) in [
        ("later", "2026-10-31", "2026-09-30", "Synthetic", "A"),
        ("earlier", "2026-08-31", "2026-07-31", "Synthetic", "A"),
        ("other-tenant", "2026-09-30", "2026-08-31", "Other", "A"),
        ("population", "2026-09-30", "2026-08-31", "Synthetic", "X"),
    ] {
        c.push(
            format!("review-comparison-{label}"),
            "review",
            vec![
                (
                    "current",
                    source(
                        "current.csv",
                        tb(date, tenant, "1500", "1500")
                            .replace(",A,", &format!(",{account},"))
                            .as_bytes(),
                    ),
                ),
                (
                    "prior",
                    source(
                        "prior.csv",
                        tb(prior_date, tenant, "1000", "1000")
                            .replace(",A,", &format!(",{account},"))
                            .as_bytes(),
                    ),
                ),
            ],
        );
    }
    let sources = [
        ("mapping",source("mapping.csv",b"AccountID,ReviewGroup\nA,Cash\nE,Equity\n")),
        ("mapping_policy",source("mapping-policy.csv",b"Section,ReviewGroup\nAssets,Cash\nEquity,Equity\n")),
        ("balance_policy",source("balance-policy.csv",b"AccountID,ExpectedBalance,ExpectMovement\nA,debit,yes\nE,credit,no\n")),
        ("subledger",source("subledger.csv",b"Tenant,AccountID,SubledgerBalance\nSynthetic,A,1499.999\nSynthetic,E,-1500\nSynthetic,Missing,1\n")),
        ("acknowledgement",source("review.json",br#"{"reviewer_initials":"RD","reviewed_on":"2026-09-30","comment":"Synthetic review"}"#)),
    ];
    for mask in 0..32 {
        let mut pairs = base.clone();
        pairs.extend(
            sources
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, v)| v.clone()),
        );
        c.push(format!("review-input-combination-{mask}"), "review", pairs);
    }
    for (key, values) in [
        (
            "absolute",
            vec![
                "0", "0.00", "500", "499.9999", "-1", "NaN", "Infinity", "sNaN",
            ],
        ),
        (
            "percentage",
            vec![
                "0",
                "0.10",
                "0.5",
                "0.4999999999999999",
                "1",
                "1.0001",
                "10",
                "-0.1",
                "NaN",
                "Infinity",
            ],
        ),
        (
            "tolerance",
            vec![
                "0", "0.001", "0.0009", "0.01", "100", "-1", "NaN", "Infinity",
            ],
        ),
    ] {
        for (index, value) in values.iter().enumerate() {
            let mut pairs = base.clone();
            pairs.extend(sources.iter().cloned());
            pairs.push((key, Json::str(value)));
            c.push(format!("review-config-{key}-{index}"), "review", pairs);
        }
    }
    for (index, required) in [
        vec![],
        vec!["payroll"],
        vec!["payroll", "payroll"],
        vec![""],
        vec!["Tax"],
        vec!["pay--roll"],
        vec!["-tax"],
        vec!["tax-"],
        vec!["payroll", "a\nb"],
        vec!["é"],
    ]
    .into_iter()
    .enumerate()
    {
        let mut pairs = base.clone();
        pairs.push((
            "required",
            Json::Array(required.into_iter().map(Json::str).collect()),
        ));
        c.push(format!("review-required-{index}"), "review", pairs);
    }
    for (index, date) in [
        "2026-08-31",
        "2026-09-30",
        "2026-10-31",
        "2026-06-30",
        "2025-09-30",
    ]
    .iter()
    .enumerate()
    {
        let mut pairs = base.clone();
        pairs[1].1 = source(
            "prior.csv",
            tb(date, "Synthetic", "1000", "1000").as_bytes(),
        );
        c.push(format!("review-report-dates-{index}"), "review", pairs);
    }
    for (index, raw) in [
        b"AccountID,ReviewGroup\n".as_slice(),
        b"AccountID,ReviewGroup\nE,Assets\n",
        b"AccountID,ReviewGroup\nA,Cash\nMissing,Cash\n",
        b"AccountID,ReviewGroup\nA,Odd\nE,Odd\n",
    ]
    .iter()
    .enumerate()
    {
        let mut pairs = base.clone();
        pairs.push(("mapping", source("mapping.csv", raw)));
        pairs.push(sources[1].clone());
        c.push(format!("review-mapping-{index}"), "review", pairs);
    }
    for (index, raw) in [
        b"AccountID,ExpectedBalance,ExpectMovement\n".as_slice(),
        b"AccountID,ExpectedBalance,ExpectMovement\nA,credit,yes\nE,debit,yes\nMissing,nil,yes\n",
        b"AccountID,ExpectedBalance,ExpectMovement\nA,nil,no\nE,nil,no\n",
    ]
    .iter()
    .enumerate()
    {
        let mut pairs = base.clone();
        pairs.push(("balance_policy", source("balance-policy.csv", raw)));
        c.push(format!("review-balance-policy-{index}"), "review", pairs);
    }
    for (index, raw) in [
        b"Tenant,AccountID,SubledgerBalance\n".as_slice(),
        b"Tenant,AccountID,SubledgerBalance\nOther,A,1500\n",
        b"Tenant,AccountID,SubledgerBalance\nSynthetic,A,1400\n",
    ]
    .iter()
    .enumerate()
    {
        let mut pairs = base.clone();
        pairs.push(("subledger", source("subledger.csv", raw)));
        c.push(format!("review-subledger-{index}"), "review", pairs);
    }
    for when in ["2026-09-29", "2026-09-30", "2026-10-01"] {
        let mut pairs = base.clone();
        pairs.push(("acknowledgement",source("review.json",format!(r#"{{"reviewer_initials":"RD","reviewed_on":"{when}","comment":"Synthetic review"}}"#).as_bytes())));
        c.push(format!("review-ack-{when}"), "review", pairs);
    }
    for (index, unicode) in [
        "é😀",
        "\u{10f7ff}",
        "\u{10f7ff}\u{10f800}",
        "literal |*`\\ account",
    ]
    .iter()
    .enumerate()
    {
        let new = tb("2026-09-30", unicode, "1500", "1500");
        let old = tb("2026-08-31", unicode, "1000", "1000");
        let note = Json::object([
            ("reviewer_initials", Json::str("RD")),
            ("reviewed_on", Json::str("2026-09-30")),
            (
                "comment",
                Json::str(&format!("Literal {unicode}\n# Review\nMore |*`\\ text")),
            ),
        ]);
        c.push(
            format!("review-unicode-{index}"),
            "review",
            vec![
                ("current", source("current.csv", new.as_bytes())),
                ("prior", source("prior.csv", old.as_bytes())),
                (
                    "acknowledgement",
                    source("note.json", probe::encode(&note).as_bytes()),
                ),
                ("absolute", Json::str("0")),
                ("percentage", Json::str("0")),
            ],
        );
    }
    for (index, description) in [
        Json::str("Synthetic equity |*`\\ <b>& 'é😀"),
        Json::str("\u{10f7ff}\u{10f800}"),
        probe::decode(r#""\ud800""#)?,
        probe::decode(r#""\ud800\udfff""#)?,
    ]
    .into_iter()
    .enumerate()
    {
        let mut schedule = probe::decode(
            r#"{"schema_version":1,"tenant":"Synthetic","opening_date":"2026-08-31","closing_date":"2026-09-30","currency":"AUD","basis":"ledger_equity","prior_source_sha256":"","current_source_sha256":"","equity_account_ids":["E"],"complete":true,"movements":[]}"#,
        )?;
        schedule.set(
            "prior_source_sha256",
            Json::str(&monthly_close_control_plane::sha256(prior.as_bytes())),
        );
        schedule.set(
            "current_source_sha256",
            Json::str(&monthly_close_control_plane::sha256(current.as_bytes())),
        );
        let mut movement = probe::decode(
            r#"{"movement_id":"m","date":"2026-09-30","account_id":"E","debit":"0","credit":"500","description":"synthetic","evidence_reference":"Synthetic record"}"#,
        )?;
        movement.set("description", description);
        schedule.set("movements", Json::Array(vec![movement]));
        let mut pairs = base.clone();
        pairs.push((
            "equity_schedule",
            source("equity.json", probe::encode(&schedule).as_bytes()),
        ));
        pairs.push(("currency", Json::str("AUD")));
        pairs.push(("currency_evidence", Json::str("Synthetic TB metadata")));
        c.push(format!("review-equity-{index}"), "review", pairs);
        if index == 0 {
            let mut invalid = schedule.clone();
            let mut movements = invalid.at("movements").as_array().unwrap().to_vec();
            movements[0].set("credit", Json::str("NaN"));
            invalid.set("movements", Json::Array(movements));
            let mut pairs = base.clone();
            pairs.push((
                "equity_schedule",
                source("equity.json", probe::encode(&invalid).as_bytes()),
            ));
            pairs.push(("currency", Json::str("AUD")));
            pairs.push(("currency_evidence", Json::str("Synthetic TB metadata")));
            c.push("review-equity-malformed", "review", pairs);
        }
    }
    let mut rng = seed;
    for index in 0..300 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let amount = format!("{}.{:04}", rng % 100000, rng.rotate_left(21) % 10000);
        let previous = format!(
            "{}.{:04}",
            rng.rotate_left(31) % 100000,
            rng.rotate_left(17) % 10000
        );
        let new = tb(
            "2026-09-30",
            "Synthetic",
            &amount,
            if index % 23 == 0 { "0" } else { &amount },
        );
        let old = tb(
            if index % 19 == 0 {
                "2026-06-30"
            } else {
                "2026-08-31"
            },
            if index % 29 == 0 {
                "Other"
            } else {
                "Synthetic"
            },
            &previous,
            &previous,
        );
        let mut pairs = vec![
            ("current", source("current.csv", new.as_bytes())),
            ("prior", source("prior.csv", old.as_bytes())),
        ];
        pairs.extend(
            sources
                .iter()
                .enumerate()
                .filter(|(i, _)| rng & (1 << i) != 0)
                .map(|(_, v)| v.clone()),
        );
        c.push(format!("review-generated-{index}"), "review", pairs);
    }
    for example in [
        "calculation-evidence-coal-lsl-levy.json",
        "calculation-evidence-refused.json",
    ] {
        let data = fs::read(root.join("examples").join(example))?;
        for count in [1, 2] {
            let mut pairs = base.clone();
            pairs.push((
                "evidence",
                Json::Array(
                    (0..count)
                        .map(|i| source(&format!("evidence-{i}.json"), &data))
                        .collect(),
                ),
            ));
            pairs.push((
                "required",
                Json::Array(vec![Json::str("payroll"), Json::str("absent")]),
            ));
            c.push(
                format!("review-evidence-{example}-{count}"),
                "review",
                pairs,
            );
        }
    }
    let data = fs::read(root.join("examples/calculation-evidence-coal-lsl-levy.json"))?;
    for (label, accepted, refused) in [
        ("false", Json::Bool(false), false),
        ("string-false", Json::str("false"), false),
        ("refused-false", Json::Bool(false), true),
    ] {
        let mut document = probe::decode(std::str::from_utf8(&data)?)?;
        let mut calculation = document.at("calculation").clone();
        let mut validation = calculation.at("validation").clone();
        validation.set("accepted", accepted);
        if refused {
            validation.set("findings", Json::Array(vec![]));
            let mut call = calculation.at("call").clone();
            call.set("status", Json::str("UPSTREAM_REFUSED"));
            calculation.set("call", call);
            let mut normalised = calculation.at("normalised").clone();
            normalised.set("values", Json::Object(vec![]));
            calculation.set("normalised", normalised);
        }
        calculation.set("validation", validation);
        let canonical = pyjson::canonical_utf8(&calculation)?;
        document.set(
            "calculation_sha256",
            Json::str(&monthly_close_control_plane::sha256(canonical.as_bytes())),
        );
        document.set("calculation", calculation);
        let mut pairs = base.clone();
        pairs.push((
            "evidence",
            Json::Array(vec![source(
                "evidence.json",
                probe::encode(&document).as_bytes(),
            )]),
        ));
        pairs.push(("required", Json::Array(vec![Json::str("coal-lsl-levy")])));
        c.push(
            format!("review-evidence-acceptance-{label}"),
            "review",
            pairs,
        );
    }
    Ok(())
}

fn reconciliation(c: &mut Corpus, seed: u64) -> Result<()> {
    use monthly_close_control_plane::{pycsv, reconciliation::COLUMNS};
    fn source(name: &str, bytes: &[u8]) -> Json {
        Json::object([("path", Json::str(name)), ("hex", Json::str(&hex(bytes)))])
    }
    fn csv(rows: &[Vec<String>]) -> String {
        let mut output = pycsv::write_record(&COLUMNS);
        for row in rows {
            output.push_str(&pycsv::write_record(
                &row.iter().map(String::as_str).collect::<Vec<_>>(),
            ));
        }
        output
    }
    let row = |id: &str, date: &str, debit: &str, credit: &str| {
        vec![
            "Synthetic".into(),
            "CLEAR".into(),
            "AUD".into(),
            id.into(),
            date.into(),
            "Shared".into(),
            "Synthetic transaction".into(),
            debit.into(),
            credit.into(),
        ]
    };
    let rows = vec![
        row("T1", "2026-09-01", "100", "0"),
        row("T2", "2026-09-02", "0", "100"),
    ];
    let base = Json::object([
        (
            "transactions",
            source("transactions.csv", csv(&rows).as_bytes()),
        ),
        ("tenant", Json::str("Synthetic")),
        ("account_id", Json::str("CLEAR")),
        ("currency", Json::str("AUD")),
        ("period_start", Json::str("2026-09-01")),
        ("period_end", Json::str("2026-09-30")),
        ("opening_balance", Json::str("0")),
        ("closing_balance", Json::str("0")),
    ]);
    let push = |c: &mut Corpus, name: &str, request: Json| {
        let Json::Object(pairs) = request else {
            panic!("fixture")
        };
        c.push(
            name,
            "reconcile",
            pairs
                .into_iter()
                .map(|(k, v)| {
                    let key = match k.to_utf8().unwrap().as_str() {
                        "transactions" => "transactions",
                        "tenant" => "tenant",
                        "account_id" => "account_id",
                        "currency" => "currency",
                        "period_start" => "period_start",
                        "period_end" => "period_end",
                        "opening_balance" => "opening_balance",
                        "closing_balance" => "closing_balance",
                        "opening_items" => "opening_items",
                        "decisions" => "decisions",
                        "decisions_escaped" => "decisions_escaped",
                        _ => unreachable!(),
                    };
                    (key, v)
                })
                .collect(),
        );
    };
    push(c, "reconcile-suggestion-is-not-acceptance", base.clone());
    for (name, amounts, closing, decisions) in [
        (
            "split-exact",
            vec![("60", "0"), ("40", "0"), ("0", "100")],
            "0",
            None,
        ),
        (
            "split-cent-difference",
            vec![("60", "0"), ("40", "0"), ("0", "100.01")],
            "-0.01",
            None,
        ),
        (
            "duplicate-looking",
            vec![("100", "0"), ("100", "0"), ("0", "100")],
            "100",
            None,
        ),
        (
            "competing-allocations",
            vec![("100", "0"), ("100", "0"), ("0", "100")],
            "100",
            Some(
                "Group,TransactionID,Decision,Note\nA,T1,accept,Checked\nA,T3,accept,Checked\nB,T2,accept,Checked\nB,T3,accept,Checked\n",
            ),
        ),
        (
            "many-to-many",
            vec![("60", "0"), ("40", "0"), ("0", "30"), ("0", "70")],
            "0",
            Some(
                "Group,TransactionID,Decision,Note\nA,T1,accept,Checked\nA,T2,accept,Checked\nA,T3,accept,Checked\nA,T4,accept,Checked\n",
            ),
        ),
        (
            "pending-many",
            vec![("60", "0"), ("40", "0"), ("0", "100")],
            "0",
            Some(
                "Group,TransactionID,Decision,Note\nA,T1,,Pending\nA,T2,,Pending\nA,T3,,Pending\n",
            ),
        ),
    ] {
        let mut changed = base.clone();
        let rows: Vec<_> = amounts
            .iter()
            .enumerate()
            .map(|(i, (debit, credit))| row(&format!("T{}", i + 1), "2026-09-01", debit, credit))
            .collect();
        changed.set(
            "transactions",
            source("transactions.csv", csv(&rows).as_bytes()),
        );
        changed.set("closing_balance", Json::str(closing));
        if let Some(decisions) = decisions {
            changed.set("decisions", source("decisions.csv", decisions.as_bytes()));
        }
        push(c, &format!("reconcile-{name}"), changed);
    }
    for (key, values) in [
        ("currency", vec!["aud", "AU", "AUDD", "USD", " AUD "]),
        ("tenant", vec!["Other", " Synthetic ", "", "a\nb"]),
        ("account_id", vec!["Other", " CLEAR ", "", "=CLEAR"]),
        (
            "period_start",
            vec![
                "2026-10-01",
                "2026-09-30",
                "20260901",
                "2026-09-00",
                "2026-09-31",
                "0000-09-01",
                "2026-00-01",
                "2026-13-01",
            ],
        ),
        (
            "period_end",
            vec![
                "2026-08-31",
                "2026-09-01",
                "2026-02-29",
                "2026-09-31",
                "2026-09-30\n",
            ],
        ),
        (
            "opening_balance",
            vec![
                "1",
                "-1",
                "-0.00",
                "0.001",
                "1000000000000000",
                "999999999999999.99",
                "NaN",
            ],
        ),
        (
            "closing_balance",
            vec!["1", "-1", "-0.0000", ".00", "0.001", "1000000000000000"],
        ),
    ] {
        for (index, value) in values.into_iter().enumerate() {
            let mut changed = base.clone();
            changed.set(key, Json::str(value));
            push(c, &format!("reconcile-config-{key}-{index}"), changed);
        }
    }
    for value in [
        "2026-09-31",
        "0000-01-01",
        "2026-00-01",
        "2026-13-01",
        "2026-01-00",
        "2026-02-29",
        "1900-02-29",
        "2000-02-29",
        "9999-12-31",
        "20260930",
        "2026-W40-3",
        "２０２６-09-30",
    ] {
        c.scalar(
            &format!("reconcile-date-{value}"),
            "reconciliation_date",
            value,
        );
    }
    for value in [
        "0",
        "-0.000000",
        "1.00",
        "1.001",
        "999999999999999.99",
        "1000000000000000",
        "-1000000000000000",
        "１２.３４",
        "$1,000.00",
        "NaN",
        "1e2",
        ".1",
    ] {
        c.scalar(
            &format!("reconcile-money-{value}"),
            "reconciliation_money",
            value,
        );
    }
    for (field, values) in [
        (
            3,
            vec![
                "T1",
                "",
                "a/b",
                "_first",
                "é",
                &"A".repeat(80),
                &"A".repeat(81),
            ],
        ),
        (
            4,
            vec!["2026-08-31", "2026-10-01", "2026-09-31", "20260901"],
        ),
        (5, vec!["", "=formula", "\u{10f7ff}\u{10f800}", "a\nb"]),
        (7, vec!["0", "-1", "0.001", "1000000000000000", "1.00"]),
        (8, vec!["1", "-1", "0.001"]),
    ] {
        for (index, value) in values.into_iter().enumerate() {
            let mut changed_rows = rows.clone();
            changed_rows[0][field] = value.into();
            let mut changed = base.clone();
            changed.set(
                "transactions",
                source("transactions.csv", csv(&changed_rows).as_bytes()),
            );
            push(c, &format!("reconcile-row-{field}-{index}"), changed);
        }
    }
    for (index, decision) in ["", "accept", "reject", "yes"].iter().enumerate() {
        let mut changed = base.clone();
        changed.set("decisions",source("decisions.csv",format!("Group,TransactionID,Decision,Note\nG,T1,{decision},Synthetic review\nG,T2,{decision},Synthetic review\n").as_bytes()));
        push(c, &format!("reconcile-decision-{index}"), changed);
    }
    for (index, raw) in [
        "Group,TransactionID,Decision,Note\nG,T1,accept,\nG,T2,accept,\n",
        "Group,TransactionID,Decision,Note\nG,T1,accept,note\n",
        "Group,TransactionID,Decision,Note\nG,T1,accept,note\nG,T2,reject,note\n",
        "Group,TransactionID,Decision,Note\nG,T1,,note\nG,T1,,note\n",
        "Group,TransactionID,Decision,Note\nG,absent,,note\nG,T2,,note\n",
        "Group,TransactionID,Decision,Note\n'=G,T1,accept,'=note\n'=G,T2,accept,'=note\n",
    ]
    .iter()
    .enumerate()
    {
        for escaped in [false, true] {
            let mut changed = base.clone();
            changed.set("decisions", source("decisions.csv", raw.as_bytes()));
            changed.set("decisions_escaped", Json::Bool(escaped));
            push(
                c,
                &format!("reconcile-decisions-invalid-{index}-{escaped}"),
                changed,
            );
        }
    }
    let mut escaped = base.clone();
    escaped.set("decisions_escaped", Json::Bool(true));
    push(c, "reconcile-escaped-without-file", escaped);
    let mut rng = seed;
    for index in 0..200 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let amount = format!("{}.{:02}", rng % 100000, rng.rotate_left(11) % 100);
        let mut changed = base.clone();
        let mut current = vec![
            row("T1", "2026-09-01", &amount, "0"),
            row("T2", "2026-09-30", "0", &amount),
        ];
        if index % 7 == 0 {
            current[1][5] = "Other reference".into();
        }
        if index % 13 == 0 {
            current[1][3] = "T1".into();
        }
        changed.set(
            "transactions",
            source("transactions.csv", csv(&current).as_bytes()),
        );
        if index % 3 == 0 {
            changed.set("decisions",source("decisions.csv",b"Group,TransactionID,Decision,Note\nG,T1,accept,Reviewed\nG,T2,accept,Reviewed\n"));
        }
        push(c, &format!("reconcile-generated-{index}"), changed);
    }
    let mut old_row = row("O1", "2026-08-30", "0", "100");
    old_row[5] = "Shared".into();
    let opening_item = Json::Object(
        COLUMNS
            .iter()
            .zip(&old_row)
            .map(|(k, v)| (PyStr::from(*k), Json::str(v)))
            .collect(),
    );
    let opening = Json::object([
        ("schema", Json::str("clearing-carry-v1")),
        (
            "identity",
            Json::object([
                ("Tenant", Json::str("Synthetic")),
                ("AccountID", Json::str("CLEAR")),
                ("Currency", Json::str("AUD")),
            ]),
        ),
        ("period_end", Json::str("2026-08-31")),
        ("balance", Json::str("-100.00")),
        ("items", Json::Array(vec![opening_item])),
        (
            "notes",
            Json::object([("O1", Json::str("Previous review"))]),
        ),
    ]);
    let mut carry_base = base.clone();
    carry_base.set("opening_balance", Json::str("-100"));
    carry_base.set(
        "transactions",
        source("transactions.csv", csv(&[rows[0].clone()]).as_bytes()),
    );
    carry_base.set(
        "opening_items",
        source("opening.json", probe::encode(&opening).as_bytes()),
    );
    push(c, "reconcile-opening-carry", carry_base.clone());
    for (key, values) in [
        ("schema", vec!["null", r#""wrong""#]),
        ("identity", vec!["null", "{}"]),
        (
            "period_end",
            vec![
                r#""2026-08-30""#,
                r#""2026-09-01""#,
                r#""20260831""#,
                r#""\ud800""#,
            ],
        ),
        (
            "balance",
            vec![r#""-99.99""#, r#""NaN""#, r#""\ud800""#, "0"],
        ),
        ("items", vec!["null", "[]", "[0]"]),
        (
            "notes",
            vec![
                "null",
                r#"{"unknown":"review"}"#,
                r#"{"O1":""}"#,
                r#"{"O1":"\ud800"}"#,
            ],
        ),
    ] {
        for (index, raw) in values.into_iter().enumerate() {
            let mut document = opening.clone();
            document.set(key, probe::decode(raw)?);
            let mut changed = carry_base.clone();
            changed.set(
                "opening_items",
                source("opening.json", probe::encode(&document).as_bytes()),
            );
            push(c, &format!("reconcile-opening-{key}-{index}"), changed);
        }
    }
    let mut accepted = carry_base.clone();
    accepted.set(
        "decisions",
        source(
            "decisions.csv",
            b"Group,TransactionID,Decision,Note\nG,O1,accept,Reviewed\nG,T1,accept,Reviewed\n",
        ),
    );
    push(c, "reconcile-opening-accepted", accepted);
    let mut note_order = base.clone();
    let mut reversed = rows.clone();
    reversed[0][3] = "T2".into();
    reversed[1][3] = "T1".into();
    reversed[0][6] = "<script>alert('synthetic')</script> & \"quoted\"".into();
    note_order.set(
        "transactions",
        source("transactions.csv", csv(&reversed).as_bytes()),
    );
    note_order.set(
        "decisions",
        source(
            "decisions.csv",
            b"Group,TransactionID,Decision,Note\nG,T1,reject,Reviewed\nG,T2,reject,Reviewed\n",
        ),
    );
    push(
        c,
        "reconcile-note-insertion-order-and-html-escaping",
        note_order,
    );
    Ok(())
}

fn schedules(c: &mut Corpus, root: &Path, seed: u64) -> Result<()> {
    let values = [
        "null", "true", "false", "0", "1", "1.0", r#""""#, r#"" ""#, r#""1""#, r#""\n""#, r#""é""#,
        "[]", "{}",
    ];
    fn push(c: &mut Corpus, name: &str, kind: &str, document: &Json) {
        c.push(
            name,
            "schedule",
            vec![
                ("path", Json::str("schedule.json")),
                ("kind", Json::str(kind)),
                ("hex", Json::str(&hex(probe::encode(document).as_bytes()))),
            ],
        );
    }
    for kind in ["expenses", "migration", "interentity"] {
        let base = probe::decode(&fs::read_to_string(
            root.join("examples/schedules").join(format!("{kind}.json")),
        )?)?;
        push(c, &format!("schedule-{kind}-example"), kind, &base);
        match kind {
            "expenses" => {
                let mut document = base.clone();
                let mut payment = base.at("payments").as_array().unwrap()[0].clone();
                payment.set("amount", Json::str("100.00"));
                document.set("payments", Json::Array(vec![payment]));
                document.set("ledger_balance", Json::str("0"));
                push(c, "schedule-legacy-expenses-pass", kind, &document);
            }
            "migration" => {
                let mut document = base.clone();
                document.set(
                    "before",
                    Json::Array(vec![base.at("before").as_array().unwrap()[0].clone()]),
                );
                let mut first_mapping = base.at("mapping").as_array().unwrap()[0].clone();
                first_mapping.set("weight", Json::str("0.4"));
                let mut second_mapping = first_mapping.clone();
                second_mapping.set("mapping_id", Json::str("M2"));
                second_mapping.set("target_account", Json::str("OTHER"));
                second_mapping.set("weight", Json::str("0.6"));
                document.set("mapping", Json::Array(vec![first_mapping, second_mapping]));
                let mut first_after = base.at("after").as_array().unwrap()[0].clone();
                first_after.set("amount", Json::str("40"));
                let mut second_after = first_after.clone();
                second_after.set("row_id", Json::str("split"));
                second_after.set("account", Json::str("OTHER"));
                second_after.set("amount", Json::str("60"));
                document.set("after", Json::Array(vec![first_after, second_after]));
                push(c, "schedule-legacy-migration-split", kind, &document);
            }
            "interentity" => {
                for (label, reference, disputed) in [
                    ("agrees", "LOAN-1", false),
                    ("unmatched", "OTHER", false),
                    ("disputed", "LOAN-1", true),
                ] {
                    let mut document = base.clone();
                    let mut records = base.at("records").as_array().unwrap().to_vec();
                    records[1].set("amount", Json::str("-50000"));
                    records[1].set("reference", Json::str(reference));
                    records[1].set("disputed", Json::Bool(disputed));
                    document.set("records", Json::Array(records));
                    push(
                        c,
                        &format!("schedule-legacy-interentity-{label}"),
                        kind,
                        &document,
                    );
                }
            }
            _ => unreachable!(),
        }
        for key in base.keys().unwrap() {
            let key = key.to_utf8().unwrap();
            for (index, raw) in values.iter().enumerate() {
                let mut document = base.clone();
                document.set(&key, probe::decode(raw)?);
                push(
                    c,
                    &format!("schedule-{kind}-field-{key}-{index}"),
                    kind,
                    &document,
                );
            }
        }
        for key in base.keys().unwrap() {
            let key = key.to_utf8().unwrap();
            let Some(rows) = base.at(&key).as_array() else {
                continue;
            };
            let Some(first) = rows.first().filter(|v| matches!(v, Json::Object(_))) else {
                continue;
            };
            for field in first.keys().unwrap() {
                let field = field.to_utf8().unwrap();
                for (index, raw) in values.iter().enumerate() {
                    let mut document = base.clone();
                    let mut changed = rows.to_vec();
                    changed[0].set(&field, probe::decode(raw)?);
                    document.set(&key, Json::Array(changed));
                    push(
                        c,
                        &format!("schedule-{kind}-{key}-{field}-{index}"),
                        kind,
                        &document,
                    );
                }
            }
            let mut duplicate = base.clone();
            let mut changed = rows.to_vec();
            changed.push(first.clone());
            duplicate.set(&key, Json::Array(changed));
            push(
                c,
                &format!("schedule-{kind}-{key}-duplicate"),
                kind,
                &duplicate,
            );
            for (field, raw) in [
                ("amount", "-0.00"),
                ("amount", "1234567890123456.99"),
                ("amount", "１２.３４"),
                ("amount", "12345678901234567"),
                ("amount", "1.001"),
                ("amount", "+1"),
                ("amount", "1e2"),
                ("date", "2026-10-01"),
                ("date", "20260930"),
                ("date", "2026-W40-3"),
                ("reference", "\u{10f7ff}\u{10f800}"),
                ("weight", "0.5"),
                ("weight", "0.000001"),
                ("weight", "0.٠١"),
                ("weight", "1.000000"),
                ("weight", "0.0000000"),
                ("weight", "-1"),
            ] {
                if first.get(field).is_none() {
                    continue;
                }
                let mut document = base.clone();
                let mut changed = rows.to_vec();
                changed[0].set(field, Json::str(raw));
                document.set(&key, Json::Array(changed));
                push(
                    c,
                    &format!("schedule-{kind}-{key}-{field}-{raw:?}"),
                    kind,
                    &document,
                );
            }
        }
        if kind == "expenses" {
            let mut rng = seed;
            for index in 0..200 {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                let mut document = base.clone();
                let mut claims = base.at("claims").as_array().unwrap().to_vec();
                claims[0].set(
                    "amount",
                    Json::str(&format!("{}.{:02}", rng % 200, rng.rotate_left(7) % 100)),
                );
                claims[0].set("approved", Json::Bool(index % 3 != 0));
                if index % 7 == 0 {
                    claims[0].set("invoice_id", Json::str("absent"));
                }
                document.set("claims", Json::Array(claims));
                let mut payments = base.at("payments").as_array().unwrap().to_vec();
                payments[0].set(
                    "amount",
                    Json::str(&format!(
                        "{}.{:02}",
                        rng.rotate_left(17) % 100,
                        rng.rotate_left(31) % 100
                    )),
                );
                if index % 11 == 0 {
                    payments[0].set("claim_id", Json::str("absent"));
                }
                document.set("payments", Json::Array(payments));
                document.set(
                    "ledger_balance",
                    Json::str(&format!(
                        "-{}.{:02}",
                        rng.rotate_left(23) % 50,
                        rng.rotate_left(11) % 100
                    )),
                );
                push(
                    c,
                    &format!("schedule-expenses-generated-{index}"),
                    kind,
                    &document,
                );
            }
            let mut duplicates = base.clone();
            let mut invoices = base.at("invoices").as_array().unwrap().to_vec();
            for (index, supplier) in ["Z supplier", "A supplier", "Z supplier", "A supplier"]
                .iter()
                .enumerate()
            {
                let mut row = invoices[0].clone();
                row.set("invoice_id", Json::str(&format!("I{}", index + 2)));
                row.set("supplier", Json::str(supplier));
                invoices.push(row);
            }
            duplicates.set("invoices", Json::Array(invoices));
            push(c, "schedule-expenses-reference-order", kind, &duplicates);
        }
    }
    for (index, raw) in [
        "",
        "{",
        "[1,]",
        "{} {}",
        "{\"x\":1,\"x\":2}",
        "{\"x\":{\"a\":1,\"a\":2}}",
        "\u{feff}{}",
        "\u{feff}\u{feff}{}",
    ]
    .iter()
    .enumerate()
    {
        c.push(
            format!("schedule-json-{index}"),
            "schedule",
            vec![
                ("path", Json::str("schedule.json")),
                ("kind", Json::str("expenses")),
                ("hex", Json::str(&hex(raw.as_bytes()))),
            ],
        );
    }
    Ok(())
}

fn classification_cases(c: &mut Corpus, seed: u64) -> Result<()> {
    use monthly_close_control_plane::{pycsv, reconciliation::COLUMNS, sha256};
    fn csv(rows: &[Vec<String>]) -> String {
        std::iter::once(COLUMNS.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
            .chain(rows.iter().cloned())
            .map(|row| pycsv::write_record(&row.iter().map(String::as_str).collect::<Vec<_>>()))
            .collect()
    }
    fn source(name: &str, bytes: &[u8]) -> Json {
        Json::object([("path", Json::str(name)), ("hex", Json::str(&hex(bytes)))])
    }
    fn document(before: &str, after: &str, policies: Vec<Json>) -> Json {
        Json::object([
            ("schema_version", Json::int(1)),
            ("tenant", Json::str("Synthetic")),
            ("currency", Json::str("AUD")),
            ("period_start", Json::str("2026-09-01")),
            ("period_end", Json::str("2026-09-30")),
            ("original_sha256", Json::str(&sha256(before.as_bytes()))),
            ("current_sha256", Json::str(&sha256(after.as_bytes()))),
            ("expectations", Json::Array(policies)),
        ])
    }
    fn policy(original: &str, current: &str) -> Json {
        Json::object([
            ("original_account_id", Json::str(original)),
            ("transaction_id", Json::str("T1")),
            ("current_account_id", Json::str(current)),
            ("expected_account_id", Json::str(current)),
            ("purpose", Json::str("Synthetic coding")),
            ("evidence_reference", Json::str("Synthetic receipt")),
        ])
    }
    fn push(c: &mut Corpus, name: &str, before: &str, after: &str, doc: &str, currency: &str) {
        c.push(
            name,
            "classify",
            vec![
                ("original", source("original.csv", before.as_bytes())),
                ("current", source("current.csv", after.as_bytes())),
                ("coding_evidence", source("coding.json", doc.as_bytes())),
                ("currency", Json::str(currency)),
            ],
        );
    }
    let row: Vec<String> = [
        "Synthetic",
        "A",
        "AUD",
        "T1",
        "2026-09-01",
        "Synthetic reference",
        "Synthetic description",
        "12.34",
        "0",
    ]
    .map(str::to_owned)
    .into();
    let before = csv(std::slice::from_ref(&row));
    let valid = document(&before, &before, vec![policy("A", "A")]);
    push(
        c,
        "classify-same-selection",
        &before,
        &before,
        &probe::encode(&valid),
        "AUD",
    );
    for (key, values) in [
        ("schema_version", vec!["true", "1.0", "2", "null"]),
        (
            "tenant",
            vec![
                r#""Other""#,
                r#"" Synthetic ""#,
                r#""""#,
                r#""a\nb""#,
                r#""\ud800""#,
                "false",
            ],
        ),
        ("currency", vec![r#""USD""#, "null"]),
        (
            "period_start",
            vec![
                r#""20260901""#,
                r#""2026-10-01""#,
                r#""2026-09-31""#,
                r#""2026-09-01 ""#,
                "false",
            ],
        ),
        (
            "period_end",
            vec![r#""2026-08-31""#, r#""2026-09-30\n""#, "0"],
        ),
        ("original_sha256", vec![r#""wrong""#, "null"]),
        ("current_sha256", vec![r#""wrong""#, "true"]),
        ("expectations", vec!["null", "{}", "[]", "[null]", "[{}]"]),
    ] {
        for (index, value) in values.iter().enumerate() {
            let mut changed = valid.clone();
            changed.set(key, probe::decode(value)?);
            push(
                c,
                &format!("classify-field-{key}-{index}"),
                &before,
                &before,
                &probe::encode(&changed),
                "AUD",
            );
        }
    }
    for (index, currency) in ["aud", "AU", "AUDD", " USD ", "USD", "ÅUD", "ＡＵＤ"]
        .iter()
        .enumerate()
    {
        push(
            c,
            &format!("classify-currency-{index}"),
            &before,
            &before,
            &probe::encode(&valid),
            currency,
        );
    }
    for key in [
        "original_account_id",
        "transaction_id",
        "current_account_id",
        "expected_account_id",
        "purpose",
        "evidence_reference",
    ] {
        for (index, raw) in [
            r#""""#,
            r#"" A ""#,
            r#""Other""#,
            r#""a\nb""#,
            r#""\u200b""#,
            r#""\udfff""#,
            "null",
            "0",
            "true",
        ]
        .iter()
        .enumerate()
        {
            let mut changed = policy("A", "A");
            changed.set(key, probe::decode(raw)?);
            let doc = document(&before, &before, vec![changed]);
            push(
                c,
                &format!("classify-policy-{key}-{index}"),
                &before,
                &before,
                &probe::encode(&doc),
                "AUD",
            );
        }
    }
    for (index, raw) in [
        "",
        "{",
        "{}",
        "[]",
        "true",
        "{\"a\":1,\"a\":2}",
        "{\"a\":[0,]}",
        "{\"a\":{\"x\":0,\"x\":1}}",
    ]
    .iter()
    .enumerate()
    {
        push(
            c,
            &format!("classify-json-{index}"),
            &before,
            &before,
            raw,
            "AUD",
        );
    }
    for (index, raw) in ["", "Tenant\n", "Tenant,Tenant\n", "Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit\nSynthetic,A,AUD,T1,2026-09-01,x,y,1\n"].iter().enumerate() {
        let doc = document(raw, &before, vec![]);
        push(c, &format!("classify-csv-{index}"), raw, &before, &probe::encode(&doc), "AUD");
    }
    for (index, (column, value)) in [
        (0, "Other"),
        (1, "B"),
        (3, "é non-ASCII id"),
        (3, ""),
        (4, "2026-08-31"),
        (4, "2026-10-01"),
        (4, "20260901"),
        (5, "=formula"),
        (6, "x\ny"),
        (7, "12.3400"),
        (7, "12.345"),
        (7, "-1"),
        (7, "0"),
        (8, "1"),
        (7, "１２.３４"),
    ]
    .iter()
    .enumerate()
    {
        let mut changed = row.clone();
        changed[*column] = (*value).into();
        let after = csv(&[changed]);
        let doc = document(&before, &after, vec![policy("A", "A")]);
        push(
            c,
            &format!("classify-posting-{index}"),
            &before,
            &after,
            &probe::encode(&doc),
            "AUD",
        );
    }
    for (index, after) in [
        before.replace("\r\n", "\n"),
        before.replace("\r\n", "\r"),
        format!("{before}\r\n"),
        format!("{before},,,,,,,,\r\n"),
        format!("{before}Synthetic,A,AUD,T2,2026-09-02,x,y,1,0,extra\r\n"),
        format!("{before}Synthetic,A,AUD,T2,2026-09-02,\"unfinished\r\n"),
    ]
    .iter()
    .enumerate()
    {
        let doc = document(&before, after, vec![policy("A", "A")]);
        push(
            c,
            &format!("classify-csv-shape-{index}"),
            &before,
            after,
            &probe::encode(&doc),
            "AUD",
        );
    }
    let mut other = row.clone();
    other[1] = "B".into();
    let two = csv(&[row.clone(), other]);
    for (index, policies) in [
        vec![policy("A", "A"), policy("A", "A")],
        vec![policy("A", "A"), policy("B", "A")],
        vec![policy("A", "B"), policy("B", "A")],
        vec![policy("A", "MISSING")],
    ]
    .into_iter()
    .enumerate()
    {
        let doc = document(&two, &two, policies);
        push(
            c,
            &format!("classify-target-{index}"),
            &two,
            &two,
            &probe::encode(&doc),
            "AUD",
        );
    }
    let mut rng = seed;
    for index in 0..160 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let mut after = row.clone();
        after[1] = if rng & 1 == 0 { "A" } else { "B" }.into();
        after[4] = format!("2026-09-{:02}", 1 + (rng >> 8) % 30);
        after[7] = format!("{}.{:02}", (rng >> 16) % 10000, (rng >> 40) % 100);
        let policies = if rng & 2 == 0 {
            vec![]
        } else {
            vec![policy("A", &after[1])]
        };
        let after = csv(&[after]);
        let doc = document(&before, &after, policies);
        push(
            c,
            &format!("classify-generated-{index}"),
            &before,
            &after,
            &probe::encode(&doc),
            "AUD",
        );
    }
    Ok(())
}

fn driver_cases(corpus: &Corpus, expected: &[Json]) -> Result<Corpus> {
    use monthly_close_control_plane::{pycsv, reconciliation::COLUMNS, viewer::PACK_NAMES};
    let mut result = Corpus(Vec::new());
    for (request, response) in corpus.0.iter().zip(expected) {
        let name = request.at("name").as_pystr().unwrap().to_utf8().unwrap();
        if ![
            "render-review-config-absolute-0",
            "render-review-input-combination-0",
            "render-review-report-dates-3",
        ]
        .contains(&name.as_str())
        {
            continue;
        }
        let Some(files) = response.get("ok").and_then(Json::as_array) else {
            continue;
        };
        let payloads = Json::Object(
            PACK_NAMES
                .iter()
                .zip(files)
                .map(|(name, value)| ((*name).into(), value.clone()))
                .collect(),
        );
        let base = [
            "Synthetic",
            "A",
            "AUD",
            "T1",
            "2026-09-01",
            "=Synthetic",
            "Synthetic transaction",
            "500",
            "0",
        ]
        .map(str::to_owned)
        .to_vec();
        let csv = |rows: &[Vec<String>]| {
            std::iter::once(COLUMNS.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
                .chain(rows.iter().cloned())
                .map(|row| pycsv::write_record(&row.iter().map(String::as_str).collect::<Vec<_>>()))
                .collect::<String>()
        };
        let mut variants = vec![
            csv(&[]),
            csv(std::slice::from_ref(&base)),
            String::new(),
            "Tenant\n".into(),
        ];
        for (column, values) in [
            (0, vec!["Other", " Synthetic "]),
            (1, vec!["E", "Other"]),
            (2, vec!["USD", "aud"]),
            (3, vec!["é unusual id", "", "=T1"]),
            (
                4,
                vec![
                    "2026-08-31",
                    "2026-09-30",
                    "2026-10-01",
                    "20260901",
                    "2026-09-31",
                ],
            ),
            (5, vec!["x\ny", "\u{10f7ff}\u{10f800}"]),
            (
                7,
                vec![
                    "500.0000",
                    "499.999",
                    "-1",
                    "0",
                    "１２.３４",
                    "1000000000000000",
                ],
            ),
            (8, vec!["1", "500"]),
        ] {
            for value in values {
                let mut row = base.clone();
                row[column] = value.into();
                variants.push(csv(&[row]));
            }
        }
        variants.push(csv(&[base.clone(), base.clone()]));
        let mut smaller = base.clone();
        smaller[3] = "T2".into();
        smaller[7] = "100".into();
        let mut credit = smaller.clone();
        credit[3] = "T3".into();
        credit[7] = "0".into();
        credit[8] = "100".into();
        variants.push(csv(&[base, smaller, credit]));
        for (index, text) in variants.iter().enumerate() {
            for top in [0, 1, 5] {
                for operation in ["drivers", "drivers_render"] {
                    result.push(
                        format!("{name}-{operation}-{index}-{top}"),
                        operation,
                        vec![
                            ("payloads", payloads.clone()),
                            (
                                "transactions",
                                Json::object([
                                    ("path", Json::str("transactions.csv")),
                                    ("hex", Json::str(&hex(text.as_bytes()))),
                                ]),
                            ),
                            ("currency", Json::str("AUD")),
                            ("top", Json::int(top)),
                        ],
                    );
                }
            }
        }
        for currency in ["USD", "aud", "", " AUD "] {
            result.push(
                format!("{name}-drivers-currency-{currency:?}"),
                "drivers",
                vec![
                    ("payloads", payloads.clone()),
                    (
                        "transactions",
                        Json::object([
                            ("path", Json::str("transactions.csv")),
                            ("hex", Json::str(&hex(variants[1].as_bytes()))),
                        ]),
                    ),
                    ("currency", Json::str(currency)),
                    ("top", Json::int(5)),
                ],
            );
        }
    }
    Ok(result)
}

fn read_utility_cases(corpus: &Corpus, expected: &[Json]) -> Result<Corpus> {
    use monthly_close_control_plane::{sha256, viewer::PACK_NAMES};
    fn unhex(value: &Json) -> Vec<u8> {
        let raw = value.as_pystr().unwrap().to_utf8().unwrap();
        raw.as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
            .collect()
    }
    fn source(name: &str, bytes: &[u8]) -> Json {
        Json::object([("path", Json::str(name)), ("hex", Json::str(&hex(bytes)))])
    }
    let mut bases = Vec::new();
    for (request, response) in corpus.0.iter().zip(expected) {
        let name = request.at("name").as_pystr().unwrap().to_utf8().unwrap();
        if !(name.starts_with("render-review-comparison-")
            || [
                "render-review-input-combination-0",
                "render-review-input-combination-31",
                "render-review-config-absolute-0",
                "render-review-required-1",
            ]
            .contains(&name.as_str()))
        {
            continue;
        }
        let Some(files) = response.get("ok").and_then(Json::as_array) else {
            continue;
        };
        let payloads = Json::Object(
            PACK_NAMES
                .iter()
                .zip(files)
                .map(|(key, value)| ((*key).into(), value.clone()))
                .collect(),
        );
        bases.push((name, payloads, unhex(request.at("current").at("hex"))));
    }
    let mut result = Corpus(Vec::new());
    let side = |payloads: &Json, tb: &[u8], previous: bool| {
        Json::object([
            ("payloads", payloads.clone()),
            (
                "trial_balance",
                source(
                    if previous {
                        "previous.csv"
                    } else {
                        "current.csv"
                    },
                    tb,
                ),
            ),
        ])
    };
    for (old_name, old_files, old_tb) in &bases {
        for (new_name, new_files, new_tb) in &bases {
            result.push(
                format!("compare-{old_name}-{new_name}"),
                "compare",
                vec![
                    ("previous", side(old_files, old_tb, true)),
                    ("current", side(new_files, new_tb, false)),
                ],
            );
        }
    }
    for (name, files, tb) in &bases {
        let old = side(files, tb, true);
        let new = side(files, tb, false);
        let json = unhex(files.at(PACK_NAMES[0]));
        let doc = probe::decode(std::str::from_utf8(&json)?)?;
        let bound = Json::object([
            ("schema_version", Json::int(1)),
            ("pack_sha256", Json::str(&sha256(&json))),
            ("responses", Json::Array(vec![])),
        ]);
        let mut push_response = |label: &str, raw: &[u8]| {
            result.push(
                format!("compare-response-{name}-{label}"),
                "compare",
                vec![
                    ("previous", old.clone()),
                    ("current", new.clone()),
                    ("responses", source("responses.json", raw)),
                ],
            )
        };
        push_response("empty-valid", probe::encode(&bound).as_bytes());
        for (index, raw) in [
            "",
            "{",
            "[]",
            "{}",
            "true",
            "{\"x\":0,\"x\":1}",
            "{\"a\":[0,]}",
        ]
        .iter()
        .enumerate()
        {
            push_response(&format!("json-{index}"), raw.as_bytes());
        }
        push_response("invalid-utf8", &[0xff]);
        for (key, raw) in [
            ("schema_version", "true"),
            ("schema_version", "1.0"),
            ("pack_sha256", r#""wrong""#),
            ("responses", "null"),
            ("responses", "[{}]"),
            ("responses", "[true]"),
        ] {
            let mut changed = bound.clone();
            changed.set(key, probe::decode(raw)?);
            push_response(
                &format!("schema-{key}-{raw}"),
                probe::encode(&changed).as_bytes(),
            );
        }
        if let Some(query) = doc
            .get("client_queries")
            .and_then(Json::as_array)
            .and_then(|rows| rows.first())
        {
            let period = doc.at("current_report_dates").as_array().unwrap()[0].clone();
            let row = Json::object([
                ("query_id", query.at("query_id").clone()),
                ("explanation", Json::str("Synthetic explanation")),
                ("evidence_reference", Json::str("Synthetic receipt")),
                ("reviewer", Json::str("RD")),
                ("reviewed_on", period),
            ]);
            let mut valid = bound.clone();
            valid.set("responses", Json::Array(vec![row.clone()]));
            push_response("valid", probe::encode(&valid).as_bytes());
            valid.set("responses", Json::Array(vec![row.clone(), row.clone()]));
            push_response("duplicate", probe::encode(&valid).as_bytes());
            for key in [
                "query_id",
                "explanation",
                "evidence_reference",
                "reviewer",
                "reviewed_on",
            ] {
                for (index, raw) in [
                    r#""""#,
                    r#"" ""#,
                    "null",
                    "true",
                    r#""\ud800""#,
                    r#""x\ny""#,
                ]
                .iter()
                .enumerate()
                {
                    let mut changed = row.clone();
                    changed.set(key, probe::decode(raw)?);
                    let mut document = bound.clone();
                    document.set("responses", Json::Array(vec![changed]));
                    push_response(
                        &format!("field-{key}-{index}"),
                        probe::encode(&document).as_bytes(),
                    );
                }
            }
            for date in [
                "20260930",
                "2026-W40-3",
                "2026-09-30 ",
                "2026-09-29",
                "2026-09-31",
                "2026-10-01",
            ] {
                let mut changed = row.clone();
                changed.set("reviewed_on", Json::str(date));
                let mut document = bound.clone();
                document.set("responses", Json::Array(vec![changed]));
                push_response(&format!("date-{date}"), probe::encode(&document).as_bytes());
            }
        }
        let mut changed = new.clone();
        changed.set("trial_balance", source("current.csv", b"bad trial balance"));
        result.push(
            format!("compare-tb-digest-{name}"),
            "compare",
            vec![("previous", old), ("current", changed)],
        );
    }
    let packs: Vec<_> = bases
        .iter()
        .enumerate()
        .map(|(index, (_, files, _))| {
            Json::object([
                ("path", Json::str(&format!("pack-{index}-é|test"))),
                ("payloads", files.clone()),
            ])
        })
        .collect();
    result.push(
        "queue-all",
        "queue",
        vec![("packs", Json::Array(packs.clone()))],
    );
    let mut duplicate = packs.clone();
    duplicate.extend(packs.iter().rev().cloned());
    result.push(
        "queue-deduplicated",
        "queue",
        vec![("packs", Json::Array(duplicate))],
    );
    result.push(
        "queue-no-packs",
        "queue",
        vec![("packs", Json::Array(vec![]))],
    );
    for (key, values) in [
        (
            "period",
            vec![
                "2026-09-30",
                "2026-10-31",
                "2026-01-01",
                "20260930",
                "2026-09-31",
                "",
                "2026-W40-3",
            ],
        ),
        (
            "status",
            vec!["PASS", "REVIEW", "BLOCKED", "pass", "", "READY"],
        ),
        (
            "reviewer",
            vec!["RD", "Other", " RD ", "", " ", "\u{1c}", "x\ny"],
        ),
    ] {
        for (index, value) in values.iter().enumerate() {
            result.push(
                format!("queue-filter-{key}-{index}"),
                "queue",
                vec![
                    ("packs", Json::Array(packs.clone())),
                    (key, Json::str(value)),
                ],
            );
        }
    }
    for state in ["PASS", "REVIEW", "BLOCKED"] {
        result.push(
            format!("queue-combined-{state}"),
            "queue",
            vec![
                ("packs", Json::Array(packs.clone())),
                ("period", Json::str("2026-09-30")),
                ("status", Json::str(state)),
                ("reviewer", Json::str("RD")),
            ],
        );
    }
    let mut broken = packs;
    broken.push(Json::object([
        ("path", Json::str("broken-pack")),
        ("error", Json::str("broken supplied witness")),
    ]));
    result.push(
        "queue-broken-before-filter",
        "queue",
        vec![
            ("packs", Json::Array(broken.clone())),
            ("period", Json::str("2026-01-01")),
        ],
    );
    result.push(
        "queue-filter-before-io",
        "queue",
        vec![
            ("packs", Json::Array(broken)),
            ("period", Json::str("bad date")),
        ],
    );
    Ok(result)
}

fn verification_cases(corpus: &Corpus, expected: &[Json]) -> Result<Corpus> {
    use monthly_close_control_plane::viewer::PACK_NAMES;
    use std::collections::BTreeMap;
    fn unhex(raw: &str) -> Vec<u8> {
        raw.as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
            .collect()
    }
    fn push(c: &mut Corpus, name: &str, payloads: &BTreeMap<String, Vec<u8>>) {
        c.push(
            name,
            "verify",
            vec![(
                "payloads",
                Json::Object(
                    payloads
                        .iter()
                        .map(|(k, v)| (PyStr::from(k.as_str()), Json::str(&hex(v))))
                        .collect(),
                ),
            )],
        );
    }
    fn remove(value: &mut Json, key: &str) {
        let Json::Object(pairs) = value else {
            panic!("object fixture");
        };
        pairs.retain(|(name, _)| !name.is(key));
    }
    let mut c = Corpus(Vec::new());
    let mut bases = Vec::new();
    for (request, response) in corpus.0.iter().zip(expected) {
        if !request.at("op").is_str("render") {
            continue;
        }
        let Some(values) = response.get("ok").and_then(Json::as_array) else {
            continue;
        };
        let name = request.at("name").as_pystr().unwrap().to_utf8().unwrap();
        if name.ends_with("combination-31")
            || name.ends_with("combination-0")
            || name.ends_with("coal-lsl-levy.json-1")
            || name.ends_with("refused.json-1")
            || name.contains("unicode")
            || name.contains("equity")
        {
            let payloads: BTreeMap<_, _> = PACK_NAMES
                .iter()
                .zip(values)
                .map(|(key, value)| {
                    (
                        key.to_string(),
                        unhex(&value.as_pystr().unwrap().to_utf8().unwrap()),
                    )
                })
                .collect();
            push(&mut c, &format!("verify-{name}"), &payloads);
            bases.push((name, payloads));
        }
    }
    for (name, base) in bases {
        let document = probe::decode(std::str::from_utf8(&base[PACK_NAMES[0]])?)?;
        let summary = std::str::from_utf8(&base[PACK_NAMES[1]])?.replace("\r\n", "\n");
        let mut json_change = |label: &str, value: Json| {
            let mut payloads = base.clone();
            payloads.insert(PACK_NAMES[0].into(), probe::encode(&value).into_bytes());
            push(&mut c, &format!("{name}-{label}"), &payloads);
        };
        json_change("reformatted", document.clone());
        for key in document.keys().unwrap() {
            let key = key.to_utf8().unwrap();
            let mut missing = document.clone();
            remove(&mut missing, &key);
            json_change(&format!("missing-{key}"), missing);
            for (index, raw) in ["null", "true", "0", r#""""#, "[]", "{}"]
                .iter()
                .enumerate()
            {
                let mut changed = document.clone();
                changed.set(&key, probe::decode(raw)?);
                json_change(&format!("type-{key}-{index}"), changed);
            }
        }
        let mut extra = document.clone();
        extra.set("extra", Json::str("unwitnessed"));
        json_change("unknown-member", extra);
        for (index, raw) in [
            "null",
            "true",
            r#""NaN""#,
            r#""Infinity""#,
            r#""-1""#,
            r#""ten""#,
            r#""0.001%""#,
        ]
        .iter()
        .enumerate()
        {
            let mut changed = document.clone();
            let mut thresholds = document.at("thresholds").clone();
            thresholds.set("absolute_variance", probe::decode(raw)?);
            changed.set("thresholds", thresholds);
            json_change(&format!("threshold-{index}"), changed);
        }
        for (index, raw) in ["null", "false", r#""PASS""#, "{}", r#"{"status":"PASS"}"#]
            .iter()
            .enumerate()
        {
            let mut changed = document.clone();
            changed.set("exceptions", Json::Array(vec![probe::decode(raw)?]));
            json_change(&format!("exception-{index}"), changed);
        }
        if !document.at("exceptions").as_array().unwrap().is_empty() {
            let original = document.at("exceptions").as_array().unwrap();
            for key in original[0].keys().unwrap() {
                let key = key.to_utf8().unwrap();
                for (index, raw) in ["null", "true", r#""edited""#].iter().enumerate() {
                    let mut rows = original.to_vec();
                    rows[0].set(&key, probe::decode(raw)?);
                    let mut changed = document.clone();
                    changed.set("exceptions", Json::Array(rows));
                    json_change(&format!("exception-field-{key}-{index}"), changed);
                }
            }
        }
        if let Some(block) = document.get("calculation_evidence") {
            for key in ["required", "supplied", "effect"] {
                let mut changed = document.clone();
                let mut evidence = block.clone();
                evidence.set(key, Json::Null);
                changed.set("calculation_evidence", evidence);
                json_change(&format!("evidence-field-{key}"), changed);
            }
            if let Some(supplied) = block.at("supplied").as_array().filter(|s| !s.is_empty()) {
                for key in supplied[0].keys().unwrap() {
                    let key = key.to_utf8().unwrap();
                    let mut rows = supplied.to_vec();
                    rows[0].set(&key, Json::Null);
                    let mut evidence = block.clone();
                    evidence.set("supplied", Json::Array(rows));
                    let mut changed = document.clone();
                    changed.set("calculation_evidence", evidence);
                    json_change(&format!("evidence-entry-{key}"), changed);
                }
            }
        }
        for (index, raw) in [
            "",
            "{",
            "[]",
            "{}",
            "{\"overall_status\":\"PASS\",\"overall_status\":\"PASS\"}",
            "{\"acknowledgement\":NaN}",
            "[1,]",
        ]
        .iter()
        .enumerate()
        {
            let mut payloads = base.clone();
            payloads.insert(PACK_NAMES[0].into(), raw.as_bytes().to_vec());
            push(&mut c, &format!("{name}-raw-json-{index}"), &payloads);
        }
        for (index, file) in PACK_NAMES.iter().enumerate() {
            let mut payloads = base.clone();
            payloads.insert((*file).into(), vec![0xff, 0xfe]);
            push(&mut c, &format!("{name}-utf8-{index}"), &payloads);
        }
        for (index, changed) in [
            format!("```\n{summary}"),
            format!("   ~~~\n{summary}"),
            format!("<div>\n{summary}"),
            format!("{summary}\n## Forged\n"),
            format!("{summary}\nForged\n---\n"),
            summary.replace("## Exceptions", "### Exceptions"),
            summary.replace("## Source evidence", "## Source evidence ##"),
            summary.replace("| --- | --- | --- | --- | ---: | --- |", "| broken |"),
            summary.replace("- Exceptions:", "- Edited exceptions:"),
            summary.replace("- Client queries drafted:", "- Query count:"),
            summary.replace("Nothing has been sent to anyone.", "Already sent."),
            summary.replace(
                "## Human acknowledgement",
                "## Human acknowledgement\n\nExtra paragraph",
            ),
            summary.replace("This pack is a review aid.", "This pack approves a close."),
        ]
        .into_iter()
        .enumerate()
        {
            let mut payloads = base.clone();
            payloads.insert(PACK_NAMES[1].into(), changed.into_bytes());
            push(&mut c, &format!("{name}-summary-{index}"), &payloads);
        }
        for file in [PACK_NAMES[2], PACK_NAMES[3]] {
            for (index, changed) in [
                b"".to_vec(),
                b"wrong,header\r\n".to_vec(),
                [base[file].clone(), b"\r\n".to_vec()].concat(),
                [base[file].clone(), b"=unwitnessed\r\n".to_vec()].concat(),
            ]
            .into_iter()
            .enumerate()
            {
                let mut payloads = base.clone();
                payloads.insert(file.into(), changed);
                push(
                    &mut c,
                    &format!("{name}-{file}-mutation-{index}"),
                    &payloads,
                );
            }
        }
        let mut half = base.clone();
        half.remove(PACK_NAMES[3]);
        push(&mut c, &format!("{name}-missing-query-file"), &half);
        let mut legacy = document.clone();
        remove(&mut legacy, "client_queries");
        half.insert(PACK_NAMES[0].into(), probe::encode(&legacy).into_bytes());
        push(&mut c, &format!("{name}-half-removed-register"), &half);
        let start = summary.find("\n## Client queries\n").unwrap();
        let end = summary.find("\n## Human acknowledgement\n").unwrap();
        let older = format!("{}{}", &summary[..start], &summary[end..]);
        let older = older
            .lines()
            .filter(|line| !line.starts_with("- Client queries drafted: "))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        half.insert(PACK_NAMES[1].into(), older.into_bytes());
        push(&mut c, &format!("{name}-archived-three-file"), &half);
    }
    Ok(c)
}

fn cli_cases(corpus: &Corpus, drivers: &Corpus, reads: &Corpus) -> Result<Corpus> {
    use monthly_close_control_plane::viewer::PACK_NAMES;
    use std::collections::BTreeMap;
    fn value(value: &Json) -> String {
        value.as_pystr().unwrap().to_utf8().unwrap()
    }
    fn push(
        c: &mut Corpus,
        name: &str,
        args: Vec<String>,
        files: Vec<(PyStr, Json)>,
        directories: Vec<Json>,
    ) {
        let files: BTreeMap<_, _> = files.into_iter().collect();
        c.push(
            name,
            "cli",
            vec![
                (
                    "args",
                    Json::Array(args.iter().map(|s| Json::str(s)).collect()),
                ),
                ("files", Json::Object(files.into_iter().collect())),
                ("directories", Json::Array(directories)),
            ],
        );
    }
    fn input(files: &mut Vec<(PyStr, Json)>, name: &str, snapshot: &Json) -> String {
        files.push((name.into(), snapshot.at("hex").clone()));
        format!("$TMP/{name}")
    }
    fn pack(files: &mut Vec<(PyStr, Json)>, directory: &str, payloads: &Json) -> String {
        for name in PACK_NAMES {
            if let Some(bytes) = payloads.get(name) {
                files.push((format!("{directory}/{name}").as_str().into(), bytes.clone()));
            }
        }
        format!("$TMP/{directory}")
    }
    let mut result = Corpus(Vec::new());
    for (index, args) in [
        vec![],
        vec!["--help"],
        vec!["--hel"],
        vec!["-h"],
        vec!["--help=x"],
        vec!["--unknown", "--help"],
        vec!["unknown", "--help"],
        vec!["--"],
        vec!["review", "--unknown", "--help"],
        vec!["review", "--p"],
        vec!["review", "--percentage-threshold=10"],
        vec!["review", "--absolute-threshold=NaN"],
        vec!["drivers", "--top=bad"],
        vec!["schedule", "--kind=bad", "--help"],
        vec!["queue", "--status=READY", "--help"],
    ]
    .into_iter()
    .enumerate()
    {
        push(
            &mut result,
            &format!("cli-presentation-{index}"),
            args.into_iter().map(str::to_owned).collect(),
            vec![],
            vec![],
        );
    }
    for command in [
        "review",
        "workbench",
        "view",
        "queue",
        "reconcile",
        "drivers",
        "compare",
        "classify",
        "schedule",
    ] {
        for suffix in [vec![], vec!["--help"], vec!["--hel"], vec!["--help=x"]] {
            let mut args = vec![command.to_owned()];
            args.extend(suffix.iter().map(|s| (*s).to_owned()));
            push(
                &mut result,
                &format!("cli-{command}-{}", suffix.join("-")),
                args,
                vec![],
                vec![],
            );
        }
    }
    for request in &corpus.0 {
        let name = value(request.at("name"));
        let operation = value(request.at("op"));
        let mut files = Vec::new();
        let mut args = Vec::new();
        match operation.as_str() {
            "classify" => {
                args.push("classify".into());
                for (key, flag) in [
                    ("original", "--original-transactions"),
                    ("current", "--current-transactions"),
                    ("coding_evidence", "--coding-evidence"),
                ] {
                    args.extend([flag.into(), input(&mut files, key, request.at(key))]);
                }
                args.extend(["--currency".into(), value(request.at("currency"))]);
            }
            "review" if !name.starts_with("review-generated-") => {
                args.push(
                    if name.ends_with("31") {
                        "workbench"
                    } else {
                        "review"
                    }
                    .into(),
                );
                for (key, flag) in [
                    ("current", "--current"),
                    ("prior", "--prior"),
                    ("mapping", "--mapping"),
                    ("mapping_policy", "--mapping-policy"),
                    ("balance_policy", "--balance-policy"),
                    ("subledger", "--subledger"),
                    ("acknowledgement", "--review-note"),
                    ("equity_schedule", "--equity-schedule"),
                ] {
                    if let Some(source) = request.get(key) {
                        args.extend([flag.into(), input(&mut files, key, source)]);
                    }
                }
                for (key, flag) in [
                    ("absolute", "--absolute-threshold"),
                    ("percentage", "--percentage-threshold"),
                    ("tolerance", "--reconciliation-tolerance"),
                    ("currency", "--equity-currency"),
                    ("currency_evidence", "--equity-currency-evidence"),
                ] {
                    if let Some(value_) = request.get(key) {
                        args.push(format!("{flag}={}", value(value_)));
                    }
                }
                if let Some(required) = request.get("required").and_then(Json::as_array) {
                    for label in required {
                        args.push(format!("--require-calculation={}", value(label)));
                    }
                }
                if let Some(evidence) = request.get("evidence").and_then(Json::as_array) {
                    for (index, snapshot) in evidence.iter().enumerate() {
                        args.extend([
                            "--calculation-evidence".into(),
                            input(&mut files, &format!("evidence-{index}.json"), snapshot),
                        ]);
                    }
                }
                args.extend(["--output".into(), "$TMP/output".into()]);
            }
            "reconcile"
                if name == "reconcile-suggestion-is-not-acceptance"
                    || name.contains("decision")
                    || name.contains("generated-0") =>
            {
                args.push("reconcile".into());
                args.extend([
                    "--transactions".into(),
                    input(&mut files, "transactions.csv", request.at("transactions")),
                ]);
                for key in [
                    "tenant",
                    "account_id",
                    "currency",
                    "period_start",
                    "period_end",
                    "opening_balance",
                    "closing_balance",
                ] {
                    args.extend([
                        format!("--{}", key.replace('_', "-")),
                        value(request.at(key)),
                    ]);
                }
                for key in ["opening_items", "decisions"] {
                    if let Some(source) = request.get(key).filter(|v| !matches!(v, Json::Null)) {
                        args.extend([
                            format!("--{}", key.replace('_', "-")),
                            input(&mut files, key, source),
                        ]);
                    }
                }
                if request.get("decisions_escaped").and_then(Json::as_bool) == Some(true) {
                    args.push("--decisions-escaped".into());
                }
                args.extend(["--output".into(), "$TMP/output".into()]);
            }
            "schedule" if name.contains("example") => {
                args.extend([
                    "schedule".into(),
                    "--kind".into(),
                    value(request.at("kind")),
                    "--input".into(),
                    input(&mut files, "schedule.json", request),
                ]);
            }
            _ => continue,
        }
        push(&mut result, &format!("cli-{name}"), args, files, vec![]);
    }
    for flag in [
        "--current",
        "--prior",
        "--mapping",
        "--mapping-policy",
        "--balance-policy",
        "--subledger",
        "--review-note",
        "--equity-schedule",
        "--calculation-evidence",
    ] {
        let mut args: Vec<String> = [
            "review",
            "--current",
            "$TMP/missing-current",
            "--prior",
            "$TMP/missing-prior",
            "--output",
            "$TMP/output",
            flag,
            "$TMP/output/close-summary.md",
        ]
        .map(str::to_owned)
        .into();
        if flag == "--mapping-policy" {
            args.extend(["--mapping".into(), "$TMP/missing-mapping".into()]);
        }
        push(
            &mut result,
            &format!("cli-collision-{flag}"),
            args,
            vec![],
            vec![],
        );
    }
    for marker in [".git", ".hg", ".svn", ".bzr"] {
        let files = vec![(
            format!("repo/{marker}").as_str().into(),
            Json::str(&hex(b"marker")),
        )];
        let args = [
            "review",
            "--current",
            "$TMP/missing-current",
            "--prior",
            "$TMP/missing-prior",
            "--output",
            "$TMP/repo/output",
        ]
        .map(str::to_owned)
        .into();
        push(
            &mut result,
            &format!("cli-vcs-{marker}"),
            args,
            files,
            vec![],
        );
    }
    for request in drivers.0.iter().filter(|r| {
        r.at("op").is_str("drivers")
            && r.at("name")
                .as_pystr()
                .unwrap()
                .to_utf8()
                .unwrap()
                .ends_with("-1")
    }) {
        let mut files = Vec::new();
        let directory = pack(&mut files, "pack", request.at("payloads"));
        let transactions = input(&mut files, "transactions.csv", request.at("transactions"));
        let args = vec![
            "drivers".into(),
            "--pack-dir".into(),
            directory,
            "--transactions".into(),
            transactions,
            "--currency".into(),
            value(request.at("currency")),
            "--top".into(),
            "1".into(),
            "--output".into(),
            "$TMP/output".into(),
        ];
        push(
            &mut result,
            &format!("cli-{}", value(request.at("name"))),
            args,
            files,
            vec![],
        );
    }
    for request in &reads.0 {
        let name = value(request.at("name"));
        let mut files = Vec::new();
        let mut args = Vec::new();
        if request.at("op").is_str("compare") {
            args.push("compare".into());
            for key in ["previous", "current"] {
                let side = request.at(key);
                let directory = pack(&mut files, key, side.at("payloads"));
                args.extend([format!("--{key}-pack"), directory]);
                args.extend([
                    format!("--{key}-tb"),
                    input(&mut files, &format!("{key}.csv"), side.at("trial_balance")),
                ]);
            }
            if let Some(response) = request.get("responses") {
                args.extend([
                    "--responses".into(),
                    input(&mut files, "responses.json", response),
                ]);
            }
        } else {
            args.push("queue".into());
            let mut names = BTreeMap::new();
            for supplied in request.at("packs").as_array().unwrap() {
                let original = value(supplied.at("path"));
                let count = names.len();
                let directory = names
                    .entry(original)
                    .or_insert_with(|| format!("pack-{count}"));
                let path = if let Some(payloads) = supplied.get("payloads") {
                    pack(&mut files, directory, payloads)
                } else {
                    format!("$TMP/{directory}")
                };
                args.extend(["--pack-dir".into(), path]);
            }
            for key in ["period", "status", "reviewer"] {
                if let Some(filter) = request.get(key) {
                    args.extend([format!("--{key}"), value(filter)]);
                }
            }
        }
        push(&mut result, &format!("cli-{name}"), args, files, vec![]);
    }
    Ok(result)
}

fn assert_legacy_financial_cases(corpus: &Corpus, responses: &[Json]) -> Result<()> {
    let mut checked = 0;
    let mut rendered = 0;
    for (request, response) in corpus.0.iter().zip(responses) {
        let name = request.at("name").as_pystr().unwrap().to_utf8().unwrap();
        if name.starts_with("render-review-equity-legacy-") {
            if !response
                .get("ok")
                .and_then(Json::as_array)
                .is_some_and(|files| files.len() == 4)
            {
                return Err(format!("legacy equity rendering failed for {name}").into());
            }
            rendered += 1;
            continue;
        }
        let status = match name.as_str() {
            "review-equity-legacy-nil-movement"
            | "review-equity-legacy-positive-at-tolerance"
            | "review-equity-legacy-negative-at-tolerance"
            | "review-equity-legacy-debit-loss"
            | "schedule-legacy-expenses-pass"
            | "schedule-legacy-migration-split"
            | "schedule-legacy-interentity-agrees" => "PASS",
            "review-equity-legacy-opposite-errors"
            | "review-equity-legacy-aggregate-tolerance"
            | "review-equity-legacy-positive-over-tolerance"
            | "review-equity-legacy-negative-over-tolerance"
            | "schedule-legacy-interentity-unmatched"
            | "schedule-legacy-interentity-disputed" => "REVIEW",
            _ => continue,
        };
        let Some(value) = response.get("ok") else {
            return Err(format!("legacy financial case {name} did not produce a result").into());
        };
        let mut holds = value.at("status").is_str(status);
        match name.as_str() {
            "review-equity-legacy-debit-loss" => {
                holds &= value
                    .at("equity_reconciliation")
                    .at("total")
                    .at("expected_close")
                    .is_str("-125");
            }
            "review-equity-legacy-nil-movement" => {
                holds &= value
                    .at("equity_reconciliation")
                    .at("total")
                    .at("movement")
                    .is_str("0");
            }
            "review-equity-legacy-opposite-errors" => {
                holds &= value
                    .at("exceptions")
                    .as_array()
                    .is_some_and(|rows| rows.len() == 2)
                    && value
                        .at("equity_reconciliation")
                        .at("total")
                        .at("unexplained")
                        .is_str("0");
            }
            "review-equity-legacy-aggregate-tolerance" => {
                holds &= value.at("exceptions").as_array().is_some_and(|rows| {
                    rows.len() == 1 && rows[0].at("difference").is_str("0.018")
                });
            }
            "schedule-legacy-interentity-unmatched" => {
                holds &= value.at("pairs").as_array().is_some_and(|rows| {
                    rows.len() == 2 && rows.iter().all(|row| row.at("status").is_str("UNMATCHED"))
                });
            }
            "schedule-legacy-interentity-disputed" => {
                holds &= value
                    .at("pairs")
                    .as_array()
                    .is_some_and(|rows| rows.len() == 1 && rows[0].at("status").is_str("AGREES"));
            }
            _ => {}
        }
        if !holds {
            return Err(format!("legacy financial outcome assertion failed for {name}").into());
        }
        checked += 1;
    }
    if checked != 13 || rendered != 8 {
        return Err(format!("expected 13 legacy financial cases and 8 equity renders, checked {checked} and {rendered}").into());
    }
    println!("13 legacy financial scenarios and 8 equity renders satisfy fixed outcome assertions");
    Ok(())
}

fn assert_legacy_comparison_cases(corpus: &Corpus, responses: &[Json]) -> Result<()> {
    let mut checked = 0;
    for (request, response) in corpus.0.iter().zip(responses) {
        let name = request.at("name").as_pystr().unwrap().to_utf8().unwrap();
        let Some(label) = name.strip_prefix("core-legacy-") else {
            continue;
        };
        let findings = response
            .get("ok")
            .and_then(|value| value.get("exceptions"))
            .and_then(Json::as_array)
            .ok_or_else(|| format!("legacy comparison {name} did not produce findings"))?;
        let variances: Vec<_> = findings
            .iter()
            .filter(|row| row.at("control").is_str("period_variance"))
            .collect();
        let holds = match label {
            "first-funding" => {
                variances.len() == 3
                    && ["777", "888", "900"].iter().all(|account| {
                        variances
                            .iter()
                            .filter(|row| row.at("account_id").is_str(account))
                            .count()
                            == 1
                    })
                    && variances.iter().any(|row| {
                        row.at("account_id").is_str("777")
                            && matches!(row.at("percentage_change"), Json::Null)
                            && row.at("prior_value").is_str("0.00")
                            && row.at("difference").is_str("900000.00")
                            && row.at("reason").is_str("YTD net balance movement met or exceeded the configured absolute threshold; no percentage change could be computed from the prior YTD balance, so the percentage threshold was not tested.")
                    })
            }
            "nil-prior-ytd" => {
                let rows: Vec<_> = variances
                    .iter()
                    .filter(|row| row.at("account_id").is_str("110"))
                    .collect();
                rows.len() == 1
                    && matches!(rows[0].at("percentage_change"), Json::Null)
                    && rows[0].at("difference").is_str("50000.00")
            }
            "april-may" | "may-june" | "december-january" => {
                variances.is_empty()
                    && findings
                        .iter()
                        .all(|row| !row.at("control").is_str("financial_year_reset"))
            }
            _ => return Err(format!("unrecognised legacy comparison {name}").into()),
        };
        if !holds {
            return Err(format!("legacy comparison outcome assertion failed for {name}").into());
        }
        checked += 1;
    }
    if checked != 5 {
        return Err(format!("expected 5 legacy comparisons, checked {checked}").into());
    }
    println!("5 legacy balance and financial-year comparisons satisfy fixed outcome assertions");
    Ok(())
}

fn assert_legacy_equity_verification(corpus: &Corpus, responses: &[Json]) -> Result<()> {
    let mut checked = 0;
    for (request, response) in corpus.0.iter().zip(responses) {
        let name = request.at("name").as_pystr().unwrap().to_utf8().unwrap();
        let Some(label) = name.strip_prefix("verify-render-review-equity-legacy-") else {
            continue;
        };
        let status = match label {
            "nil-movement" | "positive-at-tolerance" | "negative-at-tolerance" | "debit-loss" => {
                "PASS"
            }
            "opposite-errors"
            | "aggregate-tolerance"
            | "positive-over-tolerance"
            | "negative-over-tolerance" => "REVIEW",
            _ => continue,
        };
        let document = response
            .get("ok")
            .and_then(|value| value.get("verified"))
            .and_then(Json::as_array)
            .and_then(|verified| verified.first())
            .ok_or_else(|| format!("legacy equity saved-pack verification failed for {name}"))?;
        if !document
            .at("equity_reconciliation")
            .at("status")
            .is_str(status)
        {
            return Err(
                format!("legacy equity saved-pack status assertion failed for {name}").into(),
            );
        }
        if label == "aggregate-tolerance"
            && !document
                .at("client_queries")
                .as_array()
                .is_some_and(|rows| rows.is_empty())
        {
            return Err("aggregate-only equity finding must not draft a client query".into());
        }
        checked += 1;
    }
    if checked != 8 {
        return Err(format!("expected 8 verified legacy equity packs, checked {checked}").into());
    }
    println!("8 legacy equity saved packs verify with fixed status assertions");
    Ok(())
}

fn run_batch(corpus: &Corpus, python: &Path, root: &Path, executable: &Path) -> Result<Vec<Json>> {
    let names: Vec<_> = corpus
        .0
        .iter()
        .map(|r| r.at("name").as_pystr().unwrap().to_utf8().unwrap())
        .collect();
    let encoded: String = corpus.0.iter().map(|r| probe::encode(r) + "\n").collect();
    let mut oracle = Command::new(python);
    oracle
        .args(["-I", "-B", "-c", ORACLE])
        .arg(root)
        .env("PYTHONIOENCODING", "utf-8");
    let expected = exchange(oracle, &encoded)?;
    let actual = exchange(Command::new(executable), &encoded)?;
    if actual.len() != expected.len() || actual.len() != names.len() {
        return Err("oracle/native response count differs".into());
    }
    let mut mismatches = 0;
    let mut parsed = Vec::new();
    for ((name, actual), expected) in names.iter().zip(&actual).zip(&expected) {
        let value = probe::decode(expected)?;
        if probe::encode(&probe::decode(actual)?) != probe::encode(&value) {
            mismatches += 1;
            if mismatches <= 12 {
                eprintln!("difference {name}:\n  Python: {expected}\n  Rust: {actual}");
            }
        }
        parsed.push(value);
    }
    if mismatches != 0 {
        return Err(format!(
            "{mismatches} unexplained differences in {} cases",
            names.len()
        )
        .into());
    }
    Ok(parsed)
}

fn run(args: Vec<std::ffi::OsString>) -> Result<()> {
    if !(2..=3).contains(&args.len()) {
        return Err("usage: close-control-differential CPYTHON ORACLE_COMPONENT [SEED]".into());
    }
    let python = Path::new(&args[0]);
    let root = fs::canonicalize(&args[1])?;
    if !root.join("closecontrol/cli.py").is_file() {
        return Err("frozen component source is missing".into());
    }
    let seed: u64 = args
        .get(2)
        .map(|s| s.to_string_lossy().parse())
        .transpose()?
        .unwrap_or(1);
    let mut corpus = Corpus(Vec::new());
    money(&mut corpus, seed);
    dates(&mut corpus);
    csvs(&mut corpus);
    arithmetic(&mut corpus);
    core(&mut corpus, seed);
    notes(&mut corpus);
    evidence(&mut corpus, &root)?;
    equity(&mut corpus, seed)?;
    reviews(&mut corpus, &root, seed)?;
    schedules(&mut corpus, &root, seed)?;
    reconciliation(&mut corpus, seed)?;
    classification_cases(&mut corpus, seed)?;
    let renders: Vec<_> = corpus
        .0
        .iter()
        .filter(|r| r.at("op").is_str("review"))
        .cloned()
        .map(|mut r| {
            let name = r.at("name").as_pystr().unwrap().to_utf8().unwrap();
            r.set("name", Json::str(&format!("render-{name}")));
            r.set("op", Json::str("render"));
            r
        })
        .collect();
    corpus.0.extend(renders);
    let reconciled: Vec<_> = corpus
        .0
        .iter()
        .filter(|r| r.at("op").is_str("reconcile"))
        .cloned()
        .map(|mut r| {
            let name = r.at("name").as_pystr().unwrap().to_utf8().unwrap();
            r.set("name", Json::str(&format!("render-{name}")));
            r.set("op", Json::str("reconcile_render"));
            r
        })
        .collect();
    corpus.0.extend(reconciled);
    let executable = env::current_exe()?.with_file_name(if cfg!(windows) {
        "close-control-probe.exe"
    } else {
        "close-control-probe"
    });
    if !executable.is_file() {
        return Err("build probe binaries before running the differential".into());
    }
    let expected = run_batch(&corpus, python, &root, &executable)?;
    assert_legacy_financial_cases(&corpus, &expected)?;
    assert_legacy_comparison_cases(&corpus, &expected)?;
    let verification = verification_cases(&corpus, &expected)?;
    let verified = run_batch(&verification, python, &root, &executable)?;
    assert_legacy_equity_verification(&verification, &verified)?;
    let drivers = driver_cases(&corpus, &expected)?;
    run_batch(&drivers, python, &root, &executable)?;
    let read_utilities = read_utility_cases(&corpus, &expected)?;
    run_batch(&read_utilities, python, &root, &executable)?;
    let cli = cli_cases(&corpus, &drivers, &read_utilities)?;
    run_batch(&cli, python, &root, &executable)?;
    println!(
        "seed {seed}: {} value/core/evidence/render cases, {} verification cases, {} driver cases, {} comparison/queue cases and {} CLI cases; 0 unexplained differences",
        corpus.0.len(),
        verification.0.len(),
        drivers.0.len(),
        read_utilities.0.len(),
        cli.0.len()
    );
    Ok(())
}
fn main() -> Result<()> {
    let args = env::args_os().skip(1).collect();
    thread::Builder::new()
        .name("close-control-differential".into())
        .stack_size(32 * 1024 * 1024)
        .spawn(|| run(args))?
        .join()
        .map_err(|_| "differential thread panicked")?
}
