"""Differential test: the released Python package against the Rust binary.

    python differential.py --python-pkg <dir holding elizabeth_anne_alexander/> \
        --binary <target/debug/elizabeth-anne-alexander built with --features differential> \
        [--cases N] [--seed S] [--keep-failures DIR]

Each case writes the same mutated sample data and working-directory files for
both implementations, runs the same command line, and compares the exit code,
stdout, stderr and every file left in the working directory. Differences that
only reflect where each implementation keeps its bundled data, the working
directory path and the receipt's code_version are normalised away; when CPython
exits 1 with a traceback, only the exit code is compared.
"""

import argparse
import csv
import hashlib
import json
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile
from decimal import Decimal
from pathlib import Path

RUNNER = """import sys
sys.path.insert(0, sys.argv.pop(1))
sys.argv[0] = "elizabeth-anne-alexander"
from elizabeth_anne_alexander.cli import main
raise SystemExit(main(sys.argv[1:]))
"""

COLUMNS = ["ReportDate", "Tenant", "Section", "AccountID", "AccountName", "AccountCode", "Debit", "Credit", "YTDDebit", "YTDCredit"]
CONTEXT = "samples/contexts/sample-monthly-variance.context.json"
REQUEST = "samples/requests/sample-revenue-variance.request.json"
POLICY = "policy/demo-policy-v1.json"
DECISION = "samples/decisions/sample-review-decision.json"
CURRENT_MANIFEST = "samples/manifests/sample-tb-2026-06-30.manifest.json"
PRIOR_MANIFEST = "samples/manifests/sample-tb-2026-05-31.manifest.json"

AMOUNTS = [
    "0", "0.00", "-0.00", "100", "100.00", "1E+2", "1e2", "1_000.50", " 12.5 ", " 12.50", "١٢.5",
    "NaN", "inf", "1e19", "1e18", "9.99e18", "1e-18", "1e-19", "0E-999999999", "0E+999999999999",
    "12.345678901234567890123456789", "abc", "", "1.2.3", "+5", "-5", ".5", "5.", "1__0", "\x1c7\x1f",
    "99999999999999999999.99", "0.000000000000000001", "123456789012345678.9012345678", "1E-30",
]
TEXTS = ["", " ", "x", " padded ", "a'b", 'a"b', "x\u0085y", "​", "café", "\x7f", "\t", "200", "Revenue",
         "\U0001f600", "a,b", 'q"uote', "line\nbreak"]
DATES = ["2026-06-30", "2026-05-31", "20260630", "2026-W27-2", "2026-W27", "2026-06-31", "2026-6-30", "2025-06-30",
         "2026-07-01", "2026-06-30x", "20260630xx", "", " 2026-06-30 "]
TIMESTAMPS = [
    "2026-06-30T10:00:00Z", "2026-06-30 10:00+10:00", "2026-06-30t10:00:00.5-00:00", "2026-06-30T24:00:00Z",
    "2026-06-30T24:00:01Z", "2026-06-30T10:00:00", "2026-06-30", "٢026-06-30T10:00:00Z",
    "٢026-06-30T10:00:00", "2026-06-30T10:00:00.1234567Z", "2026-06-30T10:00:00+24:00",
    "2026-02-29T00:00Z", "2026-06-30T10:00:00+10", "20260630T100000Z", "2026-06-30T10:00:60Z", " 2026-06-30T10:00Z ",
    "2026-06-30T23:59:59.999999+23:59:59", "",
]
JSON_VALUES = [None, True, False, 0, 1, 1.0, -1, 100, 101, 2.5, "x", "", " ", [], {}, [1], ["a"], {"a": 1},
               float("nan"), float("inf"), 10 ** 30]


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode("utf-8")


def sha(data):
    return hashlib.sha256(data).hexdigest()


def csv_bytes(rows, *, crlf=True, bom=False, quote_all=False):
    def cell(value):
        text = str(value)
        if quote_all or any(c in text for c in ',"\r\n'):
            return '"' + text.replace('"', '""') + '"'
        return text
    end = "\r\n" if crlf else "\n"
    body = end.join(",".join(cell(v) for v in row) for row in [COLUMNS, *rows]) + end
    return (("﻿" if bom else "") + body).encode("utf-8")


