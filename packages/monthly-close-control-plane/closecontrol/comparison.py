"""Compare verified close runs without changing either evidence pack."""
from __future__ import annotations

import json
from collections import defaultdict
from datetime import date
from pathlib import Path
from typing import Any, cast

from .errors import ControlInputError
from .loader import SourceSnapshot, load_canonical_tb
from .viewer import _no_duplicate_keys, verify_pack

_BOUNDARY = (
    "A comparison records changes between supplied runs. NOT_RAISED does not mean "
    "resolved or approved. Responses do not alter findings or close a period."
)


def _verified(pack_path: Path, tb_path: Path) -> tuple[dict[str, Any], dict[str, str], str, frozenset[str]]:
    verified, _, _, _, hashes = verify_pack(pack_path)
    document = cast(dict[str, Any], verified)
    source = SourceSnapshot.capture(tb_path, label="Comparison trial balance")
    if document["source_sha256"].get("current_trial_balance") != source.sha256:
        raise ControlInputError("Comparison trial balance does not match the pack's current source.")
    rows = load_canonical_tb(source)
    if document["current_report_dates"] != [rows[0].report_date.isoformat()]:
        raise ControlInputError("Comparison trial-balance date does not match the pack.")
    tenant = rows[0].tenant
    for item in document["exceptions"]:
        if item["tenant"] and item["tenant"] != tenant:
            raise ControlInputError("Pack finding names another tenant.")
    for item in document.get("client_queries", []):
        if not item.get("tenant") or item["tenant"] != tenant:
            raise ControlInputError("Pack query names another tenant.")
    return document, hashes, tenant, frozenset(row.account_id for row in rows)


def _groups(document: dict[str, Any]) -> dict[tuple[str, str, str], list[dict[str, str]]]:
    groups: dict[tuple[str, str, str], list[dict[str, str]]] = defaultdict(list)
    for item in document["exceptions"]:
        key = (item["control"], item["tenant"], item["account_id"])
        groups[key].append(item)
    return {key: sorted(items, key=lambda item: json.dumps(item, sort_keys=True))
            for key, items in groups.items()}


def _scope_changes(previous: dict[str, Any], current: dict[str, Any]) -> list[str]:
    changes = []
    for field in ("controls_not_run", "thresholds"):
        if previous.get(field) != current.get(field):
            changes.append(field)
    # Older packs did not state omitted controls. Unknown coverage is not equal coverage.
    if "controls_not_run" not in previous or "controls_not_run" not in current:
        changes.append("control_coverage_unknown")
    for label in ("account_mapping", "mapping_policy"):
        if previous["source_sha256"].get(label) != current["source_sha256"].get(label):
            changes.append(label)
    # A digest alone cannot prove that changed subledger files cover the same accounts.
    if previous["source_sha256"].get("subledger") != current["source_sha256"].get("subledger"):
        changes.append("subledger_population_not_verified")
    old_evidence = previous.get("calculation_evidence", {})
    new_evidence = current.get("calculation_evidence", {})
    if old_evidence.get("required", []) != new_evidence.get("required", []):
        changes.append("required_calculations")
    old_equity = previous.get("equity_reconciliation")
    new_equity = current.get("equity_reconciliation")
    if (old_equity is None) != (new_equity is None):
        changes.append("equity_control_coverage")
    elif old_equity is not None and new_equity is not None:
        if (old_equity["currency"] != new_equity["currency"]
                or [row["account_id"] for row in old_equity["accounts"]]
                != [row["account_id"] for row in new_equity["accounts"]]):
            changes.append("equity_account_scope")
    if previous["current_report_dates"] != current["current_report_dates"]:
        if previous["current_report_dates"] != current["prior_report_dates"]:
            changes.append("periods_not_contiguous")
    old_reset = any(x["control"] == "financial_year_reset" for x in previous["exceptions"])
    new_reset = any(x["control"] == "financial_year_reset" for x in current["exceptions"])
    if old_reset != new_reset:
        changes.append("financial_year_reset")
    if "BLOCKED" in (previous["overall_status"], current["overall_status"]):
        changes.append("blocked_run")
    return changes


