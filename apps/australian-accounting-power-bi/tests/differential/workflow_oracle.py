"""Frozen-workflow adapter used only by the Rust differential tool."""
import hashlib
import importlib.util
import json
import sys
from pathlib import Path

spec = importlib.util.spec_from_file_location("frozen_review_workflow", Path(sys.argv[1]) / "tools/review_workflow.py")
workflow = importlib.util.module_from_spec(spec)
spec.loader.exec_module(workflow)


def evaluate(request):
    operation = request["op"]
    if operation == "json":
        return workflow.digest(workflow.json_bytes(workflow.strict_json(request["text"].encode("utf-8"), "Input")))
    if operation == "schema":
        kind = request["kind"]
        if kind in ("money", "nullable-money"):
            workflow.money_text(request["value"], "Money", nullable=kind == "nullable-money")
        elif kind in ("close-control", "review-ready"):
            workflow.producer_document(request["value"], kind)
        else:
            getattr(workflow, {"drivers": "driver"}.get(kind, kind) + "_document")(request["value"])
        return None
    if operation == "csv":
        return workflow.rows(request["text"].encode("utf-8"))
    if operation == "ledger":
        case, sources = workflow.sources(Path(sys.argv[1]))
        case.update(request.get("case", {}))
        sources.update({key: value.encode("utf-8") for key, value in request.get("sources", {}).items()})
        return [workflow.digest(value) for value in workflow.trial_balances(case, sources, request["month"])]
    if operation == "zip":
        return hashlib.sha256(workflow.export_bytes({key: value.encode("utf-8") for key, value in request["members"].items()})).hexdigest()
    if operation == "export":
        return workflow.verify_export(Path(request["path"]))
    raise ValueError("Unknown operation")


for line in sys.stdin:
    try:
        result = {"ok": True, "value": evaluate(json.loads(line))}
    except (ValueError, OSError, KeyError, ArithmeticError, TypeError, UnicodeError):
        result = {"ok": False}
    print(json.dumps(result, ensure_ascii=True, allow_nan=False), flush=True)