def parse_csv(data):

    import io
    reader = csv.reader(io.StringIO(data.decode("utf-8-sig"), newline=""))
    rows = list(reader)
    return [row for row in rows[1:] if row]


class Case:
    """Package-relative data overrides, working-directory files and a command line."""

    def __init__(self, base):
        self.data = dict(base)
        self.cwd_files = {}
        self.argv = []
        self.label = ""

    def json(self, path):
        return json.loads(self.data[path])

    def set_json(self, path, value):
        self.data[path] = json.dumps(value, indent=2).encode("utf-8")

    def reseal(self, manifest_path):
        manifest = self.json(manifest_path)
        csv_path = manifest["export"]["csv"]
        if csv_path in self.data:
            manifest["export"]["sha256"] = sha(self.data[csv_path])
            self.set_json(manifest_path, manifest)


def load_base(pkg):
    root = pkg / "elizabeth_anne_alexander"
    data = {}
    for top in ("policy", "samples"):
        for path in (root / top).rglob("*"):
            if path.is_file():
                data[path.relative_to(root).as_posix()] = path.read_bytes()
    return data


# ---------------------------------------------------------------- generators

def random_tb(rng, date, tenant, ids, section_of):
    rows = []
    total_debit = Decimal(0)
    total_credit = Decimal(0)
    ytd_debit = Decimal(0)
    ytd_credit = Decimal(0)
    for account_id in ids:
        def amount():
            if rng.random() < 0.3:
                return Decimal(0)
            digits = rng.choice([1, 2, 3, 5, 7, 10, 15])
            return Decimal(rng.randint(1, 10 ** digits)).scaleb(-rng.choice([0, 2, 2, 2, 3, 4]))
        d, c, yd, yc = amount(), amount(), amount(), amount()
        total_debit += d
        total_credit += c
        ytd_debit += yd
        ytd_credit += yc
        rows.append([date, tenant, section_of[account_id], account_id, f"Name {account_id}", rng.choice(["", "200", account_id[:3]]),
                     fmt(rng, d), fmt(rng, c), fmt(rng, yd), fmt(rng, yc)])
    gap = total_debit - total_credit
    ygap = ytd_debit - ytd_credit
    rows.append([date, tenant, section_of["balancing"], "balancing", "Balancing", "999",
                 fmt(rng, -gap if gap < 0 else Decimal(0)), fmt(rng, gap if gap > 0 else Decimal(0)),
                 fmt(rng, -ygap if ygap < 0 else Decimal(0)), fmt(rng, ygap if ygap > 0 else Decimal(0))])
    rng.shuffle(rows)
    return rows


def fmt(rng, value):
    style = rng.random()
    if style < 0.6:
        return str(value)
    if style < 0.75:
        return format(value, "f")
    if style < 0.85:
        return f"{value:E}"
    if style < 0.95:
        return format(value.quantize(Decimal("0.01")) if value.as_tuple().exponent > -2 else value, "f")
    return " " + str(value) + " "


