"""Compare the generator's Rust decimal module with CPython's decimal on random operands.

    python decimal_differential.py <path to target/debug/examples/decimal_oracle> [--cases N] [--seed S]

Operands are decimal literals of 1 to 40 digits whose magnitude lies between 1e-12 and
1e16, the range the generator's amounts, rates and growth factors occupy with room to
spare, plus signed zeros, and other constructor forms of the same values (exponents,
a plus sign, underscores, surrounding whitespace). A directed corpus adds halfway
cases, carries, cancellation, small quotients and precision edges. Results are compared
at 60 places, which is exact for every 28-digit result in that range, sign of zero
included; a refusal must be a refusal on both sides. Infinities, NaNs and non-ASCII
digits are outside the module and not generated.
"""

import argparse
import json
import random
import subprocess
import sys
from decimal import (
    ROUND_HALF_EVEN,
    Context,
    Decimal,
    DivisionByZero,
    InvalidOperation,
    localcontext,
)


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


# Constructor forms, halfway cases, carries across the 28th digit, cancellation, signed
# zeros, small quotients and results just inside and outside the context precision.
DIRECTED = [
    ("parse", "3.5e5", "0"), ("parse", "+350000.00", "0"), ("parse", "350_000.00", "0"), ("parse", " 7 ", "0"),
    ("parse", "\x1c2\x1f", "0"), ("parse", "1E-3", "0"), ("parse", ".5", "0"), ("parse", "5.", "0"),
    ("parse", "-0", "0"), ("parse", "-0.00", "0"), ("parse", "1e+5", "0"), ("parse", "_1_", "0"),
    ("parse", "1e", "0"), ("parse", "abc", "0"), ("parse", "", "0"), ("parse", "-", "0"), ("parse", "1.2.3", "0"),
    ("parse", "1 0", "0"), ("parse", "0x1", "0"), ("parse", "e5", "0"), ("parse", ".", "0"), ("parse", "1e1.5", "0"),
    ("round2", "0.125", "0"), ("round2", "0.135", "0"), ("round2", "-0.125", "0"), ("round2", "-0.001", "0"),
    ("round2", "9999999999999999999999999.995", "0"), ("round2", "99999999999999999999999999.995", "0"),
    ("round2", "1234567890123456789012345678", "0"), ("round2", "12345678901234567890123456.7", "0"),
    ("round2", "1E+30", "0"), ("fixed2", "1E+30", "0"), ("fixed2", "-0.004", "0"), ("fixed2", "0.005", "0"),
    ("add", "9999999999999999999999999999", "1"), ("add", "-0", "-0"), ("add", "-0", "0"), ("add", "-5", "5"),
    ("sub", "1.000000000000000000000000001", "1"), ("sub", "0", "0"), ("sub", "-0", "0"),
    ("mul", "-0", "5"), ("mul", "-3", "0"), ("mul", "1.000000000000000000000000001", "1.000000000000000000000000001"),
    ("div", "0", "-3"), ("div", "1", "3E+20"), ("div", "1E-12", "999999999999"), ("div", "2", "3"),
    ("div", "-1", "7"), ("div", "1", "0"), ("neg", "0", "0"), ("neg", "-0", "0"),
    ("neg", "1.0000000000000000000000000005", "0"), ("cmp", "-0", "0"), ("cmp", "1.10", "1.1"),
]


def expected(op: str, a: str, b: str, precision: int = 28) -> str:
    try:
        with localcontext(Context(prec=precision, rounding=ROUND_HALF_EVEN)):
            x = Decimal(a)
            if op == "parse":
                return format(x, ".60f")
            if op == "fixed2":
                return format(x, ".2f")
            if op == "neg":
                return format(-x, ".60f")
            if op == "round2":
                return format(round(x, 2), ".60f")
            y = Decimal(b)
            if op == "cmp":
                return {-1: "Less", 0: "Equal", 1: "Greater"}[(x > y) - (x < y)]
            result = {"add": lambda: x + y, "sub": lambda: x - y, "mul": lambda: x * y,
                      "div": lambda: x / y}[op]()
            return format(result, ".60f")
    except (InvalidOperation, DivisionByZero, ValueError):
        return "error"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("oracle")
    parser.add_argument("--cases", type=int, default=200000)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--precision", type=int, choices=(28, 40), default=28)
    args = parser.parse_args()
    rng = random.Random(args.seed)
    requests = list(DIRECTED)
    for _ in range(args.cases):
        op = rng.choice(["add", "sub", "mul", "div", "div", "round2", "fixed2", "cmp", "neg", "parse"])
        a = halfway(rng) if op in ("round2", "fixed2") and rng.random() < 0.5 else operand(rng)
        if op == "parse" and rng.random() < 0.5:
            a = constructor_form(rng, a)
        b = operand(rng)
        if op == "div" and Decimal(b) == 0:
            b = "7"
        requests.append((op, a, b))
    stdin = "".join(json.dumps(request) + "\n" for request in requests)
    command = [args.oracle] + (["--precision", "40"] if args.precision == 40 else [])
    completed = subprocess.run(command, input=stdin, capture_output=True, text=True, check=True)
    actual = completed.stdout.splitlines()
    if len(actual) != len(requests):
        print(f"oracle returned {len(actual)} results for {len(requests)} requests")
        return 1
    mismatches = [(request, got, want) for request, got in zip(requests, actual)
                  if got != (want := expected(*request, precision=args.precision))]
    for request, got, want in mismatches[:20]:
        print(f"{json.dumps(request)}\n  rust   {got}\n  python {want}")
    errors = sum(got == "error" for got in actual)
    print(f"{len(requests)} operations ({len(DIRECTED)} directed, {errors} refused by both), {len(mismatches)} mismatches")
    return 1 if mismatches else 0


def constructor_form(rng: random.Random, plain: str) -> str:
    """The same value written another way CPython's constructor accepts."""
    value = Decimal(plain)
    form = rng.choice(["sci", "plus", "underscore", "space"])
    if form == "sci":
        return f"{value:E}" if rng.random() < 0.5 else f"{value:e}"
    if form == "plus":
        return plain if plain.startswith("-") else "+" + plain
    if form == "underscore":
        return "_".join(plain[i:i + 3] for i in range(0, len(plain), 3))
    return rng.choice([" ", "\t", "\n", "\x1c"]) + plain + rng.choice([" ", "\x1f", "\r\n"])


if __name__ == "__main__":
    sys.exit(main())
