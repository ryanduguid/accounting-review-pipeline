"""Differential test: grant_workpaper.py against the Rust grant-workpaper binary.

    python differential.py --script <grant_workpaper.py from the Python release> \
        --binary <target/debug/grant-workpaper> [--cases N] [--seed S]

Each case writes the same mutated inputs into two working directories, runs
both implementations with the same arguments, and compares exit codes,
stdout, stderr and every file left behind. The program name in argparse
messages, the working directory path and random staging names are normalised;
a CPython traceback is compared by exit code only.
"""

import argparse
import csv
import io
import json
import os
import random
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

EXAMPLE = Path(__file__).resolve().parents[2] / "examples" / "two-grants"
FILES = ("agreements.json", "ledger.csv", "allocations.csv", "receipts.csv")
HEADINGS = {
    "ledger.csv": "ledger_id date description amount cash_paid evidence",
    "allocations.csv": "allocation_id ledger_id grant_id budget_line amount cash_allocated approved evidence",
    "receipts.csv": "receipt_id grant_id date amount evidence",
    "contributions.csv": "contribution_id grant_id date kind amount evidence",
}
AMOUNTS = ["0", "1", "600", "601", "480", "1000", "12.5", "12.50", "0.01", "007", "1234567890123456", "12345678901234567",
           "1.234", "", "-1", "1e3", "NaN", "١٢", " 5", "5 ", "1_000", "2319.99", "3599.99", "9999"]
DATES = ["2026-07-01", "2026-10-01", "2026-10-15", "2027-06-30", "2027-07-01", "2026-06-30", "20261015",
         "2026-W42-4", "2026-13-01", "2026-02-29", "", "2026-1-01", "٢026-10-15"]
TEXTS = ["", " ", "x", "Fabricated evidence", "=1+1", "@SUM(A1)", "-1+1", "+1", "-001", "​", "\u0080", "\u202e",
         "́", "́A", "A ́B", "Café 中", "a,b", 'q"uote', "line\nbreak", "\U0001f600", "[x](y) *z*",
         "A & B's <tag>", " ", " ", "#heading", "back\\slash"]
JSON_VALUES = [None, True, False, 0, 1, 1.0, "x", "", " ", [], {}, ["a"], {"a": 1}, float("inf"), 10 ** 30, "1000", "12.5"]


def child_env(**extra):
    keep = ("PATH", "SYSTEMROOT", "WINDIR", "TEMP", "TMP") if os.name == "nt" else ()
    base = {name: os.environ[name] for name in keep if name in os.environ}
    if os.name != "nt":
        base["PATH"] = "/usr/bin:/bin"
    return {**base, "LANG": "C.UTF-8", "PYTHONUTF8": "1", **extra}


def read_rows(data):
    reader = csv.DictReader(io.StringIO(data.decode("utf-8")))
    return reader.fieldnames, list(reader)


def write_rows(fields, rows, crlf=True):
    stream = io.StringIO()
    writer = csv.DictWriter(stream, fields, lineterminator="\r\n" if crlf else "\n")
    writer.writeheader()
    writer.writerows(rows)
    return stream.getvalue().encode("utf-8")


class Case:
    def __init__(self):
        self.files = {name: (EXAMPLE / name).read_bytes() for name in FILES}
        self.extra = {}
        self.argv = ["--input", "in", "--output", "out"]
        self.label = ""
        self.input_name = None  # the input directory's name when it is not "in"


def mutate_cells(rng, case):
    name = rng.choice(list(HEADINGS)[:3] + ["contributions.csv"])
    if name == "contributions.csv":
        rows = [{"contribution_id": f"C{i}", "grant_id": rng.choice(["G-A", "G-B", "G-X"]),
                 "date": rng.choice(DATES[:6]), "kind": rng.choice(["interest", "cash_contribution", "in_kind", "donation"]),
                 "amount": rng.choice(AMOUNTS[:10]), "evidence": rng.choice(TEXTS[:4])} for i in range(rng.randint(0, 4))]
        case.files[name] = write_rows(HEADINGS[name].split(), rows)
        return
    fields, rows = read_rows(case.files[name])
    for _ in range(rng.randint(1, 3)):
        if not rows:
            break
        row = rng.choice(rows)
        key = rng.choice(fields)
        pool = AMOUNTS if key in ("amount", "cash_paid", "cash_allocated") else DATES if key == "date" else (
            ["yes", "no", "Yes", ""] if key == "approved" else TEXTS + ["G-A", "G-B", "L1", "Delivery", "Administration"])
        row[key] = rng.choice(pool)
    if rng.random() < 0.15 and rows:
        rows.append(dict(rng.choice(rows)))
    if rng.random() < 0.1:
        rows = []
    case.files[name] = write_rows(fields, rows, crlf=rng.random() < 0.5)


