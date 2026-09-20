"""Read-only display of an existing close-review pack.

Phase B of the workbench: load the generated artefacts, prove they agree
with each other before showing anything, and render a review sheet. The viewer
never writes, renames or deletes a file, never opens a network connection, and
never changes what the engine computed. A tampered, partial or mismatched
artefact set fails closed with a named error instead of being displayed.

Every check here re-reads what ``report.write_review_pack`` emitted. The 2
renderers are independent witnesses of one engine run: if their contents stop
agreeing, the pack is no longer trustworthy evidence and the reviewer must hear
that from this command rather than infer it from a plausible-looking sheet.
"""

from __future__ import annotations

import csv
import hashlib
import io
import json
import re
from decimal import Decimal, InvalidOperation
from pathlib import Path

from .errors import ControlInputError

_JSON_NAME = "close-review-pack.json"
_SUMMARY_NAME = "close-summary.md"
_CSV_NAME = "exceptions.csv"
_QUERY_CSV_NAME = "client-queries.csv"

# The 3 files every pack has carried since the viewer existed. A pack
# written before the client-query register was added is archived evidence a
# firm may still have to display, so it must keep opening: the register is an
# extension to the pack, not a new requirement placed on old ones.
_REQUIRED_PACK_FILE_NAMES = (_JSON_NAME, _SUMMARY_NAME, _CSV_NAME)

PACK_FILE_NAMES = _REQUIRED_PACK_FILE_NAMES + (_QUERY_CSV_NAME,)

# The top-level members report._as_json has always emitted, no more and no
# less. An added or removed member means the file was edited by something other
# than the writer that produced the other artefacts.
_JSON_MEMBERS = frozenset(
    {
        "acknowledgement",
        "current_report_dates",
        "exceptions",
        "overall_status",
        "prior_report_dates",
        "source_sha256",
        "thresholds",
    }
)

# Present in a pack carrying the client-query register, absent in one written
# before it existed. Half a register is not a pack in either format, so
# verify_pack requires this member and client-queries.csv to arrive together.
#
# `calculation_evidence` joins it: the control is opt-in, so the member is
# there when it ran and absent when it did not, and both shapes are a pack the
# writer produced. Leaving it out here made every pack the control produced
# unopenable by `close-control view`.
_OPTIONAL_JSON_MEMBERS = frozenset(
    {"client_queries", "calculation_evidence", "controls_not_run"}
)

_THRESHOLD_KEYS = ("absolute_variance", "percentage_variance", "reconciliation_tolerance")

# The members report._as_json writes inside an acknowledgement, all of them
# strings. The sheet prints initials, reviewed_on and comment verbatim and
# states its own effect line, so effect is never displayed: it is compared
# against the writer's fixed text instead.
_ACKNOWLEDGEMENT_KEYS = ("reviewer_initials", "reviewed_on", "comment", "effect")

# Mirrored from report._ACKNOWLEDGEMENT_EFFECT.
_ACKNOWLEDGEMENT_EFFECT = (
    "Acknowledgement is evidence of human review only; it does not approve or close a period."
)

_STATUSES = ("PASS", "REVIEW", "BLOCKED")

_CSV_FIELDS = (
    "control",
    "status",
    "tenant",
    "account_id",
    "account_code",
    "account_name",
    "review_group",
    "current_value",
    "prior_value",
    "difference",
    "threshold",
    "percentage_change",
    "reason",
    "reviewer_action",
)

_QUERY_CSV_FIELDS = (
    "query_id",
    "control",
    "tenant",
    "account_id",
    "account_code",
    "account_name",
    "review_group",
    "difference",
    "question",
    "evidence_requested",
)

# Fields report._csv_safe guards with a leading apostrophe on the CSV side. It
# tests the value after lstrip, so the mirror below must strip too or a guarded
# value carrying leading whitespace looks like a tampered cell.
_CSV_GUARDED_FIELDS = frozenset(
    {"tenant", "account_id", "account_code", "account_name", "review_group"}
)

_QUERY_CSV_GUARDED_FIELDS = _CSV_GUARDED_FIELDS | {"question", "evidence_requested"}

_BOUNDARY_SENTENCE = (
    "This pack is a review aid. It does not approve a close, post a journal, "
    "make a payment, lodge a return, or lock a period."
)

# The client-query section's own boundary. Without it the section reads as
# correspondence somebody has already approved, which is the one way a list of
# a client's accounts and unexplained movements could do harm.
_CLIENT_QUERY_SENTENCE = (
    "These are draft questions for the preparer, derived from the exceptions "
    "above. Nothing has been sent to anyone."
)

_STATUS_LINE = re.compile(r"\*\*Overall status: (PASS|REVIEW|BLOCKED)\*\*")

# Calculation-evidence sources are keyed `calculation_evidence:<label>`, and
# a label is a slug, so the colon and hyphen belong in the label class. The
# class stays closed: `calculation_evidence.py` refuses a label outside it,
# so no caller-supplied text can widen what this pattern accepts.
_SOURCE_EVIDENCE_LINE = re.compile(r"`([a-z_0-9:-]+)`: `([0-9a-f]{64})`")


def _no_duplicate_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    """Reject a JSON object that states any member twice.

    A duplicated member is not valid evidence of anything: the 2 positions
    disagree about the pack, and standard json parsing would silently keep the
    last one, hiding the disagreement this command exists to surface.
    """
    seen: dict[str, object] = {}
    for key, value in pairs:
        if key in seen:
            raise ControlInputError(
                f"{_JSON_NAME}: member {key!r} appears more than once"
            )
        seen[key] = value
    return seen


def _read_artefact(pack_dir: Path, name: str) -> bytes:
    path = pack_dir / name
    try:
        return path.read_bytes()
    except IsADirectoryError as exc:
        raise ControlInputError(f"{name}: expected a file, found a directory") from exc
    except OSError as exc:
        if isinstance(exc, FileNotFoundError):
            raise ControlInputError(f"{name}: not found in {pack_dir}") from exc
        raise ControlInputError(f"{name}: could not be read from {pack_dir} ({exc})") from exc


def _load_artefact_bytes(pack_dir: Path) -> dict[str, bytes]:
    payloads = {name: _read_artefact(pack_dir, name) for name in _REQUIRED_PACK_FILE_NAMES}
    try:
        payloads[_QUERY_CSV_NAME] = (pack_dir / _QUERY_CSV_NAME).read_bytes()
    except FileNotFoundError:
        # A pack written before the register existed. Absence is checked
        # against the JSON member in verify_pack, so a half-converted pack is
        # still refused; only a wholly older one is displayed.
        pass
    except OSError:
        # Anything else about the path is a fault worth naming, not a reason to
        # read the pack as an older one.
        _read_artefact(pack_dir, _QUERY_CSV_NAME)
        raise  # pragma: no cover - _read_artefact always raises here.
    return payloads


def _parse_json(payload: bytes) -> dict[str, object]:
    try:
        document = json.loads(payload.decode("utf-8"), object_pairs_hook=_no_duplicate_keys)
    except UnicodeDecodeError as exc:
        raise ControlInputError(f"{_JSON_NAME}: not valid UTF-8") from exc
    except json.JSONDecodeError as exc:
        raise ControlInputError(f"{_JSON_NAME}: not valid JSON ({exc.msg})") from exc
    if not isinstance(document, dict):
        raise ControlInputError(f"{_JSON_NAME}: top level must be a JSON object")
    members = set(document)
    unknown = sorted(members - _JSON_MEMBERS - _OPTIONAL_JSON_MEMBERS)
    if unknown:
        raise ControlInputError(
            f"{_JSON_NAME}: unknown top-level member(s): {', '.join(unknown)}"
        )
    missing = sorted(_JSON_MEMBERS - members)
    if missing:
        raise ControlInputError(
            f"{_JSON_NAME}: missing top-level member(s): {', '.join(missing)}"
        )
    return document


def _require_string_list(document: dict[str, object], member: str) -> list[str]:
    values = document[member]
    if not isinstance(values, list) or not all(isinstance(item, str) for item in values):
        raise ControlInputError(f"{_JSON_NAME}: {member} must be a list of strings")
    return values


