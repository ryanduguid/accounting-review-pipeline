"""Local, synthetic document intake. Transcription is not a tax decision."""

from __future__ import annotations

import hashlib
import json
import re
import unicodedata
from dataclasses import dataclass
from decimal import Decimal
from pathlib import Path
from typing import Any

from .errors import GateInputError
from .loader import SourceSnapshot, parse_iso_date
from .models import Finding, SourceEvidence

INTAKE_NAME = "document-intake.json"
REVIEW_NAME = "document-review.json"
TEXT_NAME = "extracted.txt"
FIELDS = {
    "supplier": "Supplier",
    "invoice_number": "Invoice number",
    "invoice_date": "Invoice date",
    "currency": "Currency",
    "total": "Total",
    "gst_observed": "GST",
}
_MONEY = re.compile(r"-?(?:0|[1-9][0-9]{0,17})(?:\.[0-9]{1,2})?")
BOUNDARY = (
    "Synthetic supporting evidence only. Review confirms transcription, not tax treatment, "
    "GST entitlement, ledger completeness or accounting approval."
)


def _string(value: Any, label: str, *, empty: bool = False) -> str:
    if not isinstance(value, str) or len(value) > 500 or (not empty and not value.strip()):
        raise GateInputError(f"{label} must be a {'possibly empty ' if empty else ''}string "
                             "of at most 500 characters.")
    if any(unicodedata.category(c) in {"Cc", "Cf", "Cs"} for c in value):
        raise GateInputError(f"{label} contains a control or formatting character.")
    return value


def _capture(path: Path, limit: int) -> SourceSnapshot:
    try:
        with path.open("rb") as source:
            content = source.read(limit + 1)
    except OSError as exc:
        raise GateInputError(f"Cannot read document input {path}: {exc}.") from exc
    if not content or len(content) > limit:
        raise GateInputError(f"{path.name} must contain between 1 and {limit} bytes.")
    return SourceSnapshot(path, content, hashlib.sha256(content).hexdigest())


def _member(root: Path, name: str, limit: int) -> SourceSnapshot:
    path = root / name
    try:
        if path.resolve().parent != root.resolve():
            raise GateInputError(f"{name} must stay inside its document bundle.")
    except (OSError, RuntimeError) as exc:
        raise GateInputError(f"Cannot resolve document input {name}.") from exc
    return _capture(path, limit)


def _object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise GateInputError(f"Duplicate document JSON member: {key!r}.")
        result[key] = value
    return result


def _json(snapshot: SourceSnapshot) -> dict[str, Any]:
    try:
        value = json.loads(snapshot.text(label="Document JSON", encoding="utf-8"),
                           object_pairs_hook=_object)
    except GateInputError:
        raise
    except (ValueError, RecursionError) as exc:
        raise GateInputError(f"{snapshot.path.name} is not valid document JSON.") from exc
    if not isinstance(value, dict):
        raise GateInputError(f"{snapshot.path.name} must hold a JSON object.")
    return value


def _encoded(value: object) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True, ensure_ascii=True) + "\n").encode()


def _value(field: str, raw: Any) -> str:
    raw = _string(raw, field).strip()
    if field == "invoice_date":
        return parse_iso_date(raw, field=field, path=Path(TEXT_NAME)).isoformat()
    if field == "currency" and raw != "AUD":
        raise GateInputError("This first document template supports explicit AUD only.")
    if field in {"total", "gst_observed"}:
        if not _MONEY.fullmatch(raw):
            raise GateInputError(f"{field} requires a decimal string with at most two places.")
        # No rounding, binary float or ambient Decimal arithmetic is involved.
        return format(Decimal(raw), ".2f")
    return raw


def _extract(text: str) -> dict[str, list[dict[str, Any]]]:
    fields: dict[str, list[dict[str, Any]]] = {key: [] for key in FIELDS}
    for page, content in enumerate(text.split("\f"), start=1):
        for line, content_line in enumerate(content.splitlines(), start=1):
            # pdf-inspector can render a labelled line as a Markdown heading.
            content_line = re.sub(r"^#{1,6} +", "", content_line)
            for field, label in FIELDS.items():
                if not content_line.startswith(label + ":"):
                    continue
                raw = content_line[len(label) + 1:].strip()
                value, error = None, ""
                try:
                    value = _value(field, raw)
                except GateInputError as exc:
                    error = str(exc)
                fields[field].append({"page": page, "line": line, "raw": raw,
                                      "value": value, "error": error})
    return fields


