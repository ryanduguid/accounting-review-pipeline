"""Build and display one pinned, offline fabricated review case through existing CLIs."""
from __future__ import annotations

import argparse
import calendar
import csv
import hashlib
import html
import io
import json
import os
import re
import subprocess
from datetime import datetime, timezone
from decimal import Context, Decimal, localcontext
from pathlib import Path, PureWindowsPath
from typing import Any

APP = Path(__file__).resolve().parents[1]
NAME = "australian-accounting-power-bi"
TB_COLUMNS = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit".split(",")
CLOSE_FILES = ("close-review-pack.json", "close-summary.md", "exceptions.csv", "client-queries.csv")
READY_FILES = ("readiness-pack.json", "readiness-summary.md", "findings.csv")
BOUNDARY = "Fabricated demonstration. Verification and acknowledgement do not approve accounting or close a period."


def digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def json_bytes(value: Any) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode("utf-8")


def rows(value: bytes) -> list[dict[str, str]]:
    return list(csv.DictReader(io.StringIO(value.decode("utf-8-sig"))))


def csv_bytes(columns: list[str], values: list[dict[str, Any]]) -> bytes:
    stream = io.StringIO(newline="")
    writer = csv.DictWriter(stream, columns, lineterminator="\n")
    writer.writeheader()
    writer.writerows(values)
    return stream.getvalue().encode("utf-8")


def ordinary(path: Path) -> Path:
    windows_drive = PureWindowsPath(path).drive
    if windows_drive.upper() == "Z:" or windows_drive.startswith("\\\\"):
        raise ValueError("Use a local directory.")
    path = path.absolute()
    if str(path).startswith("\\\\") or path.drive.upper() == "Z:":
        raise ValueError("Use a local directory.")
    for part in (path, *path.parents):
        if part.exists() and (part.is_symlink() or getattr(part.stat(), "st_file_attributes", 0) & 0x400):
            raise ValueError("Linked paths are not admitted.")
    return path.resolve()


def new_output(path: Path) -> Path:
    path = ordinary(path)
    if path.exists():
        raise ValueError("Output must be new.")
    if any((parent / marker).exists() for parent in path.parents for marker in (".git", ".hg", ".svn", ".bzr")):
        raise ValueError("Generated reviews must be outside version control.")
    return path


def sources(app: Path = APP) -> tuple[dict[str, Any], dict[str, bytes]]:
    case = json.loads(ordinary(app / "samples/shared-review-case.json").read_bytes())
    source = {}
    for name, expected in case["source_sha256"].items():
        if not re.fullmatch(r"sample-[a-z-]+\.csv", name):
            raise ValueError("Case source is outside the fixture whitelist.")
        data = ordinary(app / "samples" / name).read_bytes()
        if digest(data) != expected:
            raise ValueError(f"Pinned fabricated source changed: {name}")
        source[name] = data
    if len(source) != 6:
        raise ValueError("Expected the original six fabricated sources.")
    return case, source


def trial_balances(case: dict[str, Any], source: dict[str, bytes], month: int) -> tuple[bytes, bytes, bytes]:
    """Use the first fixture FY and its explicit opening journal; never infer later closes."""
    if case["financial_year_start"] != "2024-07-01" or month not in (8, 9):
        raise ValueError("This case supports August and September 2024 only.")
    chart = rows(source["sample-chart-of-accounts.csv"])
    account_ids = case["account_ids"]
    period_end = f"2024-{month:02d}-{calendar.monthrange(2024, month)[1]}"
    ledger = [row for row in rows(source["sample-general-ledger.csv"]) if row["EntityID"] == case["entity"] and case["financial_year_start"] <= row["PostingDate"] <= period_end]
    if not ledger or {r["AccountCode"] for r in ledger} - account_ids.keys():
        raise ValueError("Ledger account population is not mapped.")
    seen = set()
    journals: dict[str, Decimal] = {}
    with localcontext(Context(prec=40)):
        for row in ledger:
            key = (row["JournalID"], row["LineNumber"])
            debit, credit = Decimal(row["Debit"]), Decimal(row["Credit"])
            if key in seen or not debit.is_finite() or not credit.is_finite() or min(debit, credit) < 0 or (debit > 0) == (credit > 0) or debit - credit != Decimal(row["Amount"]):
                raise ValueError("Invalid or duplicated journal line.")
            seen.add(key)
            journals[row["JournalID"]] = journals.get(row["JournalID"], Decimal(0)) + debit - credit
        if any(journals.values()) or not any(r["JournalID"] == case["opening_journal"] for r in ledger):
            raise ValueError("Unbalanced journal or missing explicit opening.")

        def tb(period: int) -> bytes:
            output = []
            end = f"2024-{period:02d}-{calendar.monthrange(2024, period)[1]}"
            for account in chart:
                code = account["AccountCode"]
                eligible = [r for r in ledger if r["AccountCode"] == code and case["financial_year_start"] <= r["PostingDate"] <= end]
                current = [r for r in eligible if r["PostingDate"][:7] == end[:7]]
                values = [sum((Decimal(r[col]) for r in population), Decimal(0)) for population in (current, eligible) for col in ("Debit", "Credit")]
                output.append(dict(zip(TB_COLUMNS, [end, case["tenant"], account["Class"], account_ids[code], account["AccountName"], code, *(f"{value:.2f}" for value in values)])))
            for debit, credit in (("Debit", "Credit"), ("YTDDebit", "YTDCredit")):
                if sum((Decimal(r[debit]) - Decimal(r[credit]) for r in output), Decimal(0)):
                    raise ValueError("Trial balance does not satisfy the exporter contract.")
            return csv_bytes(TB_COLUMNS, output)

        transactions = [dict(zip("Tenant,AccountID,Currency,TransactionID,Date,Reference,Description,Debit,Credit".split(","), [case["tenant"], account_ids[r["AccountCode"]], case["currency"], r["JournalID"] + ":" + r["LineNumber"], r["PostingDate"], r["JournalID"], r["Description"], r["Debit"], r["Credit"]])) for r in ledger]
        return tb(month), tb(month - 1), csv_bytes(list(transactions[0]), transactions)