def _parse_threshold(text: object, key: str) -> Decimal:
    """Parse a threshold rendered by report._money or report._percentage."""
    if not isinstance(text, str) or not text.endswith("%"):
        candidate = text if isinstance(text, str) else None
    else:
        candidate = text[:-1]
    if candidate is None:
        raise ControlInputError(f"{_JSON_NAME}: thresholds.{key} must be a string")
    try:
        value = Decimal(candidate)
    except InvalidOperation as exc:
        raise ControlInputError(
            f"{_JSON_NAME}: thresholds.{key} is not a decimal: {text!r}"
        ) from exc
    if not value.is_finite() or value < 0:
        raise ControlInputError(
            f"{_JSON_NAME}: thresholds.{key} must be finite and non-negative"
        )
    return value


def _verify_json_schema(document: dict[str, object]) -> None:
    status = document["overall_status"]
    if status not in _STATUSES:
        raise ControlInputError(
            f"{_JSON_NAME}: overall_status must be one of "
            f"{', '.join(_STATUSES)}; got {status!r}"
        )
    _require_string_list(document, "current_report_dates")
    _require_string_list(document, "prior_report_dates")

    thresholds = document["thresholds"]
    if not isinstance(thresholds, dict) or set(thresholds) != set(_THRESHOLD_KEYS):
        raise ControlInputError(
            f"{_JSON_NAME}: thresholds must hold exactly "
            f"{', '.join(_THRESHOLD_KEYS)}"
        )
    for key in _THRESHOLD_KEYS:
        _parse_threshold(thresholds[key], key)

    source_hashes = document["source_sha256"]
    if not isinstance(source_hashes, dict) or not source_hashes:
        raise ControlInputError(f"{_JSON_NAME}: source_sha256 must be a non-empty object")
    for label, digest in source_hashes.items():
        if not isinstance(digest, str) or not re.fullmatch(r"[0-9a-f]{64}", digest):
            raise ControlInputError(
                f"{_JSON_NAME}: source_sha256[{label!r}] is not a lowercase SHA-256 digest"
            )

    exceptions = document["exceptions"]
    if not isinstance(exceptions, list):
        raise ControlInputError(f"{_JSON_NAME}: exceptions must be a list")
    for index, item in enumerate(exceptions):
        if not isinstance(item, dict):
            raise ControlInputError(f"{_JSON_NAME}: exceptions[{index}] must be an object")
        # The exceptions CSV projects exactly these fields, so a member
        # outside them is one no other file witnesses: it survives every
        # cross-file comparison and leaves an edited pack verifying. The
        # top-level members and client_queries items are held to their exact
        # sets for the same reason.
        if set(item) != set(_CSV_FIELDS):
            raise ControlInputError(
                f"{_JSON_NAME}: exceptions[{index}] does not hold the "
                f"fields the writer emits: holds {sorted(item)!r}, "
                f"expected {sorted(_CSV_FIELDS)!r}"
            )
        if item.get("status") not in _STATUSES:
            raise ControlInputError(
                f"{_JSON_NAME}: exceptions[{index}].status is not a pack status"
            )

    # A query names an account and a movement, so a malformed or duplicated
    # register has to be named here rather than reach a preparer as a question
    # they might put to a client. A pack from before the register existed
    # carries no member to check.
    if "client_queries" in document:
        client_queries = document["client_queries"]
        if not isinstance(client_queries, list):
            raise ControlInputError(f"{_JSON_NAME}: client_queries must be a list")
        seen_ids: set[str] = set()
        for index, query in enumerate(client_queries):
            if not isinstance(query, dict):
                raise ControlInputError(
                    f"{_JSON_NAME}: client_queries[{index}] must be an object"
                )
            # The CSV projects exactly these fields, so a member outside them is
            # one no other file witnesses: it survives every cross-file
            # comparison and leaves an edited pack verifying. The top-level
            # members are held to their exact set for the same reason. The
            # register is new, so no earlier pack carries a different shape.
            if set(query) != set(_QUERY_CSV_FIELDS):
                raise ControlInputError(
                    f"{_JSON_NAME}: client_queries[{index}] does not hold the "
                    f"fields the writer emits: holds {sorted(query)!r}, "
                    f"expected {sorted(_QUERY_CSV_FIELDS)!r}"
                )
            for field in _QUERY_CSV_FIELDS:
                if not isinstance(query[field], str):
                    raise ControlInputError(
                        f"{_JSON_NAME}: client_queries[{index}].{field} must be a string"
                    )
            query_id = query["query_id"]
            if not query_id:
                raise ControlInputError(
                    f"{_JSON_NAME}: client_queries[{index}].query_id must be a "
                    "non-empty string"
                )
            if query_id in seen_ids:
                raise ControlInputError(
                    f"{_JSON_NAME}: client_queries holds {query_id} more than once"
                )
            seen_ids.add(query_id)

    # An acknowledgement records a human action, so a malformed one must be
    # named here rather than reach the sheet as a traceback or as text the
    # renderer never checked.
    acknowledgement = document["acknowledgement"]
    if acknowledgement is not None:
        if not isinstance(acknowledgement, dict):
            raise ControlInputError(
                f"{_JSON_NAME}: acknowledgement must be null or an object"
            )
        # An extra or renamed member is one no other file witnesses, so an
        # edited acknowledgement could verify. The writer emits exactly
        # _ACKNOWLEDGEMENT_KEYS.
        if set(acknowledgement) != set(_ACKNOWLEDGEMENT_KEYS):
            raise ControlInputError(
                f"{_JSON_NAME}: acknowledgement does not hold the fields "
                f"the writer emits: holds {sorted(acknowledgement)!r}, "
                f"expected {sorted(_ACKNOWLEDGEMENT_KEYS)!r}"
            )
        for key in _ACKNOWLEDGEMENT_KEYS:
            if not isinstance(acknowledgement.get(key), str):
                raise ControlInputError(
                    f"{_JSON_NAME}: acknowledgement.{key} must be a string"
                )
        # The sheet states the effect itself, so the JSON member is never
        # displayed and a shape check alone let a false or blank statement
        # verify. It is fixed writer text, not reviewer prose: compare it.
        if acknowledgement["effect"] != _ACKNOWLEDGEMENT_EFFECT:
            raise ControlInputError(
                f"{_JSON_NAME}: acknowledgement.effect is not the text the writer emits"
            )


def _summary_source_evidence(summary_text: str) -> dict[str, str]:
    """Collect the summary's digest lines, rejecting a contradicted label.

    As with a duplicated JSON member, keeping the last of 2 disagreeing
    digest lines for one source hides the disagreement: a falsified line
    paired with a duplicate carrying the true digest would then agree with
    the JSON pack and display. The whole document is scanned, so a forged
    second "## Source evidence" heading opens no unchecked region. A
    reviewer acknowledgement may legitimately quote a digest line, and an
    identical repeat states no second claim, so only a differing repeat is
    a disagreement.
    """
    found: dict[str, str] = {}
    for label, digest in _SOURCE_EVIDENCE_LINE.findall(summary_text):
        if found.get(label, digest) != digest:
            raise ControlInputError(
                f"{_SUMMARY_NAME}: source-evidence label {label!r} appears "
                "with two different digests"
            )
        found[label] = digest
    if not found:
        raise ControlInputError(f"{_SUMMARY_NAME}: no source-evidence digest lines found")
    return found