def _manifest(source: SourceSnapshot, text: SourceSnapshot, *, entity: str,
              period_end: str, text_origin: str, text_version: str) -> dict[str, Any]:
    _string(entity, "entity")
    parse_iso_date(period_end, field="period_end", path=Path(INTAKE_NAME))
    if source.path.suffix.lower() not in {".txt", ".pdf"}:
        raise GateInputError("Original documents must be .txt or .pdf files.")
    if text_origin not in {"manual", "pdf-inspector"}:
        raise GateInputError("text_origin must be manual or pdf-inspector.")
    _string(text_version, "text_version", empty=text_origin == "manual")
    if source.path.suffix.lower() == ".txt" and source.content != text.content:
        raise GateInputError("A text original and its extracted text must have identical bytes.")
    extracted = text.text(label="Extracted text", encoding="utf-8")
    return {
        "schema_version": "document-intake.v1", "mode": "synthetic",
        "entity_ref": entity, "period_end": period_end,
        "extractor": "labelled-invoice.v1", "text_origin": text_origin,
        "text_version": text_version,
        "source": {"filename": "original" + source.path.suffix.lower(), "sha256": source.sha256},
        "text": {"filename": TEXT_NAME, "sha256": text.sha256},
        "fields": _extract(extracted),
    }


def create_intake(*, source_path: Path, text_path: Path, output_dir: Path,
                  entity: str, period_end: str, text_origin: str, text_version: str = "") -> Path:
    # Reuse the pack writer's output policy without making engine/report imports cyclic.
    from .report import require_output_outside_repository

    output_dir = require_output_outside_repository(output_dir)
    if output_dir.exists():
        raise GateInputError("Document intake requires a new output directory; existing files stay intact.")
    source = _capture(source_path, 20 * 1024 * 1024)
    text = _capture(text_path, 1024 * 1024)
    manifest = _manifest(source, text, entity=entity, period_end=period_end,
                         text_origin=text_origin, text_version=text_version)
    encoded = _encoded(manifest)
    if len(encoded) > 1024 * 1024:
        raise GateInputError("Extracted observations exceed the 1 MiB intake limit.")
    review = {
        "schema_version": "document-review.v1",
        "intake_sha256": hashlib.sha256(encoded).hexdigest(),
        "reviewer_initials": "", "reviewed_on": "", "source_text_checked": False,
        "fields": {
            name: {"value": rows[0]["value"] if len(rows) == 1 else None,
                   "page": rows[0]["page"] if len(rows) == 1 else None, "note": ""}
            for name, rows in manifest["fields"].items()
        },
    }
    # Exclusive creation prevents retries from overwriting originals or human decisions.
    output_dir.mkdir(parents=True, exist_ok=False)
    for filename, content in (
        (manifest["source"]["filename"], source.content), (TEXT_NAME, text.content),
        (INTAKE_NAME, encoded), ("document-review.example.json", _encoded(review)),
    ):
        with (output_dir / filename).open("xb") as destination:
            destination.write(content)
    return output_dir


@dataclass(frozen=True)
class DocumentBundle:
    manifest: dict[str, Any]
    snapshots: tuple[SourceSnapshot, ...]
    review: dict[str, Any] | None
    issues: tuple[str, ...]

    def evidence(self, index: int) -> tuple[SourceEvidence, ...]:
        return tuple(SourceEvidence(
            slot=f"document_{index:03d}_{number}",
            filename=f"document_{index:03d}/{snapshot.path.name}", sha256=snapshot.sha256,
        ) for number, snapshot in enumerate(self.snapshots, start=1))


def load_document(root: Path) -> DocumentBundle:
    intake = _member(root, INTAKE_NAME, 1024 * 1024)
    manifest = _json(intake)
    source_info = manifest.get("source")
    if (not isinstance(source_info, dict) or not isinstance(source_info.get("filename"), str)
        or source_info["filename"] not in {
        "original.txt", "original.pdf",
    }):
        raise GateInputError("Document source must name original.txt or original.pdf.")
    source = _member(root, source_info["filename"], 20 * 1024 * 1024)
    text = _member(root, TEXT_NAME, 1024 * 1024)
    for field in ("entity_ref", "period_end", "text_origin"):
        _string(manifest.get(field), field)
    text_version = _string(manifest.get("text_version"), "text_version",
                           empty=manifest["text_origin"] == "manual")
    expected = _manifest(source, text, entity=manifest["entity_ref"],
                         period_end=manifest["period_end"], text_origin=manifest["text_origin"],
                         text_version=text_version)
    if _encoded(manifest) != _encoded(expected):
        raise GateInputError("Document intake does not match its sources or supported schema.")
    snapshots = (intake, source, text)
    if not (root / REVIEW_NAME).exists():
        return DocumentBundle(manifest, snapshots, None, ("Human transcription review is missing.",))
    review_source = _member(root, REVIEW_NAME, 1024 * 1024)
    review = _json(review_source)
    issues = _review_issues(review, intake.sha256, manifest, text)
    return DocumentBundle(manifest, (*snapshots, review_source), review, tuple(issues))


