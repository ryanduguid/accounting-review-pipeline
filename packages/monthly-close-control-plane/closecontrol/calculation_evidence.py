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
        """True when this file may support a close-review conclusion."""
        return not self.findings and self.status in COMPUTED_STATUSES


def _text(value: object, field: str, path: Path, *, limit: int = 400) -> str:
    if not isinstance(value, str):
        raise SchemaError(f"{path}: {field} must be a string.")
    if len(value) > limit:
        raise SchemaError(f"{path}: {field} is longer than {limit} characters.")
    # The same character classes the CSV loaders refuse. A Cf character in a
    # provider's advisory would reorder a reviewer's own words on screen.
    if any(ord(character) < 0x20 or 0x200B <= ord(character) <= 0x200F
           or 0x202A <= ord(character) <= 0x202E for character in value):
        raise SchemaError(f"{path}: {field} contains a control or formatting character.")
    return value


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

    label = _text(calculation.get("label", source.stem), "label", source, limit=120)
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
    rate_tables: list[str] = []
    if isinstance(manifest, dict):
        for entry in manifest.get("rate_table_uris", []) or []:
            if isinstance(entry, dict) and isinstance(entry.get("uri"), str):
                rate_tables.append(_text(entry["uri"], "manifest.rate_table_uris[].uri", source))
    notes: list[str] = []
    if isinstance(advisory, dict):
        for note in advisory.get("notes", []) or []:
            if isinstance(note, str):
                notes.append(_text(note, "advisory.notes[]", source, limit=600))

    values: dict[str, Decimal] = {}
    raw_values = _block(normalised, "values")
    for name, amount in sorted(raw_values.items()):
        key = _text(name, "normalised.values key", source, limit=80)
        values[key] = _decimal(amount, f"normalised.values.{key}", source)

    validation = _block(calculation, "validation")
    if validation:
        for finding in validation.get("findings", []) or []:
            if isinstance(finding, str):
                findings.append(
                    "the producer recorded a validation finding: "
                    + _text(finding, "validation.findings[]", source, limit=600)
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
        if "fbt" in evidence.period:
            start, end = date(ending - 1, 4, 1), date(ending, 3, 31)
        else:
            start, end = date(ending - 1, 7, 1), date(ending, 6, 30)
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
