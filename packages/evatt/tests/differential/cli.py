"""End-to-end parity in fabricated git worktrees at identical paths.

python3.14 tests/differential/cli.py --python-root <base-copy> --binary <evatt> --seed 1 --cases 300
Each case recreates its worktree before each implementation. Compare exit code,
stdout, stderr and every non-git file, including stale files and links. Only
new disclosure record tool_version and --version may differ.
"""

import argparse
import json
import os
from pathlib import Path
import random
import shutil
import stat
import subprocess
import sys
import tempfile
from detectors import generated

COMMANDS = ["redact", "restore", "verify", "disclosure-record", "disclosure-check"]
MAP = {
    "schema_version": 1,
    "entries": [
        {"value": "Jane Roe", "placeholder": "PERSON_01", "kind": "person", "added": "2026-09-09"},
        {
            "value": "Sample Holdings Pty Ltd",
            "placeholder": "CLIENT_01",
            "kind": "client",
            "added": "2026-09-09",
        },
    ],
}
DESTINATION = "model:sample-tenant:sample-project"
DECISION = "sample-decision:42"
ARGUMENTS = [
    [],
    ["--help"],
    ["--version"],
    ["--v"],
    ["--vers=x"],
    ["--hel=x"],
    ["-hfoo"],
    ["-h=x"],
    ["redct"],
    ["--missing", "--help"],
    ["--missing", "redact"],
    ["--", "redact"],
    ["redact", "--help"],
    ["restore", "--help"],
    ["verify", "--help"],
    ["disclosure-record", "--help"],
    ["disclosure-check", "--help"],
    ["disclosure-recrod", "Sample Sensitive Sentinel"],
    ["--dest", "Sample Sensitive Sentinel"],
    ["verify", "--i"],
    ["redact", "--map="],
    ["disclosure-record", "--d", "Sample Sensitive Sentinel"],
    ["disclosure-check", "--de", "Sample Sensitive Sentinel"],
    ["-1"],
    ["-٣"],
    ["-1.5"],
    ["redact", "--no-strict"],
    ["redact", "--in", "-1", "--map", "entities.json", "--out", "out.md"],
    ["redact", "--in=--out", "--map=entities.json", "--out=out.md", "--"],
    ["verify", "--in", "in.md", "--map", "entities.json", "--help=x"],
    ["redact", "--i", "in.md", "--m", "entities.json", "--o", "out.md"],
    ["verify", "--in", "in.md", "--map", "entities.json", "--version"],
    ["\udcff"],
    ["verify", "--in", "in\udcff.md", "--map", "entities.json"],
]
STATES = [
    "ordinary",
    "negation",
    "tracked",
    "exclude",
    "outside",
    "relative-path",
    "git-overrides",
    "missing-git",
    "map-link",
    "source-link",
    "output-link",
    "output-hardlink",
    "manifest-hardlink",
    "triage-hardlink",
    "dangling-output",
    "record-link",
    "record-hardlink",
    "linked-parent",
    "missing-dotdot-link",
    "missing-input",
    "map-directory",
    "out-directory",
    "bad-map",
    "surrogate-map",
    "unicode-placeholder",
    "duplicate-map",
    "nonfinite-map",
    "bool-map",
    "nested-ignore",
    "sidecar-collision",
    "record-parent-file",
    "readonly-directory",
    "unreadable-input",
    "invalid-utf8-input",
    "invalid-utf8-map",
]


# The real git, resolved once, so PATH changes in a case never alter the harness.
GIT = str(Path(shutil.which("git")).resolve())


def force_remove(function, path, _error):
    """Clear the read-only attribute Windows git sets on objects, then retry."""
    os.chmod(path, stat.S_IWRITE)
    function(path)


def run_git(root, *args):
    subprocess.run([GIT, *args], cwd=root, check=True, capture_output=True)


