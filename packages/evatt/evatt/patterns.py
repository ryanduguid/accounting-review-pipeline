"""Detection patterns and check digits for Australian identifiers.

The name, email, phone, TFN and statutory-vocabulary patterns are copied from
``au-tax-legislation-corpus/fadden/pii_patterns.py``. They are copied rather
than imported because repository policy forbids a component importing a
sibling, and a shared distribution for roughly 200 lines of regular
expressions would cost more than the drift it prevents. When changing one,
diff against the origin file.

Four traps in the copied material must not be undone:

* Name tokens accept U+2019, the curly apostrophe Register text actually uses.
* ``is_statutory`` checks all-caps candidates case-insensitively, because an
  all-caps token never matches the capitalised word list.
* The TFN label separator is ``\\s*(?:[.:-]\\s*)?``. Written as the equivalent
  ``\\s*[.:-]?\\s*`` it backtracks quadratically on long space runs.
* Every phone alternative pins its digit count between ``(?<!\\d)`` and
  ``(?!\\d)`` so statutory references, years and grouped amounts stay out.
"""
from __future__ import annotations

import re
from typing import Callable

_TOKEN = r"[A-Z][A-Za-z'\u2019-]{1,20}"
NAME = re.compile(r"\b%s,?\s+%s(?:\s+%s)?\b" % (_TOKEN, _TOKEN, _TOKEN))

EMAIL = re.compile(r"\b[\w.+-]+@[\w-]+(?:\.[\w-]+)+\b")
PHONE = re.compile(
    r"(?<!\d)(?:"
    r"\(0\d\)[\s-]?\d{4}[\s-]?\d{4}"
    r"|0[2-8][\s-]?\d{4}[\s-]?\d{4}"
    r"|04\d{2}[\s-]?\d{3}[\s-]?\d{3}"
    r"|\+61[\s-]?(?:\(0\)[\s-]?)?\d(?:[\s-]?\d){8}"
    r"|\+61[\s-]?\(0?\d\)[\s-]?\d(?:[\s-]?\d){7}"
    r"|1[38]00[\s-]?\d{3}[\s-]?\d{3}"
    r"|13[\s-]?\d{2}[\s-]?\d{2}"
    r")(?!\d)"
)
TFN_LABELLED = re.compile(
    r"\b(?:tax file number|TFN)\b\s*(?:[.:\u2013-]\s*)?(\d(?:[\s-]?\d){7,8})(?![\s-]?\d)",
    re.I,
)
TFN_BARE = re.compile(r"(?<![\d$])(?<![\d$][\s-])(\d{3}([\s-]?)\d{3}\2\d{3})(?![\s-]?\d)")
ABN = re.compile(r"(?<![\d$])(\d{2}[\s-]?\d{3}[\s-]?\d{3}[\s-]?\d{3})(?![\s-]?\d)")
ACN = re.compile(r"(?<![\d$])(\d{3}[\s-]?\d{3}[\s-]?\d{3})(?![\s-]?\d)")
MEDICARE = re.compile(r"(?<![\d$])(\d{4}[\s-]?\d{5}[\s-]?\d)(?![\s-]?\d)")
# No check digit exists for a BSB, so the hyphen is required. Accepting bare
# six-digit runs would swallow ordinary numbers with nothing to reject them on.
BSB = re.compile(r"(?<![\d$-])(\d{3}-\d{3})(?![\d-])")

ADDRESS = re.compile(
    r"\b\d{1,4}[A-Za-z]?\s+[A-Z][A-Za-z'\u2019-]+(?:\s+[A-Z][A-Za-z'\u2019-]+)?\s+"
    r"(?:Street|St|Road|Rd|Avenue|Ave|Drive|Dr|Court|Ct|Place|Pl|Lane|Ln|"
    r"Parade|Pde|Crescent|Cres|Highway|Hwy|Terrace|Tce)\b"
)
DOB = re.compile(
    r"\b(?:0?[1-9]|[12]\d|3[01])\s+"
    r"(?:January|February|March|April|May|June|July|August|September|October|"
    r"November|December)\s+(?:19|20)\d{2}\b"
)
PLACEHOLDER = re.compile(
    r"(?:CLIENT|PERSON|STAFF|ENTITY|TFN|ABN|ACN|BSB|MEDICARE|EMAIL|PHONE)_\d{2,}"
)

