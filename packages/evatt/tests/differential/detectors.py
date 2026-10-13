"""Compare the frozen Python implementation with the feature-gated Rust probe.

python3.14 tests/differential/detectors.py --python-root /work/tmp/evatt-oracle/packages/evatt \
    --probe /work/tmp/target/debug/evatt-probe --seed 1 --cases 2000
The oracle must be a pristine copy of repository snapshot efa253847e4ecdca5ae9b4abec045c0b16b6835c. It is not a
runtime dependency. All inputs are fabricated, and random seeds are recorded.
"""

import argparse
import json
from pathlib import Path
import random
import re
import subprocess
import sys
import unicodedata

SEPARATORS = [
    " ",
    "  ",
    "\t",
    "\r\n",
    "\n",
    "-",
    "--",
    "/",
    ".",
    ",",
    "\u00a0",
    "\u2003",
    "\u202f",
    "\u2013",
    "\u2212",
]
LABELS = [
    "TFN",
    "tax file no.",
    "tax file number (TFN)",
    "ABN",
    "A.B.N.",
    "Australian business number (ABN)",
    "ACN",
    "A.C.N",
    "Medicare",
    "BSB",
]
DIGITS = [
    "123456782",
    "123456783",
    "12345678",
    "51824753556",
    "51824753557",
    "000000019",
    "123456780",
    "2123456701",
    "2123456711",
    "062000",
]
NAMES = [
    "Jane Roe",
    "John Smith",
    "Zoë Nguyen",
    "José Núñez",
    "Iris Smith",
    "Irıs Smİth",
    "Kelvin ſmith",
    "Straße Person",
    "Income Tax John Smith",
    "Priya Sharma Superannuation",
]
JOINERS = [
    " ",
    "_",
    "*",
    "**",
    "<br>",
    "<BR/>",
    "<br />",
    "\u200b",
    "\u200c",
    "\u200d",
    "\u2060",
    "\n> ",
    "\n >> ",
]
VALUES = [
    "Jane Roe",
    "José Núñez",
    "Iris Smith",
    "Kelvin smith",
    "Straße Person",
    "A <br B",
    "(John Smith)",
    "123456783",
]
DIRECTED = [
    "TFN: 1234567\n82. 3 next",
    "TFN: 12345678\n1. Next item",
    "(02) 9876 5432jane@example.com",
    "jane@example.com0412 345 678",
    "CLIENT_01 _PERSON_01_ XCLIENT_01 CLIENT_01s TFN_01 CLIENT_٠١",
    "Tax_Income Tax John Smith",
    "x_John Smith",
    "x__John Smith",
    "A <br <br B",
    "Jane" + "<br" * 20 + "Roe",
    "A" + "*" * 100 + "<br",
    "192.168.100.107",
    "2024-02-29 12345678",
    "2023-02-29 12345678",
    "Jane\u2028Roe\rJohn\x85Smith",
    "٠١٢٣٤٥٦٧٨",
    "12 Grüner Street",
    "name\u00a0TFN:\u2003123\u202f456\t782",
    "İris IRIS ı ſ K ß",
    "Jose\u0301 Nu\u0301n\u0303ez",
    "",
    "Jane Roe" * 4,
]


def expected(request, p, entities):
    text = request["text"]
    patterns = {}
    for name, spec in json.loads(
        (Path(__file__).parents[2] / "tools/patterns.json").read_text()
    ).items():
        regex = re.compile(spec["pattern"], re.I if spec["ignorecase"] else 0)
        patterns[name] = [
            [
                None if m.start(i) < 0 else [m.start(i), m.end(i), m.group(i)]
                for i in range(regex.groups + 1)
            ]
            for m in regex.finditer(text)
        ]
    result = {
        "patterns": patterns,
        "structured": p.structured_spans(text),
        "names": p.person_name_spans(text),
        "ambiguous": [
            [a.start, a.digit_start, a.end, a.value] for a in p._ambiguous_identifiers(text)
        ],
        "nfc": unicodedata.normalize("NFC", text),
    }
    attempts = []
    pos = 0
    while (m := p.NAME.search(text, pos)) is not None:
        rejected = p.is_statutory(m.group())
        if not rejected:
            pos = m.end()
        else:
            tokens = [t.span() for t in p._NAME_TOKEN.finditer(m.group())]
            kept = len(tokens) == 3 and any(
                not p.is_statutory(m.group()[left[0] : right[1]])
                for left, right in zip(tokens, tokens[1:])
            )
            pos = m.end() if kept else m.start() + tokens[1][0]
        attempts.append([m.start(), m.end(), rejected, pos])
    result["name_attempts"] = attempts
    result["email_scanner"] = [[m.start(), m.end(), m.group()] for m in p.EMAIL.finditer(text)]
    result["values"] = [
        [
            [
                [m.start(), m.end(), m.group()],
                m.group("open"),
                m.group("close"),
                [
                    [m.start(f"j{i}"), m.end(f"j{i}"), m.group(f"j{i}")]
                    for i in range(len(p.value_parts(v)) - 1)
                ],
            ]
            for m in p.value_pattern(v).finditer(text)
        ]
        for v in request["values"]
    ]
    result["folds"] = [entities._fold(v) for v in request["values"]]
    from evatt import redact as red, verify

    entries = tuple(entities.Entity(**e) for e in request.get("entries", []))
    result["redact"] = red.redact(text, entries, strict=False)
    normal = unicodedata.normalize("NFC", text.replace("\r\n", "\n"))
    sanitised = result["redact"][0]
    result["unknowns"] = [
        [u.kind, u.value, u.line, u.context]
        for u in red._input_placeholders(normal, sanitised) + red.residual(sanitised)
    ]
    result["residual"] = [[u.kind, u.value, u.line, u.context] for u in red.residual(text)]
    result["numbers"] = list(red._number_candidates(text))
    result["findings"] = [[f.kind, f.value] for f in verify.findings(text, entries)]
    return json.loads(json.dumps(result))