def invoke(bin_dir: Path, command: str, args: list[str], run: Path, label: str, allowed: tuple[int, ...] = (0,)) -> dict[str, Any]:
    executable = ordinary(bin_dir / (command + (".exe" if os.name == "nt" else "")))
    executable_hash = digest(executable.read_bytes())
    started = datetime.now(timezone.utc).isoformat()
    result = subprocess.run([str(executable), *args], capture_output=True, cwd=run, check=False)
    if digest(ordinary(executable).read_bytes()) != executable_hash:
        raise ValueError("Producer executable changed during invocation.")
    for kind, data in (("stdout", result.stdout), ("stderr", result.stderr)):
        (run / "validation" / f"{label}.{kind}.txt").write_bytes(data)
    if result.returncode not in allowed:
        raise ValueError(f"{command} {label} refused the case (exit {result.returncode}); see validation logs.")
    return {"command": command, "args": args, "executable_sha256": executable_hash, "started": started, "finished": datetime.now(timezone.utc).isoformat(), "exit": result.returncode}


def pinned_producer(bin_dir: Path, command: str, args: list[str], expected_hash: str) -> bytes:
    executable = ordinary(bin_dir / (command + (".exe" if os.name == "nt" else "")))
    if digest(executable.read_bytes()) != expected_hash:
        raise ValueError("Producer executable hash differs from the receipt.")
    result = subprocess.run([str(executable), *args], capture_output=True, check=False)
    if digest(ordinary(executable).read_bytes()) != expected_hash:
        raise ValueError("Producer executable changed during invocation.")
    if result.returncode:
        raise ValueError("Producer verification refused portable display.")
    return result.stdout