def scenario_random_tb(rng, case):
    sections = ["Revenue", "Expenses", "Assets", "Liabilities", "Equity"]
    pool = [f"acct-{i}" for i in range(rng.randint(2, 12))] + rng.sample(["200", "400", "a'b", "café", "0001"], 2)
    current_ids = rng.sample(pool, rng.randint(1, len(pool)))
    prior_ids = rng.sample(pool, rng.randint(1, len(pool)))
    current_date, prior_date = rng.choice([("2026-06-30", "2026-05-31")] * 4 + [("2026-06-30", "2025-06-30"),
                                           ("2026-06-30", "2026-07-31"), ("2026-07-31", "2026-06-30"),
                                           ("2027-01-31", "2026-08-31")])
    tenant = rng.choice(["Demo Pty Ltd", "Tenant é", "T"])
    section_of = {account: rng.choice(sections[:2] if rng.random() < 0.7 else sections) for account in pool + ["balancing"]}
    current = random_tb(rng, current_date, tenant, current_ids, section_of)
    prior = random_tb(rng, prior_date, tenant, prior_ids, section_of)
    if rng.random() < 0.2 and current and prior:
        drift = rng.choice(current)[3]
        for row in prior:
            if row[3] == drift:
                row[2] = "Revenue" if row[2] != "Revenue" else "Expenses"
    for name, rows, date, manifest_path in (("2026-06-30", current, current_date, CURRENT_MANIFEST),
                                            ("2026-05-31", prior, prior_date, PRIOR_MANIFEST)):
        csv_path = f"samples/inputs/sample-tb-{name}.csv"
        case.data[csv_path] = csv_bytes(rows, crlf=rng.random() < 0.5, bom=rng.random() < 0.2, quote_all=rng.random() < 0.2)
        manifest = case.json(manifest_path)
        manifest["report"]["as_at"] = date
        case.set_json(manifest_path, manifest)
        case.reseal(manifest_path)
    policy = case.json(POLICY)
    operation = policy["operations"]["trial_balance_variance"]
    operation["minimum_absolute_delta"] = rng.choice(["0", "0.01", "100", "1000.00", "1E+3", "5000"])
    operation["minimum_percent_delta"] = rng.choice(["0", "0.5", "10", "33.3333", "1E+2", "0.0001"])
    operation["max_results"] = rng.choice([1, 2, 5, 100])
    operation["allowed_sections"] = sections
    case.set_json(POLICY, policy)
    request = case.json(REQUEST)
    request["section"] = rng.choice(sections)
    case.set_json(REQUEST, request)


def mutate_csv_cells(rng, case):
    name = rng.choice(["2026-06-30", "2026-05-31"])
    path = f"samples/inputs/sample-tb-{name}.csv"
    rows = parse_csv(case.data[path])
    for _ in range(rng.randint(1, 3)):
        if not rows:
            break
        row = rng.choice(rows)
        column = rng.randrange(len(COLUMNS))
        pool = AMOUNTS if column >= 6 else DATES if column == 0 else TEXTS
        if column < len(row):
            row[column] = rng.choice(pool)
    if rng.random() < 0.2 and rows:
        rows.append(list(rng.choice(rows)))
    if rng.random() < 0.15 and rows:
        rng.choice(rows).append("extra")
    if rng.random() < 0.15 and rows:
        victim = rng.choice(rows)
        del victim[rng.randrange(len(victim)):]
    case.data[path] = csv_bytes(rows, crlf=rng.random() < 0.5, bom=rng.random() < 0.2, quote_all=rng.random() < 0.3)
    if rng.random() < 0.85:
        case.reseal(CURRENT_MANIFEST if name == "2026-06-30" else PRIOR_MANIFEST)


def mutate_csv_text(rng, case):
    name = rng.choice(["2026-06-30", "2026-05-31"])
    path = f"samples/inputs/sample-tb-{name}.csv"
    data = case.data[path]
    choice = rng.randrange(9)
    if choice == 0:
        data = data.replace(b",", b";", 1)
    elif choice == 1:
        data = b"\n" + data + b"\n\n"
    elif choice == 2:
        data = data.replace(b"\n", b"\r", 2)
    elif choice == 3:
        data = data + b'"unterminated'
    elif choice == 4:
        data = data[: rng.randrange(len(data))]
    elif choice == 5:
        data = b"\xff" + data
    elif choice == 6:
        data = data.replace(b"ReportDate", b"reportdate")
    elif choice == 7:
        data = data.replace(b"\n", b'\n"x"y,', 1)
    else:
        data = data + b"," * 3 + b"\n" + ("x" * 131073).encode()
    case.data[path] = data
    if rng.random() < 0.85:
        case.reseal(CURRENT_MANIFEST if name == "2026-06-30" else PRIOR_MANIFEST)