def _verify_rows_match(
    *,
    csv_name: str,
    member: str,
    noun: str,
    json_items: list,
    csv_rows: list[dict[str, str]],
    fields: tuple[str, ...],
    guarded: frozenset[str] | set[str],
) -> None:
    """Prove one CSV states exactly what the JSON pack states, cell by cell.

    Shared by the exception and client-query files: both are a flat projection
    of a JSON member, and a row that disagrees with the member it came from
    means the pack is no longer one run's evidence. ``member`` names the JSON
    member for a shape complaint and ``noun`` reads as one item in a
    disagreement, because a reviewer is being told about one row, not a list.
    """
    if len(csv_rows) != len(json_items):
        raise ControlInputError(
            f"{noun} counts disagree: {_JSON_NAME} holds {len(json_items)}, "
            f"{csv_name} holds {len(csv_rows)} data rows"
        )
    for index, (item, row) in enumerate(zip(json_items, csv_rows)):
        for field in fields:
            expected = item.get(field)
            if not isinstance(expected, str):
                raise ControlInputError(
                    f"{_JSON_NAME}: {member}[{index}].{field} must be a string"
                )
            actual = row.get(field)
            if actual is None:
                raise ControlInputError(
                    f"{csv_name}: row {index + 1} has no {field} column value"
                )
            if field in guarded and expected.lstrip().startswith(("=", "+", "-", "@")):
                expected = "'" + expected
            if actual != expected:
                raise ControlInputError(
                    f"{field} disagrees on {noun} {index + 1}: {_JSON_NAME} says "
                    f"{expected!r}, {csv_name} says {actual!r}"
                )


# Anchored to a whole line. The writer emits this as its own line, while every
# source-derived value it renders is prefixed by something: a table cell by
# '| ', a one-line reviewer comment by '- Comment: ', a quoted comment line by
# '  > '. Searching the document instead would count a client's own figure as a
# second count line and refuse a pack whose 4 artefacts agree.
_QUERY_COUNT_LINE = re.compile(r"^- Client queries drafted: (\d+)\.\r?$", re.MULTILINE)

_CLIENT_QUERY_HEADING = "## Client queries"

# The heading levels report._as_markdown emits, and so where one of its
# sections ends. Anything else inside the section is content the writer never
# wrote, and the line-by-line comparison below refuses it rather than treating
# it as a boundary.
_MD_SECTION_HEADING = re.compile(r"^#{1,2}\s")

# The headings report._as_markdown emits, in the order it emits them. A pack
# written before the client-query register existed carries the same list
# without that one.
#
# Asking whether the document's heading structure is the writer's, rather than
# whether some line spells "Client queries" the way this file expects, is what
# ends an argument that cannot otherwise be won. CommonMark renders a heading
# from ATX hashes at 6 levels with an optional closing run, from a Setext
# underline with no hashes at all, and from inline content that may carry
# emphasis, code spans or character references: '## Client queries ##',
# 'Client queries' over a rule, and '## Client &#113;ueries' all display as the
# writer's own heading does. Enumerating those spellings is a list that is
# never finished, and each round of it leaves whatever was not enumerated as a
# region no check reads. Recognising a heading, on the other hand, needs no
# knowledge of what it says.
_CALCULATION_EVIDENCE_HEADING = "## Calculation evidence"

_SUMMARY_HEADINGS = (
    "# Monthly Close Review Pack",
    "## Scope",
    "## Source evidence",
    _CALCULATION_EVIDENCE_HEADING,
    "## Exceptions",
    _CLIENT_QUERY_HEADING,
    "## Human acknowledgement",
)

# A heading opener: ATX hashes, or a line underlined by a Setext rule. Neither
# pattern reads the heading's text, and neither can fire on source-derived
# content, because every value the writer renders is prefixed by something: a
# table cell by '| ', a one-line comment by '- Comment: ', a quoted comment
# line by '  > ', a digest by '- `'. The writer emits no horizontal rule, so a
# '---' line under a paragraph is not something it produces either.
_ATX_HEADING = re.compile(r"^ {0,3}#{1,6}(?:[ \t]|$)")
_SETEXT_UNDERLINE = re.compile(r"^ {0,3}(?:=+|-+)[ \t]*$")

# Two block constructs the writer never emits, and that change what a reader
# sees rather than adding to it. An unclosed code fence turns the whole rest of
# the document into a code block, so the headings and both tables stop
# rendering at all while their source lines still read exactly as they should.
# A raw HTML block does the opposite: '<h2>Client queries</h2>' renders as a
# heading that no scan of Markdown syntax will see. Both were confirmed against
# a CommonMark renderer.
#
# Neither can come from source data. Every value the writer renders is prefixed
# by something, so no line it produces begins with a backtick, a tilde or a
# '<'. Refusing them outright is therefore free, and it closes the whole class
# rather than the 2 spellings: what is being asked is not "is this a fence"
# but "did the writer write anything of this shape", and the answer is no.
_RENDER_ALTERING_BLOCK = re.compile(r"^ {0,3}(?:`{3,}|~{3,}|<)")


def _heading_lines(summary_text: str) -> list[str]:
    """Return every line a Markdown renderer displays as a heading, in order."""
    lines = summary_text.splitlines()
    headings = []
    for index, line in enumerate(lines):
        if _ATX_HEADING.match(line):
            headings.append(line)
        elif (
            line.strip()
            and index + 1 < len(lines)
            and _SETEXT_UNDERLINE.match(lines[index + 1])
        ):
            headings.append(line)
    return headings


def _verify_summary_headings(summary_text: str, *, register: bool, evidence: bool) -> None:
    """Prove the summary's headings are the writer's, exactly and in order.

    A forged section needs a heading, or a reviewer scrolling past it reads a
    bare table with nothing claiming to be the register. Requiring the whole
    heading structure to match refuses that heading whatever syntax spells it,
    and refuses a second Exceptions or Source evidence section too, which
    nothing else here looks for.

    Reading the headings out of the source presumes the source is what renders.
    A code fence or a raw HTML block breaks that presumption in either
    direction, so both are refused first.
    """
    for number, line in enumerate(summary_text.splitlines(), start=1):
        if _RENDER_ALTERING_BLOCK.match(line):
            raise ControlInputError(
                f"{_SUMMARY_NAME}: line {number} opens a code fence or an HTML "
                f"block, which the writer never emits: {line!r}"
            )

    expected = [
        heading
        for heading in _SUMMARY_HEADINGS
        if (register or heading != _CLIENT_QUERY_HEADING)
        and (evidence or heading != _CALCULATION_EVIDENCE_HEADING)
    ]
    found = _heading_lines(summary_text)
    if found == expected:
        return
    with_register = [
        heading
        for heading in _SUMMARY_HEADINGS
        if evidence or heading != _CALCULATION_EVIDENCE_HEADING
    ]
    if not register and found == with_register:
        # The one shape worth naming for itself: a current pack whose register
        # files were removed while its summary kept the section. Compared
        # against the headings this pack should otherwise have, not against
        # every heading the writer can emit: a pack with no calculation
        # evidence has no such section either, and matching the full list
        # would lose this message the moment an optional section was added.
        raise ControlInputError(
            f"{_SUMMARY_NAME}: holds a client-query section while the pack "
            "carries no client-query register; the register is half removed, "
            "not absent"
        )
    raise ControlInputError(
        f"{_SUMMARY_NAME}: the headings are not the ones the writer emits: "
        f"holds {found!r}, expected {expected!r}"
    )

_CLIENT_QUERY_TABLE_HEADER = (
    "| Query | Control | Tenant | Account | Difference | Question | Evidence requested |"
)

_CLIENT_QUERY_TABLE_DELIMITER = "| --- | --- | --- | --- | ---: | --- | --- |"

# report._CLIENT_QUERY_PREAMBLE, and the line it writes when no exception
# raised a question, mirrored here as _md_cell_mirror mirrors report._md_cell.
# The section is rebuilt from these and compared, rather than searched for
# landmarks, because searching leaves everything it does not look for
# unchecked: a replaced delimiter row stops the register rendering as a table
# at all, and an inserted line puts a sentence in front of a preparer that no
# run produced. Neither disturbs a landmark.
_CLIENT_QUERY_PREAMBLE = (
    "These are draft questions for the preparer, derived from the exceptions above. "
    "Nothing has been sent to anyone. Read and edit them before they reach a client, "
    "and record the answers in the firm's own tracker: this pack is evidence of one "
    "run, and editing it breaks the check that its files still agree.",
    "",
    "Answering every query does not close the period, and a query nobody raised is "
    "not evidence that nothing needs asking.",
    "",
)