def setup(root, spec, implementation, args):
    root.mkdir()
    env = os.environ.copy()
    env["GIT_CONFIG_GLOBAL"] = str(root / "absent-global")
    env["GIT_CONFIG_SYSTEM"] = str(root / "absent-system")
    env["GIT_CEILING_DIRECTORIES"] = str(root.parent)
    env["PYTHONPATH"] = args.python_root
    env["PYTHON_COLORS"] = spec.get("colour", "0")
    env["COLUMNS"] = str(spec.get("columns", 80))
    env["LINES"] = "24"
    if "ioencoding" in spec:
        env["PYTHONIOENCODING"] = spec["ioencoding"]
    state = spec.get("state", "ordinary")
    if state != "outside":
        run_git(root, "init", "-q")
    rules = "entities.json\n*.entities.json\n*.triage.md\n*.disclosure.json\n"
    if state == "negation":
        rules += "!entities.json\n"
    if state == "exclude":
        rules = "*.triage.md\n*.disclosure.json\n"
        (root / ".git/info/exclude").write_text("entities.json\n")
    (root / ".gitignore").write_text(rules)
    (root / "entities.json").write_text(json.dumps(spec.get("mapping", MAP)))
    (root / "in.md").write_bytes(spec.get("source", "Jane Roe\nTFN: 123 456 782\n").encode("utf-8"))
    (root / "out.md").write_bytes(b"fabricated stale output\n")
    (root / "out.md.manifest.json").write_bytes(b'{"schema_version":1,"counts":{}}\n')
    (root / "out.md.triage.md").write_bytes(b"fabricated stale triage\n")
    if spec.get("command") == "disclosure-check":
        from evatt import disclosure, entities

        entry_map = entities.load(root / "entities.json")
        encoded = disclosure.create_record(
            (root / "in.md").read_bytes(), entry_map, destination=DESTINATION, decision_ref=DECISION
        )
        if implementation == "rust":
            encoded = encoded.replace(b'"tool_version": "0.2.0"', b'"tool_version": "1.0.0"')
        (root / "out.disclosure.json").write_bytes(encoded)
    argv = spec.get("argv")
    command = spec.get("command", "redact")
    if argv is None:
        argv = [command, "--in", "in.md", "--map", "entities.json"]
        if command not in ("verify",):
            argv += [
                "--record" if command == "disclosure-check" else "--out",
                "out.disclosure.json" if command.startswith("disclosure-") else "out.md",
            ]
        if command.startswith("disclosure-"):
            argv += ["--destination", DESTINATION, "--decision-ref", DECISION]
    argv = list(argv)
    if state == "tracked":
        run_git(root, "add", "-f", "entities.json")
    if state == "relative-path":
        (root / "bin").mkdir()
        (root / "bin" / Path(GIT).name).symlink_to(GIT)
        env["PATH"] = "bin"
    if state == "missing-git":
        env["PATH"] = str(root / "absent-bin")
    if state == "git-overrides":
        for var in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        ]:
            env[var] = str(root / "absent")
    if state in ("map-link", "source-link", "output-link"):
        name = {"map-link": "entities.json", "source-link": "in.md", "output-link": "out.md"}[state]
        (root / name).rename(root / ("target-" + name))
        (root / name).symlink_to("target-" + name)
        if state == "map-link":
            (root / ".gitignore").write_text(rules + "target-entities.json\n")
    if state in ("output-hardlink", "manifest-hardlink", "triage-hardlink"):
        name = {
            "output-hardlink": "out.md",
            "manifest-hardlink": "out.md.manifest.json",
            "triage-hardlink": "out.md.triage.md",
        }[state]
        (root / name).unlink()
        (root / name).hardlink_to(
            root / ("entities.json" if state == "output-hardlink" else "out.md")
        )
    if state == "dangling-output":
        (root / "out.md").unlink()
        (root / "out.md").symlink_to("out.md.manifest.json")
    if state == "record-link":
        path = root / "out.disclosure.json"
        path.unlink(missing_ok=True)
        path.symlink_to("entities.json")
    if state == "record-hardlink":
        path = root / "out.disclosure.json"
        path.unlink(missing_ok=True)
        path.hardlink_to(root / "entities.json")
    if state in ("linked-parent", "missing-dotdot-link"):
        (root / "records").mkdir()
        (root / "linked").symlink_to("records", target_is_directory=True)
        target = (
            "linked/out.disclosure.json"
            if state == "linked-parent"
            else "missing/../linked/../out.disclosure.json"
        )
        if "--out" in argv:
            argv[argv.index("--out") + 1] = target
        if "--record" in argv:
            argv[argv.index("--record") + 1] = target
    if state == "missing-input":
        (root / "in.md").unlink()
    if state == "map-directory":
        (root / "entities.json").unlink()
        (root / "entities.json").mkdir()
    if state == "out-directory":
        (root / "out.md").unlink()
        (root / "out.md").mkdir()
    if state == "bad-map":
        (root / "entities.json").write_text(spec.get("raw_map", "{not json"))
    if state in (
        "surrogate-map",
        "unicode-placeholder",
        "duplicate-map",
        "nonfinite-map",
        "bool-map",
    ):
        mapping = json.loads(json.dumps(MAP))
        if state == "surrogate-map":
            mapping["entries"][0]["value"] = "Jane \ud800Roe"
        if state == "unicode-placeholder":
            mapping["entries"][0]["placeholder"] = "PERSON_٠١"
        if state == "duplicate-map":
            mapping["entries"].append(
                {**mapping["entries"][0], "value": "jane_ROE", "placeholder": "PERSON_02"}
            )
        if state == "nonfinite-map":
            mapping["schema_version"] = float("nan")
        if state == "bool-map":
            mapping["schema_version"] = True
        (root / "entities.json").write_text(json.dumps(mapping))
    if state == "nested-ignore":
        (root / "nested").mkdir()
        (root / "nested/entities.json").write_bytes((root / "entities.json").read_bytes())
        (root / "nested/.gitignore").write_text("!entities.json\n")
        argv[argv.index("--map") + 1] = "nested/entities.json"
    if state == "sidecar-collision" and "--out" in argv:
        argv[argv.index("--out") + 1] = "in.md"
    if state == "record-parent-file":
        (root / "blocked").write_bytes(b"fabricated non-directory")
        if "--out" in argv:
            argv[argv.index("--out") + 1] = (
                "blocked/out.disclosure.json"
                if command.startswith("disclosure-")
                else "blocked/out.md"
            )
        if "--record" in argv:
            argv[argv.index("--record") + 1] = "blocked/out.disclosure.json"
    if state == "readonly-directory":
        (root / "readonly").mkdir()
        (root / "readonly").chmod(0o555)
        if "--out" in argv:
            argv[argv.index("--out") + 1] = (
                "readonly/out.disclosure.json"
                if command.startswith("disclosure-")
                else "readonly/out.md"
            )
    if state == "unreadable-input":
        (root / "in.md").chmod(0)
    if state in ("invalid-utf8-input", "invalid-utf8-map"):
        filename = "in.md" if state.endswith("input") else "entities.json"
        (root / filename).write_bytes(b"safe\n\xf0\x9f\x98")
    return argv, env


