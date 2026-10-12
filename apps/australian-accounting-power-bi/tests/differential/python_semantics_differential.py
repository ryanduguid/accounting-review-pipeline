"""Compare the test suite's reimplementations of Python behaviour with Python itself.

    python python_semantics_differential.py <path to target/debug/examples/py_oracle> \
        [--cases N] [--seed S]

Four behaviours the Rust tests rely on, each over generated input plus a directed
corpus: csv.reader and csv.DictReader over text read with newline="" (including the
default 131,072-character field limit), date.fromisoformat,
and the BPA rule evaluation the Python suite performed by translating Dynamic LINQ to
Python and calling eval.
"""

import argparse
import csv
import datetime
import io
import json
import random
import re
import subprocess
import sys
from typing import Any

CSV_TOKENS = ['a', 'b', ',', '"', '""', '\r', '\n', '\r\n', ' ', '1', 'x"y', '","', '\t']
CSV_DIRECTED = ["", "\n", "a,b\n\n", "a,\n", '"a\nb",c', '"unterminated', 'x"y,z', '"a"b,c', "\r\r\n\n",
                "\nAccountCode,Class\n100,Asset\n", "a,b\n1\n", "a,b\n1,2,3\n", "a,a\n1,2\n", "a\n\n\n1\n",
                "a" * 131072, "a" * 131073, '"' + "b" * 131072 + '"', '"' + "b" * 131073 + '"',
                "x," + "c" * 131073 + "\n", "h\n" + "d" * 131072 + "\n" + "e" * 131072]
DATE_DIRECTED = ["2026-07-29", "20260729", "2026-W31-3", "2026W313", "2026-W31", "2026W31", "2026-W53-1",
                 "2020-W53-7", "2026-02-29", "2024-02-29", "0000-01-01", "9999-12-31", "2026-1-015",
                 "2026101599", "2026-10-1x", "2026W31-3", "2026-W313", "2026-W00-1", "2026-W01-8", "٢026-01-01"]
PROPERTIES = ["A", "N", "F", "Name"]
BPA_DIRECTED = [
    'A == "never" && string.IsNullOrEmpty(Unknown)', "N > 2 || Unknown == 1", "F || Unknown",
    '!(F || (A == "y" && Unknown < 3))', "string.IsNullOrEmpty(FormatString) && DataType <> \"String\"",
    "!IsHidden && string.IsNullOrEmpty(Description)", 'CrossFilteringBehavior <> "OneDirection"',
    "Precedence <= 0", "A < 3", "N == F", "!N != 1", "(A", "A &&", "N >= 3 >= 1",
]


def python_evaluate(expression: str, obj: dict[str, object]) -> bool:
    """The deleted test_model_bpa_rules.evaluate, unchanged."""
    translated = re.sub(
        r"string\.IsNullOrEmpty\((\w+)\)", r"(not \1)", expression, flags=re.IGNORECASE
    )
    translated = translated.replace("&&", " and ").replace("||", " or ")
    translated = translated.replace("<>", "!=")
    translated = re.sub(r"!(?!=)", " not ", translated)
    return bool(eval(translated, {"__builtins__": {}}, dict(obj)))  # pylint: disable=eval-used


def expected(request: dict[str, Any]) -> Any:
    text = request["text"]
    mode = request["mode"]
    try:
        if mode == "csv":
            return [list(row) for row in csv.reader(io.StringIO(text, newline=""))]
        if mode == "dict":
            reader = csv.DictReader(io.StringIO(text, newline=""))
            rows = [{key: value for key, value in row.items() if key is not None} for row in reader]
            return {"header": list(reader.fieldnames or []), "rows": rows}
    except csv.Error:
        return "error"
    if mode == "date":
        try:
            return datetime.date.fromisoformat(text).isoformat()
        except ValueError:
            return "error"
    try:
        return python_evaluate(text, request["object"])
    except Exception:  # noqa: BLE001 - any Python failure is a failing verdict
        return "error"


def random_expression(rng: random.Random, depth: int = 0) -> str:
    if depth > 2 or rng.random() < 0.3:
        atom = rng.choice(["A", "N", "F", "Name", "Unknown", '"x"', '""', "0", "3", "string.IsNullOrEmpty(A)",
                           "string.IsNullOrEmpty(Unknown)"])
        if rng.random() < 0.4:
            other = rng.choice(["A", "N", "F", "Unknown", '"x"', "3", "0"])
            atom = f"{atom} {rng.choice(['==', '<>', '!=', '<', '<=', '>', '>='])} {other}"
        return atom
    choice = rng.random()
    if choice < 0.4:
        return f"{random_expression(rng, depth + 1)} {rng.choice(['&&', '||'])} {random_expression(rng, depth + 1)}"
    if choice < 0.6:
        return f"!{random_expression(rng, depth + 1)}"
    return f"({random_expression(rng, depth + 1)})"


def random_object(rng: random.Random) -> dict[str, object]:
    values: list[object] = ["", "x", "y", 0, 3, -1, True, False]
    return {name: rng.choice(values) for name in PROPERTIES if rng.random() < 0.85}


def requests(rng: random.Random, cases: int) -> list[dict[str, Any]]:
    batch: list[dict[str, Any]] = []
    for text in CSV_DIRECTED:
        batch += [{"mode": "csv", "text": text}, {"mode": "dict", "text": text}]
    batch += [{"mode": "date", "text": text} for text in DATE_DIRECTED]
    batch += [{"mode": "bpa", "text": text, "object": random_object(rng)} for text in BPA_DIRECTED for _ in range(20)]
    for _ in range(cases):
        kind = rng.random()
        if kind < 0.3:
            text = "".join(rng.choice(CSV_TOKENS) for _ in range(rng.randint(0, 14)))
            batch.append({"mode": rng.choice(["csv", "dict"]), "text": text})
        elif kind < 0.6:
            text = "".join(rng.choice("0123456789-W2") for _ in range(rng.choice([7, 8, 10, rng.randint(0, 12)])))
            batch.append({"mode": "date", "text": text})
        else:
            batch.append({"mode": "bpa", "text": random_expression(rng), "object": random_object(rng)})
    return batch


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("oracle")
    parser.add_argument("--cases", type=int, default=100000)
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()
    batch = requests(random.Random(args.seed), args.cases)
    stdin = "".join(json.dumps(request) + "\n" for request in batch)
    completed = subprocess.run([args.oracle], input=stdin, capture_output=True, text=True, check=True)
    answers = [json.loads(line) for line in completed.stdout.splitlines()]
    mismatches = [(request, got, want) for request, got in zip(batch, answers)
                  if got != (want := expected(request))]
    for request, got, want in mismatches[:20]:
        print(f"{json.dumps(request)}\n  rust   {json.dumps(got)}\n  python {json.dumps(want)}")
    counts: dict[str, int] = {}
    for request in batch:
        counts[request["mode"]] = counts.get(request["mode"], 0) + 1
    print(f"{len(batch)} requests {counts}, {len(mismatches)} mismatches")
    return 1 if mismatches or len(answers) != len(batch) else 0


if __name__ == "__main__":
    sys.exit(main())