_CLIENT_QUERY_EMPTY_REGISTER = (
    "No exception raised a question for the client. Every exception in this "
    "pack, if any, is one the firm settles from its own records."
)

# The renderer's placeholder for a tenant, account or difference it has none of.
_ABSENT = "n/a"

_SCOPE_HEADING = "## Scope"
_EXCEPTIONS_HEADING = "## Exceptions"
_ACKNOWLEDGEMENT_HEADING = "## Human acknowledgement"

# report._as_markdown's exception table and the 2 lines it writes in place of a
# section, mirrored here for the same reason _CLIENT_QUERY_PREAMBLE is.
_EXCEPTION_TABLE_HEADER = (
    "| Status | Control | Tenant | Account | Difference | Reason |"
)

_EXCEPTION_TABLE_DELIMITER = "| --- | --- | --- | --- | ---: | --- |"

_EMPTY_EXCEPTIONS_LINE = (
    "No exceptions were raised. A human must still decide whether the close "
    "is appropriate."
)

_NO_ACKNOWLEDGEMENT_LINE = (
    "No reviewer acknowledgement was supplied. This does not create or imply "
    "an approval."
)

# The writer states this effect itself rather than printing the JSON member, so
# it is a fixed line, not a value to compare against the pack.
_ACKNOWLEDGEMENT_EFFECT_LINE = (
    "- Effect: acknowledgement records a human action only; it does not change "
    "the control status or approve a close."
)


def _section_lines(summary_text: str, heading: str) -> list[str]:
    """Return the lines under the one occurrence of ``heading``, or raise.

    The heading has to be a heading. report._md_cell escapes a cell's pipes,
    backslashes, asterisks and backticks but not its hashes, and a one-line
    reviewer comment is rendered inline, so under a substring search an account
    named '## Client queries' or a comment quoting the phrase would count as a
    second section and refuse a pack whose 4 artefacts were written together
    and agree.

    _verify_summary_headings has already proved the document's headings are the
    writer's, exactly and in order, so there is one such line and it is the one
    the writer emitted. This finds it and returns what sits under it.
    """
    lines = summary_text.splitlines()
    headings = [index for index, line in enumerate(lines) if line == heading]
    if len(headings) != 1:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: expected exactly one {heading!r} heading, "
            f"found {len(headings)}"
        )
    start = headings[0] + 1
    for index in range(start, len(lines)):
        if _MD_SECTION_HEADING.match(lines[index]):
            return lines[start:index]
    return lines[start:]


def _client_query_section_lines(summary_text: str) -> list[str]:
    """Return the lines under the one client-query heading, or raise."""
    return _section_lines(summary_text, _CLIENT_QUERY_HEADING)


def _md_cell_mirror(value: str) -> str:
    """Mirror report._md_cell, as _CSV_GUARDED_FIELDS mirrors _csv_safe.

    Reimplemented rather than imported so the viewer stays an independent
    witness: a renderer that changes how it escapes a cell has to be reflected
    here deliberately, and until it is, this check fails rather than agreeing
    with whatever the writer now produces.
    """
    return (
        " ".join(value.split())
        .replace("\\", "\\\\")
        .replace("|", "\\|")
        .replace("*", "\\*")
        .replace("`", "\\`")
    )


def _expected_summary_row(query: dict) -> str:
    """Rebuild the table row report._as_markdown renders for one query.

    Every cell is reconstructed, not a chosen few, so the check is a row-level
    comparison rather than a search for text that appears somewhere. Two rows
    whose questions were swapped hold the same identifiers, the same count and
    the same set of questions; only rebuilding the row catches it, and a
    preparer reading a swapped table asks the right question about the wrong
    account.
    """
    fields = {}
    for name in ("query_id", "control", "tenant", "account_id", "account_code",
                 "account_name", "difference", "question", "evidence_requested"):
        value = query.get(name)
        if not isinstance(value, str):
            raise ControlInputError(
                f"{_JSON_NAME}: client_queries[{query.get('query_id')!r}].{name} "
                "must be a string"
            )
        fields[name] = value
    account = " / ".join(
        piece for piece in (fields["account_code"], fields["account_name"]) if piece
    ) or fields["account_id"] or _ABSENT
    cells = (
        fields["query_id"],
        fields["control"],
        _md_cell_mirror(fields["tenant"] or _ABSENT),
        _md_cell_mirror(account),
        fields["difference"] or _ABSENT,
        _md_cell_mirror(fields["question"]),
        _md_cell_mirror(fields["evidence_requested"]),
    )
    return "| " + " | ".join(cells) + " |"


def _verify_summary_states_the_register(
    summary_text: str, client_queries: list
) -> None:
    """Prove close-summary.md holds the register the JSON pack holds.

    The summary is the artefact a preparer reads and copies a question out of,
    so a row edited there alone is the divergence that matters most: the pack
    would verify while the file somebody actually works from asks something the
    run never asked.

    Every line of the section is rebuilt from the JSON and compared, rather than
    the section being searched for landmarks. A search leaves whatever it does
    not look for unchecked, and 2 of those matter: the delimiter row, without
    which the register stops rendering as a table at all, and any line inserted
    between the ones the writer wrote, which puts a sentence in front of a
    preparer that no run produced. Neither disturbs a landmark.
    """
    section = _client_query_section_lines(summary_text)
    if _CLIENT_QUERY_SENTENCE not in " ".join(" ".join(section).split()):
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the client-query boundary statement is missing or altered"
        )

    counts = _QUERY_COUNT_LINE.findall(summary_text)
    if len(counts) != 1:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: expected exactly one client-query count line, "
            f"found {len(counts)}"
        )
    if int(counts[0]) != len(client_queries):
        raise ControlInputError(
            f"client-query counts disagree: {_JSON_NAME} holds "
            f"{len(client_queries)}, {_SUMMARY_NAME} states {counts[0]}"
        )

    lead = ["", *_CLIENT_QUERY_PREAMBLE]
    if section[: len(lead)] != lead:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the client-query section's preamble is missing or "
            "altered"
        )
    body = section[len(lead) :]
    if not body or body[-1] != "":
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the client-query section does not end where the "
            "writer ends it"
        )
    body = body[:-1]

    if not client_queries:
        if body != [_CLIENT_QUERY_EMPTY_REGISTER]:
            raise ControlInputError(
                f"{_SUMMARY_NAME}: holds a client-query table while {_JSON_NAME} "
                "holds no queries"
            )
        return
    if not body or body[0] != _CLIENT_QUERY_TABLE_HEADER:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the client-query table header is missing or altered"
        )
    if len(body) < 2 or body[1] != _CLIENT_QUERY_TABLE_DELIMITER:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the client-query table delimiter is missing or "
            "altered; the register would not render as a table"
        )
    rows = body[2:]
    if len(rows) != len(client_queries):
        raise ControlInputError(
            f"client-query counts disagree: {_JSON_NAME} holds "
            f"{len(client_queries)}, {_SUMMARY_NAME} renders {len(rows)} table rows"
        )
    for index, (query, row) in enumerate(zip(client_queries, rows)):
        expected = _expected_summary_row(query)
        if row != expected:
            raise ControlInputError(
                f"{_SUMMARY_NAME}: client-query row {index + 1} disagrees with "
                f"{_JSON_NAME}: renders {row!r}, expected {expected!r}"
            )


def _verify_rebuilt_section(
    summary_text: str, heading: str, expected: list[str], *, trailing_blank: bool
) -> None:
    """Prove one summary section is line for line what the writer would emit.

    Rebuilding beats searching for the same reason it does in the client-query
    section: a search leaves whatever it does not look for unchecked, and what
    a reviewer reads off the sheet is the whole section, not the parts a check
    happened to name. The leading blank line and, for every section but the
    last, the trailing one are part of what the writer emits, so they are
    compared too.
    """
    observed = _section_lines(summary_text, heading)
    wanted = ["", *expected, ""] if trailing_blank else ["", *expected]
    if observed != wanted:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the {heading.lstrip('# ')!r} section disagrees with "
            f"{_JSON_NAME}: renders {observed!r}, expected {wanted!r}"
        )


