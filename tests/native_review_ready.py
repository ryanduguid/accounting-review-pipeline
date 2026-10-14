"""Run the Rust readiness conformance probe from its owning toolchain directory."""
import json
import subprocess
from pathlib import Path

COMPONENT = Path(__file__).resolve().parents[1] / "packages/review-ready-gate"


def probe(request: dict) -> dict:
    result = subprocess.run(
        ["cargo", "run", "--quiet", "--locked", "--features", "probe", "--bin", "review-ready-probe"],
        input=json.dumps(request) + "\n", text=True, encoding="utf-8",
        capture_output=True, cwd=COMPONENT, check=False, timeout=60,
    )
    if result.returncode:
        raise AssertionError(result.stderr)
    return json.loads(result.stdout)
