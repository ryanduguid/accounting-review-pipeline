"""Optional ingestion of calculation-evidence files. Offline, always.

A calculation-evidence file records what a calculator was asked, what it
answered, and what produced the answer. Something outside this pipeline made
the call; this module reads the file from disk and decides whether the close
review may rely on it. Nothing here opens a socket, and nothing here calls a
calculator.

The control is opt-in twice over. Without `--calculation-evidence` nothing runs
and a pack is byte-identical to one produced before this file existed. With it,
every supplied file is validated, and a calculation named by
`--require-calculation` that is absent is an exception rather than a silent
omission: the failure this exists to prevent is a pack that passes while the
calculation it was meant to carry never happened.

An evidence file is untrusted input. It may have been produced anywhere, it
carries text from a third-party service, and it reaches a reviewer's screen. It
is parsed defensively, its paths are constrained, and every string that ends up
in the pack goes through the same escaping as any other untrusted cell.
"""

from __future__ import annotations

import hashlib
import json
import re
import unicodedata
from dataclasses import dataclass
from datetime import date
from decimal import Decimal, InvalidOperation
from pathlib import Path

from .errors import ControlInputError, SchemaError
from .loader import SourceSnapshot

#: Evidence schemas this consumer knows how to read. A file naming anything
#: else is refused: an unknown schema is not a shape to guess at.
SUPPORTED_SCHEMAS = ("lodgeit-calculation-evidence/1",)

#: The statuses that mean a figure was produced. Everything else is a refusal,
#: an outage or a contract failure, and none of those is a calculation.
COMPUTED_STATUSES = ("COMPUTED",)

MAX_EVIDENCE_BYTES = 2 * 1024 * 1024
_MONEY = ("0123456789.-")


@dataclass(frozen=True)
class CalculationEvidence:
    """One validated evidence file, and what it is allowed to support."""

    label: str
    path: Path
    sha256: str
    schema: str
    provider: str
    calculator: str
    period: str
    status: str
    engine: str
    calculation_sha256: str
    values: dict[str, Decimal]
    rate_tables: tuple[str, ...]
    advisory_notes: tuple[str, ...]
    synthetic_input: bool
    findings: tuple[str, ...]

    @property
    def usable(self) -> bool:
        """True when the file itself holds together and carries a computed figure.

        This is the file's own condition and nothing else. Whether the figure
        covers the period under review is a question about the close, not
        about the file, and `engine.CloseReviewPack.relied_on` asks both
        before anything is relied on. The pack's `usable` key is written from
        `relied_on` for that reason; read that, not this, when the question is
        whether a reviewer may use the figure.
        """
        return not self.findings and self.status in COMPUTED_STATUSES


def _is_hidden(character: str) -> bool:
    """True for a character that can change how surrounding text reads.

    Every control (Cc), format (Cf) and surrogate (Cs) character, the set the
    loader refuses in source files: the zero-width, directional, embedding,
    override and isolate marks, and also the soft hyphen, word joiner, byte
    order mark, interlinear annotation marks and C1 controls that a list of
    ranges here used to miss, so they reached the pack unescaped.
    """
    if ord(character) in (0x2028, 0x2029):
        # LINE SEPARATOR and PARAGRAPH SEPARATOR, like NEL among the controls:
        # str.splitlines() breaks a line at each, so a status or figure name
        # carrying one is written into a summary row as one line and read back
        # as two. The writer's own pack then fails to verify.
        return True
    return unicodedata.category(character) in {"Cc", "Cf", "Cs"}


def _text(value: object, field: str, path: Path, *, limit: int = 400) -> str:
    if not isinstance(value, str):
        raise SchemaError(f"{path}: {field} must be a string.")
    if len(value) > limit:
        raise SchemaError(f"{path}: {field} is longer than {limit} characters.")
    if any(_is_hidden(character) for character in value):
        raise SchemaError(f"{path}: {field} contains a control or formatting character.")
    return value


#: A label names a required calculation, keys a source digest in the pack and
#: is rendered into the summary, so it is a slug and nothing else. The viewer's
#: source-evidence pattern accepts exactly this class.
_LABEL = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*")


def _label(value: object, path: Path) -> str:
    text = _text(value, "label", path, limit=120)
    if not _LABEL.fullmatch(text):
        raise SchemaError(
            f"{path}: label {text!r} is not a slug. A label names a required calculation and "
            "keys a digest in the pack, so it is lower-case letters, digits and single hyphens."
        )
    return text


def _canonical(payload: object) -> bytes:
    return json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False,
                      allow_nan=False).encode("utf-8")