def _expected_scope_lines(document: dict[str, object], *, register: bool) -> list[str]:
    """Rebuild the scope bullets report._as_markdown renders.

    Every one of them is a number a reviewer acts on: the periods compared, the
    thresholds that decided what counted as an exception, the tolerance that
    decided what counted as reconciled, and the counts that say how much there
    is to look at. Left uncompared, each could be edited in the sheet alone
    while the pack still verified.
    """
    current = _require_string_list(document, "current_report_dates")
    prior = _require_string_list(document, "prior_report_dates")
    thresholds = document["thresholds"]
    assert isinstance(thresholds, dict)
    exceptions = document["exceptions"]
    assert isinstance(exceptions, list)
    blocked = sum(1 for item in exceptions if _exception_field(item, "status") == "BLOCKED")
    review = sum(1 for item in exceptions if _exception_field(item, "status") == "REVIEW")
    lines = [
        f"- Current report date(s): {', '.join(current)}",
        f"- Prior report date(s): {', '.join(prior)}",
        f"- Material variance thresholds: ${thresholds['absolute_variance']}"
        f" and {thresholds['percentage_variance']}",
        f"- Reconciliation tolerance: ${thresholds['reconciliation_tolerance']}",
        f"- Exceptions: {len(exceptions)} total; {blocked} blocked; "
        f"{review} requiring review.",
    ]
    if register:
        queries = document["client_queries"]
        assert isinstance(queries, list)
        lines.append(f"- Client queries drafted: {len(queries)}.")
    # A pack written before the control-coverage line existed carries neither
    # the member nor the bullet, the same way the register is handled above.
    if "controls_not_run" in document:
        lines.append(
            f"- Controls not run: "
            f"{', '.join(_require_string_list(document, 'controls_not_run')) or 'none'}."
        )
    return lines


def _exception_field(item: object, name: str) -> str:
    if not isinstance(item, dict) or not isinstance(item.get(name), str):
        raise ControlInputError(
            f"{_JSON_NAME}: exceptions[].{name} must be a string"
        )
    value = item[name]
    assert isinstance(value, str)
    return value


def _expected_exception_row(item: object) -> str:
    """Rebuild the table row report._as_markdown renders for one exception."""
    account = " / ".join(
        piece
        for piece in (
            _exception_field(item, "account_code"),
            _exception_field(item, "account_name"),
        )
        if piece
    ) or _exception_field(item, "account_id") or _ABSENT
    cells = (
        _exception_field(item, "status"),
        _exception_field(item, "control"),
        _md_cell_mirror(_exception_field(item, "tenant") or _ABSENT),
        _md_cell_mirror(account),
        _exception_field(item, "difference") or _ABSENT,
        _md_cell_mirror(_exception_field(item, "reason")),
    )
    return "| " + " | ".join(cells) + " |"


def _expected_exception_lines(exceptions: list) -> list[str]:
    """Rebuild the exceptions section, table and all.

    This is the section the finding turned on: a difference of -$250 read as
    -$999 on the sheet while exceptions.csv and the JSON still said -$250. The
    CSV was already compared with the JSON; the sheet the reviewer reads was
    not.
    """
    if not exceptions:
        return [_EMPTY_EXCEPTIONS_LINE]
    return [
        _EXCEPTION_TABLE_HEADER,
        _EXCEPTION_TABLE_DELIMITER,
        *(_expected_exception_row(item) for item in exceptions),
    ]


def _md_note_lines_mirror(comment: str) -> list[str]:
    """Mirror report._md_note_lines, as _md_cell_mirror mirrors report._md_cell."""
    if "\n" not in comment and "\r" not in comment:
        return [f"- Comment: {_md_cell_mirror(comment)}"]
    lines = ["- Comment:"]
    for raw in comment.splitlines():
        line = _md_cell_mirror(raw)
        if line.startswith("#"):
            line = "\\" + line
        lines.append(f"  > {line}".rstrip())
    return lines


def _expected_acknowledgement_lines(acknowledgement: object) -> list[str]:
    """Rebuild the acknowledgement section.

    Initials and a date are the only claim in the pack about who looked at it
    and when. An acknowledgement is not an approval, which is exactly why the
    name against it has to be the name the run recorded.
    """
    if acknowledgement is None:
        return [_NO_ACKNOWLEDGEMENT_LINE]
    assert isinstance(acknowledgement, dict)
    initials = acknowledgement["reviewer_initials"]
    reviewed_on = acknowledgement["reviewed_on"]
    comment = acknowledgement["comment"]
    assert isinstance(initials, str) and isinstance(reviewed_on, str)
    assert isinstance(comment, str)
    return [
        f"- Reviewer initials: {_md_cell_mirror(initials)}",
        f"- Reviewed on: {reviewed_on}",
        *_md_note_lines_mirror(comment),
        _ACKNOWLEDGEMENT_EFFECT_LINE,
    ]


_CALCULATION_EVIDENCE_TABLE_HEADER = (
    "| Calculation | Status | Period | Figures | Relied on | Entry digest |"
)
_CALCULATION_EVIDENCE_TABLE_DELIMITER = "| --- | --- | --- | --- | --- | --- |"
_CALCULATION_EVIDENCE_REQUIRED = re.compile(r"^Required: (none|`[a-z0-9]+(?:-[a-z0-9]+)*`(?:; `[a-z0-9]+(?:-[a-z0-9]+)*`)*)$")

# Mirrored from report._CALCULATION_EVIDENCE_EFFECT. The block's effect text is
# reviewer-facing and lives only in the JSON, so it is compared against this
# fixed text rather than accepted as whatever the file says.
_CALCULATION_EVIDENCE_EFFECT = (
    "Evidence read from files. This pack made no calculation and contacted no "
    "service. A figure here supports review; it does not approve anything. "
    "`usable` is true only where the file hangs together, carries a figure and "
    "covers this close's period; read the exceptions for why one is false."
)

# Mirrored from report._CALCULATION_EVIDENCE_PREAMBLE and the line it writes
# when a calculation was required and no file supplied one, for the same
# reason _CLIENT_QUERY_PREAMBLE is mirrored: the viewer stays an independent
# witness, so a renderer that changes its wording has to be reflected here
# deliberately rather than agreeing with whatever the writer now produces.
_CALCULATION_EVIDENCE_PREAMBLE = (
    "Read from files supplied to this run. This pack made no calculation and "
    "contacted no service. A figure here supports review and approves nothing.",
    "",
)
_NO_CALCULATION_EVIDENCE = (
    "A calculation was required for this close and no evidence file was supplied "
    "for it. The exceptions below say which."
)

#: The members report._as_json writes for one supplied calculation. Fixed, so
#: an added or renamed field is refused rather than displayed as though the
#: writer had put it there.
_EVIDENCE_ITEM_MEMBERS = frozenset({
    "label", "schema", "provider", "calculator", "period", "status", "engine",
    "calculation_sha256", "file_sha256", "synthetic_input", "rate_tables",
    "advisory_notes", "values", "usable",
})
_EVIDENCE_BLOCK_MEMBERS = frozenset({"required", "supplied", "effect"})


def _entry_digest_mirror(entry: dict[str, object]) -> str:
    """Mirror report._entry_digest: SHA-256 of the entry's canonical JSON.

    Reimplemented rather than imported for the reason _md_cell_mirror is: the
    viewer is an independent witness, and a writer that changes how it
    digests an entry has to be reflected here deliberately.
    """
    return hashlib.sha256(
        json.dumps(entry, sort_keys=True, separators=(",", ":"), ensure_ascii=False,
                   allow_nan=False).encode("utf-8")
    ).hexdigest()


