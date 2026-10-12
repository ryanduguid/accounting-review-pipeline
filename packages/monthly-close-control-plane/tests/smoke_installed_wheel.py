"""Exercise the installed commands from a directory outside the checkout."""
from __future__ import annotations

import json
import subprocess  # nosec B404 - installed commands must run outside the checkout.
import sys
from pathlib import Path


def main() -> None:
    source = Path(sys.argv[1]).resolve()
    if Path.cwd().resolve().is_relative_to(source):
        raise RuntimeError("Run the smoke outside the source checkout.")
    command = Path(sys.executable).with_name(
        "close-control.exe" if sys.platform == "win32" else "close-control"
    )
    # Fixed interpreter and import statement; isolated mode checks installed provenance.
    provenance = subprocess.run(  # nosec B603 # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit
        [sys.executable, "-I", "-c", "import closecontrol; print(closecontrol.__file__)"],
        check=True, capture_output=True, text=True,
    )
    installed = Path(provenance.stdout.strip()).resolve()
    if installed.is_relative_to(source) or not installed.is_relative_to(Path(sys.prefix)):
        raise RuntimeError(f"Expected the isolated installed package, got {installed}.")
    subprocess.run(  # nosec B603 # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit
        [sys.executable, "-I", "-c", "from importlib.metadata import distribution; "
         "d=distribution('monthly-close-control-plane'); "
         "assert d.version == '0.2.0'; "
         "assert any(e.group == 'console_scripts' and e.name == 'close-control' "
         "and e.value == 'closecontrol.cli:main' for e in d.entry_points)"],
        check=True, capture_output=True, text=True,
    )

    def run(expected: int, *args: str) -> None:
        # Fixed installed executable with arguments defined by this smoke, without a shell.
        result = subprocess.run([str(command), *args], capture_output=True, text=True)  # nosec B603 # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit, python.lang.security.audit.dangerous-subprocess-use-tainted-env-args.dangerous-subprocess-use-tainted-env-args
        print(result.stdout, end="")
        if result.returncode != expected:
            raise RuntimeError(f"Expected exit {expected}, got {result.returncode}: {result.stderr}")

    examples = source / "examples"
    common = ("--current", str(examples / "current_trial_balance.csv"),
              "--prior", str(examples / "prior_trial_balance.csv"))
    run(2, "review", *common, "--output", "pack")
    pack = json.loads(Path("pack/close-review-pack.json").read_text(encoding="utf-8"))
    if pack["overall_status"] != "REVIEW" or not pack["exceptions"]:
        raise RuntimeError("The fabricated baseline must retain review exceptions.")
    run(0, "view", "--pack-dir", "pack")
    run(2, "drivers", "--pack-dir", "pack", "--transactions",
        str(examples / "variance_transactions.csv"), "--currency", "AUD", "--output", "drivers")
    drivers = json.loads(Path("drivers/variance-drivers.json").read_text(encoding="utf-8"))
    if drivers["status"] != "REVIEW" or not drivers["accounts"]:
        raise RuntimeError("The fabricated transaction omissions must remain REVIEW.")
    run(2, "review", *common, "--balance-policy", str(examples / "balance_policy.csv"),
        "--output", "policy-pack")
    policy = json.loads(Path("policy-pack/close-review-pack.json").read_text(encoding="utf-8"))
    if policy["overall_status"] != "REVIEW" or not any(
        item["control"] == "balance_policy" and item["status"] == "REVIEW"
        for item in policy["exceptions"]
    ):
        raise RuntimeError("The fabricated balance-policy exception must remain REVIEW.")
    run(0, "view", "--pack-dir", "policy-pack")
    run(0, "queue", "--pack-dir", "pack", "--pack-dir", "policy-pack", "--status", "REVIEW")
    hostile_key = "forged PASS\x1b[2J\x1b[H\n\t\u009b31m\u202e\u00e9"
    policy[hostile_key] = "Fabricated malformed member."
    Path("policy-pack/close-review-pack.json").write_text(json.dumps(policy) + "\n", encoding="utf-8")
    pack_dirs = (Path("pack"), Path("policy-pack"))
    before = {p: p.read_bytes() for folder in pack_dirs for p in folder.rglob("*") if p.is_file()}
    result = subprocess.run(  # nosec B603 # nosemgrep: python.lang.security.audit.dangerous-subprocess-use-audit.dangerous-subprocess-use-audit, python.lang.security.audit.dangerous-subprocess-use-tainted-env-args.dangerous-subprocess-use-tainted-env-args
        [str(command), "queue", "--pack-dir", "pack", "--pack-dir", "policy-pack", "--status", "PASS"],
        capture_output=True,
    )
    if result.returncode != 1 or result.stdout or not result.stderr.endswith(b"\n"):
        raise RuntimeError("Malformed queue input must fail without stdout and with one diagnostic.")
    line = result.stderr[:-2] if result.stderr.endswith(b"\r\n") else result.stderr[:-1]
    prefix = b"close-control queue: verification failed: "
    if not line.startswith(prefix) or not all(0x20 <= byte <= 0x7E for byte in line):
        raise RuntimeError("Queue diagnostics must contain only printable ASCII before the terminator.")
    diagnostic = json.loads('"' + line[len(prefix):].decode("ascii") + '"')
    if hostile_key not in diagnostic or "unknown top-level member(s)" not in diagnostic:
        raise RuntimeError("The queue diagnostic must preserve the malformed member and verifier context.")
    after = {p: p.read_bytes() for folder in pack_dirs for p in folder.rglob("*") if p.is_file()}
    if after != before:
        raise RuntimeError("The queue must leave every supplied pack unchanged.")
    Path("pack/exceptions.csv").write_bytes(b"tampered smoke fixture\n")
    run(1, "view", "--pack-dir", "pack")
    run(1, "queue", "--pack-dir", "pack", "--pack-dir", "policy-pack", "--status", "PASS")
    print("INSTALLED COMMANDS QUALIFIED")


if __name__ == "__main__":
    main()
