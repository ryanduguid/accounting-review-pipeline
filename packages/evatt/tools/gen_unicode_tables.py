"""Freeze evatt's CPython 3.14 / Unicode 16 predicates and case mappings.

Adapted from elizabeth-anne-alexander/tools/gen_unicode_tables.py. Output is
standalone: the binary never loads Python, a sibling, or a host Unicode table.
Run: python3.14 tools/gen_unicode_tables.py > src/tables.rs
Run generator verification: python3.14 tools/gen_unicode_tables.py --test
"""

import argparse
import hashlib
import io
import re
import sys
import unicodedata
import _sre
from re._casefix import _EXTRA_CASES

LIMIT = 0x110000
LINE_BREAKS = "\n\r\v\f\x1c\x1d\x1e\x85\u2028\u2029"


def ranges(predicate):
    result, start = [], None
    for cp in range(LIMIT):
        if predicate(chr(cp)):
            if start is None:
                start = cp
        elif start is not None:
            result.append((start, cp - 1))
            start = None
    if start is not None:
        result.append((start, LIMIT - 1))
    return result


def ignorecase_groups():
    """The compiler's unicode_tolower + _casefix, separately from casefold."""
    buckets = {}
    for cp in range(LIMIT):
        lower = _sre.unicode_tolower(cp)
        buckets.setdefault(lower, []).append(cp)
    groups = {}
    for lower, members in buckets.items():
        merged = sorted(
            set(members).union(*(buckets[other] for other in _EXTRA_CASES.get(lower, ())))
        )
        if len(merged) > 1:
            for cp in members:
                groups[cp] = tuple(merged)
    return groups


def regex_class(points):
    # Explicit scalars only. Lone surrogates never match valid UTF-8 text.
    points = sorted(cp for cp in points if not 0xD800 <= cp <= 0xDFFF)
    pairs, start, end = [], None, None
    for cp in points:
        if start is None:
            start = end = cp
        elif cp == end + 1:
            end = cp
        else:
            pairs.append((start, end))
            start = end = cp
    if start is not None:
        pairs.append((start, end))
    return (
        "["
        + "".join(f"\\x{{{a:X}}}" + (f"-\\x{{{b:X}}}" if a != b else "") for a, b in pairs)
        + "]"
    )


def verify(groups):
    assert unicodedata.unidata_version == "16.0.0"
    assert sys.version_info[:2] == (3, 14)
    # Verify every non-trivial equivalence with the actual CPython engine in
    # both directions, including the extra-case closure. Verify adjacent
    # non-members too, and exhaustively check the ASCII range optimisation.
    for cp, members in groups.items():
        compiled = re.compile(re.escape(chr(cp)), re.I)
        for member in members:
            assert compiled.fullmatch(chr(member)), (cp, member)
        for neighbour in (cp - 1, cp + 1):
            if 0 <= neighbour < LIMIT and neighbour not in members:
                assert not compiled.fullmatch(chr(neighbour)), (cp, neighbour)
    for pattern, letters in (("[a-z]", range(97, 123)), ("[A-Z]", range(65, 91))):
        expected = set().union(*(groups.get(cp, (cp,)) for cp in letters))
        compiled = re.compile(pattern, re.I)
        actual = {cp for cp in range(LIMIT) if compiled.fullmatch(chr(cp))}
        assert actual == expected
    for zero in decimal_zeros():
        assert all(unicodedata.decimal(chr(zero + d)) == d for d in range(10))
    print(
        f"Unicode 16: {len(groups)} IGNORECASE literals verified against compiled re",
        file=sys.stderr,
    )


def decimal_zeros():
    return [cp for cp in range(LIMIT) if unicodedata.decimal(chr(cp), None) == 0]


def emit_ranges(out, name, predicate):
    out.write(f"pub const {name}: &[(u32, u32)] = &[\n")
    for low, high in ranges(predicate):
        out.write(f"    (0x{low:04X}, 0x{high:04X}),\n")
    out.write("];\n\n")


def emit_mapping(out, name, transform):
    out.write(f"pub const {name}: &[(u32, &str)] = &[\n")
    for cp in range(LIMIT):
        mapped = transform(chr(cp))
        if mapped != chr(cp):
            escaped = "".join(f"\\u{{{ord(ch):X}}}" for ch in mapped)
            out.write(f'    (0x{cp:04X}, "{escaped}"),\n')
    out.write("];\n\n")


def generate(groups):
    out = io.StringIO()
    for name, predicate in (
        ("PY_SPACE", str.isspace),
        ("PY_DECIMAL", str.isdecimal),
        ("PY_DIGIT", str.isdigit),
        ("PY_ALNUM", str.isalnum),
        ("PY_UPPER", str.isupper),
        ("PY_CASED", lambda ch: ch.islower() or ch.isupper() or ch.istitle()),
        # Derive Case_Ignorable from CPython's actual contextual Sigma rule,
        # including cased combining marks, rather than category approximations.
        (
            "PY_CASE_IGNORABLE",
            lambda ch: ("AΣ" + ch + "A").lower()[1] == "σ" and ("AΣ" + ch).lower()[1] == "ς",
        ),
        ("PY_NOT_PRINTABLE", lambda ch: not ch.isprintable()),
        ("PY_LINE_BREAK", lambda ch: ch in LINE_BREAKS),
    ):
        emit_ranges(out, name, predicate)
    # strip and split(None) use isspace; splitlines uses LINE_BREAK, treating
    # CRLF as one break. Separate names document those independent contracts.
    out.write("pub const PY_STRIP: &[(u32, u32)] = PY_SPACE;\n")
    out.write("pub const PY_SPLIT: &[(u32, u32)] = PY_SPACE;\n\n")
    out.write("pub const DECIMAL_ZEROS: &[u32] = &[\n")
    for zero in decimal_zeros():
        out.write(f"    0x{zero:04X},\n")
    out.write("];\n\n")
    for name, transform in (
        ("PY_LOWER", str.lower),
        ("PY_TO_UPPER", str.upper),
        ("PY_CASEFOLD", str.casefold),
    ):
        emit_mapping(out, name, transform)
    out.write("pub const RE_IGNORECASE: &[(u32, &[u32])] = &[\n")
    for cp, members in sorted(groups.items()):
        out.write(f"    (0x{cp:04X}, &[" + ", ".join(f"0x{m:04X}" for m in members) + "]),\n")
    out.write("];\n")
    body = out.getvalue()
    header = (
        f"// Generated by tools/gen_unicode_tables.py from CPython {sys.version.split()[0]}.\n"
        f"// unicodedata.unidata_version = {unicodedata.unidata_version}; do not edit.\n"
        f"// Table body SHA-256: {hashlib.sha256(body.encode()).hexdigest()}\n\n"
    )
    return header + body


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--test", action="store_true")
    args = parser.parse_args()
    if sys.version_info[:2] != (3, 14) or unicodedata.unidata_version != "16.0.0":
        parser.error("requires CPython 3.14 with Unicode 16.0.0")
    groups = ignorecase_groups()
    if args.test:
        verify(groups)
    else:
        sys.stdout.write(generate(groups))


if __name__ == "__main__":
    main()