def set_path(value, keys, new):
    target = value
    for key in keys[:-1]:
        target = target[key]
    target[keys[-1]] = new


def drop_path(value, keys):
    target = value
    for key in keys[:-1]:
        target = target[key]
    del target[keys[-1]]


MANIFEST_FIELDS = [["schema_version"], ["mode"], ["source_system"], ["entity_ref"], ["report"], ["export"],
                   ["report", "name"], ["report", "as_at"], ["report", "basis"], ["report", "currency"],
                   ["report", "tracking_filters"], ["report", "include_drafts"], ["export", "schema"],
                   ["export", "csv"], ["export", "sha256"], ["export", "generated_at"]]


def mutate_manifest(rng, case):
    path = rng.choice([CURRENT_MANIFEST, PRIOR_MANIFEST])
    manifest = case.json(path)
    keys = rng.choice(MANIFEST_FIELDS)
    action = rng.randrange(6)
    if action == 0:
        drop_path(manifest, keys)
    elif action == 1:
        set_path(manifest, keys, rng.choice(JSON_VALUES))
    elif action == 2 and keys[-1] in ("as_at",):
        set_path(manifest, keys, rng.choice(DATES))
    elif action == 2 and keys[-1] == "generated_at":
        set_path(manifest, keys, rng.choice(TIMESTAMPS))
    elif action == 3 and keys == ["export", "csv"]:
        set_path(manifest, keys, rng.choice(["samples/inputs/../inputs/sample-tb-2026-06-30.csv", "/etc/hostname",
                                             "../samples/inputs/sample-tb-2026-06-30.csv", "samples/inputs",
                                             "samples/inputs/missing.csv", "policy/demo-policy-v1.json",
                                             "samples\\inputs\\sample-tb-2026-06-30.csv", "samples//inputs/./sample-tb-2026-05-31.csv"]))
    elif action == 4 and keys == ["export", "sha256"]:
        manifest["export"]["sha256"] = manifest["export"]["sha256"].upper()
    elif action == 5:
        set_path(manifest, keys, rng.choice(TEXTS))
    else:
        manifest["report"]["tracking_filters"] = rng.choice([[], [1], [1.0], [True], ["x"], [float("nan")]])
    case.set_json(path, manifest)


def mutate_json_text(rng, case):
    path = rng.choice([CONTEXT, REQUEST, POLICY, CURRENT_MANIFEST, PRIOR_MANIFEST])
    data = case.data[path].decode("utf-8")
    choice = rng.randrange(10)
    if choice == 0:
        data = data.replace("{", '{"mode": "synthetic", ', 1)
    elif choice == 1:
        data = "﻿" + data
    elif choice == 2:
        data = data.rstrip().rstrip("}") + ',}'
    elif choice == 3:
        data = data.replace('"synthetic"', '"synth\\u0065tic"')
    elif choice == 4:
        data = data.replace('"synthetic"', '"\\ud800"')
    elif choice == 5:
        data = data + " x"
    elif choice == 6:
        data = "[" + data + "]"
    elif choice == 7:
        data = data.replace("{", '{"zz": ' + "[" * 3000 + "]" * 3000 + ", ", 1)
    elif choice == 8:
        data = data.replace("{", '{"zz": ' + "9" * 4400 + ", ", 1)
    else:
        data = data.replace('"', "'", 1)
    case.data[path] = data.encode("utf-8")


def mutate_context(rng, case):
    context = case.json(CONTEXT)
    key = rng.choice(["schema_version", "mode", "current_manifest", "prior_manifest"])
    context[key] = rng.choice(JSON_VALUES + [CURRENT_MANIFEST, PRIOR_MANIFEST, " " + PRIOR_MANIFEST + " ",
                                             "samples/manifests/../manifests/sample-tb-2026-05-31.manifest.json",
                                             "../x.json", "/tmp", "samples\x00x", "policy/demo-policy-v1.json"])
    case.set_json(CONTEXT, context)


