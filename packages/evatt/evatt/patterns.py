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
* The TFN label separator is ``\\s*(?:[.:#\\u2013-]\\s*)?``. Written as the
  equivalent ``\\s*[.:#\\u2013-]?\\s*`` it backtracks quadratically on long
  space runs.
* Every phone alternative pins its digit count between ``(?<!\\d)`` and
  ``(?!\\d)`` so statutory references, years and grouped amounts stay out.

A label is evidence on its own. Something wrote "TFN" next to those digits, so
every labelled pattern here takes ``_always`` and no check digit: a labelled
identifier is reported even when its check digit fails, because a typo in a
real TFN is still a real TFN. Only a bare digit run has to earn its place,
since nothing but the check digit separates one from an ordinary number. The
origin module splits it the same way, applying its plausibility test inside
``if pattern is TFN_BARE`` and admitting the labelled pattern unconditionally.

Four deliberate divergences from the origin:

* The twelve month names are not in ``_STATUTORY_WORDS`` here. Over Register
  text they filter citation noise; over a workpaper they suppress ``June
  Smith`` and ``April Jones``, and a person who is never detected is never
  redacted. Terms such as "June Quarter" now reach triage instead, which is
  the safe direction: over-detection costs a triage decision, under-detection
  leaks.
* The labelled ABN, ACN and Medicare patterns are new. The origin scanned
  legislation, where only a labelled TFN turned up.
* ``TFN_BARE`` drops the origin's second money lookbehind, ``(?<![\\d$][\\s-])``.
  The origin scanned published legislation, where a false positive blocked a
  release; here it costs one placeholder in a private file. See the money-guard
  comment above the bare patterns.
* The name token accepts the Latin-1 accented letters, and ``Appeals`` joins the
  statutory vocabulary. Both follow from the same asymmetry as the months: over
  a workpaper an undetected person leaks and an over-detected phrase costs one
  triage decision.
"""
from __future__ import annotations

import re
from typing import Callable

# The token class carries the Latin-1 accented letters as well as A-Z. Without
# them "Zo\u00eb Nguyen", "Jos\u00e9 Ram\u00edrez" and "S\u00f8ren Kierkegaard"
# are never candidates and never reach triage, and Australian client data is
# full of such names. The ranges skip \u00d7 and \u00f7, the multiplication and
# division signs sitting inside the Latin-1 letter block. Cyrillic and CJK stay
# out: a script with no case distinction needs a different rule than
# "capitalised word", and that is a larger decision than this one.
_UPPER = r"A-Z\u00c0-\u00d6\u00d8-\u00de"
_LOWER = r"a-z\u00df-\u00f6\u00f8-\u00ff"
_TOKEN = r"[%s][%s%s'\u2019-]{1,20}" % (_UPPER, _UPPER, _LOWER)
NAME = re.compile(r"\b%s,?\s+%s(?:\s+%s)?\b" % (_TOKEN, _TOKEN, _TOKEN))
# One token on its own, used to find the token boundaries inside a NAME match.
_NAME_TOKEN = re.compile(_TOKEN)

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
# Every labelled pattern is built from the same two pieces and ends with the
# same trailing ``(?![\s-]?\d)`` digit-count pin. The TFN label alone accepts
# eight digits as well as nine, because TFNs issued before 1988 were eight.
#
# _QUALIFIER takes up to two of "number", "no" and "card", the last also spelled
# "cardholder", each with an optional full stop: a workpaper writes "Medicare
# card number", "ABN no." and "Medicare cardholder", and without the qualifier
# those digits fall through to the bare pattern and are lost the moment the
# check digit fails. Uniform across all four, because the shape of the label
# says nothing about which qualifier a typist reaches for.
#
# _SEP keeps the ``(?:[.:#\u2013-]\s*)?`` form: the equivalent ``[.:#\u2013-]?\s*``
# backtracks quadratically on long space runs. "#" earns its place beside "."
# and ":" because a transcribed card writes "Medicare card # 2123456701".
# _QUALIFIER adds no backtracking risk of its own: each repetition must consume
# at least one space and then a literal word, so over a run of spaces with no
# word after it the group fails once and the search falls through to _SEP.
_QUALIFIER = r"(?:\s+(?:number|no|card(?:holder)?)\b\.?){0,2}"
_SEP = r"\s*(?:[.:#\u2013-]\s*)?"
TFN_LABELLED = re.compile(
    r"\b(?:tax file number|TFN)\b%s%s(\d(?:[\s-]?\d){7,8})(?![\s-]?\d)" % (_QUALIFIER, _SEP),
    re.I,
)
# The trailing dot of the spaced-out form is optional: "A.B.N 51 824 753 556"
# is written as often as "A.B.N.", and the label is the evidence either way.
ABN_LABELLED = re.compile(
    r"(?:\bABN\b|\bA\.B\.N\.?)%s%s(\d(?:[\s-]?\d){10})(?![\s-]?\d)" % (_QUALIFIER, _SEP),
    re.I,
)
ACN_LABELLED = re.compile(
    r"(?:\bACN\b|\bA\.C\.N\.?)%s%s(\d(?:[\s-]?\d){8})(?![\s-]?\d)" % (_QUALIFIER, _SEP),
    re.I,
)
MEDICARE_LABELLED = re.compile(
    r"\bMedicare\b%s%s(\d(?:[\s-]?\d){9})(?![\s-]?\d)" % (_QUALIFIER, _SEP),
    re.I,
)
# All four bare runs take the same one-character money guard, ``(?<![\d$])``,
# and so favour over-detection. The origin's second lookbehind,
# ``(?<![\d$][\s-])``, also stopped a run starting part-way through a grouped
# amount, but it deleted real detections in the two shapes a workpaper is full
# of, the table row ("row 7 123456782") and the dated sentence ("in 2019
# 123456782 was issued"), and it deleted them for the most sensitive identifier
# of the four as readily as for the rest.
#
# The trade-off accepted here is the other direction: a grouped amount whose
# nine-digit tail happens to satisfy the mod-11 check is redacted as a TFN,
# which for such tails is roughly one in eleven. That price is worth paying,
# because over-redaction costs one placeholder in a private file while
# under-detection leaks a tax file number.
TFN_BARE = re.compile(r"(?<![\d$])(\d{3}([\s-]?)\d{3}\2\d{3})(?![\s-]?\d)")
ABN = re.compile(r"(?<![\d$])(\d{2}[\s-]?\d{3}[\s-]?\d{3}[\s-]?\d{3})(?![\s-]?\d)")
ACN = re.compile(r"(?<![\d$])(\d{3}[\s-]?\d{3}[\s-]?\d{3})(?![\s-]?\d)")
MEDICARE = re.compile(r"(?<![\d$])(\d{4}[\s-]?\d{5}[\s-]?\d)(?![\s-]?\d)")
# No check digit exists for a BSB, so the hyphen is required. Accepting bare
# six-digit runs would swallow ordinary numbers with nothing to reject them on.
BSB = re.compile(r"(?<![\d$-])(\d{3}-\d{3})(?![\d-])")

# The street-name tokens take the same accented letters as ``_TOKEN``. While
# they were ASCII-only, "12 Gr\u00fcner Street" missed ADDRESS and reached the
# name sweep instead, which reported the street as the person "Gr\u00fcner
# Street": the halt still fired, so nothing leaked, but the operator was asked
# the wrong question about the wrong kind of thing. The house-number suffix
# stays ASCII, because "12A" is a unit letter and never an accented one.
ADDRESS = re.compile(
    r"\b\d{1,4}[A-Za-z]?\s+[%s][%s%s'\u2019-]+(?:\s+[%s][%s%s'\u2019-]+)?\s+"
    r"(?:Street|St|Road|Rd|Avenue|Ave|Drive|Dr|Court|Ct|Place|Pl|Lane|Ln|"
    r"Parade|Pde|Crescent|Cres|Highway|Hwy|Terrace|Tce|Close|Way|Circuit|"
    r"Boulevard|Esplanade|Grove)\b" % (_UPPER, _UPPER, _LOWER, _UPPER, _UPPER, _LOWER)
)
# Australian workpapers write a date of birth numerically far more often than
# they spell the month. ``yyyy-mm-dd`` is deliberately absent: it collides with
# ordinary accounting period labels.
DOB = re.compile(
    r"\b(?:(?:0?[1-9]|[12]\d|3[01])\s+"
    r"(?:January|February|March|April|May|June|July|August|September|October|"
    r"November|December)\s+(?:19|20)\d{2}"
    r"|(?:0?[1-9]|[12]\d|3[01])/(?:0?[1-9]|1[0-2])/(?:19|20)\d{2}"
    r"|(?:0?[1-9]|[12]\d|3[01])-(?:0?[1-9]|1[0-2])-(?:19|20)\d{2})\b"
)
# The left boundary matters because Task 5 reaches for this with ``search``, not
# ``fullmatch``: without it "XCLIENT_01" reads as an assigned placeholder.
PLACEHOLDER = re.compile(
    r"(?<![A-Za-z0-9_])"
    r"(?:CLIENT|PERSON|STAFF|ENTITY|TFN|ABN|ACN|BSB|MEDICARE|EMAIL|PHONE)_\d{2,}"
)

# The origin list ends with the twelve month names. They are dropped here: over
# a workpaper they suppress "June Smith", "April Jones", "August Meyer" and
# "Ray May", and a person the sweep never reports is a person nobody redacts.
# DOB keeps its own month names; it is a separate pattern and unaffected.
#
# "Appeals" is added to the origin list. It is the one word the three-token
# window retry in ``person_name_spans`` needs to keep "Administrative Appeals
# Tribunal" filtered whole: the retry drops "Tribunal" and would otherwise
# offer "Administrative Appeals" as a person. Nothing is lost by it, because no
# Australian is named Appeals, and the alternative was leaving every real
# "<Given> <Family> Superannuation" undetected.
_STATUTORY_WORDS = (
    r"\b(Act|Regulation|Schedule|Division|Subdivision|Part|Chapter|Section|"
    r"Commissioner|Minister|Treasurer|Commonwealth|Australian|Australia|Board|"
    r"Tax|Taxation|Income|Superannuation|Court|Tribunal|Appeals|Determination|"
    r"Notice|Instrument|Amendment)\b"
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


# Order is the tie-break when two kinds match the identical span, and a labelled
# pattern captures the same digits as its bare equivalent, so every labelled
# entry is listed ahead of every bare one: the label names the kind, and a bare
# run of the same length must not take the span off it. Within each group a
# nine-digit run can satisfy both the TFN and the ACN check, so the more
# sensitive kind is listed first and wins.
#
# Labelled entries take ``_always``: the label is the evidence, so a labelled
# identifier is reported whether or not its check digit holds. Only the bare
# runs are validated.
_STRUCTURED: tuple[tuple[str, re.Pattern[str], Callable[[str], bool]], ...] = (
    ("abn", ABN_LABELLED, _always),
    ("medicare", MEDICARE_LABELLED, _always),
    ("tfn", TFN_LABELLED, _always),
    ("acn", ACN_LABELLED, _always),
    ("abn", ABN, valid_abn),
    ("medicare", MEDICARE, valid_medicare),
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


def person_name_spans(text: str) -> list[tuple[int, int, str]]:
    """Every surviving NAME candidate as ``(start, end, value)``, in document order.

    A three-token candidate rejected as statutory is retried over its two
    two-token windows. NAME is greedy and takes a third token whenever one is
    there, so a single statutory word beside a person absorbs the person and
    ``finditer`` never offers the shorter match: "Priya Sharma Superannuation"
    is how an SMSF is named, "the Board Daniel Okafor" is ordinary workpaper
    prose, and both used to pass through silently. A window that is itself
    statutory stays dropped, so "Income Tax Assessment" still reports nothing.

    The two-token case reports nothing of its own, for the same reason: dropping
    the statutory word leaves one token, which is not a name shape.

    A candidate that keeps nothing must not consume its own tokens, which is why
    the scan is a ``search`` loop rather than ``finditer``. A rejected candidate
    resumes at its SECOND token, so every token but the first is offered again
    inside the next candidate. ``finditer`` resumes past the whole match instead,
    and a heading ending in "<Cap> <Statutory>" then swallowed the family name on
    the line below it: NAME's ``\\s+`` spans the newline, so "Payroll Tax\\nJohn"
    is one three-token candidate whose windows are both statutory, and "Smith"
    was never offered to anything. Retrying from "Tax" instead yields
    "Tax\\nJohn Smith", whose second window is the person. The single-line form,
    "Income Tax John Smith called", leaked the same way and is fixed by the same
    resume.

    The loop terminates because ``pos`` strictly increases: NAME matches at least
    two tokens, so the second token starts after the candidate does, and every
    other path advances to ``match.end()``.

    Spans are returned rather than a bare set because the residual sweep has to
    map each candidate back to the line it starts on.
    """
    found: list[tuple[int, int, str]] = []
    pos = 0
    match = NAME.search(text, pos)
    while match is not None:
        candidate = match.group(0)
        if not is_statutory(candidate):
            found.append((match.start(), match.end(), candidate))
            pos = match.end()
        else:
            tokens = [token.span() for token in _NAME_TOKEN.finditer(candidate)]
            kept = False
            if len(tokens) == 3:
                for (start, _), (_, end) in zip(tokens, tokens[1:]):
                    window = candidate[start:end]
                    if not is_statutory(window):
                        found.append((match.start() + start, match.start() + end, window))
                        kept = True
            pos = match.end() if kept else match.start() + tokens[1][0]
        match = NAME.search(text, pos)
    return found


def person_names(text: str) -> set[str]:
    """The set of NAME matches in *text* that survive the statutory filter."""
    return {value for _start, _end, value in person_name_spans(text)}


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