def _decimal(value: object, field: str, path: Path) -> Decimal:
    """Read money the way this package reads all money: exactly, or not at all."""
    if not isinstance(value, str):
        raise SchemaError(
            f"{path}: {field} is {type(value).__name__}, not a decimal string. A JSON number "
            "has already lost whatever the calculator meant by it."
        )
    if not value or any(character not in _MONEY for character in value):
        raise SchemaError(f"{path}: {field} is not a decimal string.")
    try:
        amount = Decimal(value)
    except InvalidOperation as exc:
        raise SchemaError(f"{path}: {field} is not a decimal.") from exc
    if not amount.is_finite():
        raise SchemaError(f"{path}: {field} is not finite.")
    return amount


def _block(record: dict, name: str) -> dict:
    """One nested object, or an empty one. A missing block is a shape, not a crash."""
    value = record.get(name)
    return value if isinstance(value, dict) else {}


def _array(value: object, field: str, path: Path) -> list:
    """One array, or an empty one. A scalar is an error, never an iteration.

    `for entry in block.get(name, []) or []` reads an absent member safely and
    raises TypeError on a truthy scalar such as `"notes": 1`. Nothing above
    catches that, so it left this command as a traceback rather than as the
    unreadable-evidence state the control already has.
    """
    if value is None:
        return []
    if not isinstance(value, list):
        raise SchemaError(f"{path}: {field} is {type(value).__name__}, not an array.")
    return value


def load(path: Path | SourceSnapshot) -> CalculationEvidence:
    """Read and validate one evidence file. Raises on anything unreadable."""
    snapshot = path if isinstance(path, SourceSnapshot) else SourceSnapshot.capture(
        path, label="Calculation-evidence file",
    )
    source = snapshot.path
    if len(snapshot.content) > MAX_EVIDENCE_BYTES:
        raise SchemaError(f"{source}: evidence file exceeds {MAX_EVIDENCE_BYTES} bytes.")
    try:
        record = json.loads(snapshot.text(label="Calculation-evidence file", encoding="utf-8-sig"))
    except json.JSONDecodeError as exc:
        raise SchemaError(f"{source}: evidence is not valid JSON.") from exc
    except RecursionError as exc:
        # A file well under the byte ceiling can still nest thousands of arrays
        # deep. That is an unreadable evidence file, which this control already
        # has a state for, not a crash for the caller to see.
        raise SchemaError(f"{source}: evidence is nested too deeply to read.") from exc
    except ValueError as exc:
        # Covers the integer-digit limit, among others. Anything json raises
        # that is not a decode error still means the file could not be read.
        raise SchemaError(f"{source}: evidence could not be read ({exc}).") from exc
    if not isinstance(record, dict):
        raise SchemaError(f"{source}: evidence must be a JSON object.")

    schema = _text(record.get("schema"), "schema", source, limit=120)
    if schema not in SUPPORTED_SCHEMAS:
        raise SchemaError(
            f"{source}: evidence schema {schema!r} is not one this pipeline reads "
            f"({', '.join(SUPPORTED_SCHEMAS)}). An unknown schema is refused rather than guessed at."
        )
    calculation = record.get("calculation")
    if not isinstance(calculation, dict):
        raise SchemaError(f"{source}: evidence carries no calculation block.")

    recorded_digest = _text(record.get("calculation_sha256"), "calculation_sha256", source, limit=64)
    try:
        actual_digest = hashlib.sha256(_canonical(calculation)).hexdigest()
    except (TypeError, ValueError) as exc:
        raise SchemaError(f"{source}: the calculation block cannot be canonicalised.") from exc

    findings: list[str] = []
    if recorded_digest != actual_digest:
        findings.append(
            f"calculation_sha256 {recorded_digest} does not match the calculation block "
            f"({actual_digest}). The file has been changed since it was produced."
        )

    call = _block(calculation, "call")
    provider = _block(calculation, "provider")
    upstream = _block(calculation, "upstream")
    engine = _block(calculation, "engine")
    normalised = _block(calculation, "normalised")

    label = _label(calculation.get("label", source.stem), source)
    status = _text(call.get("status", "UNKNOWN"), "call.status", source, limit=60)
    calculator = _text(call.get("calculator") or "", "call.calculator", source, limit=200)
    period = _text(call.get("period") or "", "call.period", source, limit=200)
    provider_name = _text(provider.get("name") or "", "provider.name", source, limit=120)
    engine_name = _text(
        engine.get("name") or provider.get("contract_snapshot") or "", "engine", source, limit=200,
    )

    manifest = upstream.get("manifest")
    advisory = upstream.get("advisory")
    if status in COMPUTED_STATUSES:
        if not isinstance(manifest, dict) or not manifest:
            findings.append(
                "the evidence records a computed figure with no manifest, so nothing names the "
                "rate tables it consumed"
            )
        if not isinstance(advisory, dict) or not advisory.get("notes"):
            findings.append(
                "the evidence records a computed figure with no advisory, so the calculator's "
                "own boundary statement is missing"
            )
    # An entry of the wrong type is a finding, not something to step over.
    # Dropping it silently let a manifest of one malformed row look like a
    # manifest naming nothing, and an advisory of [123] satisfy the presence
    # check above while producing no boundary statement at all.
    rate_tables: list[str] = []
    if isinstance(manifest, dict):
        entries = _array(manifest.get("rate_table_uris"), "manifest.rate_table_uris", source)
        for index, entry in enumerate(entries):
            where = f"manifest.rate_table_uris[{index}]"
            if not isinstance(entry, dict):
                findings.append(
                    f"the manifest entry at {where} is {type(entry).__name__}, not an object, "
                    "so nothing there names a rate table"
                )
                continue
            uri = entry.get("uri")
            if not isinstance(uri, str):
                findings.append(
                    f"the manifest entry at {where} carries no uri string, so nothing there "
                    "names a rate table"
                )
                continue
            rate_tables.append(_text(uri, f"{where}.uri", source))
    notes: list[str] = []
    if isinstance(advisory, dict):
        entries = _array(advisory.get("notes"), "advisory.notes", source)
        for index, note in enumerate(entries):
            if not isinstance(note, str):
                findings.append(
                    f"the advisory note at advisory.notes[{index}] is "
                    f"{type(note).__name__}, not text, so the calculator's boundary statement "
                    "is not readable"
                )
                continue
            notes.append(_text(note, f"advisory.notes[{index}]", source, limit=600))

    values: dict[str, Decimal] = {}
    raw_values = _block(normalised, "values")
    for name, amount in sorted(raw_values.items()):
        key = _text(name, "normalised.values key", source, limit=80)
        values[key] = _decimal(amount, f"normalised.values.{key}", source)

    if status in COMPUTED_STATUSES and not values:
        findings.append(
            "the evidence records a computed figure and carries no normalised value, so there "
            "is no figure in it. A label alone does not satisfy a required calculation."
        )

    validation = _block(calculation, "validation")
    if validation:
        entries = _array(validation.get("findings"), "validation.findings", source)
        for index, finding in enumerate(entries):
            where = f"validation.findings[{index}]"
            if not isinstance(finding, str):
                findings.append(
                    f"the producer recorded a validation finding at {where} that is "
                    f"{type(finding).__name__}, not text, so what it found cannot be read"
                )
                continue
            findings.append(
                "the producer recorded a validation finding: "
                + _text(finding, where, source, limit=600)
            )

    return CalculationEvidence(
        label=label,
        path=source,
        sha256=snapshot.sha256,
        schema=schema,
        provider=provider_name,
        calculator=calculator,
        period=period,
        status=status,
        engine=engine_name,
        calculation_sha256=recorded_digest,
        values=values,
        rate_tables=tuple(rate_tables),
        advisory_notes=tuple(notes),
        synthetic_input=bool(calculation.get("synthetic_input")),
        findings=tuple(findings),
    )