def generated(rng):
    atoms = []
    for _ in range(rng.randint(1, 8)):
        choice = rng.randrange(6)
        if choice < 2:
            label = rng.choice(LABELS)
            number = rng.choice(DIGITS)
            if rng.random() < 0.15:
                number = "".join(chr(0x660 + int(c)) for c in number)
            number = rng.choice(SEPARATORS).join(number) if rng.random() < 0.5 else number
            fence = rng.choice(["", "*", "__", "***`", "`__", "````"])
            atoms.append(
                fence
                + label
                + fence
                + rng.choice([": ", " no. ", " card number ", " was ", "", " is ", " | "])
                + fence
                + number
                + fence
            )
            if rng.random() < 0.3:
                atoms[-1] += rng.choice([".00", " 0", "/1", "\n1. next", "\n82) next", " 2026"])
        elif choice == 2:
            name = rng.choice(NAMES)
            if rng.random() < 0.2:
                name = unicodedata.normalize("NFD", name)
            atoms.append(
                rng.choice(["", "*", "__", "paid**"])
                + rng.choice(JOINERS).join(name.split())
                + rng.choice(["", "*", "__"])
            )
        elif choice == 3:
            atoms.append(rng.choice(DIRECTED))
        elif choice == 4:
            atoms.append(
                rng.choice(
                    [
                        "12345678",
                        "1,234,567.89",
                        "123456782.00",
                        "2026-13-01",
                        "14/03/1982",
                        "2123 45670 1/1",
                    ]
                )
            )
        else:
            atoms.append(
                rng.choice(
                    [
                        "CLIENT_01",
                        "_PERSON_01_",
                        "XCLIENT_01",
                        "CLIENT_01s",
                        "TFN_01",
                        "CLIENT_٠١",
                        "Tax",
                        "Board",
                        "Zzz",
                    ]
                )
            )
    return rng.choice([" ", "\r\n", "\u2028", "|"]).join(atoms)


def compare(request, answer, p, entities):
    want = expected(request, p, entities)
    return answer == want, want


def shrink(request, probe, p, entities):
    # Deterministic deletion minimisation; retain the first local minimum as a
    # fabricated regression input, rather than hiding a discrepancy in a seed.
    text = request["text"]
    size = max(1, len(text) // 2)
    while size:
        changed = False
        for start in range(0, len(text), size):
            candidate = {**request, "text": text[:start] + text[start + size :]}
            got = json.loads(
                subprocess.run(
                    [probe],
                    input=json.dumps(candidate) + "\n",
                    text=True,
                    capture_output=True,
                    check=True,
                ).stdout
            )
            if not compare(candidate, got, p, entities)[0]:
                text = candidate["text"]
                changed = True
                break
        if not changed:
            size //= 2
    return {**request, "text": text}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--python-root", required=True)
    parser.add_argument("--probe", required=True)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--cases", type=int, default=2000)
    parser.add_argument("--shrink", action="store_true")
    args = parser.parse_args()
    sys.path.insert(0, args.python_root)
    from evatt import patterns as p, entities

    rng = random.Random(args.seed)
    mapping = [
        {"value": v, "placeholder": f"ENTITY_{i:02d}", "kind": "entity", "added": "2026-10-13"}
        for i, v in enumerate(VALUES, 1)
    ]

    def request(text):
        return {
            "text": text,
            "values": VALUES,
            "entries": rng.sample(mapping, rng.randint(0, len(mapping))),
        }

    batch = [request(text) for text in DIRECTED]
    batch += [request(generated(rng)) for _ in range(args.cases)]
    alphabet = "aA.0-+_@²é中\u2003\u200c"
    batch += [
        request("".join(rng.choice(alphabet) for _ in range(rng.randint(1, 80))))
        for _ in range(2000)
    ]
    completed = subprocess.run(
        [args.probe],
        input="".join(json.dumps(r) + "\n" for r in batch),
        text=True,
        capture_output=True,
        check=True,
    )
    answers = [json.loads(line) for line in completed.stdout.splitlines()]
    errors = []
    for i, (request, answer) in enumerate(zip(batch, answers)):
        same, want = compare(request, answer, p, entities)
        if not same:
            errors.append((i, request, answer, want))
    for i, request, got, want in errors[:5]:
        print(f"case {i}: {json.dumps(request, ensure_ascii=True)}")
        for key in sorted(set(got) | set(want)):
            if got.get(key) != want.get(key):
                print(
                    f"  {key}: rust={json.dumps(got.get(key))} python={json.dumps(want.get(key))}"
                )
    if errors and args.shrink:
        minimal = shrink(errors[0][1], args.probe, p, entities)
        print("minimal:", json.dumps(minimal))
    print(f"seed {args.seed}: {len(batch)} detector cases; {len(errors)} differences")
    return int(bool(errors) or len(answers) != len(batch))


if __name__ == "__main__":
    sys.exit(main())
