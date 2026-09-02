from __future__ import annotations

import csv
import io
import json
import os
import uuid
from pathlib import Path

from .engine import ReadinessPack
from .models import Finding

REVIEW_BOUNDARY = (
    "This pack is a review aid. It does not approve a file, lodge a return, "
    "or replace professional judgement."
)

PACK_FILE_NAMES = ("readiness-pack.json", "readiness-summary.md", "findings.csv")


def _money(value) -> str:
    if value is None:
        return ""
    as_tuple = getattr(value, "as_tuple", None)
    places = 2
    if as_tuple is not None:
        exponent = as_tuple().exponent
        if isinstance(exponent, int):
            places = max(2, -exponent)
    return f"{value:.{places}f}"


def _csv_safe(value: str) -> str:
    stripped = value.lstrip()
    if stripped.startswith(("=", "+", "-", "@")):
        return "'" + value
    return value


def _md_cell(value: str) -> str:
    # Backslash first, or the escapes below would be escaped again. Asterisk and
    # backtick are structural in the summary the viewer verifies: without them a
    # preparer-supplied cell can forge a status line or an evidence digest line.
    return (
        " ".join(value.split())
        .replace("\\", "\\\\")
        .replace("|", "\\|")
        .replace("*", "\\*")
        .replace("`", "\\`")
    )


def _md_note_lines(comment: str) -> list[str]:
    if "\n" not in comment and "\r" not in comment:
        return [f"- Comment: {_md_cell(comment)}"]
    lines = ["- Comment:"]
    for raw in comment.splitlines():
        line = _md_cell(raw)
        if line.startswith("#"):
            line = "\\" + line
        lines.append(f"  > {line}".rstrip())
    return lines


def _finding_dict(item: Finding) -> dict[str, str]:
    return {
        "code": item.code,
        "status": item.status,
        "slot": item.slot,
        "repeat": "true" if item.repeat else "false",
        "reason": item.reason,
        "reviewer_action": item.reviewer_action,
    }


def _as_json(pack: ReadinessPack) -> dict:
    acknowledgement = None
    if pack.acknowledgement is not None:
        acknowledgement = {
            "reviewer_initials": pack.acknowledgement.reviewer_initials,
            "reviewed_on": pack.acknowledgement.reviewed_on.isoformat(),
            "comment": pack.acknowledgement.comment,
            "effect": (
                "Acknowledgement is evidence of human review only; it does not "
                "change readiness or approve a file."
            ),
        }
    return {
        "acknowledgement": acknowledgement,
        "engagement_type": pack.engagement_type,
        "findings": [_finding_dict(item) for item in pack.findings],
        "overall_status": pack.status,
        "period_end": pack.period_end,
        "preparer_initials": pack.preparer_initials,
        "review_boundary": REVIEW_BOUNDARY,
        "source_sha256": {
            evidence.slot: {"filename": evidence.filename, "sha256": evidence.sha256}
            for evidence in pack.source_evidence
        },
        "thresholds": {"tieout_tolerance": _money(pack.tieout_tolerance)},
    }


_ABSENT = "n/a"