def covers_period(evidence: CalculationEvidence, report_date: date) -> bool | None:
    """Does the evidence period cover the pack's report date?

    Returns None when the period cannot be read as a date range, which is a
    finding rather than a pass: an unreadable period is not a matching one.

    Two shapes are understood, because they are the two the producers emit: a
    reporting month `...:YYYY-MM` and an Australian income or FBT year
    `...:fyYYYY`, which runs to 30 June or to 31 March respectively. Anything
    else is unreadable here on purpose.
    """
    tail = evidence.period.rsplit(":", 1)[-1] if evidence.period else ""
    if len(tail) == 7 and tail[4] == "-" and tail[:4].isdigit() and tail[5:].isdigit():
        return tail == report_date.strftime("%Y-%m")
    if len(tail) == 6 and tail.startswith("fy") and tail[2:].isdigit():
        ending = int(tail[2:])
        try:
            if "fbt" in evidence.period:
                start, end = date(ending - 1, 4, 1), date(ending, 3, 31)
            else:
                start, end = date(ending - 1, 7, 1), date(ending, 6, 30)
        except ValueError:
            # `fy0000` and the like. A year the calendar has no room for is a
            # period this pipeline cannot read, which is the same answer as any
            # other unreadable period rather than an error out of the engine.
            return None
        return start <= report_date <= end
    return None


def load_all(paths: list[Path] | None) -> tuple[list[CalculationEvidence], list[str]]:
    """Load every supplied evidence file, keeping unreadable ones as findings.

    A file that cannot be read at all is not dropped: it comes back as a
    message the engine turns into an exception, because a pack that quietly
    ignored an unreadable evidence file would be worse than one that never had
    it.
    """
    if not paths:
        return [], []
    loaded: list[CalculationEvidence] = []
    unreadable: list[str] = []
    seen: set[str] = set()
    for path in paths:
        try:
            evidence = load(path)
        except ControlInputError as exc:
            unreadable.append(str(exc))
            continue
        if evidence.label in seen:
            unreadable.append(
                f"{path}: more than one evidence file carries the label {evidence.label!r}; "
                "a required calculation cannot be matched to two files."
            )
            continue
        seen.add(evidence.label)
        loaded.append(evidence)
    return loaded, unreadable
