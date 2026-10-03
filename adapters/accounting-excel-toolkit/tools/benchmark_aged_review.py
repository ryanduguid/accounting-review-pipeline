"""Measure fabricated file-to-review workflows, without a performance threshold."""
from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
import platform
import statistics
import subprocess
import sys
import tempfile
import time
import tracemalloc
from pathlib import Path

from debtor_evidence import briefing
from xero_aged_receivables import inspect_export

HERE = Path(__file__).resolve().parent


def workload(count):
    # Each workload is fabricated afresh, with an explicitly supplied zero control.
    records = [["Aged Receivables Summary"], ["Fabricated Benchmark Entity"],
               ["As at 30 September 2026"], ["Ageing by due date"], [],
               ["Contact", "Current", "< 1 Month", "1 Month", "2 Months", "3 Months", "Older", "Total"]]
    records += [[f"{number:05}", *["0"] * 7] for number in range(count)]
    records.append(["Total", *["0"] * 7])
    stream = io.StringIO(newline="")
    csv.writer(stream).writerows(records)
    content = stream.getvalue().encode()
    manifest = {"report_name": "Aged Receivables Summary", "entity": "Fabricated Benchmark Entity",
                "as_at": "2026-09-30", "ageing_basis": "due date", "accounting_basis": "accrual",
                "currency": "AUD", "filters": "all", "population": "fabricated zero contacts",
                "generated_at": "2026-10-01T00:00:00+10:00", "source_format": "CSV",
                "source_sha256": hashlib.sha256(content).hexdigest(), "footer_record": count + 7,
                "on_screen_rows": count, "on_screen_total": "0"}
    control = {key: manifest[key] for key in ("entity", "as_at", "currency", "accounting_basis", "filters", "population")}
    control.update(amount="0", source="Fabricated independent zero control", sign_convention="debit-positive")
    return content, manifest, control


def measure(root):
    tracemalloc.start()
    started = time.perf_counter_ns()
    content = (root / "source.csv").read_bytes()
    manifest = json.loads((root / "manifest.json").read_text(encoding="utf-8"))
    control = json.loads((root / "control.json").read_text(encoding="utf-8"))
    loaded = time.perf_counter_ns()
    result = inspect_export(content, manifest, control)
    reviewed = time.perf_counter_ns()
    rendered = json.dumps(result, ensure_ascii=True) + "\n" + briefing(result)
    (root / "review.txt").write_text(rendered, encoding="utf-8")
    finished = time.perf_counter_ns()
    _, peak = tracemalloc.get_traced_memory()
    tracemalloc.stop()
    if result["summary_status"] != "PASS" or result["debtor_decisions"] != "REVIEW" or len(result["rows"]) != manifest["on_screen_rows"]:
        raise RuntimeError("Fabricated workload failed its review contract.")
    return {"rows": len(result["rows"]), "input_bytes": len(content), "output_bytes": len(rendered.encode()),
            "load_ns": loaded - started, "review_ns": reviewed - loaded,
            "render_write_ns": finished - reviewed, "workflow_ns": finished - started,
            "peak_traced_python_bytes": peak, "source_sha256": hashlib.sha256(content).hexdigest()}


def benchmark():
    results = []
    with tempfile.TemporaryDirectory(prefix="fabricated-aged-benchmark-") as temporary:
        root = Path(temporary)
        for count in (10, 1000, 10000):
            content, manifest, control = workload(count)
            (root / "source.csv").write_bytes(content)
            for name, value in (("manifest", manifest), ("control", control)):
                (root / f"{name}.json").write_text(json.dumps(value), encoding="utf-8")
            measure(root)  # One explicit warm-up per fixed workload.
            warm = [measure(root) for _ in range(3)]
            started = time.perf_counter_ns()
            cold = subprocess.run([sys.executable, str(Path(__file__)), "--worker", str(root)],
                                  capture_output=True, text=True, check=True)
            fresh = json.loads(cold.stdout)
            fresh["process_wall_ns"] = time.perf_counter_ns() - started
            results.append({"workload": f"zero-contacts-{count}", "warm_up_count": 1, "repetitions": 3,
                            "warm_runs": warm, "warm_median_ns": statistics.median(row["workflow_ns"] for row in warm),
                            "fresh_process": fresh})
    return {"measurement_schema_version": 1, "python": sys.version, "platform": platform.platform(),
            "workloads": results, "boundary": "Synthetic file loading, validation, rendering and writing. "
            "Fresh process includes interpreter startup; filesystem cache is uncontrolled. "
            "Traced Python allocation is not RSS. No human time, production SLA or speed improvement is measured."}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--output", type=Path, help="new external measurement JSON file")
    args = parser.parse_args()
    if args.worker:
        print(json.dumps(measure(args.worker)))
    else:
        if args.output is None or args.output.exists() or args.output.resolve().is_relative_to(HERE.parent):
            parser.error("Choose a new measurement file outside the checkout.")
        results = benchmark()
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with args.output.open("x", encoding="utf-8") as stream:
            stream.write(json.dumps(results, indent=2) + "\n")
        print("WORKFLOW MEASUREMENTS QUALIFIED")