def mutate_policy(rng, case):
    policy = case.json(POLICY)
    operation = policy["operations"]["trial_balance_variance"]
    action = rng.randrange(7)
    if action == 0:
        operation["max_results"] = rng.choice([0, 1, 100, 101, True, 1.0, "1", None, 10 ** 30, -1])
    elif action == 1:
        operation["minimum_absolute_delta"] = rng.choice(AMOUNTS + [1, None])
    elif action == 2:
        operation["minimum_percent_delta"] = rng.choice(AMOUNTS + [1, None])
    elif action == 3:
        operation["allowed_sections"] = rng.choice([["Revenue"], [], "Revenue", [1], ["Revenue", 2], ["revenue"]])
    elif action == 4:
        policy["model_projection"] = rng.choice([policy["model_projection"][::-1], "x", [], policy["model_projection"] + ["x"]])
    elif action == 5:
        policy["policy_id"] = rng.choice(JSON_VALUES)
    else:
        policy["operations"] = rng.choice([{}, {"trial_balance_variance": operation, "x": {}}, [], operation])
    case.set_json(POLICY, policy)


def mutate_request(rng, case):
    request = case.json(REQUEST)
    key = rng.choice(["schema_version", "request_id", "operation", "section", "policy_id"])
    request[key] = rng.choice(JSON_VALUES + ["Revenue", "Expenses", " ", "trial_balance_variance"])
    case.set_json(REQUEST, request)


EVALUATE_ARGS = [
    [CONTEXT, REQUEST, POLICY, "build/demo"],
]
PATH_VARIANTS = ["build/demo", "build", "build/a/b/c", "build/../build/x", "out", "/tmp", "build/demo/model-result.json",
                 "build/./x", "", "build/été", "build/x\x7fy"]
INPUT_VARIANTS = [CONTEXT, "./" + CONTEXT, "samples//contexts/sample-monthly-variance.context.json", "samples/contexts",
                  "samples/contexts/missing.json", "../samples/x.json", "/etc/hostname", "/nonexistent/x.json",
                  POLICY, "samples/requests/../requests/sample-revenue-variance.request.json"]


def evaluate_argv(rng, case):
    context, request, policy, out = CONTEXT, REQUEST, POLICY, "build/demo"
    if rng.random() < 0.15:
        context = rng.choice(INPUT_VARIANTS)
    if rng.random() < 0.1:
        request = rng.choice(INPUT_VARIANTS + [REQUEST])
    if rng.random() < 0.1:
        policy = rng.choice(INPUT_VARIANTS + ["policy//demo-policy-v1.json"])
    if rng.random() < 0.15:
        out = rng.choice(PATH_VARIANTS)
    if rng.random() < 0.1:
        case.cwd_files["build"] = b"not a directory"
    elif rng.random() < 0.1:
        case.cwd_files["build/demo/receipt.json"] = b"old"
    case.argv = ["evaluate", "--context", context, "--request", request, "--policy", policy, "--out", out]


def argv_fuzz(rng, case):
    tokens = ["evaluate", "validate-review", "evalute", "valid", "-h", "--help", "--he", "--context", "--cont", "--context=" + CONTEXT,
              CONTEXT, REQUEST, POLICY, "--request", "--policy", "--out", "build/x", "--evidence", "--receipt", "--decision",
              "--bogus", "-x", "-1", "-1.5", "--", "-", "", "a b", "--cont=a b", "--o", "--version", DECISION]
    case.argv = [rng.choice(tokens) for _ in range(rng.randint(0, 9))]


# ------------------------------------------------------------- validate-review

def pack_from_python(env, case):
    """Evaluate the case's data with Python and return the pack's files, or None."""
    work = Path(tempfile.mkdtemp(dir=env.scratch))
    try:
        pkg = env.materialise(case, work / "pkg")
        cwd = work / "cwd"
        cwd.mkdir()
        result = env.run_python(pkg, cwd, ["evaluate", "--context", CONTEXT, "--request", REQUEST, "--policy", POLICY, "--out", "build/demo"])
        if result.returncode != 0:
            return None
        return {name: (cwd / "build/demo" / name).read_bytes() for name in ("model-result.json", "reviewer-evidence.json", "receipt.json")}
    finally:
        shutil.rmtree(work, ignore_errors=True)


