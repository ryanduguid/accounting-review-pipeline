"""Record fabricated Python 0.2.0 test calls for standalone Rust regression tests.

This developer tool requires pytest and a pristine external oracle copy. It
never ships in the evatt binary. Removed Python object-type API calls are
excluded; their equivalent file-map validation remains covered.
"""

import argparse
import sys

import dataclasses
import functools
import json
from pathlib import Path
import pytest
import importlib

RECORDS = {}
ACTIVE = ""
NESTED = 0
MODULES = []
OUTPUT = ""
HALT_TYPE = ()


def serial(value):
    if dataclasses.is_dataclass(value):
        return dataclasses.asdict(value)
    if isinstance(value, (list, tuple)):
        return [serial(v) for v in value]
    if isinstance(value, dict):
        return {k: serial(v) for k, v in value.items()}
    if isinstance(value, bytes):
        return {"hex": value.hex()}
    if isinstance(value, (str, bool, int, float)) or value is None:
        return value
    raise TypeError()


def capture(module, name):
    original = getattr(module, name)
    mode = module.__name__.split(".")[-1] + "." + name

    @functools.wraps(original)
    def recorded(*args, **kwargs):
        global NESTED
        outer = NESTED == 0
        NESTED += 1
        try:
            result = original(*args, **kwargs)
            try:
                outcome = {"ok": serial(result)}
            except TypeError:
                outcome = None
            return result
        except Exception as error:
            outcome = {"error": type(error).__name__, "message": str(error)}
            if isinstance(error, HALT_TYPE):
                outcome["unknowns"] = serial(error.unknowns)
            raise
        finally:
            NESTED -= 1
            if (
                outer
                and outcome is not None
                and ACTIVE
                and "monkeypatch" not in ACTIVE
                and "never_reports_a_placeholder" not in ACTIVE
            ):
                try:
                    if mode == "entities.load":
                        data = args[0].read_text(encoding="utf-8")
                        outcome = {
                            k: (v.replace(str(args[0]), "<map>") if isinstance(v, str) else v)
                            for k, v in outcome.items()
                        }
                        inputs = [data]
                    else:
                        inputs = serial(args)
                    if any(isinstance(v, str) and len(v) > 5000 for v in inputs):
                        raise TypeError()
                    record = {
                        "mode": mode,
                        "args": inputs,
                        "kwargs": serial(kwargs),
                        "outcome": outcome,
                        "test": ACTIVE,
                    }
                    key = json.dumps(
                        {k: v for k, v in record.items() if k != "test"}, sort_keys=True
                    )
                    RECORDS[key] = record
                except TypeError, OSError, UnicodeError, UnboundLocalError:
                    pass

    for target in MODULES:
        for attr, value in list(vars(target).items()):
            if value is original:
                setattr(target, attr, recorded)


@pytest.hookimpl(tryfirst=True)
def pytest_runtest_setup(item):
    global ACTIVE
    ACTIVE = item.name


def pytest_sessionfinish(session, exitstatus):
    rows = []
    for row in RECORDS.values():
        if row["mode"] in ("redact.redact", "restore.restore", "verify.findings") and any(
            not isinstance(e[key], str)
            for e in row["args"][1]
            for key in ("value", "placeholder", "kind", "added")
        ):
            continue
        rows.append(row)

    def version(value):
        if isinstance(value, dict):
            if set(value) == {"hex"}:
                data = (
                    bytes.fromhex(value["hex"])
                    .replace(b'"tool_version": "0.2.0"', b'"tool_version": "1.0.0"')
                    .replace(b'"tool_version":"0.2.0"', b'"tool_version":"1.0.0"')
                )
                return {"hex": data.hex()}
            return {k: version(v) for k, v in value.items()}
        if isinstance(value, list):
            return [version(v) for v in value]
        return value

    Path(OUTPUT).write_text(
        json.dumps(version(rows), ensure_ascii=True, separators=(",", ":")) + "\n"
    )


def main():
    global MODULES, OUTPUT, HALT_TYPE
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--python-root", required=True)
    parser.add_argument("--output", required=True)
    options = parser.parse_args()
    sys.path.insert(0, options.python_root)
    OUTPUT = options.output
    modules = {
        name: importlib.import_module("evatt." + name)
        for name in ["entities", "patterns", "redact", "restore", "verify", "disclosure"]
    }
    MODULES = list(modules.values())
    HALT_TYPE = modules["redact"].Halt
    entities, patterns, redact, restore, verify, disclosure = (
        modules[name]
        for name in ["entities", "patterns", "redact", "restore", "verify", "disclosure"]
    )
    for module, names in [
        (patterns, ["structured_spans", "person_name_spans", "_ambiguous_identifiers"]),
        (entities, ["_fold", "load"]),
        (redact, ["redact", "residual"]),
        (restore, ["restore"]),
        (verify, ["findings"]),
        (disclosure, ["create_record", "check_record"]),
    ]:
        for name in names:
            capture(module, name)
    return pytest.main([str(Path(options.python_root) / "tests")], plugins=[sys.modules[__name__]])


if __name__ == "__main__":
    sys.exit(main())
