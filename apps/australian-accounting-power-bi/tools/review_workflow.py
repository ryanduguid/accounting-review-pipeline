"""Build and display one pinned, offline fabricated review case through existing CLIs."""
from __future__ import annotations

import argparse
import ast
import calendar
import csv
import hashlib
import html
import io
import json
import math
import os
import re
import shlex
import stat
import subprocess
import tempfile
import zipfile
from contextlib import contextmanager
from datetime import datetime, timezone
from decimal import Context, Decimal, DecimalException, localcontext
from pathlib import Path, PureWindowsPath
from typing import Any, Iterator

APP = Path(__file__).resolve().parents[1]
NAME = "australian-accounting-power-bi"
TB_COLUMNS = "ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit".split(",")
CLOSE_FILES = ("close-review-pack.json", "close-summary.md", "exceptions.csv", "client-queries.csv")
READY_FILES = ("readiness-pack.json", "readiness-summary.md", "findings.csv")
BOUNDARY = "Fabricated demonstration. Verification and acknowledgement do not approve accounting or close a period."
PRODUCERS = {
    "close-control": ("monthly-close-control-plane", "closecontrol", "closecontrol.cli:main"),
    "review-ready": ("review-ready-gate", "reviewready", "reviewready.cli:main"),
}


def digest(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def json_bytes(value: Any) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode("utf-8")


def strict_json(data: bytes | str, label: str) -> Any:
    """Refuse ambiguous or excessively nested JSON before consumers index it."""
    def members(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
        result: dict[str, Any] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("Duplicate object member.")
            result[key] = value
        return result

    def constant(value: str) -> Any:
        raise ValueError(f"Non-standard numeric token: {value}")

    def floating(token: str) -> float:
        value = float(token)
        if not math.isfinite(value):
            raise ValueError("JSON number is outside the finite float range.")
        return value

    try:
        text = data.decode("utf-8-sig") if isinstance(data, bytes) else data
        depth, quoted, escaped = 0, False, False
        for character in text:
            if quoted:
                if escaped:
                    escaped = False
                elif character == "\\":
                    escaped = True
                elif character == '"':
                    quoted = False
            elif character == '"':
                quoted = True
            elif character in "[{":
                depth += 1
                if depth > 64:
                    raise ValueError("JSON nesting exceeds 64 levels.")
            elif character in "]}":
                depth -= 1
        return json.loads(text, object_pairs_hook=members, parse_constant=constant, parse_float=floating)
    except (UnicodeError, ValueError, RecursionError) as exc:
        raise ValueError(f"{label}: {exc}") from exc


def string_fields(record: Any, names: tuple[str, ...], label: str) -> dict[str, Any]:
    if not isinstance(record, dict) or any(not isinstance(record.get(name), str) for name in names):
        raise ValueError(f"{label} requires an object with string fields: {', '.join(names)}.")
    return record


def object_rows(value: Any, label: str) -> list[dict[str, Any]]:
    if not isinstance(value, list) or any(not isinstance(row, dict) for row in value):
        raise ValueError(f"{label} requires a list of objects.")
    return value


def string_map(value: Any, label: str, *, hashes: bool = False, required: tuple[str, ...] = ()) -> dict[str, str]:
    if not isinstance(value, dict) or any(not isinstance(key, str) or not isinstance(item, str)
                                          or (hashes and not re.fullmatch(r"[a-f0-9]{64}", item))
                                          for key, item in value.items()):
        raise ValueError(f"{label} requires a string map{' of lowercase SHA-256 values' if hashes else ''}.")
    if not set(required).issubset(value):
        raise ValueError(f"{label} is missing required fields.")
    return value


def case_document(value: Any) -> dict[str, Any]:
    case = string_fields(value, ("case_id", "entity", "tenant", "basis", "currency",
                                "financial_year_start", "opening_journal", "population"), "Case")
    string_map(case.get("account_ids"), "Case account identifiers")
    string_map(case.get("source_sha256"), "Case source hashes", hashes=True)
    return case


def money_text(value: Any, label: str, *, nullable: bool = False) -> str:
    """Match the model's fixed-case display grammar without converting or rounding."""
    if not isinstance(value, str) or not ((nullable and value == "") or re.fullmatch(r"-?[0-9]+\.[0-9]{2}", value)):
        raise ValueError(f"{label} requires a two-decimal money string.")
    return value


def close_document(value: Any) -> dict[str, Any]:
    doc = string_fields(value, ("overall_status",), "Close pack")
    string_map(doc.get("source_sha256"), "Close source hashes", hashes=True, required=("current_trial_balance", "prior_trial_balance"))
    for name in ("current_report_dates", "prior_report_dates", "controls_not_run"):
        items = doc.get(name)
        if not isinstance(items, list) or any(not isinstance(item, str) for item in items):
            raise ValueError(f"Close {name} requires a list of strings.")
    if not doc["current_report_dates"] or not doc["prior_report_dates"]:
        raise ValueError("Close report dates cannot be empty.")
    if "acknowledgement" not in doc or (doc["acknowledgement"] is not None and not isinstance(doc["acknowledgement"], dict)):
        raise ValueError("Close acknowledgement requires an object or null.")
    thresholds = string_fields(doc.get("thresholds"), ("absolute_variance", "percentage_variance"), "Close thresholds")
    money_text(thresholds["absolute_variance"], "Absolute threshold")
    if "reconciliation_tolerance" in thresholds:
        money_text(thresholds["reconciliation_tolerance"], "Reconciliation tolerance")
    if not re.fullmatch(r"[0-9]+\.[0-9]{2,}%", thresholds["percentage_variance"]):
        raise ValueError("Percentage threshold requires decimal percentage text.")
    for item in object_rows(doc.get("exceptions"), "Close exceptions"):
        string_fields(item, ("control", "tenant", "account_id", "account_name", "status", "current_value",
                             "prior_value", "difference", "threshold", "reason", "reviewer_action"), "Close exception")
        for name in ("current_value", "prior_value", "difference", "threshold"):
            money_text(item[name], f"Close {name}", nullable=True)
    query_ids, query_keys = set(), set()
    for item in object_rows(doc.get("client_queries"), "Close queries"):
        string_fields(item, ("query_id", "control", "tenant", "account_id", "question", "evidence_requested"), "Close query")
        key = (item["control"], item["tenant"], item["account_id"])
        if not item["query_id"] or item["query_id"] in query_ids or key in query_keys:
            raise ValueError("Duplicate or empty close query identity.")
        query_ids.add(item["query_id"])
        query_keys.add(key)
    return doc


def driver_document(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError("Driver pack requires an object.")
    string_map(value.get("source_sha256"), "Driver source hashes", hashes=True, required=("transactions", *("pack:" + name for name in CLOSE_FILES)))
    for account in object_rows(value.get("accounts"), "Driver accounts"):
        string_fields(account, ("tenant", "account_id", "movement", "transactions_total", "unexplained"), "Driver account")
        if type(account.get("transactions_in_window")) is not int or account["transactions_in_window"] < 0:
            raise ValueError("Driver transaction count requires a non-negative integer.")
        amounts = [account[name] for name in ("movement", "transactions_total", "unexplained")]
        for row in object_rows(account.get("drivers"), "Driver rows"):
            string_fields(row, ("TransactionID", "Date", "Reference", "Description", "Amount"), "Driver row")
            amounts.append(row["Amount"])
        for amount in amounts:
            money_text(amount, "Driver amount")
    return value


def readiness_document(value: Any) -> dict[str, Any]:
    doc = string_fields(value, ("overall_status",), "Readiness pack")
    for control in object_rows(doc.get("controls_not_run"), "Readiness controls not run"):
        names = ("slot", "filename", "reason")
        string_fields(control, names, "Readiness control not run")
        if set(control) != set(names) or any(not control[name] for name in names):
            raise ValueError("Readiness controls not run requires exactly its non-empty producer fields.")
    return doc


def comparison_document(value: Any) -> dict[str, Any]:
    doc = string_fields(value, ("review_boundary",), "Comparison")
    previous: Any = None
    current: Any = None
    for group in ("findings", "queries"):
        for item in object_rows(doc.get(group), f"Comparison {group}"):
            string_fields(item, ("change",), f"Comparison {group} record")
            if item["change"] not in ("NEW", "CHANGED", "RECURRING", "NOT_RAISED", "NOT_COMPARABLE"):
                raise ValueError("Unrecognised comparison classification.")
            if group == "findings":
                string_fields(item, ("control", "account_id"), "Finding comparison")
                previous = object_rows(item.get("previous"), "Previous findings")
                current = object_rows(item.get("current"), "Current findings")
                for row in previous + current:
                    string_fields(row, ("account_name", "current_value", "status"), "Displayed finding")
            else:
                string_fields(item, ("query_id",), "Query comparison")
                previous, current = item.get("previous"), item.get("current")
                for row in (previous, current):
                    if row is not None:
                        string_fields(row, ("account_name", "control", "account_id"), "Displayed query")
            if not previous and not current:
                raise ValueError("Comparison requires at least one populated side.")
            change = item["change"]
            if ((change == "NEW" and (previous or not current))
                    or (change in ("NOT_RAISED", "NOT_COMPARABLE") and (not previous or current))
                    or (change in ("CHANGED", "RECURRING") and
                        (not previous or not current or (previous == current) != (change == "RECURRING")))):
                raise ValueError("Comparison classification contradicts its previous/current records.")
    scope = doc.get("scope_changes")
    if not isinstance(scope, list) or any(not isinstance(item, str) for item in scope):
        raise ValueError("Comparison scope changes requires a list of strings.")
    return doc


def producer_document(value: Any, command: str) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != {"manifest_version", "command", "launcher_name", "launcher_sha256", "interpreter", "distribution", "package"}:
        raise ValueError("Producer manifest requires its complete identity fields; rebuild the review run.")
    if type(value["manifest_version"]) is not int or value["manifest_version"] != 1 or value["command"] != command:
        raise ValueError("Unsupported producer manifest.")
    string_fields(value, ("launcher_name", "launcher_sha256"), "Producer launcher")
    if value["launcher_name"] not in (command, command + ".exe"):
        raise ValueError("Producer launcher name differs.")
    interpreter = string_fields(value["interpreter"], ("implementation", "version", "cache_tag", "executable_sha256"), "Producer interpreter")
    if set(interpreter) != {"implementation", "version", "cache_tag", "executable_sha256"}:
        raise ValueError("Unsupported interpreter identity fields.")
    distribution = string_fields(value["distribution"], ("name", "version", "requires_python", "entry_point"), "Producer distribution")
    name, package, entry = PRODUCERS[command]
    if set(distribution) != {"name", "version", "requires_python", "entry_point"} or distribution["name"] != name or distribution["entry_point"] != entry:
        raise ValueError("Producer distribution contract differs.")
    module = string_fields(value["package"], ("name",), "Producer package")
    if set(module) != {"name", "files"} or module["name"] != package:
        raise ValueError("Producer package contract differs.")
    files = string_map(module.get("files"), "Producer package files", hashes=True)
    if not files or any(not re.fullmatch(r"[a-zA-Z0-9_.-]+(?:/[a-zA-Z0-9_.-]+)*", filename) or any(part in (".", "..", "__pycache__") for part in filename.split("/")) for filename in files):
        raise ValueError("Producer package file paths differ.")
    if any(not re.fullmatch(r"[a-f0-9]{64}", value) for value in (value["launcher_sha256"], interpreter["executable_sha256"])):
        raise ValueError("Producer executable hashes require lowercase SHA-256 values.")
    return value


def receipt_document(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != {"schema_version", "case", "period", "invocations", "files", "boundary", "producer_manifests"}:
        raise ValueError("Receipt requires complete producer identities; rebuild the review run.")
    if type(value["schema_version"]) is not int or value["schema_version"] != 1 or value["boundary"] != BOUNDARY:
        raise ValueError("Unsupported review receipt.")
    case_document(value["case"])
    string_fields(value, ("period", "boundary"), "Receipt")
    if not re.fullmatch(r"2024-(08-31|09-30)", value["period"]):
        raise ValueError("Receipt period is outside the fixed review case.")
    string_map(value["files"], "Receipt files", hashes=True)
    manifests = value["producer_manifests"]
    if not isinstance(manifests, dict) or set(manifests) != set(PRODUCERS):
        raise ValueError("Receipt requires exactly the two producer manifests.")
    for command, manifest in manifests.items():
        producer_document(manifest, command)
    for invocation in object_rows(value["invocations"], "Receipt invocations"):
        string_fields(invocation, ("command", "executable_sha256", "producer_manifest_sha256", "started", "finished"), "Receipt invocation")
        if set(invocation) != {"command", "args", "executable_sha256", "producer_manifest_sha256", "started", "finished", "exit"} or type(invocation["exit"]) is not int:
            raise ValueError("Receipt invocation fields differ.")
        if not isinstance(invocation["args"], list) or any(not isinstance(arg, str) for arg in invocation["args"]):
            raise ValueError("Receipt invocation arguments require strings.")
        command = invocation["command"]
        if command not in PRODUCERS or invocation["producer_manifest_sha256"] != digest(json_bytes(manifests[command])) or invocation["executable_sha256"] != manifests[command]["launcher_sha256"]:
            raise ValueError("Receipt invocation producer identity differs.")
    return value


def rows(value: bytes) -> list[dict[str, str]]:
    return list(csv.DictReader(io.StringIO(value.decode("utf-8-sig"))))


def csv_bytes(columns: list[str], values: list[dict[str, Any]]) -> bytes:
    stream = io.StringIO(newline="")
    writer = csv.DictWriter(stream, columns, lineterminator="\n")
    writer.writeheader()
    writer.writerows(values)
    return stream.getvalue().encode("utf-8")


def ordinary(path: Path) -> Path:
    windows_drive = PureWindowsPath(str(path)).drive
    if windows_drive.upper() == "Z:" or windows_drive.startswith("\\\\"):
        raise ValueError("Use a local directory.")
    path = path.absolute()
    if str(path).startswith("\\\\") or path.drive.upper() == "Z:":
        raise ValueError("Use a local directory.")
    current = Path()
    missing_depth = 0
    for component in path.parts:
        if component == "..":
            current = current.parent
            missing_depth = max(0, missing_depth - 1)
            continue
        current /= component
        if missing_depth:
            missing_depth += 1
            continue
        try:
            metadata = current.lstat()
        except FileNotFoundError:
            missing_depth = 1
            continue
        if stat.S_ISLNK(metadata.st_mode) or getattr(metadata, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
            raise ValueError("Linked paths are not admitted.")
    return path.resolve()


def new_output(path: Path) -> Path:
    path = ordinary(path)
    if path.exists():
        raise ValueError("Output must be new.")
    if any((parent / marker).exists() for parent in path.parents for marker in (".git", ".hg", ".svn", ".bzr")):
        raise ValueError("Generated reviews must be outside version control.")
    return path


def launcher_script(source: bytes, command: str) -> None:
    """Recognise the two normal console-script bodies without executing them."""
    module = PRODUCERS[command][2].split(":")[0]
    try:
        tree = ast.parse(source.decode("utf-8"))
    except (UnicodeError, SyntaxError, ValueError, RecursionError) as exc:
        raise ValueError("Producer entry-point script is malformed.") from exc
    body = list(tree.body)
    if body and isinstance(body[0], ast.Expr) and isinstance(body[0].value, ast.Constant) and isinstance(body[0].value.value, str):
        body.pop(0)
    if body and isinstance(body[-1], ast.If) and all(isinstance(node, (ast.Import, ast.ImportFrom)) for node in body[:-1]):
        guarded = body[-1]
        if ast.dump(guarded.test) != ast.dump(ast.parse('__name__ == "__main__"', mode="eval").body) or guarded.orelse:
            raise ValueError("Producer script guard differs.")
        body = body[:-1] + guarded.body
    imports = 0
    while body and isinstance(body[0], (ast.Import, ast.ImportFrom)):
        node = body.pop(0)
        if isinstance(node, ast.Import):
            if any(alias.name not in ("sys", "re") or alias.asname is not None for alias in node.names):
                raise ValueError("Producer script has an unsupported import.")
        elif isinstance(node, ast.ImportFrom) and node.module == module and node.level == 0 and len(node.names) == 1 and node.names[0].name == "main" and node.names[0].asname is None:
            imports += 1
        else:
            raise ValueError("Producer script entry point differs.")
    if imports != 1:
        raise ValueError("Producer script requires its exact entry-point import.")
    normalisations = (
        'sys.argv[0] = re.sub(r"(-script\\.pyw|\\.exe)?$", "", sys.argv[0])',
        'if sys.argv[0].endswith("-script.pyw"):\n    sys.argv[0] = sys.argv[0][:-11]\nelif sys.argv[0].endswith(".exe"):\n    sys.argv[0] = sys.argv[0][:-4]',
    )
    if body and any(ast.dump(body[0]) == ast.dump(ast.parse(form).body[0]) for form in normalisations):
        body = body[1:]
    terminals = ('main()', 'sys.exit(main())', 'raise SystemExit(main())')
    if len(body) != 1 or not any(ast.dump(body[0]) == ast.dump(ast.parse(form).body[0]) for form in terminals):
        raise ValueError("Producer script has an unsupported call or side effect.")


def launcher_zip(data: bytes, command: str, *, stored: bool = False) -> bytes:
    try:
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            entries = archive.infolist()
            if len(entries) != 1 or entries[0].filename != "__main__.py" or entries[0].file_size > 65536 or entries[0].flag_bits & 1 or (stored and entries[0].compress_type != zipfile.ZIP_STORED):
                raise ValueError("Producer launcher ZIP differs from the supported format.")
            source = archive.read(entries[0])
    except (zipfile.BadZipFile, RuntimeError, NotImplementedError) as exc:
        raise ValueError("Producer launcher ZIP is malformed.") from exc
    launcher_script(source, command)
    return source


def windows_resources(executable: Path) -> dict[str, bytes | None]:
    """Map fixed PE resources as data, never as executable code."""
    import ctypes

    api = getattr(ctypes, "WinDLL")("kernel32", use_last_error=True)
    signatures = {
        "LoadLibraryExW": ([ctypes.c_wchar_p, ctypes.c_void_p, ctypes.c_uint32], ctypes.c_void_p),
        "FindResourceW": ([ctypes.c_void_p, ctypes.c_wchar_p, ctypes.c_void_p], ctypes.c_void_p),
        "SizeofResource": ([ctypes.c_void_p, ctypes.c_void_p], ctypes.c_uint32),
        "LoadResource": ([ctypes.c_void_p, ctypes.c_void_p], ctypes.c_void_p),
        "LockResource": ([ctypes.c_void_p], ctypes.c_void_p),
        "FreeLibrary": ([ctypes.c_void_p], ctypes.c_int),
    }
    for name, (arguments, return_type) in signatures.items():
        function = getattr(api, name)
        function.argtypes, function.restype = arguments, return_type
    handle = api.LoadLibraryExW(str(executable), None, 0x22)
    if not handle:
        raise OSError("Producer launcher cannot be mapped as PE resources.")
    try:
        if not handle & 3:
            raise ValueError("Producer launcher did not yield a data-only resource mapping.")
        result: dict[str, bytes | None] = {}
        for name, limit in (("UV_TRAMPOLINE_KIND", 1), ("UV_PYTHON_PATH", 131072), ("UV_SCRIPT_DATA", 1048576)):
            resource = api.FindResourceW(handle, name, 10)
            if not resource:
                result[name] = None
                continue
            size = api.SizeofResource(handle, resource)
            if not 0 < size <= limit:
                raise ValueError("Producer launcher resource size is invalid.")
            loaded = api.LoadResource(handle, resource)
            pointer = api.LockResource(loaded) if loaded else None
            if not pointer:
                raise OSError("Producer launcher resource cannot be read.")
            result[name] = ctypes.string_at(pointer, size)
        return result
    finally:
        if not api.FreeLibrary(handle):
            raise OSError("Producer launcher resource mapping cannot be released.")


def declared_interpreter(executable: Path, data: bytes, command: str) -> Path:
    try:
        if os.name == "nt":
            resources = windows_resources(executable)
            if any(value is not None for value in resources.values()):
                if any(value is None for value in resources.values()) or resources["UV_TRAMPOLINE_KIND"] != b"\x01":
                    raise ValueError("Producer uv launcher resources are incomplete or malformed.")
                path_bytes = resources["UV_PYTHON_PATH"]
                script_bytes = resources["UV_SCRIPT_DATA"]
                if path_bytes is None or script_bytes is None:
                    raise ValueError("Producer uv launcher resources are incomplete.")
                path = path_bytes.decode("utf-8")
                if not path or len(path) > 32767 or any(character in path for character in '\x00\r\n"\ufeff') or not PureWindowsPath(path).is_absolute():
                    raise ValueError("Producer uv interpreter path is malformed.")
                launcher_zip(script_bytes, command, stored=True)
                return Path(path)
            launcher_zip(data, command)
            with zipfile.ZipFile(io.BytesIO(data)) as archive:
                prefix = data[:archive.infolist()[0].header_offset]
            offset = prefix.rfind(b"#!", max(0, len(prefix) - 65536))
            if offset < 0:
                raise ValueError("Producer PyPA launcher has no interpreter shebang.")
            shebang = prefix[offset:].decode("utf-8").rstrip("\r\n")
        else:
            if len(data) > 65536:
                raise ValueError("Producer console script is too large.")
            source = data.decode("utf-8")
            lines = source.splitlines()
            shebang = lines[0] if lines else ""
            if shebang == "#!/bin/sh":
                if len(lines) < 4 or not lines[1].startswith("'''exec' ") or lines[2] != "' '''":
                    raise ValueError("Producer shell wrapper is unsupported.")
                parts = shlex.split(lines[1][9:])
                if len(parts) != 3 or parts[1:] != ["$0", "$@"]:
                    raise ValueError("Producer shell wrapper arguments differ.")
                shebang = "#!" + parts[0]
                source = "\n".join(lines[3:])
            launcher_script(source.encode("utf-8"), command)
        path = shebang[2:] if shebang.startswith("#!") else ""
        if os.name == "nt" and path.startswith('"') and path.endswith('"'):
            path = path[1:-1]
        if not path or any(character in path for character in '\x00\r\n"') or not Path(path).is_absolute() or path == "/usr/bin/env" or path.startswith("/usr/bin/env "):
            raise ValueError("Producer interpreter shebang is unsupported.")
        return Path(path)
    except (UnicodeError, zipfile.BadZipFile) as exc:
        raise ValueError("Producer launcher format is unsupported.") from exc


def interpreter_file(path: Path) -> tuple[Path, Path]:
    """Allow only the interpreter's POSIX leaf link, retaining its venv path."""
    original = path
    seen = set()
    for _ in range(9):
        lexical = PureWindowsPath(str(path))
        if lexical.drive.upper() == "Z:" or lexical.drive.startswith("\\\\") or str(path).startswith(("\\\\", "//")):
            raise ValueError("Use a local directory, never a network or NAS path.")
        if not path.is_absolute():
            raise ValueError("Producer interpreter requires an absolute path.")
        path = ordinary(path.parent) / path.name
        if path in seen:
            raise ValueError("Producer interpreter link loop.")
        seen.add(path)
        metadata = path.lstat()
        if os.name != "nt" and stat.S_ISLNK(metadata.st_mode):
            target = Path(os.readlink(path))
            target_lexical = PureWindowsPath(str(target))
            if target_lexical.drive.upper() == "Z:" or target_lexical.drive.startswith("\\\\") or str(target).startswith(("\\\\", "//")):
                raise ValueError("Use a local directory, never a network or NAS path.")
            path = target if target.is_absolute() else path.parent / target
            continue
        if not stat.S_ISREG(metadata.st_mode) or getattr(metadata, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
            raise ValueError("Producer interpreter must be a regular local executable.")
        return original, path
    raise ValueError("Producer interpreter has too many leaf links.")


@contextmanager
def producer_environment() -> Iterator[dict[str, str]]:
    with tempfile.TemporaryDirectory(prefix="review-bytecode-") as cache:
        environment = {name: os.environ[name] for name in ("PATH", "SystemRoot", "WINDIR", "COMSPEC", "TEMP", "TMP", "LANG", "LC_ALL", "LC_CTYPE") if name in os.environ}
        environment.update(PYTHONNOUSERSITE="1", PYTHONDONTWRITEBYTECODE="1", PYTHONPYCACHEPREFIX=cache)
        yield environment


PRODUCER_PROBE = '''import importlib.metadata as metadata
import importlib.util
import json
import re
import sys
from pathlib import Path
name, package, entry, script_directory = sys.argv[1:]
sys.path[0] = script_directory
normalise = lambda value: re.sub(r"[-_.]+", "-", value).lower()
distributions = [item for item in metadata.distributions() if normalise(item.metadata.get("Name", "")) == name]
if len(distributions) != 1:
    raise SystemExit("Producer distribution is missing or ambiguous.")
distribution = distributions[0]
entries = [item for item in distribution.entry_points if item.group == "console_scripts" and item.name == {"closecontrol": "close-control", "reviewready": "review-ready"}[package]]
if len(entries) != 1 or entries[0].value != entry:
    raise SystemExit("Producer entry point differs.")
spec = importlib.util.find_spec(package)
if spec is None or spec.origin is None or not spec.submodule_search_locations or len(spec.submodule_search_locations) != 1:
    raise SystemExit("Producer package is missing or ambiguous.")
root = Path(next(iter(spec.submodule_search_locations)))
if Path(spec.origin) != root / "__init__.py":
    raise SystemExit("Producer package origin differs.")
print(json.dumps({"executable": sys.executable, "root": str(root), "implementation": sys.implementation.name, "version": ".".join(map(str, sys.version_info[:3])), "cache_tag": sys.implementation.cache_tag, "distribution": {"name": normalise(distribution.metadata["Name"]), "version": distribution.version, "requires_python": distribution.metadata.get("Requires-Python", ""), "entry_point": entries[0].value}}))
'''


def package_files(root: Path) -> dict[str, str]:
    root = ordinary(root)
    files: dict[str, str] = {}

    def visit(directory: Path) -> None:
        for entry in sorted(directory.iterdir()):
            metadata = entry.lstat()
            if stat.S_ISLNK(metadata.st_mode) or getattr(metadata, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
                raise ValueError("Linked producer package paths are not admitted.")
            if entry.name.startswith(".env") or entry.name in ("direct_url.json", "credentials", "id_rsa", "id_ed25519") or entry.suffix.lower() in (".pem", ".key", ".p12", ".pfx"):
                raise ValueError("Producer package contains an excluded private file.")
            if entry.name == "__pycache__" or entry.suffix == ".pyc":
                continue
            if stat.S_ISDIR(metadata.st_mode):
                visit(entry)
            elif stat.S_ISREG(metadata.st_mode):
                files[entry.relative_to(root).as_posix()] = digest(entry.read_bytes())
            else:
                raise ValueError("Producer package contains a non-regular file.")
    visit(root)
    return files


def producer_manifest(bin_dir: Path, command: str) -> dict[str, Any]:
    executable = ordinary(bin_dir / (command + (".exe" if os.name == "nt" else "")))
    launcher = executable.read_bytes()
    interpreter, target = interpreter_file(declared_interpreter(executable, launcher, command))
    interpreter_hash = digest(target.read_bytes())
    distribution, package, entry = PRODUCERS[command]
    with producer_environment() as environment:
        probe = subprocess.run([str(interpreter), "-B", "-c", PRODUCER_PROBE, distribution, package, entry, str(executable.parent)], capture_output=True, cwd=environment["PYTHONPYCACHEPREFIX"], env=environment, check=False)
    if probe.returncode:
        raise ValueError("Producer installed identity probe refused the installation.")
    identity = string_fields(strict_json(probe.stdout, "Producer identity"), ("executable", "root", "implementation", "version", "cache_tag"), "Producer identity")
    if interpreter_file(Path(identity["executable"]))[1] != target:
        raise ValueError("Producer interpreter identity differs from its declared path.")
    manifest = {"manifest_version": 1, "command": command, "launcher_name": executable.name, "launcher_sha256": digest(launcher), "interpreter": {name: identity[name] for name in ("implementation", "version", "cache_tag")} | {"executable_sha256": interpreter_hash}, "distribution": identity.get("distribution"), "package": {"name": package, "files": package_files(Path(identity["root"]))}}
    if executable.read_bytes() != launcher or digest(target.read_bytes()) != interpreter_hash:
        raise ValueError("Producer executable changed during identity inspection.")
    return producer_document(manifest, command)


def sources(app: Path = APP) -> tuple[dict[str, Any], dict[str, bytes]]:
    case = case_document(strict_json(ordinary(app / "samples/shared-review-case.json").read_bytes(), "Shared case"))
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


def invoke(bin_dir: Path, command: str, args: list[str], run: Path, label: str, allowed: tuple[int, ...] = (0,), expected_manifest: dict[str, Any] | None = None) -> dict[str, Any]:
    executable = ordinary(bin_dir / (command + (".exe" if os.name == "nt" else "")))
    manifest = producer_manifest(bin_dir, command)
    if expected_manifest is not None and manifest != expected_manifest:
        raise ValueError("Producer installed identity differs from the receipt.")
    started = datetime.now(timezone.utc).isoformat()
    with producer_environment() as environment:
        result = subprocess.run([str(executable), *args], capture_output=True, cwd=run, env=environment, check=False)
    if producer_manifest(bin_dir, command) != manifest:
        raise ValueError("Producer installed identity changed during invocation.")
    for kind, data in (("stdout", result.stdout), ("stderr", result.stderr)):
        (run / "validation" / f"{label}.{kind}.txt").write_bytes(data)
    if result.returncode not in allowed:
        raise ValueError(f"{command} {label} refused the case (exit {result.returncode}); see validation logs.")
    return {"command": command, "args": args, "executable_sha256": manifest["launcher_sha256"], "producer_manifest_sha256": digest(json_bytes(manifest)), "started": started, "finished": datetime.now(timezone.utc).isoformat(), "exit": result.returncode}


def pinned_producer(bin_dir: Path, command: str, args: list[str], expected_manifest: dict[str, Any]) -> bytes:
    executable = ordinary(bin_dir / (command + (".exe" if os.name == "nt" else "")))
    if digest(executable.read_bytes()) != expected_manifest["launcher_sha256"]:
        raise ValueError("Producer executable hash differs from the receipt.")
    if producer_manifest(bin_dir, command) != expected_manifest:
        raise ValueError("Producer installed identity differs from the receipt.")
    with producer_environment() as environment:
        result = subprocess.run([str(executable), *args], capture_output=True, env=environment, check=False)
    if producer_manifest(bin_dir, command) != expected_manifest:
        raise ValueError("Producer installed identity changed during invocation.")
    if result.returncode:
        raise ValueError("Producer verification refused portable display.")
    return result.stdout


def build(run: Path, bin_dir: Path, month: int, note: Path | None = None) -> Path:
    run = new_output(run)
    manifests = {command: producer_manifest(bin_dir, command) for command in PRODUCERS}
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
    commands.append(invoke(bin_dir, "review-ready", ["gate", "--profile", "month_end", "--pack", "inputs", "--output", "readiness"], run, "readiness", (0, 2), manifests["review-ready"]))
    commands.append(invoke(bin_dir, "review-ready", ["view", "--pack-dir", "readiness"], run, "readiness-view", expected_manifest=manifests["review-ready"]))
    args = ["review", "--current", "inputs/trial_balance.csv", "--prior", "inputs/prior_trial_balance.csv", "--output", "close"]
    if note:
        (run / "inputs/review-note.json").write_bytes(ordinary(note).read_bytes())
        args += ["--review-note", "inputs/review-note.json"]
    commands.append(invoke(bin_dir, "close-control", args, run, "close", (0, 2), manifests["close-control"]))
    before = {name: digest((run / "close" / name).read_bytes()) for name in CLOSE_FILES}
    commands.append(invoke(bin_dir, "close-control", ["view", "--pack-dir", "close"], run, "close-view", expected_manifest=manifests["close-control"]))
    if before != {name: digest((run / "close" / name).read_bytes()) for name in CLOSE_FILES}:
        raise ValueError("Close outputs changed during verification.")
    commands.append(invoke(bin_dir, "close-control", ["drivers", "--pack-dir", "close", "--transactions", "inputs/transactions.csv", "--currency", case["currency"], "--top", "100000", "--output", "drivers"], run, "drivers", expected_manifest=manifests["close-control"]))
    doc = close_document(strict_json((run / "close/close-review-pack.json").read_bytes(), "Close pack"))
    if doc["source_sha256"]["current_trial_balance"] != digest(current) or doc["source_sha256"]["prior_trial_balance"] != digest(prior) or doc["current_report_dates"] != [period] or any(r["tenant"] not in ("", case["tenant"]) for r in doc["exceptions"]):
        raise ValueError("Close provenance or context differs from the case.")
    files = {str(path.relative_to(run)).replace("\\", "/"): digest(path.read_bytes()) for folder in ("inputs", "close", "readiness", "drivers", "validation") for path in (run / folder).iterdir()}
    receipt = {"schema_version": 1, "case": case, "period": period, "invocations": commands, "files": files, "boundary": BOUNDARY, "producer_manifests": manifests}
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
    receipt = receipt_document(strict_json(data, "Receipt"))
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
    if case_document(strict_json(snapshot["inputs/case.json"], "Case input")) != case or case != expected_case:
        raise ValueError("Case identity differs.")
    source = {name: snapshot["inputs/" + name] for name in case["source_sha256"]}
    if any(digest(data) != case["source_sha256"][name] for name, data in source.items()):
        raise ValueError("Fixture provenance differs.")
    current, prior, transactions = trial_balances(case, source, int(receipt["period"][5:7]))
    if any(snapshot[name] != data for name, data in (("inputs/trial_balance.csv", current), ("inputs/prior_trial_balance.csv", prior), ("inputs/transactions.csv", transactions))):
        raise ValueError("Ledger and review sources do not tie.")
    if receipt["period"] != rows(current)[0]["ReportDate"]:
        raise ValueError("Receipt period differs from the ledger case.")
    doc = close_document(strict_json(snapshot["close/close-review-pack.json"], "Close pack"))
    drivers = driver_document(strict_json(snapshot["drivers/variance-drivers.json"], "Driver pack"))
    readiness_document(strict_json(snapshot["readiness/readiness-pack.json"], "Readiness pack"))
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
                if any(driver[k] != original[k] for k in ("Date", "Reference", "Description")) or driver["Amount"] != format(Decimal(original["Debit"]) - Decimal(original["Credit"]), ".2f"):
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
    doc = strict_json(snapshot["close/close-review-pack.json"], "Close pack")
    drivers = strict_json(snapshot["drivers/variance-drivers.json"], "Driver pack")
    driver_index = {(a["tenant"], a["account_id"]): a for a in drivers["accounts"]}
    query_index = {(q["control"], q["tenant"], q["account_id"]): q for q in doc["client_queries"]}
    run_id = digest(snapshot["close/close-review-pack.json"])
    context = {"RunID": run_id, "Entity": case["entity"], "Tenant": case["tenant"], "Period": receipt["period"], "Basis": case["basis"], "Currency": case["currency"]}
    finding_rows, evidence_rows = [], []
    for index, item in enumerate(doc["exceptions"]):
        key = f"{run_id}:{index}"
        query: dict[str, Any] = query_index.get((item["control"], item["tenant"], item["account_id"]), {})
        evidence = driver_index.get((item["tenant"], item["account_id"])) if item["control"] == "period_variance" else None
        available = evidence is not None and Decimal(evidence["unexplained"]) == 0 and evidence["transactions_in_window"] == len(evidence["drivers"])
        finding_rows.append(context | {"ExceptionKey": key, "Control": item["control"], "Status": item["status"], "AccountID": item["account_id"], "Account": item["account_name"], "Current": item["current_value"], "Prior": item["prior_value"], "Difference": item["difference"], "Threshold": item["threshold"], "Reason": item["reason"], "Action": item["reviewer_action"], "Question": query.get("question", ""), "EvidenceRequested": query.get("evidence_requested", ""), "EvidenceState": "Journal rows reconciled" if available else "Line-level evidence is unavailable under this control contract"})
        if available and evidence is not None:
            for row in evidence["drivers"]:
                evidence_rows.append(context | {"ExceptionKey": key, "AccountID": item["account_id"], "TransactionID": row["TransactionID"], "Date": row["Date"], "Reference": row["Reference"], "Description": row["Description"], "Amount": row["Amount"]})
    if not finding_rows:
        raise ValueError("The fixed review case requires at least one finding.")
    ready = strict_json(snapshot["readiness/readiness-pack.json"], "Readiness pack")
    run_row = context | {"CloseStatus": doc["overall_status"], "ReadinessStatus": ready["overall_status"], "PriorPeriod": doc["prior_report_dates"][0], "AbsoluteThreshold": doc["thresholds"]["absolute_variance"], "PercentageThreshold": doc["thresholds"]["percentage_variance"], "ControlsNotRun": "; ".join(doc["controls_not_run"]), "ReadinessControlsNotRun": json.dumps(ready["controls_not_run"], ensure_ascii=False), "Acknowledgement": json.dumps(doc["acknowledgement"], ensure_ascii=False), "UnmappedExceptions": str(sum(r["EvidenceState"] != "Journal rows reconciled" for r in finding_rows)), "SourceTBHash": digest(snapshot["inputs/trial_balance.csv"]), "SourceGLHash": case["source_sha256"]["sample-general-ledger.csv"], "Verification": "CLI verified synthetic case; unsigned local receipt", "Boundary": BOUNDARY}
    return {"sample-review-run.csv": csv_bytes(list(run_row), [run_row]), "sample-review-exceptions.csv": csv_bytes(list(finding_rows[0]), finding_rows), "sample-review-evidence.csv": csv_bytes(list(context) + ["ExceptionKey", "AccountID", "TransactionID", "Date", "Reference", "Description", "Amount"], evidence_rows)}


def render_html(run: Path, bin_dir: Path, previous: Path | None = None) -> str:
    receipt, snapshot = verify(run)
    manifests = receipt["producer_manifests"]
    # Reuse the producer verifier again at display time; no second pack validator.
    for command, folder in (("close-control", "close"), ("review-ready", "readiness")):
        pinned_producer(bin_dir, command, ["view", "--pack-dir", str(run / folder)], manifests[command])
    projected = projection(run)
    doc = strict_json(snapshot["close/close-review-pack.json"], "Close pack")
    change = "No previous verified run supplied."
    if previous:
        old_receipt, old_snapshot = verify(previous)
        if old_receipt["case"] != receipt["case"]:
            raise ValueError("Comparison context differs.")
        if old_receipt["producer_manifests"]["close-control"] != manifests["close-control"]:
            raise ValueError("Comparison producer installed identities differ.")
        comparison = comparison_document(strict_json(pinned_producer(bin_dir, "close-control", ["compare", "--previous-pack", str(previous / "close"), "--current-pack", str(run / "close"), "--previous-tb", str(previous / "inputs/trial_balance.csv"), "--current-tb", str(run / "inputs/trial_balance.csv")], manifests["close-control"]), "Comparison"))
        changed = sorted(name for name in receipt["files"].keys() | old_receipt["files"].keys() if name.startswith("inputs/") and receipt["files"].get(name) != old_receipt["files"].get(name))
        change = json.dumps({"changed_inputs": changed, "findings": comparison["findings"], "queries": comparison["queries"], "scope_changes": comparison["scope_changes"], "acknowledgement_changed": strict_json(old_snapshot["close/close-review-pack.json"], "Previous close pack")["acknowledgement"] != doc["acknowledgement"], "meaning": comparison["review_boundary"]}, ensure_ascii=False, indent=2)
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
    except (ValueError, OSError, KeyError, DecimalException) as exc:
        parser.exit(1, f"Review workflow refused: {exc}\n")


if __name__ == "__main__":
    main()