def _as_markdown(pack: ReadinessPack) -> str:
    blocked = sum(finding.status == "BLOCKED" for finding in pack.findings)
    not_ready = sum(finding.status == "NOT_READY" for finding in pack.findings)
    repeats = sum(finding.repeat for finding in pack.findings)
    lines = [
        "# Review-Ready Pack",
        "",
        f"**Overall status: {pack.status}**",
        "",
        REVIEW_BOUNDARY,
        "",
        "## Scope",
        "",
        f"- Engagement type: {pack.engagement_type}",
        f"- Period end: {pack.period_end or _ABSENT}",
        f"- Preparer initials: {_md_cell(pack.preparer_initials) if pack.preparer_initials else _ABSENT}",
        f"- Tie-out tolerance: ${_money(pack.tieout_tolerance)}",
        f"- Findings: {len(pack.findings)} total; {blocked} blocked; {not_ready} not ready; {repeats} repeats.",
        "",
        "## Source evidence",
        "",
    ]
    if not pack.source_evidence:
        lines.append("No source files were present in the pack directory.")
    else:
        for evidence in pack.source_evidence:
            lines.append(f"- `{evidence.slot}` (`{evidence.filename}`): `{evidence.sha256}`")
    lines += ["", "## Findings", ""]
    if not pack.findings:
        lines.append(
            "No readiness findings were raised. A human must still decide whether the file is acceptable."
        )
    else:
        lines += [
            "| Status | Code | Slot | Repeat | Reason |",
            "| --- | --- | --- | --- | --- |",
        ]
        for finding in pack.findings:
            lines.append(
                f"| {finding.status} | {finding.code} | {_md_cell(finding.slot)} | "
                f"{'yes' if finding.repeat else 'no'} | {_md_cell(finding.reason)} |"
            )
    lines += ["", "## Human acknowledgement", ""]
    if pack.acknowledgement is None:
        lines.append("No reviewer acknowledgement was supplied. This does not create or imply an approval.")
    else:
        lines += [
            f"- Reviewer initials: {_md_cell(pack.acknowledgement.reviewer_initials)}",
            f"- Reviewed on: {pack.acknowledgement.reviewed_on.isoformat()}",
            *_md_note_lines(pack.acknowledgement.comment),
            "- Effect: acknowledgement records a human action only; it does not change readiness or approve a file.",
        ]
    lines.append("")
    return "\n".join(lines)


def _as_csv(pack: ReadinessPack) -> str:
    fields = ["code", "status", "slot", "repeat", "reason", "reviewer_action"]
    buffer = io.StringIO()
    writer = csv.DictWriter(buffer, fieldnames=fields)
    writer.writeheader()
    for item in pack.findings:
        row = _finding_dict(item)
        for field in ("slot", "reason", "reviewer_action"):
            row[field] = _csv_safe(row[field])
        writer.writerow(row)
    return buffer.getvalue()


def _remove_quietly(path: Path) -> None:
    try:
        path.unlink()
    except OSError:
        pass


def _restore_quietly(parked: Path, destination: Path) -> None:
    try:
        os.replace(parked, destination)
    except OSError:
        pass


def _sibling_partial(destination: Path) -> Path:
    while True:
        candidate = destination.with_name(f"{destination.name}.{uuid.uuid4().hex[:12]}.partial")
        try:
            with candidate.open("x", encoding="utf-8"):
                pass
        except FileExistsError:  # pragma: no cover
            continue
        return candidate


def _swap_into_place(staged_path: Path, destination: Path) -> Path | None:
    parked: Path | None = None
    if destination.is_file():
        parked = _sibling_partial(destination)
        try:
            os.replace(destination, parked)
        except OSError:
            _remove_quietly(parked)
            raise
    try:
        os.replace(staged_path, destination)
    except OSError:
        if parked is not None:
            _restore_quietly(parked, destination)
        raise
    return parked


def write_review_pack(pack: ReadinessPack, output_dir: Path) -> dict[str, Path]:
    output_dir.mkdir(parents=True, exist_ok=True)
    json_path = output_dir / "readiness-pack.json"
    summary_path = output_dir / "readiness-summary.md"
    findings_path = output_dir / "findings.csv"
    rendered = (
        (json_path, json.dumps(_as_json(pack), indent=2, sort_keys=True) + "\n", "utf-8", None),
        (summary_path, _as_markdown(pack), "utf-8", None),
        (findings_path, _as_csv(pack), "utf-8-sig", ""),
    )

    staged: list[tuple[Path, Path]] = []
    try:
        for destination, text, encoding, newline in rendered:
            staged_path = _sibling_partial(destination)
            staged.append((staged_path, destination))
            staged_path.write_text(text, encoding=encoding, newline=newline)
    except BaseException:
        for staged_path, _ in staged:
            _remove_quietly(staged_path)
        raise

    replaced: list[tuple[Path, Path | None]] = []
    try:
        for staged_path, destination in staged:
            replaced.append((destination, _swap_into_place(staged_path, destination)))
    except OSError:
        for destination, parked in reversed(replaced):
            if parked is None:
                _remove_quietly(destination)
            else:
                _restore_quietly(parked, destination)
        for staged_path, _ in staged:
            _remove_quietly(staged_path)
        raise
    for _, parked in replaced:
        if parked is not None:
            _remove_quietly(parked)
    return {"json": json_path, "summary": summary_path, "findings": findings_path}
