"""Draft source-only manifests and render unresolved debtor evidence requests."""
from __future__ import annotations

import argparse
import csv
import hashlib
import io
import json
import re
import sys
from datetime import datetime
from pathlib import Path

from xero_aged_receivables import BUCKETS, MANIFEST_FIELDS, inspect_export, unique_object

REPOSITORY = Path(__file__).resolve().parents[3]

def draft_manifest(content):
    records = list(csv.reader(io.StringIO(content.decode("utf-8-sig"), newline=""), strict=True))
    headers = [index for index, row in enumerate(records) if row and row[0] in ("Contact", "Customer")
               and "Total" in row and all(bucket in row for bucket in BUCKETS)]
    if len(headers) != 1:
        raise ValueError("Expected one summary header before drafting source declarations.")
    metadata = [row[0].strip() for row in records[1:headers[0]] if row and row[0].strip()]
    entities = [value for value in metadata if not value.startswith(("As at ", "Ageing by "))]
    dates = [value[6:] for value in metadata if value.startswith("As at ")]
    ageing = [value[10:] for value in metadata if value.startswith("Ageing by ")]
    cutoff = ""
    if len(dates) == 1:
        try:
            cutoff = datetime.strptime(dates[0], "%d %B %Y").date().isoformat()
        except ValueError:
            pass
    manifest = dict.fromkeys(MANIFEST_FIELDS, "")
    manifest.update(report_name=records[0][0].strip() if records and records[0] else "",
                    entity=entities[0] if len(entities) == 1 else "", as_at=cutoff,
                    ageing_basis=ageing[0] if len(ageing) == 1 else "", source_format="CSV",
                    source_sha256=hashlib.sha256(content).hexdigest(), footer_record=None,
                    on_screen_rows=None, on_screen_total="",
                    draft_boundary="Source declarations only. Independently confirm the timestamp, settings, "
                    "footer role, on-screen observations and control before relying on the summary.")
    return manifest


def evidence_requests(result):
    requests = []
    for issue in result["exceptions"]:
        records = [int(number) for number in re.findall(r"(?:record|Source record) (\d+)", issue)]
        action = ("Obtain a matching dated receivables control and its source reference."
                  if "control" in issue.lower() else
                  "Check the report on screen and independently record its population and total."
                  if "screen" in issue.lower() else
                  "Confirm the terminal footer's logical source record without changing contact rows."
                  if "footer has not" in issue.lower() or "footer record is absent" in issue.lower() else
                  "Recheck the identified source records and export settings; preserve the difference and re-export if needed.")
        requests.append({"source_records": records, "reason": issue, "owner": "Export preparer",
                         "status": "UNRESOLVED", "next_action": action})
    for row in result["rows"]:
        requests.append({"source_records": [row["source_record"]], "contact": row["Contact"],
                         "reason": "Summary data cannot establish invoice or collection decisions.",
                         "owner": "Authorised debtor reviewer", "status": "REVIEW",
                         "next_action": "Obtain dated Detail with stable contact/invoice identities, "
                         "subsequent receipts and dispute evidence; record an authorised decision."})
    return requests


def briefing(result):
    lines = [f"Summary: {result['summary_status']}. Debtor decisions: {result['debtor_decisions']}.",
             f"Source SHA-256: {result['source_sha256']}", "", "Supported arithmetic:"]
    for tie in result["ties"]:
        lines.append(f"{tie['check']}: {tie['left']} versus {tie['right']}; "
                     f"difference {tie['difference']}; {'agrees' if tie['agrees'] else 'unresolved'}.")
    lines += ["", "Evidence requests (for preparation, not sending):"]
    for request in evidence_requests(result):
        # JSON quoting preserves untrusted contact text and control characters.
        lines.append(json.dumps(request, ensure_ascii=True, sort_keys=True))
    lines += ["", result["boundary"], ""]
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("draft", "review"))
    parser.add_argument("source", type=Path)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--control", type=Path)
    parser.add_argument("--output", type=Path, required=True, help="new evidence directory outside the checkout")
    args = parser.parse_args(argv)
    try:
        output = args.output.resolve()
        if output.is_relative_to(REPOSITORY):
            raise ValueError("Evidence output must be outside the checkout.")
        if output.exists():
            raise ValueError("Output already exists; preserve the previous evidence.")
        content = args.source.read_bytes()
        if args.mode == "draft":
            if args.manifest or args.control:
                raise ValueError("Draft mode extracts source declarations only.")
            result = draft_manifest(content)
            files = {"manifest-draft.json": json.dumps(result, indent=2, ensure_ascii=True) + "\n"}
            exit_code = 2
        else:
            if args.manifest is None:
                raise ValueError("Review mode requires a manifest.")
            manifest = json.loads(args.manifest.read_text(encoding="utf-8"), object_pairs_hook=unique_object)
            control = json.loads(args.control.read_text(encoding="utf-8"), object_pairs_hook=unique_object) if args.control else None
            result = inspect_export(content, manifest, control)
            files = {"summary-review.json": json.dumps(result, indent=2, ensure_ascii=True) + "\n",
                     "evidence-requests.json": json.dumps(evidence_requests(result), indent=2, ensure_ascii=True) + "\n",
                     "summary-review.txt": briefing(result)}
            exit_code = 0 if result["summary_status"] == "PASS" else 2
        output.mkdir(parents=True, exist_ok=False)
        for name, text in files.items():
            (output / name).write_text(text, encoding="utf-8")
        print("Source-only draft: REVIEW." if args.mode == "draft" else
              f"Summary: {result['summary_status']}; debtor decisions: REVIEW.")
        return exit_code
    except (OSError, UnicodeError, ValueError, csv.Error) as error:
        print(json.dumps({"summary_status": "INVALID_INPUT", "error": str(error)}), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