def _evidence_summary_rows(
    summary_text: str,
) -> tuple[list[str], dict[str, tuple[str, str, str, str, str]]]:
    """The calculation-evidence section, read as a whole and line by line.

    Read from the summary's own section, so the JSON pack has a second
    artefact to agree with. Without it the block's figures and its `usable`
    flag were the only copy in the pack, and editing them left every other
    check passing.

    Every line is accounted for, not just the ones that parse as table rows.
    Skipping the rest would let a paragraph of reviewer-facing instructions sit
    under this heading and still verify, which is the forgery the whole-section
    comparison exists to refuse: the writer emits a fixed preamble, then either
    one fixed sentence or a header, a delimiter and one row per calculation,
    and nothing else belongs between them.
    """
    section = _section_lines(summary_text, _CALCULATION_EVIDENCE_HEADING)
    lead = ["", *_CALCULATION_EVIDENCE_PREAMBLE]
    if section[: len(lead)] != lead:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the calculation-evidence preamble is missing or altered"
        )
    body = section[len(lead) :]
    if not body or body[-1] != "":
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the calculation-evidence section does not end where the "
            "writer ends it"
        )
    body = body[:-1]

    if len(body) < 2 or body[1] != "":
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the calculation-evidence section's required line is missing "
            "or not where the writer puts it"
        )
    required_match = _CALCULATION_EVIDENCE_REQUIRED.match(body[0])
    if required_match is None:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the calculation-evidence required line is malformed: {body[0]!r}"
        )
    required_text = required_match.group(1)
    required = (
        [] if required_text == "none"
        else [name.strip("`") for name in required_text.split("; ")]
    )
    body = body[2:]

    if body == [_NO_CALCULATION_EVIDENCE]:
        return required, {}
    if not body or body[0] != _CALCULATION_EVIDENCE_TABLE_HEADER:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the calculation-evidence table header is missing or altered"
        )
    if len(body) < 2 or body[1] != _CALCULATION_EVIDENCE_TABLE_DELIMITER:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the calculation-evidence table delimiter is missing or altered"
        )

    rows: dict[str, tuple[str, str, str, str, str]] = {}
    for line in body[2:]:
        if not line.startswith("| ") or not line.endswith(" |"):
            raise ControlInputError(
                f"{_SUMMARY_NAME}: the calculation-evidence section holds a line the writer "
                f"never emits: {line!r}"
            )
        cells = [cell.strip() for cell in line.strip("|").split("|")]
        if len(cells) != 6:
            raise ControlInputError(
                f"{_SUMMARY_NAME}: a calculation-evidence row holds {len(cells)} cells, "
                "not 6"
            )
        label = cells[0]
        if label in rows:
            raise ControlInputError(
                f"{_SUMMARY_NAME}: calculation {label!r} appears twice in the table"
            )
        if not re.fullmatch(r"[0-9a-f]{64}", cells[5]):
            raise ControlInputError(
                f"{_SUMMARY_NAME}: calculation {label!r} carries an entry digest that is "
                "not a lowercase SHA-256"
            )
        rows[label] = (cells[1], cells[2], cells[3], cells[4], cells[5])
    if not rows:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the calculation-evidence table has a header and no rows"
        )
    return required, rows


def _verify_calculation_evidence(
    block: object,
    summary_text: str,
    json_hashes: dict[str, object],
) -> None:
    """Check the evidence block's shape, and that the summary says the same.

    Every other member a reviewer acts on is witnessed by a second artefact.
    This one carried the figures and the relied-on flag in the JSON alone, so
    editing a figure there left the pack verifying and the review sheet
    displaying the edited number.
    """
    if block is None:
        if _CALCULATION_EVIDENCE_TABLE_HEADER in {
            line.strip() for line in summary_text.splitlines()
        }:
            raise ControlInputError(
                f"{_SUMMARY_NAME}: a calculation-evidence table is present while "
                f"{_JSON_NAME} carries no calculation_evidence member"
            )
        # The witnessed source list is the one thing that survives removing
        # the block and its section together. A pack that read an evidence
        # file and no longer says what it found is half removed, not absent.
        orphaned = sorted(
            key for key in json_hashes if str(key).startswith("calculation_evidence:")
        )
        if orphaned:
            raise ControlInputError(
                f"{_JSON_NAME}: source_sha256 records calculation evidence "
                f"({', '.join(orphaned)}) while the pack carries no calculation_evidence "
                "member; the evidence is half removed, not absent"
            )
        return
    if not isinstance(block, dict):
        raise ControlInputError(f"{_JSON_NAME}: calculation_evidence must be an object")
    unknown = sorted(set(block) - _EVIDENCE_BLOCK_MEMBERS)
    if unknown:
        raise ControlInputError(
            f"{_JSON_NAME}: calculation_evidence has unknown member(s): {', '.join(unknown)}"
        )
    missing = sorted(_EVIDENCE_BLOCK_MEMBERS - set(block))
    if missing:
        raise ControlInputError(
            f"{_JSON_NAME}: calculation_evidence is missing member(s): {', '.join(missing)}"
        )
    supplied = block["supplied"]
    if not isinstance(supplied, list):
        raise ControlInputError(f"{_JSON_NAME}: calculation_evidence.supplied must be a list")
    required = block["required"]
    if not isinstance(required, list) or not all(isinstance(name, str) for name in required):
        raise ControlInputError(
            f"{_JSON_NAME}: calculation_evidence.required must be a list of strings"
        )

    if block["effect"] != _CALCULATION_EVIDENCE_EFFECT:
        raise ControlInputError(
            f"{_JSON_NAME}: calculation_evidence.effect is not the text the writer emits"
        )
    summary_required, summary_rows = _evidence_summary_rows(summary_text)
    if list(required) != summary_required:
        raise ControlInputError(
            f"calculation evidence disagrees: {_JSON_NAME} requires {required!r}, "
            f"{_SUMMARY_NAME} states {summary_required!r}"
        )
    json_rows: dict[str, tuple[str, str, str, str, str]] = {}
    for item in supplied:
        if not isinstance(item, dict):
            raise ControlInputError(
                f"{_JSON_NAME}: calculation_evidence.supplied holds a non-object entry"
            )
        item_unknown = sorted(set(item) - _EVIDENCE_ITEM_MEMBERS)
        item_missing = sorted(_EVIDENCE_ITEM_MEMBERS - set(item))
        if item_unknown or item_missing:
            raise ControlInputError(
                f"{_JSON_NAME}: a calculation_evidence entry has the wrong members "
                f"(unknown: {', '.join(item_unknown) or 'none'}; "
                f"missing: {', '.join(item_missing) or 'none'})"
            )
        label = item["label"]
        values = item["values"]
        usable = item["usable"]
        if isinstance(label, str) and _md_cell_mirror(label) in json_rows:
            # A dict keyed by label kept the last of two entries and the
            # summary witnessed only that one, while the sheet rendered both.
            raise ControlInputError(
                f"{_JSON_NAME}: calculation_evidence.supplied lists {label!r} twice"
            )
        if not isinstance(label, str) or not isinstance(usable, bool):
            raise ControlInputError(
                f"{_JSON_NAME}: a calculation_evidence entry has a non-string label or a "
                "non-boolean usable flag"
            )
        if not isinstance(values, dict) or not all(
            isinstance(name, str) and isinstance(amount, str) for name, amount in values.items()
        ):
            raise ControlInputError(
                f"{_JSON_NAME}: calculation_evidence[{label!r}].values must map names to "
                "decimal strings"
            )
        # The digest the pack quotes for this file has to be the one the
        # witnessed source list carries, or the provenance names another file.
        witnessed = json_hashes.get(f"calculation_evidence:{label}")
        if witnessed != item["file_sha256"]:
            raise ControlInputError(
                f"{_JSON_NAME}: calculation_evidence[{label!r}] quotes file digest "
                f"{item['file_sha256']!r}, which is not the digest source_sha256 records"
            )
        figures = "; ".join(f"{name} {amount}" for name, amount in sorted(values.items()))
        # Mirrored through the same escaping the writer applies, so the
        # comparison is against what the summary renders rather than against
        # what the JSON happens to spell.
        json_rows[_md_cell_mirror(label)] = (
            _md_cell_mirror(str(item["status"])),
            _md_cell_mirror(str(item["period"]) or _ABSENT),
            _md_cell_mirror(figures or _ABSENT),
            "yes" if usable else "no",
            # Every member, printed or not, is inside this digest.
            _entry_digest_mirror(item),
        )

    if not supplied and summary_rows:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: holds a calculation-evidence table while {_JSON_NAME} "
            "records no supplied evidence"
        )
    # The writer records one source digest per evidence file it read, keyed
    # by that file's label, and one supplied entry per file. The two sets are
    # the same set or the pack was edited: an entry deleted while its digest
    # stayed, or a digest added for a file no entry describes.
    witnessed_labels = sorted(
        str(key)[len("calculation_evidence:"):]
        for key in json_hashes if str(key).startswith("calculation_evidence:")
    )
    supplied_labels = sorted(str(item["label"]) for item in supplied)
    if witnessed_labels != supplied_labels:
        raise ControlInputError(
            f"{_JSON_NAME}: source_sha256 witnesses calculation evidence for "
            f"{witnessed_labels!r} while calculation_evidence.supplied describes "
            f"{supplied_labels!r}; the evidence is half removed, not absent"
        )
    if json_rows != summary_rows:
        raise ControlInputError(
            f"calculation evidence disagrees: {_JSON_NAME} and {_SUMMARY_NAME} state "
            "different calculations, figures or relied-on flags"
        )