def snapshot(root):
    result = {}
    for path in sorted(root.rglob("*")):
        if ".git" in path.relative_to(root).parts:
            continue
        name = str(path.relative_to(root))
        if path.is_symlink():
            result[name] = ["link", os.readlink(path)]
        elif path.is_file():
            mode = path.stat().st_mode & 0o777
            try:
                data = path.read_bytes()
            except PermissionError:
                path.chmod(mode | 0o400)
                data = path.read_bytes()
                path.chmod(mode)
            if name.endswith(".disclosure.json"):
                data = data.replace(b'"tool_version": "0.2.0"', b'"tool_version": "1.0.0"')
            result[name] = ["file", data.hex(), path.stat().st_mode & 0o777, path.stat().st_nlink]
        else:
            result[name] = ["directory"]
    return result


def run(root, spec, implementation, args):
    argv, env = setup(root, spec, implementation, args)
    program = [args.python, "-m", "evatt"] if implementation == "python" else [args.binary]
    # os.fsencode preserves the non-UTF-8 argv corpus on POSIX.
    completed = subprocess.run(
        program + [os.fsencode(v) for v in argv], cwd=root, env=env, capture_output=True, timeout=40
    )
    stdout = completed.stdout
    if argv in (["--version"], ["--v"]):
        stdout = stdout.replace(b"0.2.0", b"1.0.0")
    answer = (completed.returncode, stdout.hex(), completed.stderr.hex(), snapshot(root))
    for path in root.rglob("*"):
        if path.is_dir() and not path.is_symlink():
            path.chmod(0o700)
    root.chmod(0o700)
    shutil.rmtree(root, onexc=force_remove)
    return answer


def cases(rng, count):
    batch = [
        {"argv": argv, "columns": columns, "colour": colour}
        for argv in ARGUMENTS
        for columns, colour in [(80, "0"), (40, "1")]
    ]
    for state in STATES:
        for command in ["redact", "verify", "disclosure-record", "disclosure-check"]:
            source = "CLIENT_01\n" if command.startswith("disclosure-") else "John Smith\n"
            batch.append({"state": state, "command": command, "source": source})
    for _ in range(count):
        command = rng.choice(COMMANDS)
        source = (
            rng.choice(["CLIENT_01\n", "PERSON_01\r\n", "safe prose\n"])
            if command.startswith("disclosure-")
            else generated(rng)
        )
        batch.append(
            {
                "command": command,
                "state": rng.choice(STATES),
                "source": source,
                "columns": rng.choice([20, 40, 80, 120]),
                "colour": rng.choice(["0", "1"]),
            }
        )
    return batch


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--python-root", required=True)
    parser.add_argument("--python", default=sys.executable)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--cases", type=int, default=300)
    args = parser.parse_args()
    sys.path.insert(0, args.python_root)
    regressions = json.loads(Path(__file__).with_name("cli-regressions.json").read_text())
    batch = regressions + cases(random.Random(args.seed), args.cases)
    errors = []
    with tempfile.TemporaryDirectory(prefix="evatt-e2e-", dir=os.environ.get("EVATT_DIFFERENTIAL_TMP")) as scratch:
        for i, spec in enumerate(batch):
            root = Path(scratch) / "case"
            py = run(root, spec, "python", args)
            rust = run(root, spec, "rust", args)
            if py != rust:
                errors.append((i, spec, py, rust))
                if len(errors) <= 10:
                    print("difference", i, json.dumps(spec), flush=True)
                    for label, want, got in zip(["exit", "stdout", "stderr", "files"], py, rust):
                        if want != got:
                            print(label, "python", want, "rust", got, flush=True)
    print(f"seed {args.seed}: {len(batch)} end-to-end cases; {len(errors)} differences")
    if errors:
        Path(tempfile.gettempdir(), "evatt-e2e-differences.json").write_text(json.dumps(errors, indent=2))
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