def _responses(path: Path | None, pack_hash: str,
               queries: dict[str, dict[str, str]], period: str) -> tuple[list[dict], str | None]:
    if path is None:
        return [], None
    source = SourceSnapshot.capture(path, label="Response file")
    try:
        document = json.loads(source.text(label="Response file", encoding="utf-8-sig"),
                              object_pairs_hook=_no_duplicate_keys)
    except (ValueError, UnicodeError) as exc:
        raise ControlInputError(f"Invalid response JSON: {exc}") from exc
    if (not isinstance(document, dict)
            or set(document) != {"schema_version", "pack_sha256", "responses"}
            or type(document["schema_version"]) is not int
            or document["schema_version"] != 1
            or document["pack_sha256"] != pack_hash
            or not isinstance(document["responses"], list)):
        raise ControlInputError("Responses must use schema 1 and bind to the current pack JSON digest.")
    seen: set[str] = set()
    result = []
    fields = {"query_id", "explanation", "evidence_reference", "reviewer", "reviewed_on"}
    for row in document["responses"]:
        if not isinstance(row, dict) or set(row) != fields:
            raise ControlInputError("A response must contain exactly the documented response fields.")
        if any(not isinstance(value, str) or not value.strip() for value in row.values()):
            raise ControlInputError("Every response field must be a non-empty string.")
        key = row["query_id"]
        if key in seen or key not in queries:
            raise ControlInputError("Response query_id is duplicated or absent from the current pack.")
        try:
            reviewed = date.fromisoformat(row["reviewed_on"])
        except ValueError as exc:
            raise ControlInputError("Response reviewed_on must be an ISO date.") from exc
        if reviewed.isoformat() != row["reviewed_on"] or reviewed < date.fromisoformat(period):
            raise ControlInputError("Response date must be canonical and not precede the current close.")
        seen.add(key)
        result.append(row)
    return sorted(result, key=lambda row: row["query_id"]), source.sha256


def compare_packs(*, previous_pack: Path, current_pack: Path,
                  previous_tb: Path, current_tb: Path,
                  responses: Path | None = None) -> dict[str, Any]:
    """Return JSON-ready evidence; verification reads each pack once and writes nothing."""
    previous, old_hashes, old_tenant, old_accounts = _verified(previous_pack, previous_tb)
    current, new_hashes, tenant, new_accounts = _verified(current_pack, current_tb)
    if old_tenant != tenant:
        raise ControlInputError("Compared packs must belong to the same tenant.")
    old_date = previous["current_report_dates"][0]
    new_date = current["current_report_dates"][0]
    if date.fromisoformat(new_date) < date.fromisoformat(old_date):
        raise ControlInputError("Current pack cannot precede the previous pack.")
    coverage = _scope_changes(previous, current)
    if old_accounts != new_accounts:
        coverage.append("trial_balance_account_population")
    old_groups, new_groups = _groups(previous), _groups(current)
    findings = []
    for key in sorted(old_groups.keys() | new_groups.keys()):
        before, after = old_groups.get(key, []), new_groups.get(key, [])
        if not before:
            change = "NEW"
        elif not after:
            change = "NOT_COMPARABLE" if coverage else "NOT_RAISED"
        else:
            change = "RECURRING" if before == after else "CHANGED"
        findings.append({"control": key[0], "tenant": key[1], "account_id": key[2],
                         "change": change, "previous": before, "current": after})
    old_queries = {row["query_id"]: row for row in previous.get("client_queries", [])}
    new_queries = {row["query_id"]: row for row in current.get("client_queries", [])}
    query_changes = []
    for key in sorted(old_queries.keys() | new_queries.keys()):
        old_query, new_query = old_queries.get(key), new_queries.get(key)
        change = ("NEW" if old_query is None else
                  ("NOT_COMPARABLE" if coverage else "NOT_RAISED") if new_query is None else
                  "RECURRING" if old_query == new_query else "CHANGED")
        query_changes.append({"query_id": key, "change": change,
                              "previous": old_query, "current": new_query})
    notes, note_hash = _responses(responses, new_hashes["close-review-pack.json"],
                                  new_queries, new_date)
    return {
        "schema_version": 1, "tenant": tenant, "previous_period": old_date,
        "current_period": new_date, "scope_changes": coverage,
        "previous_status": previous["overall_status"], "current_status": current["overall_status"],
        "findings": findings, "queries": query_changes, "responses": notes,
        "response_sha256": note_hash, "previous_artefact_sha256": old_hashes,
        "current_artefact_sha256": new_hashes, "review_boundary": _BOUNDARY,
    }