def mutate_csv_text(rng, case):
    name = rng.choice(list(HEADINGS)[:3])
    data = case.files[name]
    choice = rng.randrange(9)
    if choice == 0:
        data = data.replace(b",", b";", 1)
    elif choice == 1:
        data = b"\xef\xbb\xbf" + data
    elif choice == 2:
        data = data.replace(b"\n", b"\r", 1)
    elif choice == 3:
        data = data + b'"unterminated'
    elif choice == 4:
        data = data[: rng.randrange(len(data))]
    elif choice == 5:
        data = b"\xff" + data
    elif choice == 6:
        data = data.replace(b"\n", b'\n"x"y,', 1)
    elif choice == 7:
        data = data + b"a,b,c,d,e,f,g,h,i,j\n"
    else:
        data = data + ("x" * 131073).encode() + b"\n"
    case.files[name] = data


def mutate_agreements(rng, case):
    document = json.loads(case.files["agreements.json"])
    target = rng.randrange(7)
    if target == 0:
        document[rng.choice(list(document))] = rng.choice(JSON_VALUES + DATES[:3] + TEXTS[:6])
    elif target == 1:
        grant = rng.choice(document["agreements"])
        grant[rng.choice(list(grant))] = rng.choice(JSON_VALUES + DATES + AMOUNTS + TEXTS)
    elif target == 2:
        grant = rng.choice(document["agreements"])
        grant["budget"] = rng.choice([{}, {"Delivery": "3600"}, {"Delivery": "3599.99", "Administration": "2000"},
                                      {"́": "1"}, {"Delivery": 10}, [], {"Delivery": "10000", "Administration": "2000"}])
    elif target == 3:
        document["agreements"] = rng.choice([[], "x", [1], document["agreements"] * 2, document["agreements"][:1]])
    elif target == 4:
        del document[rng.choice(list(document))]
    elif target == 5:
        document["schema_version"] = rng.choice([1, True, 1.0, 2, "1"])
    else:
        document["entity"] = rng.choice(TEXTS)
    case.files["agreements.json"] = json.dumps(document, indent=rng.choice([None, 2])).encode("utf-8")


def mutate_json_text(rng, case):
    data = case.files["agreements.json"].decode("utf-8")
    choice = rng.randrange(12)
    if choice == 0:
        data = data.replace("{", '{"currency": "AUD", ', 1)
    elif choice == 1:
        data = "﻿" + data
    elif choice == 2:
        data = data.rstrip().rstrip("}") + ",}"
    elif choice == 3:
        data = data + " x"
    elif choice == 4:
        data = data.replace('"', "'", 1)
    elif choice == 5:
        data = data[: rng.randrange(len(data))]
    elif choice == 6:
        data = data.replace('"G-A"', '"G-\\u0041"')
    elif choice == 7:
        data = data.replace('"G-A"', '"\\ud800"')
    elif choice == 8:
        data = data.replace("{", '{"zz": ' + "[" * 3000 + "]" * 3000 + ", ", 1)
    elif choice == 9:
        data = data.replace('"schema_version": 1', '"schema_version": ' + "1" * 4400)
    elif choice == 10:
        case.files["agreements.json"] = data.encode("utf-16")
        return
    else:
        data = data.replace('"G-A"', '"G\\x"')
    case.files["agreements.json"] = data.encode("utf-8")


# Names Python decodes with surrogateescape (POSIX bytes) or keeps as lone
# surrogates (Windows UTF-16), so the filesystem sees the original name.
ODD_NAMES = [b"in\xff", b"\xe9t\xe9", b"a\x80b"] if os.name != "nt" else ["in\ud800", "\udc80x", "a\udfffb"]
# Root and drive inputs, read only. UNC forms are left out on Windows: they would contact a server.
ROOT_INPUTS = ["/", "//", "///", "//x/./y"] if os.name != "nt" else ["C:", "\\", "C:x", "\\\\?\\C:", "/"]


def output_variant(rng, case):
    choice = rng.randrange(12)
    if choice == 0:
        case.extra["out"] = b"existing file"
    elif choice == 1:
        case.extra["out/"] = None
    elif choice == 2:
        case.argv[3] = "nested/new/out"
    elif choice == 3:
        case.extra["blocker"] = b"file"
        case.argv[3] = "blocker/out"
    elif choice == 4:
        case.argv[3] = "./out/"
    elif choice == 5:
        case.argv[1] = rng.choice(["missing", "in/agreements.json", ".", ""])
    elif choice == 6:
        case.extra["in/contributions.csv/"] = None
    elif choice == 7:
        case.argv[3] = "outé"
    elif choice == 8:
        case.argv[1] = rng.choice(ROOT_INPUTS)
    elif choice == 9:
        case.input_name = case.argv[1] = rng.choice(ODD_NAMES)
    elif choice == 10:
        case.argv[3] = rng.choice(ODD_NAMES)
    else:
        case.argv.insert(rng.randrange(len(case.argv) + 1), rng.choice(ODD_NAMES))