_STATUTORY_WORDS = (
    r"\b(Act|Regulation|Schedule|Division|Subdivision|Part|Chapter|Section|"
    r"Commissioner|Minister|Treasurer|Commonwealth|Australian|Australia|Board|"
    r"Tax|Taxation|Income|Superannuation|Court|Tribunal|Determination|Notice|"
    r"Instrument|Amendment|January|February|March|April|May|June|July|August|"
    r"September|October|November|December)\b"
)
STATUTORY = re.compile(_STATUTORY_WORDS)
_STATUTORY_CI = re.compile(_STATUTORY_WORDS, re.I)

_TFN_WEIGHTS = (1, 4, 3, 7, 5, 8, 6, 9, 10)
_ABN_WEIGHTS = (10, 1, 3, 5, 7, 9, 11, 13, 15, 17, 19)
_ACN_WEIGHTS = (8, 7, 6, 5, 4, 3, 2, 1)
_MEDICARE_WEIGHTS = (1, 3, 7, 9, 1, 3, 7, 9)


def valid_tfn(digits: str) -> bool:
    """True when a nine-digit run satisfies the ATO weighted mod-11 sum."""
    if len(digits) != 9 or not digits.isdigit():
        return False
    return sum(w * int(d) for w, d in zip(_TFN_WEIGHTS, digits)) % 11 == 0


def valid_abn(digits: str) -> bool:
    """True when an eleven-digit run satisfies the mod-89 sum after the leading subtraction."""
    if len(digits) != 11 or not digits.isdigit():
        return False
    values = [int(d) for d in digits]
    values[0] -= 1
    return sum(w * v for w, v in zip(_ABN_WEIGHTS, values)) % 89 == 0


def valid_acn(digits: str) -> bool:
    """True when a nine-digit run satisfies the ASIC complement check digit."""
    if len(digits) != 9 or not digits.isdigit():
        return False
    total = sum(w * int(d) for w, d in zip(_ACN_WEIGHTS, digits))
    return (10 - total % 10) % 10 == int(digits[8])


def valid_medicare(digits: str) -> bool:
    """True when a ten-digit run has a leading 2 to 6 and a matching ninth check digit."""
    if len(digits) != 10 or not digits.isdigit() or digits[0] not in "23456":
        return False
    return sum(w * int(d) for w, d in zip(_MEDICARE_WEIGHTS, digits)) % 10 == int(digits[8])


def _always(_digits: str) -> bool:
    return True


def _length_six(digits: str) -> bool:
    return len(digits) == 6


# Order is the tie-break when two kinds match the identical span. A nine-digit
# run can satisfy both the TFN and the ACN check, so the more sensitive kind is
# listed first and wins.
_STRUCTURED: tuple[tuple[str, re.Pattern[str], Callable[[str], bool]], ...] = (
    ("abn", ABN, valid_abn),
    ("medicare", MEDICARE, valid_medicare),
    ("tfn", TFN_LABELLED, valid_tfn),
    ("tfn", TFN_BARE, valid_tfn),
    ("acn", ACN, valid_acn),
    ("bsb", BSB, _length_six),
    ("email", EMAIL, _always),
    ("phone", PHONE, _always),
)


def is_statutory(candidate: str) -> bool:
    """True when a NAME match is legislative vocabulary rather than a person."""
    if STATUTORY.search(candidate):
        return True
    if any(len(t) > 1 and t.isupper() for t in re.split(r"[^A-Za-z]+", candidate)):
        return bool(_STATUTORY_CI.search(candidate))
    return False


def person_names(text: str) -> set[str]:
    """The set of NAME matches in *text* that survive the statutory filter."""
    return {m.group(0) for m in NAME.finditer(text) if not is_statutory(m.group(0))}


def structured_spans(text: str) -> list[tuple[int, int, str, str]]:
    """Return non-overlapping ``(start, end, kind, matched_text)``, sorted by start.

    Every pattern is run over the whole input and the results are resolved once,
    rather than replacing as we go. Replacing in place and re-scanning is how a
    redactor ends up matching its own placeholders.
    """
    found: list[tuple[int, int, str, str, int]] = []
    for priority, (kind, pattern, validator) in enumerate(_STRUCTURED):
        for match in pattern.finditer(text):
            group = 1 if pattern.groups else 0
            value = match.group(group)
            digits = "".join(c for c in value if c.isdigit())
            if kind not in {"email", "phone"} and not validator(digits):
                continue
            start, end = match.span(group)
            found.append((start, end, kind, value, priority))
    # Leftmost wins; at one start the longest wins; on an exact tie the kind
    # listed first in _STRUCTURED wins.
    found.sort(key=lambda f: (f[0], -(f[1] - f[0]), f[4]))
    taken: list[tuple[int, int, str, str]] = []
    consumed = -1
    for start, end, kind, value, _priority in found:
        if start >= consumed:
            taken.append((start, end, kind, value))
            consumed = end
    return taken
