"""Run the frozen converter for the native comparison harness."""

import ast
import csv
import io
import json
import runpy
import sys
from datetime import datetime

module = runpy.run_path(sys.argv[1])
AMOUNT_ERROR = "Expected an explicit non-negative amount with at most 2 places: "


def evaluate(request):
    kind = request["kind"]
    if kind == "amount":
        return str(module["amount"](request["text"]))
    if kind == "date":
        text = request["text"]
        pattern = "%d %B %Y" if request["full"] else "%d/%m/%Y" if "/" in text else "%d %b %Y"
        try:
            return datetime.strptime(text, pattern).date().isoformat()
        except ValueError:
            return None
    if kind == "csv":
        return list(csv.reader(io.StringIO(request["source"].removeprefix("\ufeff"))))
    content = request["source"].encode("utf-8")
    identity = {"tenant": request["tenant"], "account_name": request["account_name"]}
    if kind == "inspect":
        return module["inspect_export"](content, **identity)
    if kind == "convert":
        return module["convert"](content, request["mapping"], **identity,
                                 account_id=request["account_id"], currency=request["currency"])
    raise ValueError("Unknown comparison kind")


for line in sys.stdin:
    request = json.loads(line)
    try:
        result = {"ok": evaluate(request)}
    except (ValueError, csv.Error) as error:
        message = str(error)
        result = ({"amount_error": ast.literal_eval(message[len(AMOUNT_ERROR):])}
                  if message.startswith(AMOUNT_ERROR) else {"error": message})
    print(json.dumps(result, ensure_ascii=True), flush=True)
