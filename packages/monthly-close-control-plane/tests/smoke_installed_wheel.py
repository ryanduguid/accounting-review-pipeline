"""Exercise the installed commands from a directory outside the checkout."""
from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


def main() -> None:
    source = Path(sys.argv[1]).resolve()
    if Path.cwd().resolve().is_relative_to(source):
        raise RuntimeError("Run the smoke outside the source checkout.")
    command = Path(sys.executable).with_name(
        "close-control.exe" if sys.platform == "win32" else "close-control"
    )
    provenance = subprocess.run(
        [sys.executable, "-I", "-c", "import closecontrol; print(closecontrol.__file__)"],
        check=True, capture_output=True, text=True,
    )
    installed = Path(provenance.stdout.strip()).resolve()
    if installed.is_relative_to(source) or not installed.is_relative_to(Path(sys.prefix)):
        raise RuntimeError(f"Expected the isolated installed package, got {installed}.")
    subprocess.run(
        [sys.executable, "-I", "-c", "from importlib.metadata import distribution; "
         "d=distribution('monthly-close-control-plane'); "
         "assert d.version == '0.1.8'; "
         "assert any(e.group == 'console_scripts' and e.name == 'close-control' "
         "and e.value == 'closecontrol.cli:main' for e in d.entry_points)"],
        check=True, capture_output=True, text=True,
    )

    def run(expected: int, *args: str) -> None:
        result = subprocess.run([str(command), *args], capture_output=True, text=True)
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
    Path("pack/exceptions.csv").write_bytes(b"tampered smoke fixture\n")
    run(1, "view", "--pack-dir", "pack")
    print("INSTALLED COMMANDS QUALIFIED")


if __name__ == "__main__":
    main()
