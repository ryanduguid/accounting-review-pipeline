"""JSON-lines access to the frozen Python aged tools, for comparison only."""
import csv
import io
import json
import runpy
import sys
from datetime import datetime
from pathlib import Path

root = Path(sys.argv[1])
sys.path.insert(0, str(root))
verifier = runpy.run_path(str(root / "xero_aged_receivables.py"))
evidence = runpy.run_path(str(root / "debtor_evidence.py"))
replay = runpy.run_path(str(root / "replay_aged_review.py"))


def apply(request):
    kind = request["kind"]
    if kind == "amount":
        return str(verifier["amount"](request["text"]))
    if kind == "csv":
        return list(csv.reader(io.StringIO(request["source"], newline=""), strict=True))
    if kind == "generated":
        try:
            text = request["text"]
            parsed = datetime.fromisoformat(text)
            return "T" in text and parsed.tzinfo is not None
        except (ValueError, TypeError):
            return False
    if kind == "json":
        try:
            value = json.loads(request["text"], object_pairs_hook=verifier["unique_object"])
        except ValueError as error:
            if str(error) == "Duplicate JSON field.":
                raise
            raise ValueError("JSON parse error") from None
        rendered = json.dumps(value, indent=2, ensure_ascii=request["ascii"])
        if not request["ascii"]:
            rendered.encode("utf-8")
        return rendered
    if kind == "case":
        case = next(item for item in replay["catalogue"]() if item["case"] == request["name"])
        return {"content": case["content"].decode(), "manifest": case["manifest"], "control": case["control"]}
    content = request["source"].encode("utf-8")
    if kind == "draft":
        return evidence["draft_manifest"](content)
    result = verifier["inspect_export"](content, request["manifest"], request.get("control"))
    if kind == "requests":
        return evidence["evidence_requests"](result)
    if kind == "briefing":
        return evidence["briefing"](result)
    if kind == "pretty":
        rendered = json.dumps(result, indent=2, ensure_ascii=request["ascii"])
        if not request["ascii"]:
            rendered.encode("utf-8")
        return rendered
    return result


for line in sys.stdin:
    try:
        result = {"ok": apply(json.loads(line))}
    except Exception as error:  # The protocol compares refusals as well as success.
        result = {"error": str(error)}
    print(json.dumps(result, ensure_ascii=True), flush=True)
