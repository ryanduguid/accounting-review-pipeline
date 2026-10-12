"""Differential test: generate_fixtures.py against the Rust generator.

    python generator_differential.py --script <tools/generate_fixtures.py from the Python base> \
        --binary <target/debug/generate-fixtures> --oracle <target/debug/examples/payday_oracle> \
        [--cases N] [--seed S]

The end-to-end check runs the Python script from a scratch application directory and
the Rust binary in place, then compares exit codes, stdout (with each samples
directory normalised) and the six files byte for byte. The function check sends
generated requests to the payday oracle and compares notional earnings, business-day
offsets, calendar refusals and GIC rate selection with the Python functions,
including a replaced December-quarter rate as the Python suite patches it.
"""

import argparse
import datetime
import importlib.util
import random
import shutil
import subprocess
import sys
import tempfile
from decimal import Decimal
from pathlib import Path

APP = Path(__file__).resolve().parents[2]
FILES = ("sample-entities.csv", "sample-chart-of-accounts.csv", "sample-general-ledger.csv", "sample-budgets.csv",
         "sample-payroll-super.csv", "sample-ato-benchmarks.csv")


def end_to_end(script: Path, binary: Path) -> list[str]:
    problems = []
    with tempfile.TemporaryDirectory(prefix="pbi-generator-") as scratch:
        tools = Path(scratch) / "app" / "tools"
        tools.mkdir(parents=True)
        shutil.copy2(script, tools / "generate_fixtures.py")
        python = subprocess.run([sys.executable, "-B", str(tools / "generate_fixtures.py"), "ignored", "--flag"],
                                capture_output=True)
        rust = subprocess.run([str(binary), "ignored", "--flag"], capture_output=True)
        python_samples, rust_samples = (Path(scratch) / "app" / "samples").resolve(), APP / "samples"
        if python.returncode != rust.returncode:
            problems.append(f"exit {python.returncode} != {rust.returncode}")
        python_out = python.stdout.replace(str(python_samples).encode(), b"<samples>")
        rust_out = rust.stdout.replace(str(rust_samples).encode(), b"<samples>")
        if python_out != rust_out or python.stderr != rust.stderr:
            problems.append(f"output {python_out!r} {python.stderr!r} != {rust_out!r} {rust.stderr!r}")
        for name in FILES:
            if (python_samples / name).read_bytes() != (rust_samples / name).read_bytes():
                problems.append(f"{name} differs")
    return problems


def load(script: Path):
    spec = importlib.util.spec_from_file_location("generate_fixtures", script)
    module = importlib.util.module_from_spec(spec)  # type: ignore[arg-type]
    spec.loader.exec_module(module)  # type: ignore[union-attr]
    return module


def day(rng: random.Random, first: datetime.date, last: datetime.date) -> datetime.date:
    return first + datetime.timedelta(days=rng.randint(0, (last - first).days))


def amount(rng: random.Random) -> str:
    style = rng.random()
    if style < 0.6:
        return f"{rng.randint(1, 10_000_000) / 100:.2f}"
    if style < 0.8:
        return str(Decimal(rng.randint(1, 10**12)).scaleb(-rng.randint(0, 8)))
    return rng.choice(["0.01", "1000.0", "0.0", "999999999.99", "123.455", "8.333333333333333333333333333333"])


def request(rng: random.Random) -> list[str]:
    kind = rng.random()
    if kind < 0.6:
        due = day(rng, datetime.date(2025, 12, 1), datetime.date(2028, 3, 1))
        received = due + datetime.timedelta(days=rng.choice([-3, 0, 1, 2]) if rng.random() < 0.2 else rng.randint(1, 500))
        parts = ["notional", amount(rng), due.isoformat(), received.isoformat()]
        if rng.random() < 0.25:
            parts.append(rng.choice(["0.2000", "0.1151", "0.0", "0.123456789", "0.11515"]))
        return parts
    if kind < 0.9:
        start = day(rng, datetime.date(2025, 12, 15), datetime.date(2027, 8, 10))
        return ["business", start.isoformat(), str(rng.choice([0, 1, 4, 7, 10, rng.randint(0, 40)]))]
    return ["rate", day(rng, datetime.date(2025, 1, 1), datetime.date(2029, 12, 31)).isoformat()]


def expected(module, parts: list[str]) -> str:
    when = [datetime.date.fromisoformat(p) for p in parts[1:4] if p.count("-") == 2]
    if parts[0] == "notional":
        original = dict(module.GIC_PUBLISHED_RATES)
        try:
            if len(parts) == 5:
                module.GIC_PUBLISHED_RATES[(2026, 4)] = Decimal(parts[4])
            return f"{module.notional_earnings(Decimal(parts[1]), when[0], when[1]):.2f}"
        finally:
            module.GIC_PUBLISHED_RATES.clear()
            module.GIC_PUBLISHED_RATES.update(original)
    if parts[0] == "business":
        try:
            return module.add_business_days(when[0], int(parts[2])).isoformat()
        except ValueError as error:
            return str(error)
    return f"{module.gic_annual_rate(when[0]):.4f} {str(module.gic_rate_is_published(when[0])).lower()}"


def functions(script: Path, oracle: Path, cases: int, seed: int) -> list[str]:
    module = load(script)
    rng = random.Random(seed)
    requests = [request(rng) for _ in range(cases)]
    stdin = "".join(" ".join(parts) + "\n" for parts in requests)
    actual = subprocess.run([str(oracle)], input=stdin, capture_output=True, text=True, check=True).stdout.splitlines()
    if len(actual) != len(requests):
        return [f"oracle answered {len(actual)} of {len(requests)} requests"]
    return [f"{' '.join(parts)}: rust {got} python {want}"
            for parts, got in zip(requests, actual) if got != (want := expected(module, parts))]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--script", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--oracle", type=Path, required=True)
    parser.add_argument("--cases", type=int, default=20000)
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()
    problems = end_to_end(args.script.resolve(), args.binary.resolve())
    print(f"end to end: {len(problems)} differences")
    mismatches = functions(args.script.resolve(), args.oracle.resolve(), args.cases, args.seed)
    print(f"functions: {args.cases} requests, {len(mismatches)} mismatches")
    for line in (problems + mismatches)[:20]:
        print("  " + line)
    return 1 if problems or mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
