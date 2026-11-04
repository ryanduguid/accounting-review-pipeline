"""Compare the Rust decimal module with CPython's decimal on random operands.

    python decimal_differential.py <path to target/debug/examples/decimal_oracle> [--cases N] [--seed S]

Operands stay inside the gateway's domain: any parsed string for `parse`, and
for arithmetic finite values whose adjusted exponent is within [-18, 18], plus
zeros with any exponent, which the gateway also admits.
"""

import argparse
import json
import random
import subprocess
import sys
from decimal import ROUND_HALF_EVEN, ROUND_HALF_UP, Context, Decimal, Inexact, InvalidOperation, localcontext

PARSE_EXTRAS = [
    " 1.5 ", "\x1c2\x1f", "1_000", "_1", "1__0", "1_.5", "٣٤", "１２", "\U0001d7cf", "1\xa0",
    "  1", "nan", "-sNaN12", "inf", "-Infinity", "infinit", "1e", ".", "-", "+.5", "1.", "1 0", "1,5",
    "1e999999999999999999", "1e1000000000000000000", "0e999999999999999999", "0e1000000000000000000",
    "1e-1999999999999999997", "1e-1999999999999999998", "1E+2", "-0", "-0.00", "١٫5", "1\x00",
]


def operand(rng, *, zeros=True):
    if zeros and rng.random() < 0.08:
        return f"{rng.choice(['', '-'])}0E{rng.choice([-999999999999, -1000030, -40, -2, 0, 3, 999999, 999999999999])}"
    digits = "".join(rng.choice("0123456789") for _ in range(rng.choice([1, 2, 3, 5, 8, 15, 27, 28, 29, 30, 40])))
    digits = digits.lstrip("0") or str(rng.randint(1, 9))
    if rng.random() < 0.3:
        digits += "0" * rng.randint(1, 6)
    exp = rng.randint(-20 - len(digits), 18 - len(digits) + 1)
    sign = rng.choice(["", "-", "+"]) if rng.random() < 0.6 else ""
    style = rng.random()
    if style < 0.5:
        value = Decimal(f"{digits}E{exp}")
        return sign + format(value, "f")
    return f"{sign}{digits}E{exp:+d}"


def in_domain(text):
    try:
        value = Decimal(text)
    except InvalidOperation:
        return False
    return value.is_finite() and (value == 0 or -18 <= value.adjusted() <= 18)


def expected(op, a, b=None):
    with localcontext(Context(prec=28, rounding=ROUND_HALF_EVEN)) as ctx:
        if op == "parse":
            try:
                x = Decimal(a)
            except InvalidOperation:
                return ["invalid"]
            return ["special"] if not x.is_finite() else [str(x), x.adjusted()]
        x = Decimal(a)
        y = Decimal(b if b is not None else "0")
        ctx.clear_flags()
        if op in ("add", "sub"):
            r = x + y if op == "add" else x - y
            return [str(r), bool(ctx.flags[Inexact])]
        if op == "mul":
            return [str(x * y)]
        if op == "div":
            return [str(x / y)]
        if op == "cmp":
            return [(x > y) - (x < y)]
        if op == "abs":
            return [str(abs(x))]
        if op == "fixed":
            # A zero with a huge exponent is where CPython raises MemoryError.
            if x == 0 and x.as_tuple().exponent < -(1 << 30):
                return ["MemoryError"]
            return [format(x, "f")]
        if op in ("quant4up", "quant2"):
            try:
                if op == "quant4up":
                    return [str(x.quantize(Decimal("0.0001"), rounding=ROUND_HALF_UP))]
                return [str(x.quantize(Decimal("0.01")))]
            except InvalidOperation:
                return ["InvalidOperation"]
    raise ValueError(op)


def requests(rng, count):
    for extra in PARSE_EXTRAS:
        yield ["parse", extra]
    ops = ["add", "sub", "mul", "div", "cmp", "abs", "fixed", "quant4up", "quant2", "parse"]
    produced = 0
    while produced < count:
        op = rng.choice(ops)
        a = operand(rng)
        if op == "parse":
            yield ["parse", a]
        elif op in ("abs", "fixed", "quant4up", "quant2"):
            yield [op, a]
        else:
            b = operand(rng, zeros=op != "div")
            if op == "div" and Decimal(b) == 0:
                continue
            if op == "mul" and not (in_domain(a) and in_domain(b)):
                continue
            yield [op, a, b]
        produced += 1


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("oracle")
    parser.add_argument("--cases", type=int, default=200_000)
    parser.add_argument("--seed", type=int, default=20261011)
    args = parser.parse_args()
    rng = random.Random(args.seed)
    batch = [r for r in requests(rng, args.cases) if r[0] == "parse" or all(in_domain(x) for x in r[1:])]
    payload = "".join(json.dumps(r) + "\n" for r in batch)
    result = subprocess.run([args.oracle], input=payload, capture_output=True, text=True, encoding="utf-8")
    if result.returncode:
        print(result.stderr[-2000:])
        return 1
    failures = 0
    for request, line in zip(batch, result.stdout.splitlines(), strict=True):
        want = expected(*request)
        got = json.loads(line)
        if got != want:
            failures += 1
            if failures <= 20:
                print("MISMATCH", json.dumps(request), "python", want, "rust", got)
    print(f"python {sys.version.split()[0]}: {len(batch)} cases, {failures} mismatches")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