def mutate_pack(rng, decision, evidence, model, receipt):
    target = rng.randrange(8)
    if target == 0:
        decision["reviewed_at"] = rng.choice(TIMESTAMPS + JSON_VALUES)
    elif target == 1:
        decision["decisions"] = rng.choice([[], "x", [{}], decision["decisions"] * 2,
                                            [{"finding_id": "nope", "decision": "ACKNOWLEDGED", "rationale": "x"}],
                                            [dict(d, rationale=rng.choice(["", "  ", None, "\t"])) for d in decision["decisions"]],
                                            [dict(d, decision=rng.choice(["acknowledged", "APPROVED", 1])) for d in decision["decisions"]]])
    elif target == 2:
        decision[rng.choice(["run_id", "reviewer_ref", "schema_version"])] = rng.choice(JSON_VALUES + ["x"])
    elif target == 3:
        evidence["total_findings"] = rng.choice([True, 0, len(evidence["items"]) + 1, 1.0, -1, "1", 10 ** 30])
    elif target == 4:
        evidence["truncated"] = rng.choice([True, False, 0, None])
    elif target == 5 and evidence["items"]:
        item = rng.choice(evidence["items"])
        key = rng.choice(list(item))
        item[key] = rng.choice(JSON_VALUES + AMOUNTS[:8] + [{"debit": "1", "credit": "1", "ytd_debit": "x", "ytd_credit": "1"}])
    elif target == 6:
        model[rng.choice(list(model))] = rng.choice(JSON_VALUES)
    else:
        receipt["run_id"] = rng.choice(JSON_VALUES + [receipt["run_id"]])


def validate_case(rng, env, case):
    if rng.random() < 0.7:
        scenario_random_tb(rng, case)
    pack = pack_from_python(env, case)
    if pack is None:
        return False
    model, evidence, receipt = (json.loads(pack[n]) for n in ("model-result.json", "reviewer-evidence.json", "receipt.json"))
    findings = [item["finding_id"] for item in evidence["items"]]
    decision = {
        "schema_version": "xero-human-review-decision.v1",
        "run_id": receipt["run_id"],
        "reviewer_ref": "reviewer:1",
        "reviewed_at": "2026-07-01T09:00:00+10:00",
        "decisions": [{"finding_id": f, "decision": rng.choice(["ACKNOWLEDGED", "NEEDS_EVIDENCE", "ESCALATED"]), "rationale": "Checked."}
                      for f in findings if rng.random() < 0.8],
    }
    reseal = rng.random() < 0.7
    for _ in range(rng.randint(0, 3)):
        try:
            mutate_pack(rng, decision, evidence, model, receipt)
        except (TypeError, ValueError, AttributeError, KeyError, IndexError):
            pass  # an earlier mutation replaced what this one edits
    if reseal:
        receipt["evidence_sha256"] = "sha256:" + sha(canonical(evidence))
        receipt["result_sha256"] = "sha256:" + sha(canonical(model))
    files = {
        "build/demo/model-result.json": json.dumps(model, indent=2, sort_keys=True).encode() + b"\n",
        "build/demo/reviewer-evidence.json": json.dumps(evidence, indent=2, sort_keys=True).encode() + b"\n",
        "build/demo/receipt.json": json.dumps(receipt, indent=2, sort_keys=True).encode() + b"\n",
        "build/decision.json": json.dumps(decision, indent=2).encode() + b"\n",
    }
    if rng.random() < 0.15:
        del files["build/demo/model-result.json"]
    if rng.random() < 0.1:
        files["build/demo/reviewer-evidence.json"] = files["build/demo/reviewer-evidence.json"].replace(b'"', b'\\u0022', 0) + b" "
    case.cwd_files.update(files)
    decision_arg = rng.choice(["build/decision.json"] * 6 + [DECISION, "build/missing.json", "/etc/hostname", "samples/decisions",
                                                              "build/../build/decision.json", "build"])
    case.argv = ["validate-review", "--evidence", rng.choice(["build/demo/reviewer-evidence.json"] * 8 + ["build/demo/receipt.json", "/etc/hostname"]),
                 "--receipt", rng.choice(["build/demo/receipt.json"] * 8 + ["build/demo/missing.json", "receipt.json"]),
                 "--decision", decision_arg]
    if rng.random() < 0.5:
        case.argv += ["--out", rng.choice(["build/validation.json", "build/out/v.json", "build/decision.json",
                                           "build/demo/receipt.json", "build/demo/model-result.json", "validation.json",
                                           "build/demo", "build/./demo/../v.json", "build/link.json"])]
        if "build/link.json" in case.argv and rng.random() < 0.5:
            case.cwd_files["build/link.json"] = ("symlink", "demo/reviewer-evidence.json")
    return True