def _verify_summary_holds_no_register(summary_text: str) -> None:
    """A pack without the register must not have a summary that shows one.

    Deleting client-queries.csv and the JSON member from a current pack leaves
    a summary still carrying the heading, the count and the table. Reading that
    as an older pack would display a sheet saying the register never existed
    beside a file that lists it, so the absence has to hold across all 3
    artefacts or the pack is refused.

    The heading is covered by _verify_summary_headings, which the caller runs
    with register=False for such a pack: an older summary's headings are the
    writer's 5, and a client-query heading in any syntax is a sixth. What is
    left here is the table and the count line, matched on a whole line, because
    a genuinely old pack whose account or reviewer comment quotes one of those
    phrases is still an old pack and refusing it would take archived evidence
    out of a firm's hands.
    """
    lines = summary_text.splitlines()
    if _CLIENT_QUERY_TABLE_HEADER in {line.strip() for line in lines}:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: holds a client-query table while the pack "
            "carries no client-query register; the register is half removed, "
            "not absent"
        )
    if _QUERY_COUNT_LINE.search(summary_text):
        raise ControlInputError(
            f"{_SUMMARY_NAME}: states a client-query count while the pack carries "
            "no client-query register; the register is half removed, not absent"
        )


def _verify_cross_file_agreement(
    document: dict[str, object],
    summary_text: str,
    csv_rows: list[dict[str, str]],
    query_rows: list[dict[str, str]] | None,
) -> None:
    # Before anything is read out of the summary, prove its sections are the
    # writer's. Every later check reads one region of this document; this is
    # what says there is no other region pretending to be one.
    evidence_block = document.get("calculation_evidence")
    _verify_summary_headings(
        summary_text,
        register=query_rows is not None,
        evidence=evidence_block is not None,
    )

    status = document["overall_status"]
    assert isinstance(status, str)

    status_lines = _STATUS_LINE.findall(summary_text)
    if len(status_lines) != 1:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: expected exactly one overall-status line, "
            f"found {len(status_lines)}"
        )
    if status_lines[0] != status:
        raise ControlInputError(
            f"overall status disagrees: {_JSON_NAME} says {status}, "
            f"{_SUMMARY_NAME} says {status_lines[0]}"
        )

    if _BOUNDARY_SENTENCE not in summary_text:
        raise ControlInputError(
            f"{_SUMMARY_NAME}: the review-boundary statement is missing or altered"
        )

    summary_hashes = _summary_source_evidence(summary_text)
    json_hashes = document["source_sha256"]
    assert isinstance(json_hashes, dict)
    if summary_hashes != json_hashes:
        raise ControlInputError(
            f"source evidence disagrees: {_SUMMARY_NAME} and {_JSON_NAME} "
            f"list different source digests"
        )

    _verify_calculation_evidence(evidence_block, summary_text, json_hashes)

    exceptions = document["exceptions"]
    assert isinstance(exceptions, list)

    _verify_rows_match(
        csv_name=_CSV_NAME,
        member="exceptions",
        noun="exception",
        json_items=exceptions,
        csv_rows=csv_rows,
        fields=_CSV_FIELDS,
        guarded=_CSV_GUARDED_FIELDS,
    )

    if query_rows is None:
        _verify_summary_holds_no_register(summary_text)
    else:
        client_queries = document["client_queries"]
        assert isinstance(client_queries, list)
        _verify_rows_match(
            csv_name=_QUERY_CSV_NAME,
            member="client_queries",
            noun="client query",
            json_items=client_queries,
            csv_rows=query_rows,
            fields=_QUERY_CSV_FIELDS,
            guarded=_QUERY_CSV_GUARDED_FIELDS,
        )
        _verify_summary_states_the_register(summary_text, client_queries)

    # The 3 sections left: the numbers a reviewer acts on, the exception table
    # the sheet displays, and the name and date against the acknowledgement.
    # Each is rebuilt from the JSON, so a value edited in the sheet alone is a
    # refusal rather than a pack that still reports all 4 artefacts verified.
    # These run last so the member-by-member checks above keep naming the one
    # file and field that disagree before a whole-section diff is reported.
    _verify_rebuilt_section(
        summary_text,
        _SCOPE_HEADING,
        _expected_scope_lines(document, register=query_rows is not None),
        trailing_blank=True,
    )
    _verify_rebuilt_section(
        summary_text,
        _EXCEPTIONS_HEADING,
        _expected_exception_lines(exceptions),
        trailing_blank=True,
    )
    _verify_rebuilt_section(
        summary_text,
        _ACKNOWLEDGEMENT_HEADING,
        _expected_acknowledgement_lines(document["acknowledgement"]),
        trailing_blank=False,
    )


def _read_csv_rows(
    payload: bytes, name: str, fields: tuple[str, ...]
) -> list[dict[str, str]]:
    """Read a pack CSV, requiring every row to hold exactly the declared cells.

    csv.DictReader would collect a surplus cell under a key of None and pad a
    short row with a default, and the field-by-field comparison downstream
    reads only the declared columns, so neither would ever be looked at. The
    surplus cell is the one that matters: the writer's formula guard covers
    named fields only, so an appended ``=...`` cell would pass verification and
    be live the moment a reviewer opened the file in a spreadsheet. Counting
    cells here closes that for both pack CSVs. A blank line is refused for the
    same reason, DictReader having skipped it in silence: the writer emits
    none, so one means the file was edited after it was written.
    """
    try:
        text = payload.decode("utf-8-sig")
    except UnicodeDecodeError as exc:
        raise ControlInputError(f"{name}: not valid UTF-8") from exc
    reader = csv.reader(io.StringIO(text, newline=""))
    header = next(reader, None)
    if header != list(fields):
        raise ControlInputError(
            f"{name}: header row does not match the written contract"
        )
    rows: list[dict[str, str]] = []
    for number, cells in enumerate(reader, start=1):
        if len(cells) != len(fields):
            raise ControlInputError(
                f"{name}: row {number} holds {len(cells)} cells, but the header "
                f"declares {len(fields)}"
            )
        rows.append(dict(zip(fields, cells)))
    return rows


