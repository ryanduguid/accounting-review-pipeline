"""Compare the test suite's CSV reader with Python's csv.reader on random text.

    python csv_differential.py <path to target/debug/examples/csv_oracle> [--cases N] [--seed S]

The text is read as `open(..., newline="")` reads it: the strings go to csv.reader
through io.StringIO with newline="", so carriage returns reach the parser.
"""

import argparse
import csv
import io
import json
import random
import subprocess
import sys

ALPHABET = ['a', 'b', ',', '"', '""', '\r', '\n', '\r\n', ' ', '1', 'x"y', '","', '\t']


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("oracle")
    parser.add_argument("--cases", type=int, default=50000)
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()
    rng = random.Random(args.seed)
    texts = ["".join(rng.choice(ALPHABET) for _ in range(rng.randint(0, 14))) for _ in range(args.cases)]
    texts += ["", "\n", "a,b\n\n", "a,\n", '"a\nb",c', '"unterminated', 'x"y,z', '"a"b,c', "\r\r\n\n"]
    stdin = "".join(json.dumps(text) + "\n" for text in texts)
    completed = subprocess.run([args.oracle], input=stdin, capture_output=True, text=True, check=True)
    actual = [json.loads(line) for line in completed.stdout.splitlines()]
    mismatches = []
    for text, got in zip(texts, actual):
        want = [list(row) for row in csv.reader(io.StringIO(text, newline=""))]
        if got != want:
            mismatches.append((text, got, want))
    for text, got, want in mismatches[:20]:
        print(f"{text!r}\n  rust   {got}\n  python {want}")
    print(f"{len(texts)} texts, {len(mismatches)} mismatches")
    return 1 if mismatches or len(actual) != len(texts) else 0


if __name__ == "__main__":
    sys.exit(main())