def build(run: Path, bin_dir: Path, month: int, note: Path | None = None) -> Path:
    run = new_output(run)
    case, source = sources()
    current, prior, transactions = trial_balances(case, source, month)
    (run / "inputs").mkdir(parents=True)
    (run / "validation").mkdir()
    for name, data in source.items():
        (run / "inputs" / name).write_bytes(data)
    (run / "inputs/case.json").write_bytes(json_bytes(case))
    for name, data in (("trial_balance.csv", current), ("prior_trial_balance.csv", prior), ("transactions.csv", transactions)):
        (run / "inputs" / name).write_bytes(data)
    period = rows(current)[0]["ReportDate"]
    (run / "inputs/open_items.csv").write_bytes(b"ItemID,Severity,Owner,DueDate,Status,Description,Resolution\n")
    self_review = {"preparer_initials": "DEMO", "prepared_on": period, "engagement_type": "month_end", "period_end": period, "assertions": dict.fromkeys(("pack_complete", "tie_outs_done", "open_items_listed", "variances_explained", "self_reviewed"), True)}
    (run / "inputs/self_review.json").write_bytes(json_bytes(self_review))
    commands = []
    commands.append(invoke(bin_dir, "review-ready", ["gate", "--profile", "month_end", "--pack", "inputs", "--output", "readiness"], run, "readiness", (0, 2)))
    commands.append(invoke(bin_dir, "review-ready", ["view", "--pack-dir", "readiness"], run, "readiness-view"))
    args = ["review", "--current", "inputs/trial_balance.csv", "--prior", "inputs/prior_trial_balance.csv", "--output", "close"]
    if note:
        (run / "inputs/review-note.json").write_bytes(ordinary(note).read_bytes())
        args += ["--review-note", "inputs/review-note.json"]
    commands.append(invoke(bin_dir, "close-control", args, run, "close", (0, 2)))
    before = {name: digest((run / "close" / name).read_bytes()) for name in CLOSE_FILES}
    commands.append(invoke(bin_dir, "close-control", ["view", "--pack-dir", "close"], run, "close-view"))
    if before != {name: digest((run / "close" / name).read_bytes()) for name in CLOSE_FILES}:
        raise ValueError("Close outputs changed during verification.")
    commands.append(invoke(bin_dir, "close-control", ["drivers", "--pack-dir", "close", "--transactions", "inputs/transactions.csv", "--currency", case["currency"], "--top", "100000", "--output", "drivers"], run, "drivers"))
    doc = json.loads((run / "close/close-review-pack.json").read_bytes())
    if doc["source_sha256"]["current_trial_balance"] != digest(current) or doc["source_sha256"]["prior_trial_balance"] != digest(prior) or doc["current_report_dates"] != [period] or any(r["tenant"] not in ("", case["tenant"]) for r in doc["exceptions"]):
        raise ValueError("Close provenance or context differs from the case.")
    files = {str(path.relative_to(run)).replace("\\", "/"): digest(path.read_bytes()) for folder in ("inputs", "close", "readiness", "drivers", "validation") for path in (run / folder).iterdir()}
    receipt = {"schema_version": 1, "case": case, "period": period, "invocations": commands, "files": files, "boundary": BOUNDARY}
    data = json_bytes(receipt)
    (run / "receipt.json").write_bytes(data)
    (run / "receipt.sha256").write_text(digest(data), encoding="ascii")
    verify(run)
    return run


def verify(run: Path) -> tuple[dict[str, Any], dict[str, bytes]]:
    run = ordinary(run)
    data = ordinary(run / "receipt.json").read_bytes()
    if digest(data) != ordinary(run / "receipt.sha256").read_text(encoding="ascii"):
        raise ValueError("Receipt digest differs.")
    receipt = json.loads(data)
    if receipt.get("schema_version") != 1 or receipt.get("boundary") != BOUNDARY:
        raise ValueError("Unsupported review receipt.")
    expected = {f"close/{n}" for n in CLOSE_FILES} | {f"readiness/{n}" for n in READY_FILES} | {"inputs/trial_balance.csv", "inputs/prior_trial_balance.csv", "inputs/transactions.csv", "inputs/case.json", "drivers/variance-drivers.json", "drivers/variance-drivers.csv"}
    if not expected.issubset(receipt["files"]):
        raise ValueError("Required review artefacts are missing.")
    snapshot = {}
    for name, expected_hash in receipt["files"].items():
        if not re.fullmatch(r"(inputs|close|readiness|drivers|validation)/[a-zA-Z0-9_.-]+", name) or ".." in name:
            raise ValueError("Receipt path is outside the whitelist.")
        snapshot[name] = ordinary(run / name).read_bytes()
        if digest(snapshot[name]) != expected_hash:
            raise ValueError(f"Sealed artefact changed: {name}")
    case = receipt["case"]
    expected_case, _ = sources()
    if json.loads(snapshot["inputs/case.json"]) != case or case != expected_case:
        raise ValueError("Case identity differs.")
    source = {name: snapshot["inputs/" + name] for name in case["source_sha256"]}
    if any(digest(data) != case["source_sha256"][name] for name, data in source.items()):
        raise ValueError("Fixture provenance differs.")
    current, prior, transactions = trial_balances(case, source, int(receipt["period"][5:7]))
    if any(snapshot[name] != data for name, data in (("inputs/trial_balance.csv", current), ("inputs/prior_trial_balance.csv", prior), ("inputs/transactions.csv", transactions))):
        raise ValueError("Ledger and review sources do not tie.")
    if receipt["period"] != rows(current)[0]["ReportDate"]:
        raise ValueError("Receipt period differs from the ledger case.")
    doc = json.loads(snapshot["close/close-review-pack.json"])
    drivers = json.loads(snapshot["drivers/variance-drivers.json"])
    population = {r["TransactionID"]: r for r in rows(transactions)}
    driver_accounts = set()
    with localcontext(Context(prec=40)):
        for account in drivers["accounts"]:
            identity = (account["tenant"], account["account_id"])
            if identity in driver_accounts:
                raise ValueError("Duplicate driver account identity.")
            driver_accounts.add(identity)
            eligible = [r for r in population.values() if r["Tenant"] == account["tenant"] and r["AccountID"] == account["account_id"] and r["Date"][:7] == receipt["period"][:7]]
            identifiers = [r["TransactionID"] for r in account["drivers"]]
            if len(identifiers) != len(set(identifiers)) or set(identifiers) != {r["TransactionID"] for r in eligible} or account["transactions_in_window"] != len(eligible):
                raise ValueError("Driver journal population differs from the source.")
            total = sum((Decimal(r["Debit"]) - Decimal(r["Credit"]) for r in eligible), Decimal(0))
            if total != Decimal(account["transactions_total"]) or Decimal(account["movement"]) - total != Decimal(account["unexplained"]):
                raise ValueError("Driver totals differ from the source.")
            for driver in account["drivers"]:
                original = population[driver["TransactionID"]]
                if any(driver[k] != original[k] for k in ("Date", "Reference", "Description")) or Decimal(driver["Amount"]) != Decimal(original["Debit"]) - Decimal(original["Credit"]):
                    raise ValueError("Driver journal detail differs from the source.")
    if doc["source_sha256"]["current_trial_balance"] != digest(current) or doc["source_sha256"]["prior_trial_balance"] != digest(prior) or doc["current_report_dates"] != [rows(current)[0]["ReportDate"]] or doc["prior_report_dates"] != [rows(prior)[0]["ReportDate"]]:
        raise ValueError("Close source binding differs.")
    if drivers["source_sha256"]["transactions"] != digest(transactions) or any(drivers["source_sha256"]["pack:" + name] != digest(snapshot["close/" + name]) for name in CLOSE_FILES):
        raise ValueError("Driver source binding differs.")
    expected_commands = ["review-ready", "review-ready", "close-control", "close-control", "close-control"]
    if [r["command"] for r in receipt["invocations"]] != expected_commands or any(r["exit"] not in ((0, 2) if index in (0, 2) else (0,)) for index, r in enumerate(receipt["invocations"])):
        raise ValueError("Required validation invocation did not succeed.")
    for command in set(expected_commands):
        hashes = {r["executable_sha256"] for r in receipt["invocations"] if r["command"] == command}
        if len(hashes) != 1 or not re.fullmatch(r"[a-f0-9]{64}", next(iter(hashes))):
            raise ValueError("Producer executable identity differs within the receipt.")
    return receipt, snapshot