# ------------------------------------------------------------------- running

def child_env(**extra):
    """A minimal environment for both implementations; Windows needs its system variables."""
    keep = ("PATH", "SYSTEMROOT", "WINDIR", "TEMP", "TMP") if os.name == "nt" else ()
    base = {name: os.environ[name] for name in keep if name in os.environ}
    if os.name != "nt":
        base["PATH"] = "/usr/bin:/bin"
    return {**base, "LANG": "C.UTF-8", **extra}


class Env:
    def __init__(self, args):
        self.python = args.python
        self.binary = Path(args.binary).resolve()
        self.base = load_base(Path(args.python_pkg))
        self.pkg_source = Path(args.python_pkg) / "elizabeth_anne_alexander"
        self.scratch = Path(tempfile.mkdtemp(prefix="eaa-diff-"))
        self.runner = self.scratch / "runner.py"
        self.runner.write_text(RUNNER)

    def materialise(self, case, pkg):
        target = pkg / "elizabeth_anne_alexander"
        shutil.copytree(self.pkg_source, target, ignore=shutil.ignore_patterns("policy", "samples", "__pycache__"))
        for rel, data in case.data.items():
            path = target / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        return pkg

    def run_python(self, pkg, cwd, argv):
        return subprocess.run([self.python, "-I", "-B", str(self.runner), str(pkg), *argv], cwd=cwd, capture_output=True,
                              timeout=120, env=child_env(PYTHONUTF8="1"))

    def run_rust(self, bundle, cwd, argv):
        return subprocess.run([str(self.binary), *argv], cwd=cwd, capture_output=True, timeout=120,
                              env=child_env(EAA_DIFFERENTIAL_BUNDLE=str(bundle)))


def place(cwd, files):
    for rel, data in files.items():
        path = cwd / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(data, tuple):
            try:
                path.symlink_to(data[1])
            except OSError:
                pass  # Windows without symbolic link privilege: both sides lack the link
        else:
            path.write_bytes(data)


def snapshot(cwd):
    out = {}
    for path in sorted(cwd.rglob("*")):
        rel = path.relative_to(cwd).as_posix()
        if path.is_symlink():
            out[rel] = ("symlink", os.readlink(path))
        elif path.is_file():
            out[rel] = path.read_bytes()
        elif path.is_dir():
            out[rel] = "dir"
    return out


def normalise(text, replacements):
    for old, new in replacements:
        # Paths also appear inside repr() in OS error text, with doubled backslashes.
        for form in (old, old.replace("\\", "\\\\")):
            text = text.replace(form.encode(), new.encode())
    # Staging files carry random names in both implementations.
    return re.sub(rb"\.[A-Za-z0-9_]{8}\.partial", b".<random>.partial", text)