def argv_fuzz(rng, case):
    tokens = ["--input", "--in", "--input=in", "in", "--output", "--out", "--o=out", "out", "-h", "--help", "-hx", "--help=1",
              "--bogus", "-x", "-1", "--", "-", "", "a b", "extra"]
    case.argv = [rng.choice(tokens) for _ in range(rng.randint(0, 7))]


SCENARIOS = [("sample", lambda rng, c: None), ("cells", mutate_cells), ("csv-text", mutate_csv_text),
             ("agreements", mutate_agreements), ("json-text", mutate_json_text), ("output", output_variant)]


def place(cwd, case):
    (cwd / "in").mkdir()
    for name, data in case.files.items():
        (cwd / "in" / name).write_bytes(data)
    if case.input_name is not None:
        (cwd / "in").rename(cwd / os.fsdecode(case.input_name))
    for rel, data in case.extra.items():
        path = cwd / rel
        if data is None:
            if not path.is_file():
                path.mkdir(parents=True, exist_ok=True)
        elif not path.is_dir():  # an earlier extra already made this a directory
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)


def snapshot(cwd):
    out = {}
    for path in sorted(cwd.rglob("*")):
        rel = path.relative_to(cwd).as_posix()
        out[rel] = path.read_bytes() if path.is_file() else "dir"
    return out


def normalise(data, cwd, progs):
    for form in (str(cwd), str(cwd).replace("\\", "\\\\")):
        data = data.replace(form.encode(), b"<x>")
    # Only argparse names the program; "grant-workpaper:" in other messages is literal.
    for prog in progs:
        data = data.replace(b"usage: " + prog.encode(), b"usage: <prog>")
        data = data.replace(prog.encode() + b": error:", b"<prog>: error:")
    return re.sub(rb"\.grant-workpaper-[a-z0-9_]{8}", b".grant-workpaper-<random>", data)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--script", required=True)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--cases", type=int, default=400)
    parser.add_argument("--seed", type=int, default=20261012)
    args = parser.parse_args()
    script, binary = Path(args.script).resolve(), Path(args.binary).resolve()
    rng = random.Random(args.seed)
    scratch = Path(tempfile.mkdtemp(prefix="grant-diff-"))
    failures, outcomes = 0, {}
    try:
        for index in range(args.cases):
            case = Case()
            if rng.random() < 0.1:
                argv_fuzz(rng, case)
                name = "argv"
            else:
                name, mutate = rng.choice(SCENARIOS)
                mutate(rng, case)
                if rng.random() < 0.3:
                    try:
                        rng.choice(SCENARIOS)[1](rng, case)
                    except (ValueError, KeyError, TypeError, IndexError, AttributeError, csv.Error, UnicodeError):
                        pass  # the first mutation broke what the second one edits
            case.label = f"{index}-{name}"
            work = scratch / case.label
            cwd_py, cwd_rs = work / "py", work / "rs"
            for cwd in (cwd_py, cwd_rs):
                cwd.mkdir(parents=True)
                place(cwd, case)
            python = subprocess.run([sys.executable, "-I", str(script), *case.argv], cwd=cwd_py, capture_output=True,
                                    timeout=120, env=child_env())
            rust = subprocess.run([str(binary), *case.argv], cwd=cwd_rs, capture_output=True, timeout=120, env=child_env())
            problems = []
            if python.returncode != rust.returncode:
                problems.append(f"exit python={python.returncode} rust={rust.returncode}")
            if b"Traceback (most recent call last)" not in python.stderr:
                for stream in ("stdout", "stderr"):
                    a = normalise(getattr(python, stream), cwd_py, (script.name,))
                    b = normalise(getattr(rust, stream), cwd_rs, (binary.name,))
                    if a != b:
                        problems.append(f"{stream}\n  python: {a[:500]!r}\n  rust:   {b[:500]!r}")
                files_py, files_rs = snapshot(cwd_py), snapshot(cwd_rs)
                for rel in sorted(set(files_py) | set(files_rs)):
                    if files_py.get(rel) != files_rs.get(rel):
                        problems.append(f"file {rel}\n  python: {str(files_py.get(rel))[:300]}\n  rust:   {str(files_rs.get(rel))[:300]}")
            key = (name, python.returncode)
            outcomes[key] = outcomes.get(key, 0) + 1
            if problems:
                failures += 1
                if failures <= 25:
                    print(f"--- case {case.label}: argv={case.argv!r}")
                    for problem in problems:
                        print("  " + problem)
            shutil.rmtree(work, ignore_errors=True)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    summary = ", ".join(f"{n}/exit {c}: {k}" for (n, c), k in sorted(outcomes.items()))
    print(f"python {sys.version.split()[0]}: {sum(outcomes.values())} cases, {failures} with differences")
    print(f"python exit codes by scenario: {summary}")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