def projection(run: Path) -> dict[str, bytes]:
    receipt, snapshot = verify(run)
    case = receipt["case"]
    doc = json.loads(snapshot["close/close-review-pack.json"])
    drivers = json.loads(snapshot["drivers/variance-drivers.json"])
    driver_index = {(a["tenant"], a["account_id"]): a for a in drivers["accounts"]}
    run_id = digest(snapshot["close/close-review-pack.json"])
    context = {"RunID": run_id, "Entity": case["entity"], "Tenant": case["tenant"], "Period": receipt["period"], "Basis": case["basis"], "Currency": case["currency"]}
    finding_rows, evidence_rows = [], []
    for index, item in enumerate(doc["exceptions"]):
        key = f"{run_id}:{index}"
        query: dict[str, Any] = next((q for q in doc["client_queries"] if (q["control"], q["tenant"], q["account_id"]) == (item["control"], item["tenant"], item["account_id"])), {})
        evidence = driver_index.get((item["tenant"], item["account_id"])) if item["control"] == "period_variance" else None
        available = evidence is not None and Decimal(evidence["unexplained"]) == 0 and evidence["transactions_in_window"] == len(evidence["drivers"])
        finding_rows.append(context | {"ExceptionKey": key, "Control": item["control"], "Status": item["status"], "AccountID": item["account_id"], "Account": item["account_name"], "Current": item["current_value"], "Prior": item["prior_value"], "Difference": item["difference"], "Threshold": item["threshold"], "Reason": item["reason"], "Action": item["reviewer_action"], "Question": query.get("question", ""), "EvidenceRequested": query.get("evidence_requested", ""), "EvidenceState": "Journal rows reconciled" if available else "Line-level evidence is unavailable under this control contract"})
        if available and evidence is not None:
            for row in evidence["drivers"]:
                evidence_rows.append(context | {"ExceptionKey": key, "AccountID": item["account_id"], "TransactionID": row["TransactionID"], "Date": row["Date"], "Reference": row["Reference"], "Description": row["Description"], "Amount": row["Amount"]})
    ready = json.loads(snapshot["readiness/readiness-pack.json"])
    run_row = context | {"CloseStatus": doc["overall_status"], "ReadinessStatus": ready["overall_status"], "PriorPeriod": doc["prior_report_dates"][0], "AbsoluteThreshold": doc["thresholds"]["absolute_variance"], "PercentageThreshold": doc["thresholds"]["percentage_variance"], "ControlsNotRun": "; ".join(doc["controls_not_run"]), "ReadinessControlsNotRun": json.dumps(ready["controls_not_run"], ensure_ascii=False), "Acknowledgement": json.dumps(doc["acknowledgement"], ensure_ascii=False), "UnmappedExceptions": str(sum(r["EvidenceState"] != "Journal rows reconciled" for r in finding_rows)), "SourceTBHash": digest(snapshot["inputs/trial_balance.csv"]), "SourceGLHash": case["source_sha256"]["sample-general-ledger.csv"], "Verification": "CLI verified synthetic case; unsigned local receipt", "Boundary": BOUNDARY}
    return {"sample-review-run.csv": csv_bytes(list(run_row), [run_row]), "sample-review-exceptions.csv": csv_bytes(list(finding_rows[0]), finding_rows), "sample-review-evidence.csv": csv_bytes(list(context) + ["ExceptionKey", "AccountID", "TransactionID", "Date", "Reference", "Description", "Amount"], evidence_rows)}


