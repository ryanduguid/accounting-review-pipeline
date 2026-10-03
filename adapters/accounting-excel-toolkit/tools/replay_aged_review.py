"""Replay deterministic fabricated success and failure families through the real CLI."""
from __future__ import annotations

import argparse
import copy
import csv
import hashlib
import io
import json
import subprocess  # nosec B404 - each fabricated case exercises the real CLI.
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPOSITORY = Path(__file__).resolve().parents[3]
SAMPLES = HERE.parent / "samples"


def catalogue():
    base = (SAMPLES / "sample-aged-receivables-review.csv").read_bytes()
    manifest = json.loads((SAMPLES / "sample-aged-receivables-manifest.json").read_text())
    control = json.loads((SAMPLES / "sample-aged-receivables-control.json").read_text())
    records = list(csv.reader(io.StringIO(base.decode(), newline="")))
    cases = []
    for name, expected, message in [
        ("clean", 0, ""), ("wrong-hash", 2, "exact source bytes"),
        ("row-total", 2, "record 9 buckets"), ("column-offset", 2, "contact column Current"),
        ("wrong-period", 2, "control tie is unsupported"), ("missing-control", 2, "control evidence is missing"),
        ("missing-footer", 2, "footer has not been identified"),
        ("short-record", 1, "field count"), ("extra-record", 1, "field count"),
        ("bad-amount", 1, "finite signed amount"), ("unterminated-quote", 1, "unexpected end"),
        ("after-footer", 1, "Unexpected record"),
    ]:
        rows = copy.deepcopy(records)
        evidence, independent = copy.deepcopy(manifest), copy.deepcopy(control)
        if name == "row-total":
            rows[8][-1] = "11"
        elif name == "column-offset":
            rows[-1][1], rows[-1][-2] = "90", "10"
        elif name == "wrong-period":
            independent["as_at"] = "2026-08-31"
        elif name == "missing-control":
            independent = None
        elif name == "missing-footer":
            evidence["footer_record"] = None
        elif name == "short-record":
            rows[6].pop()
        elif name == "extra-record":
            rows[6].append("extra")
        elif name == "bad-amount":
            rows[6][1] = "TBC"
        elif name == "after-footer":
            rows.append(["Fabricated", "0", "0", "0", "0", "0", "0", "0"])
        stream = io.StringIO(newline="")
        csv.writer(stream).writerows(rows)
        content = stream.getvalue().encode("utf-8")
        if name == "unterminated-quote":
            content += b'"unfinished'
        evidence["source_sha256"] = "0" * 64 if name == "wrong-hash" else hashlib.sha256(content).hexdigest()
        cases.append({"case": name, "content": content, "manifest": evidence, "control": independent,
                      "expected_exit": expected, "expected_status": {0: "PASS", 2: "REVIEW", 1: "INVALID_INPUT"}[expected],
                      "expected_message": message})
    return cases


def replay(selected="all"):
    cases = catalogue()
    if selected != "all":
        cases = [case for case in cases if case["case"] == selected]
        if not cases:
            raise ValueError("Unknown replay case.")
    outcomes = []
    with tempfile.TemporaryDirectory(prefix="fabricated-aged-replay-") as temporary:
        root = Path(temporary)
        for case in cases:
            source, manifest, control = root / "source.csv", root / "manifest.json", root / "control.json"
            source.write_bytes(case["content"])
            manifest.write_text(json.dumps(case["manifest"]), encoding="utf-8")
            command = [sys.executable, str(HERE / "xero_aged_receivables.py"), str(source), "--manifest", str(manifest)]
            if case["control"] is not None:
                control.write_text(json.dumps(case["control"]), encoding="utf-8")
                command += ["--control", str(control)]
            # The list selects our interpreter and reviewed CLI, with fabricated local file paths.
            result = subprocess.run(command, capture_output=True, text=True)  # nosec B603 # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit
            body = result.stderr if result.returncode == 1 else result.stdout
            output = json.loads(body)
            expected = case["expected_message"]
            if (result.returncode != case["expected_exit"] or output["summary_status"] != case["expected_status"]
                    or (expected and expected.lower() not in body.lower())
                    or (result.returncode != 1 and output["debtor_decisions"] != "REVIEW")):
                raise RuntimeError(f"Replay {case['case']} failed its output contract: {result.returncode}: {body}")
            outcomes.append({"case": case["case"], "expected_exit": case["expected_exit"],
                             "actual_exit": result.returncode, "source_sha256": hashlib.sha256(case["content"]).hexdigest(),
                             "response": output})
    return {"base_sha256": hashlib.sha256((SAMPLES / "sample-aged-receivables-review.csv").read_bytes()).hexdigest(),
            "cases": outcomes, "boundary": "Fabricated failure demonstrations; no accounting decision or assurance."}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case", default="all", choices=["all", *[case["case"] for case in catalogue()]])
    parser.add_argument("--output", type=Path, required=True, help="new external replay JSON file")
    args = parser.parse_args()
    if args.output.resolve().is_relative_to(REPOSITORY) or args.output.exists():
        parser.error("Choose a new output outside the checkout.")
    pack = replay(args.case)
    # The CLI intentionally creates the operator's new external output directory.
    args.output.parent.mkdir(parents=True, exist_ok=True)  # NOSONAR
    with args.output.open("x", encoding="utf-8") as stream:
        stream.write(json.dumps(pack, indent=2, ensure_ascii=True) + "\n")
    print("FAILURE REPLAY QUALIFIED")