def verify_pack(pack_dir: Path) -> tuple[
    dict[str, object],
    str,
    list[dict[str, str]],
    list[dict[str, str]] | None,
    dict[str, str],
]:
    """Verify one artefact set end to end and return its parsed contents.

    The returned mapping also carries the SHA-256 of each artefact's exact
    bytes under ``artefact_sha256``, so a displayed sheet can state what it
    actually read. The query rows are None for a pack written before the
    client-query register existed; such a pack still verifies and displays,
    because it is archived evidence that predates the register rather than a
    pack missing part of itself.
    """
    payloads = _load_artefact_bytes(pack_dir)
    document = _parse_json(payloads[_JSON_NAME])
    _verify_json_schema(document)

    # Both halves of the register or neither. A pack holding one without the
    # other was assembled from 2 runs, or edited, and is not evidence of
    # either format.
    has_file = _QUERY_CSV_NAME in payloads
    has_member = "client_queries" in document
    if has_file != has_member:
        present, absent = (
            (_QUERY_CSV_NAME, f"{_JSON_NAME} member client_queries")
            if has_file
            else (f"{_JSON_NAME} member client_queries", _QUERY_CSV_NAME)
        )
        raise ControlInputError(
            f"client-query register is half present: {present} exists but "
            f"{absent} does not"
        )

    try:
        summary_text = payloads[_SUMMARY_NAME].decode("utf-8")
    except UnicodeDecodeError as exc:
        raise ControlInputError(f"{_SUMMARY_NAME}: not valid UTF-8") from exc
    csv_rows = _read_csv_rows(payloads[_CSV_NAME], _CSV_NAME, _CSV_FIELDS)
    query_rows = (
        _read_csv_rows(payloads[_QUERY_CSV_NAME], _QUERY_CSV_NAME, _QUERY_CSV_FIELDS)
        if has_file
        else None
    )
    _verify_cross_file_agreement(document, summary_text, csv_rows, query_rows)
    artefact_digests = {
        name: hashlib.sha256(payload).hexdigest() for name, payload in payloads.items()
    }
    return document, summary_text, csv_rows, query_rows, artefact_digests


def render_review_sheet(pack_dir: Path) -> tuple[str, dict[str, str]]:
    """Verify a pack and render it as a plain-text review sheet.

    Returns the sheet and the per-artefact digests it displays. Raises
    ``ControlInputError`` instead of rendering whenever any artefact is
    missing, malformed or inconsistent with its siblings.
    """
    document, summary_text, csv_rows, query_rows, artefact_digests = verify_pack(pack_dir)
    thresholds = document["thresholds"]
    assert isinstance(thresholds, dict)
    exceptions = document["exceptions"]
    assert isinstance(exceptions, list)
    acknowledgement = document["acknowledgement"]

    blocked = sum(1 for item in exceptions if item["status"] == "BLOCKED")
    review = sum(1 for item in exceptions if item["status"] == "REVIEW")

    lines = [
        "Close Review Sheet",
        "",
        f"Overall status: {document['overall_status']}",
        "",
        "Review states only. This sheet does not approve a close, post a journal, make a payment, lodge a return, or lock a period.",
        "",
        "Scope",
        "",
    ]
    current_dates = document["current_report_dates"]
    assert isinstance(current_dates, list)
    prior_dates = document["prior_report_dates"]
    assert isinstance(prior_dates, list)
    lines.append(f"- Current report date(s): {', '.join(current_dates) or 'n/a'}")
    lines.append(f"- Prior report date(s): {', '.join(prior_dates) or 'n/a'}")
    lines.append(f"- Material variance thresholds: {thresholds['absolute_variance']} and {thresholds['percentage_variance']}")
    lines.append(f"- Reconciliation tolerance: {thresholds['reconciliation_tolerance']}")
    lines.append(
        f"- Exceptions: {len(exceptions)} total; {blocked} blocked; {review} requiring review."
    )
    queries_in_scope = document.get("client_queries")
    if queries_in_scope is not None:
        assert isinstance(queries_in_scope, list)
        lines.append(f"- Client queries drafted: {len(queries_in_scope)}.")
    # The pack records which controls had no input and _expected_scope_lines
    # verifies the summary's line, but the displayed sheet rebuilds this section
    # itself: without this, `view` showed a status and no sign that a control had
    # been skipped, which is the one thing the record exists to surface.
    if "controls_not_run" in document:
        skipped = _require_string_list(document, "controls_not_run")
        lines.append(f"- Controls not run: {', '.join(skipped) or 'none'}.")
    lines += ["", "Source evidence", ""]
    source_hashes = document["source_sha256"]
    assert isinstance(source_hashes, dict)
    for label, digest in sorted(source_hashes.items()):
        lines.append(f"- {label}: {digest}")
    evidence_block = document.get("calculation_evidence")
    if evidence_block is not None:
        assert isinstance(evidence_block, dict)
        lines += ["", "Calculation evidence", ""]
        required_names = evidence_block["required"]
        assert isinstance(required_names, list)
        lines.append(f"- Required: {', '.join(required_names) or 'none'}")
        supplied_items = evidence_block["supplied"]
        assert isinstance(supplied_items, list)
        if not supplied_items:
            lines.append("- Supplied: none. The exceptions say which calculation is missing.")
        for entry in supplied_items:
            assert isinstance(entry, dict)
            values = entry["values"]
            assert isinstance(values, dict)
            figures = "; ".join(f"{name} {amount}" for name, amount in sorted(values.items()))
            lines.append(
                f"- {entry['label']}: {entry['status']}, period {entry['period'] or 'n/a'}, "
                f"{figures or 'no figure'}, relied on: {'yes' if entry['usable'] else 'no'}"
            )
        lines.append(
            "- A figure here supports review. It approves nothing, and the "
            "calculation-evidence exceptions say why one is not relied on."
        )
    lines += ["", "Exceptions", ""]
    if not exceptions:
        lines.append("No exceptions were raised. A human must still decide whether the close is appropriate.")
    else:
        width = max(len(str(index + 1)) for index in range(len(exceptions)))
        for index, item in enumerate(exceptions):
            account = " / ".join(
                piece
                for piece in (item.get("account_code"), item.get("account_name"))
                if piece
            ) or item.get("account_id") or "n/a"
            lines.append(
                f"[{str(index + 1).rjust(width)}] {item['status']} {item['control']}"
                f" | {item['tenant'] or 'n/a'} | {account}"
                f" | difference {item['difference'] or 'n/a'}"
            )
            lines.append(f"     reason: {item['reason']}")
            lines.append(f"     action: {item['reviewer_action']}")
    lines += ["", "Client queries", ""]
    lines.append(
        "Draft questions for the preparer. Nothing has been sent. Edit them before "
        "they reach a client and record answers in the firm's own tracker."
    )
    lines.append("")
    client_queries = document.get("client_queries")
    predates_the_register = client_queries is None
    if predates_the_register:
        lines.append(
            "This pack was written before the client-query register existed, so it "
            "carries none. Its exceptions are unchanged."
        )
        client_queries = []
    assert isinstance(client_queries, list)
    if not client_queries:
        # Only a run that derived the register can report an empty one. An
        # archived pack carries no register at all, and its exceptions may well
        # include ones this version would turn into questions, so saying none
        # arose would be a claim nothing in the pack supports.
        if not predates_the_register:
            lines.append(
                "No exception raised a question for the client."
            )
    else:
        width = max(len(str(index + 1)) for index in range(len(client_queries)))
        for index, query in enumerate(client_queries):
            account = " / ".join(
                piece
                for piece in (query.get("account_code"), query.get("account_name"))
                if piece
            ) or query.get("account_id") or "n/a"
            lines.append(
                f"[{str(index + 1).rjust(width)}] {query['query_id']} {query['control']}"
                f" | {query['tenant'] or 'n/a'} | {account}"
                f" | difference {query['difference'] or 'n/a'}"
            )
            lines.append(f"     ask: {query['question']}")
            lines.append(f"     evidence: {query['evidence_requested']}")
    lines += ["", "Human acknowledgement", ""]
    if acknowledgement is None:
        lines.append("No reviewer acknowledgement was supplied. This does not create or imply an approval.")
    else:
        assert isinstance(acknowledgement, dict)
        lines.append(f"- Reviewer initials: {acknowledgement['reviewer_initials']}")
        lines.append(f"- Reviewed on: {acknowledgement['reviewed_on']}")
        comment = acknowledgement["comment"]
        if comment:
            lines.append(f"- Comment: {comment}")
        lines.append(
            "- Effect: acknowledgement records a human action only; it does not change the control status or approve a close."
        )
    lines += ["", "Artefacts verified", ""]
    for name in PACK_FILE_NAMES:
        if name in artefact_digests:
            lines.append(f"- {name}: sha256 {artefact_digests[name]}")
    lines.append("")
    return "\n".join(lines), artefact_digests