def _review_issues(review: dict[str, Any], digest: str, manifest: dict[str, Any],
                   text: SourceSnapshot) -> list[str]:
    keys = {"schema_version", "intake_sha256", "reviewer_initials", "reviewed_on",
            "source_text_checked", "fields"}
    if set(review) != keys or review["schema_version"] != "document-review.v1":
        raise GateInputError("Unsupported document review schema.")
    if review["intake_sha256"] != digest:
        raise GateInputError("Document review is bound to different intake bytes.")
    initials = _string(review["reviewer_initials"], "reviewer_initials", empty=True)
    reviewed_on = _string(review["reviewed_on"], "reviewed_on", empty=True)
    if reviewed_on:
        parse_iso_date(reviewed_on, field="reviewed_on", path=Path(REVIEW_NAME))
    if type(review["source_text_checked"]) is not bool:
        raise GateInputError("source_text_checked must be a boolean.")
    issues = []
    if not initials.strip() or not reviewed_on or not review["source_text_checked"]:
        issues.append("Record the reviewer, date and confirmation against the original document.")
    decisions = review["fields"]
    if not isinstance(decisions, dict) or set(decisions) != set(FIELDS):
        raise GateInputError("Document review must include every field exactly once.")
    pages = text.text(label="Extracted text", encoding="utf-8").count("\f") + 1
    values = {}
    for name in FIELDS:
        decision = decisions[name]
        if not isinstance(decision, dict) or set(decision) != {"value", "page", "note"}:
            raise GateInputError(f"Invalid review shape for {name}.")
        note = _string(decision["note"], f"{name}.note", empty=True)
        page = decision["page"]
        if page is not None and (type(page) is not int or not 1 <= page <= pages):
            raise GateInputError(f"{name}.page must identify a page in the extracted text.")
        if decision["value"] is None:
            issues.append(f"{name}: unresolved.")
            continue
        value = _value(name, decision["value"])
        if type(page) is not int or not 1 <= page <= pages:
            raise GateInputError(f"{name}.page must identify a page in the extracted text.")
        rows = manifest["fields"][name]
        if (len(rows) != 1 or value != rows[0]["value"] or page != rows[0]["page"]) and not note.strip():
            issues.append(f"{name}: explain the correction or choice between observations.")
        values[name] = value
    if "total" in values and "gst_observed" in values:
        total, gst = Decimal(values["total"]), Decimal(values["gst_observed"])
        if gst.copy_abs() > total.copy_abs() or (
            total != 0 and gst != 0 and total.is_signed() != gst.is_signed()
        ):
            issues.append("Observed GST has a different sign from the total or exceeds it.")
    return issues


def document_controls(bundles: list[DocumentBundle], *, entity: str | None,
                      period_end: str | None) -> tuple[list[Finding], list[SourceEvidence]]:
    findings: list[Finding] = []
    evidence: list[SourceEvidence] = []
    seen: set[str] = set()
    for index, bundle in enumerate(bundles, start=1):
        slot = f"document_{index:03d}"
        evidence.extend(bundle.evidence(index))
        if (entity is not None and bundle.manifest["entity_ref"] != entity) or (
            period_end is not None and bundle.manifest["period_end"] != period_end
        ):
            findings.append(Finding("DOCUMENT_CONTEXT_MISMATCH", "BLOCKED", slot,
                                    "Document entity or review period differs from the pack.",
                                    "Check the original and attach evidence for this entity and period."))
        if bundle.issues:
            findings.append(Finding("DOCUMENT_REVIEW_REQUIRED", "NOT_READY", slot,
                                    " ".join(bundle.issues),
                                    "Resolve transcription against the original; keep tax treatment separate."))
        digest = bundle.manifest["source"]["sha256"]
        if digest in seen:
            findings.append(Finding("DOCUMENT_DUPLICATE", "NOT_READY", slot,
                                    "These original bytes already occur in the supplied documents.",
                                    "Review the duplicate attachment; no document was deleted or merged."))
        seen.add(digest)
    return findings, evidence


def render_document(bundle: DocumentBundle) -> str:
    manifest = bundle.manifest
    lines = ["Document evidence", BOUNDARY,
             "Entity: " + json.dumps(manifest["entity_ref"]),
             "Review period: " + manifest["period_end"],
             "Text extraction: " + json.dumps([manifest["text_origin"], manifest["text_version"]])]
    if bundle.review is not None:
        lines.append("Transcription reviewer: " + json.dumps(bundle.review["reviewer_initials"]))
        lines.append("Reviewed on: " + json.dumps(bundle.review["reviewed_on"]))
        lines.append("Original checked: " + json.dumps(bundle.review["source_text_checked"]))
    for snapshot in bundle.snapshots:
        lines.append(f"{snapshot.path.name}: SHA-256 {snapshot.sha256}")
    for name, label in FIELDS.items():
        lines.append(f"{label}:")
        for row in manifest["fields"][name]:
            lines.append(f"  observed page {row['page']}, line {row['line']}: "
                         + json.dumps(row["raw"], ensure_ascii=True))
            lines.append("  proposed: " + json.dumps(row["value"], ensure_ascii=True))
            if row["error"]:
                lines.append("  validation: " + json.dumps(row["error"], ensure_ascii=True))
        if not manifest["fields"][name]:
            lines.append("  no observation")
        if bundle.review is not None:
            decision = bundle.review["fields"][name]
            lines.append("  review: " + json.dumps(decision, sort_keys=True, ensure_ascii=True))
    lines.extend("Unresolved: " + issue for issue in bundle.issues)
    if not bundle.issues:
        lines.append("Transcription review recorded. " + BOUNDARY)
    return "\n".join(lines)