def compare(env, case, keep):
    work = Path(tempfile.mkdtemp(dir=env.scratch))
    try:
        pkg = env.materialise(case, work / "pkg")
        bundle = pkg / "elizabeth_anne_alexander"
        cwd_py, cwd_rs = work / "py", work / "rs"
        for cwd in (cwd_py, cwd_rs):
            cwd.mkdir()
            place(cwd, case.cwd_files)
        python = env.run_python(pkg, cwd_py, case.argv)
        rust = env.run_rust(bundle, cwd_rs, case.argv)
        problems = []
        if python.returncode != rust.returncode:
            problems.append(f"exit python={python.returncode} rust={rust.returncode}")
        if python.returncode != 1:
            py_map = [(str(bundle), "<bundled>"), (str(cwd_py), "<cwd>")]
            rs_map = [(str(cwd_rs), "<cwd>")]
            for stream in ("stdout", "stderr"):
                a = normalise(getattr(python, stream), py_map)
                b = normalise(getattr(rust, stream), rs_map)
                if a != b:
                    problems.append(f"{stream}\n  python: {a[:600]!r}\n  rust:   {b[:600]!r}")
            files_py, files_rs = snapshot(cwd_py), snapshot(cwd_rs)
            for rel in sorted(set(files_py) | set(files_rs)):
                a, b = files_py.get(rel), files_rs.get(rel)
                if rel.endswith("receipt.json") and isinstance(a, bytes) and isinstance(b, bytes):
                    a, b = (x.replace(b'"code_version": "0.2.6"', b'"code_version": "<v>"').replace(b'"code_version": "1.0.0"', b'"code_version": "<v>"') for x in (a, b))
                if a != b:
                    problems.append(f"file {rel}\n  python: {str(a)[:400]}\n  rust:   {str(b)[:400]}")
        if problems and keep:
            dest = Path(keep) / f"case-{case.label}"
            shutil.rmtree(dest, ignore_errors=True)
            shutil.copytree(work, dest, symlinks=True)
            (dest / "argv.json").write_text(json.dumps(case.argv))
        return problems, python.returncode
    finally:
        shutil.rmtree(work, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--python-pkg", required=True)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--python", default=sys.executable)
    parser.add_argument("--cases", type=int, default=500)
    parser.add_argument("--seed", type=int, default=20261011)
    parser.add_argument("--keep-failures")
    args = parser.parse_args()
    env = Env(args)
    rng = random.Random(args.seed)
    scenarios = [
        ("sample", lambda _: None),
        ("random-tb", lambda c: scenario_random_tb(rng, c)),
        ("csv-cells", lambda c: mutate_csv_cells(rng, c)),
        ("csv-text", lambda c: mutate_csv_text(rng, c)),
        ("manifest", lambda c: mutate_manifest(rng, c)),
        ("json-text", lambda c: mutate_json_text(rng, c)),
        ("context", lambda c: mutate_context(rng, c)),
        ("policy", lambda c: mutate_policy(rng, c)),
        ("request", lambda c: mutate_request(rng, c)),
    ]
    failures = 0
    outcomes = {}
    try:
        for index in range(args.cases):
            case = Case(env.base)
            kind = rng.random()
            if kind < 0.1:
                argv_fuzz(rng, case)
                name = "argv"
            elif kind < 0.4:
                if not validate_case(rng, env, case):
                    continue
                name = "validate"
            else:
                name, mutate = rng.choice(scenarios)
                mutate(case)
                if rng.random() < 0.3:
                    try:
                        rng.choice(scenarios)[1](case)
                    except (ValueError, KeyError, TypeError, IndexError, AttributeError, csv.Error):
                        pass  # the first mutation broke what the second one edits
                evaluate_argv(rng, case)
            case.label = f"{index}-{name}"
            problems, code = compare(env, case, args.keep_failures)
            outcomes[(name, code)] = outcomes.get((name, code), 0) + 1
            if problems:
                failures += 1
                if failures <= 25:
                    print(f"--- case {case.label}: argv={case.argv!r}")
                    for problem in problems:
                        print("  " + problem)
    finally:
        shutil.rmtree(env.scratch, ignore_errors=True)
    summary = ", ".join(f"{name}/exit {code}: {count}" for (name, code), count in sorted(outcomes.items()))
    print(f"python {sys.version.split()[0]} as oracle {args.python}: {sum(outcomes.values())} cases, {failures} with differences")
    print(f"python exit codes by scenario: {summary}")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
