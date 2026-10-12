"""Compare the generator's Rust decimal module with CPython's decimal on random operands.

    python decimal_differential.py <path to target/debug/examples/decimal_oracle> [--cases N] [--seed S]

Operands are plain decimal literals of 1 to 40 digits whose magnitude lies between
1e-12 and 1e16, the range the generator's amounts, rates and growth factors occupy
with room to spare, plus signed zeros. Results are compared at 60 places, which is exact
for every 28-digit result in that range, sign of zero included.
"""

import argparse
import random
import subprocess
import sys
from decimal import ROUND_HALF_EVEN, Context, Decimal, localcontext


def operand(rng: random.Random) -> str:
    if rng.random() < 0.05:
        return rng.choice(["0", "0.0", "0.00", "-0.0", "-0", "0.000000"])
    count = rng.choice([1, 2, 3, 4, 6, 9, 15, 27, 28, 29, 30, 40])
    digits = str(rng.randint(1, 9)) + "".join(rng.choice("0123456789") for _ in range(count - 1))
    if rng.random() < 0.3:
        digits += "0" * rng.randint(1, 4)
        digits = digits[: max(1, count)]
    lowest, highest = -12, 15
    exponent = rng.randint(lowest - len(digits) + 1, highest - len(digits) + 1)
    value = Decimal(f"{digits}E{exponent}")
    text = format(value, "f")
    if rng.random() < 0.4:
        text = "-" + text
    return text


def halfway(rng: random.Random) -> str:
    """A value exactly halfway between two cents, or a hair either side."""
    cents = rng.randint(-10**9, 10**9)
    tail = rng.choice(["5", "50", "500000000000000000001", "49999999999999999999", "5000000000000000000000000000"])
    sign = "-" if cents < 0 else ""
    return f"{sign}{abs(cents) // 100}.{abs(cents) % 100:02d}{tail}"


def expected(op: str, a: str, b: str) -> str:
    with localcontext(Context(prec=28, rounding=ROUND_HALF_EVEN)):
        x, y = Decimal(a), Decimal(b)
        if op == "cmp":
            return {-1: "Less", 0: "Equal", 1: "Greater"}[(x > y) - (x < y)]
        if op == "fixed2":
            return format(x, ".2f")
        result = {"add": lambda: x + y, "sub": lambda: x - y, "mul": lambda: x * y,
                  "div": lambda: x / y, "round2": lambda: round(x, 2), "neg": lambda: -x}[op]()
        return format(result, ".60f")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("oracle")
    parser.add_argument("--cases", type=int, default=200000)
    parser.add_argument("--seed", type=int, default=1)
    args = parser.parse_args()
    rng = random.Random(args.seed)
    requests = []
    for _ in range(args.cases):
        op = rng.choice(["add", "sub", "mul", "div", "div", "round2", "fixed2", "cmp", "neg"])
        a = halfway(rng) if op in ("round2", "fixed2") and rng.random() < 0.5 else operand(rng)
        b = operand(rng)
        if op == "div" and Decimal(b) == 0:
            b = "7"
        requests.append((op, a, b))
    stdin = "".join(f"{op} {a} {b}\n" for op, a, b in requests)
    completed = subprocess.run([args.oracle], input=stdin, capture_output=True, text=True, check=True)
    actual = completed.stdout.splitlines()
    mismatches = [(request, got, want) for request, got in zip(requests, actual)
                  if got != (want := expected(*request))]
    if len(actual) != len(requests):
        print(f"oracle returned {len(actual)} results for {len(requests)} requests")
        return 1
    for request, got, want in mismatches[:20]:
        print(f"{' '.join(request)}\n  rust   {got}\n  python {want}")
    print(f"{len(requests)} operations, {len(mismatches)} mismatches")
    return 1 if mismatches else 0


if __name__ == "__main__":
    sys.exit(main())