def render_html(run: Path, bin_dir: Path, previous: Path | None = None) -> str:
    receipt, snapshot = verify(run)
    executable_hashes = {r["command"]: r["executable_sha256"] for r in receipt["invocations"]}
    # Reuse the producer verifier again at display time; no second pack validator.
    for command, folder in (("close-control", "close"), ("review-ready", "readiness")):
        pinned_producer(bin_dir, command, ["view", "--pack-dir", str(run / folder)], executable_hashes[command])
    projected = projection(run)
    doc = json.loads(snapshot["close/close-review-pack.json"])
    change = "No previous verified run supplied."
    if previous:
        old_receipt, old_snapshot = verify(previous)
        if old_receipt["case"] != receipt["case"]:
            raise ValueError("Comparison context differs.")
        if any(r["executable_sha256"] != executable_hashes["close-control"] for r in old_receipt["invocations"] if r["command"] == "close-control"):
            raise ValueError("Comparison producer executable identities differ.")
        comparison = json.loads(pinned_producer(bin_dir, "close-control", ["compare", "--previous-pack", str(previous / "close"), "--current-pack", str(run / "close"), "--previous-tb", str(previous / "inputs/trial_balance.csv"), "--current-tb", str(run / "inputs/trial_balance.csv")], executable_hashes["close-control"]))
        changed = [name for name in receipt["files"] if name.startswith("inputs/") and receipt["files"][name] != old_receipt["files"].get(name)]
        change = json.dumps({"changed_inputs": changed, "findings": comparison["findings"], "queries": comparison["queries"], "scope_changes": comparison["scope_changes"], "acknowledgement_changed": json.loads(old_snapshot["close/close-review-pack.json"])["acknowledgement"] != doc["acknowledgement"], "meaning": comparison["review_boundary"]}, ensure_ascii=False, indent=2)
        if verify(previous)[1] != old_snapshot:
            raise ValueError("Previous run changed during display.")
    if verify(run)[1] != snapshot:
        raise ValueError("Current run changed during display.")
    def esc(value: Any) -> str:
        return html.escape(str(value), quote=True)
    comparison_html = f"<p>{esc(change)}</p>"
    if previous:
        changes = json.loads(change)
        labels = {"NEW": "New", "CHANGED": "Changed", "RECURRING": "Recurring",
                  "NOT_RAISED": "Not raised", "NOT_COMPARABLE": "Not comparable"}

        def comparison_table(title: str, headings: tuple[str, ...], body: str) -> str:
            return (
                '<div class="journal-scroll comparison" tabindex="0" role="region" '
                f'aria-label="{esc(title)}"><table><caption>{esc(title)}</caption><thead><tr>'
                + "".join('<th scope="col"{}>{}</th>'.format(
                    ' class="amount"' if label in ("Finding groups", "Queries") or label.startswith("Value in ") else "",
                    esc(label)) for label in headings)
                + f"</tr></thead><tbody>{body}</tbody></table></div>"
            )

        def finding_values(items: list[dict[str, str]]) -> str:
            if not items:
                return "No finding raised in this run."
            return "".join(
                '<div class="comparison-value">'
                f'<span class="amount">{esc(item["current_value"]) if item["current_value"] else "Empty producer value"}</span>'
                f'<span class="control">Status: {esc(item["status"])}</span></div>' for item in items
            )

        def comparison_list(title: str, body: str) -> str:
            return f'<h3>{esc(title)}</h3><ul class="comparison-records" role="list">{body}</ul>'

        counts = "".join(
            f'<tr><th scope="row" data-change="{key}">{esc(label)}</th>'
            + "".join(f'<td class="amount">{sum(row["change"] == key for row in changes[group])}</td>'
                      for group in ("findings", "queries")) + "</tr>"
            for key, label in labels.items()
        )
        finding_changes = "".join(
            f'<li><h4>{esc((row["current"] or row["previous"])[0]["account_name"])}</h4>'
            f'<p class="control">{esc(row["control"])} · {esc(row["account_id"])}</p>'
            f'<dl><div><dt>Change</dt><dd>{esc(labels.get(row["change"], row["change"]))}</dd></div>'
            f'<div class="comparison-period"><dt>Value in {esc(old_receipt["period"])}</dt><dd>{finding_values(row["previous"])}</dd></div>'
            f'<div class="comparison-period"><dt>Value in {esc(receipt["period"])}</dt><dd>{finding_values(row["current"])}</dd></div></dl></li>'
            for row in changes["findings"]
        )
        query_changes = "".join(
            f'<li><h4>{esc((row["current"] or row["previous"])["account_name"])}</h4>'
            f'<p class="control">{esc((row["current"] or row["previous"])["control"])} · '
            f'{esc((row["current"] or row["previous"])["account_id"])}</p>'
            f'<dl><div><dt>Change</dt><dd>{esc(labels.get(row["change"], row["change"]))}</dd></div>'
            f'<div><dt>Query ID</dt><dd>{esc(row["query_id"])}</dd></div></dl></li>'
            for row in changes["queries"]
        )
        scope = ", ".join(changes["scope_changes"]) or "No scope changes reported by the producer."
        inputs = ", ".join(changes["changed_inputs"]) or "No input file changes recorded."
        comparison_html = (
            f'<p>Previous run: <strong>{esc(old_receipt["period"])}</strong>. '
            f'Current run: <strong>{esc(receipt["period"])}</strong>.</p>'
            f'<p>{esc(changes["meaning"])}</p>'
            + comparison_table("Change counts", ("Classification", "Finding groups", "Queries"), counts)
            + '<p>Change labels compare complete records. Changed does not necessarily mean a balance '
            'or status changed. Recurring means the complete record is identical.</p>'
            f'<dl><dt>Scope changes</dt><dd>{esc(scope)}</dd><dt>Changed input files</dt><dd>{esc(inputs)}</dd>'
            f'<dt>Acknowledgement changed</dt><dd>{"Yes" if changes["acknowledgement_changed"] else "No"}</dd></dl>'
            + (comparison_list("Finding comparison", finding_changes)
               if finding_changes else "<p>No finding records occur in either run.</p>")
            + '<p>Values and statuses retain the producer\'s exact text and units. Each record is '
            'shown separately. Full reasons, actions and query details appear in the full comparison record.</p>'
            + (comparison_list("Query comparison", query_changes)
               if query_changes else "<p>No query records occur in either run.</p>")
            + '<nav class="finding-nav" aria-label="Comparison details"><a href="#comparison-record">'
            'Read full comparison record</a></nav>'
        )
    findings = rows(projected["sample-review-exceptions.csv"])
    evidence = rows(projected["sample-review-evidence.csv"])
    context = rows(projected["sample-review-run.csv"])[0]
    index_links = []
    sections = []
    fields = (
        ("Action", "Review action"), ("Reason", "Reason"), ("Account", "Account"),
        ("Status", "Status"), ("Current", "Current"), ("Prior", "Prior"),
        ("Difference", "Difference"), ("Threshold", "Threshold"), ("Question", "Question"),
        ("EvidenceRequested", "Evidence requested"), ("EvidenceState", "Evidence state"),
    )
    for number, item in enumerate(findings, 1):
        anchor = f"finding-{number}"
        index_links.append(
            f'<li><a href="#{anchor}">{esc(item["Account"])}'
            f'<span>{esc(item["Control"])} · {esc(item["Status"])}</span></a></li>'
        )
        detail = "".join(f"<dt>{esc(label)}</dt><dd>{esc(item[key])}</dd>" for key, label in fields)
        journal = [row for row in evidence if row["ExceptionKey"] == item["ExceptionKey"]]
        journal_count = f"{len(journal)} reconciled journal {'row' if len(journal) == 1 else 'rows'}"
        lines = "".join(
            "<tr>" + "".join(
                f'<td class="amount">{esc(row[key])}</td>' if key == "Amount"
                else f"<td>{esc(row[key])}</td>"
                for key in ("TransactionID", "Date", "Reference", "Description", "Amount")
            ) + "</tr>" for row in journal
        )
        if journal:
            journal_html = (
                '<p class="scroll-hint">On a narrow screen, scroll the journal table sideways '
                'to read every column.</p><div class="journal-scroll" tabindex="0" role="region" '
                f'aria-label="Journal evidence for finding {number}"><table>'
                f'<caption>{journal_count}. Amounts in {esc(context.get("Currency", "AUD"))}.</caption>'
                '<thead><tr><th scope="col">Journal line</th><th scope="col">Date</th>'
                '<th scope="col">Reference</th><th scope="col">Description</th>'
                f'<th scope="col" class="amount">Amount</th></tr></thead><tbody>{lines}</tbody>'
                '</table></div>'
            )
        else:
            journal_html = '<p>No journal rows are supplied for this finding. See the evidence state above.</p>'
        following = (
            f'<a href="#finding-{number + 1}">Next finding: {esc(findings[number]["Account"])}</a>'
            if number < len(findings) else ""
        )
        sections.append(
            f'<article class="finding" id="{anchor}" tabindex="-1" aria-labelledby="{anchor}-title">'
            f'<h2 id="{anchor}-title">{number}. {esc(item["Account"])}</h2>'
            f'<p class="control">Control: {esc(item["Control"])}</p><dl>{detail}</dl>'
            f'<h3>Journal evidence</h3>{journal_html}'
            f'<nav class="finding-nav" aria-label="Navigation after finding {number}">'
            f'<a href="#finding-index">Back to finding index</a>{following}</nav></article>'
        )
    metadata = "".join(f"<dt>{esc(k)}</dt><dd>{esc(v)}</dd>" for k, v in context.items())
    heading_context = " · ".join(esc(context[key]) for key in ("Tenant", "Period", "Basis") if key in context)
    return f'''<!doctype html>
<html lang="en-AU"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'">
<title>Fabricated close review</title><style>
:root{{color-scheme:light dark;--page:#f3f5f6;--panel:#ffffff;--text:#182b32;--muted:#43535b;--line:#cad4d8;--link:#075f67;--focus:#b95715}}
*{{box-sizing:border-box}}body{{font:17px/1.5 system-ui,sans-serif;margin:0;background:var(--page);color:var(--text)}}
main{{max-width:1100px;margin:auto;padding:32px}}article,header,section{{background:var(--panel);padding:24px;margin:24px 0;border:1px solid var(--line);border-radius:8px}}
h1,h2,h3,h4{{line-height:1.25;text-wrap:balance;overflow-wrap:anywhere}}h1,h2{{margin-top:0}}h1{{font-size:2rem}}h2{{font-size:1.5rem}}h3{{font-size:1.125rem}}
p{{max-width:65ch}}a{{color:var(--link);text-underline-offset:3px}}a:hover{{text-decoration-thickness:2px}}a:active{{background:var(--page)}}
:focus-visible{{outline:3px solid var(--focus);outline-offset:4px}}.finding:target{{outline:3px solid var(--link);outline-offset:4px}}
dl{{display:grid;grid-template-columns:176px minmax(0,1fr);gap:8px 16px}}dt{{font-weight:650;overflow-wrap:anywhere}}dd{{margin:0;overflow-wrap:anywhere;white-space:pre-wrap}}
.control,.scroll-hint{{color:var(--muted)}}.index{{padding-left:32px}}.index li{{padding:0 0 8px 8px}}.index a{{display:block;min-height:44px;padding:8px;overflow-wrap:anywhere}}
.index span{{display:block;font-size:0.875rem;color:var(--muted);font-weight:400}}.finding-nav,.section-nav{{display:flex;flex-wrap:wrap;gap:8px 24px;margin-top:24px}}
.finding-nav a,.section-nav a{{display:flex;align-items:center;min-height:44px;padding:8px 0;overflow-wrap:anywhere}}
.journal-scroll{{overflow-x:auto;max-width:100%}}table{{border-collapse:collapse;width:100%;min-width:640px;font-size:14px}}
caption{{text-align:left;padding:8px 0;font-weight:650}}th,td{{padding:8px;text-align:left;border-bottom:1px solid var(--line);vertical-align:top;overflow-wrap:anywhere}}
.amount{{text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}}pre{{font-size:14px;white-space:pre-wrap;overflow-wrap:anywhere}}
.comparison table{{min-width:0}}.comparison th{{overflow-wrap:normal}}
.comparison-records{{list-style:none;padding:0}}.comparison-records li{{padding:16px 0;border-bottom:1px solid var(--line)}}.comparison-records li:last-child{{border-bottom:0}}.comparison-records h4{{font-size:1.125rem;margin:0}}.comparison-records p{{margin:4px 0 8px;overflow-wrap:anywhere}}
.comparison-records dl{{grid-template-columns:repeat(3,minmax(0,1fr));gap:16px;margin:0}}.comparison-records dd{{margin:4px 0 0}}.comparison-records dl>div{{min-width:0}}.comparison-period{{text-align:right}}
.comparison-value span{{display:block;white-space:normal;overflow-wrap:anywhere}}.comparison-value+.comparison-value{{margin-top:8px}}
@media screen and (max-width:650px){{.comparison-records dl{{display:grid;grid-template-columns:minmax(0,1fr)}}}}
@media(min-width:651px){{.scroll-hint{{display:none}}}}
@media(max-width:650px){{main{{padding:12px}}article,header,section{{padding:16px}}h1{{font-size:1.75rem}}dl{{display:block}}dd{{margin:4px 0 16px}}.finding-nav{{display:block}}.finding-nav a{{margin-top:8px}}.section-nav{{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px 16px}}.section-nav a{{padding:8px;min-height:64px;font-size:16px}}}}
@media(prefers-color-scheme:dark){{:root{{--page:#102229;--panel:#182f36;--text:#eef4f6;--muted:#bacdd3;--line:#56717b;--link:#87dbdf;--focus:#efb06a}}}}
@media print{{:root{{color-scheme:light;--page:white;--panel:white;--text:black;--muted:#333;--line:#777;--link:black}}body{{font-size:11pt}}main{{max-width:none;padding:0}}article,header,section{{border:0;border-radius:0;padding:0}}nav,.finding-nav,.section-nav,.scroll-hint,#finding-index{{display:none}}.finding:target,:focus-visible{{outline:0}}h2,h3,h4,dt,.finding>.control,caption,thead{{break-after:avoid}}tr{{break-inside:avoid}}.comparison-records li{{break-inside:avoid}}table{{min-width:0;font-size:10pt}}.journal-scroll{{overflow:visible}}pre{{font-size:9pt}}}}
</style></head><body><main>
<header><h1>Exceptions requiring review</h1><p>{heading_context}</p><p>{esc(BOUNDARY)}</p><p><strong>{len(findings)} findings</strong>. Journal amounts are in {esc(context.get("Currency", "AUD"))}. Review values retain the producer's units.</p><nav class="section-nav" aria-label="Report sections"><a href="#finding-index">Finding index</a><a href="#run-context">Run context</a><a href="#run-changes">Run changes</a><a href="#run-hashes">Input and result hashes</a></nav></header>
<section id="finding-index" tabindex="-1" aria-labelledby="index-title"><h2 id="index-title">Finding index</h2><p>Choose a finding to read its review action and journal evidence. Findings retain the producer's order.</p><nav aria-label="Findings"><ol class="index">{''.join(index_links)}</ol></nav></section>
{''.join(sections)}
<section id="run-context" tabindex="-1"><h2>Run context and control coverage</h2><p>Amounts retain the producer's exact text. Self-review assertions and any demonstration note are fabricated.</p><dl>{metadata}</dl></section>
<section id="run-changes" tabindex="-1"><h2>Changes from the previous verified run</h2>{comparison_html}</section>
{f'<section id="comparison-record" tabindex="-1"><h2>Full comparison record</h2><pre>{esc(change)}</pre><nav class="finding-nav" aria-label="Return from comparison record"><a href="#run-changes">Back to comparison summary</a></nav></section>' if previous else ''}
<section id="run-hashes" tabindex="-1"><h2>Input and result hashes</h2><p>Local hash receipts detect accidental edits. They are unsigned and can be replaced by anyone who can replace the whole run.</p><pre>{esc(json.dumps(receipt['files'], indent=2))}</pre></section>
</main></body></html>'''


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("build", "html", "fixtures", "verify"))
    parser.add_argument("--run", type=Path, required=True)
    parser.add_argument("--bin-dir", type=Path, default=APP.parents[1] / ".venv/Scripts")
    parser.add_argument("--month", type=int, default=9)
    parser.add_argument("--review-note", type=Path)
    parser.add_argument("--previous", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "build":
            build(args.run, args.bin_dir, args.month, args.review_note)
        elif args.command == "verify":
            verify(args.run)
        elif args.command == "html":
            if args.output is None:
                raise ValueError("--output is required.")
            output = new_output(args.output)
            content = render_html(args.run, args.bin_dir, args.previous)
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_text(content, encoding="utf-8")
        else:
            if args.output is None:
                raise ValueError("--output is required.")
            output = new_output(args.output)
            values = projection(args.run)
            output.mkdir(parents=True)
            for name, value in values.items():
                (output / name).write_bytes(value)
        print("Review workflow verification completed.")
    except (ValueError, OSError, KeyError, json.JSONDecodeError) as exc:
        parser.exit(1, f"Review workflow refused: {exc}\n")


if __name__ == "__main__":
    main()
