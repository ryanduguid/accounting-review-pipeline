# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional terms under AGPL-3.0 section 7(b): see NOTICE.

"""Replay the fuzz seed corpus so a broken harness fails ordinary CI."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
COMPONENT = ROOT / "packages/monthly-close-control-plane"
CORPUS = ROOT / "fuzz" / "corpus"
SEEDS = sorted(CORPUS.iterdir())
HARNESS = "close-control-fuzz"


@pytest.fixture(scope="module")
def harness_executable() -> Path:
    # Build once with a separate deadline, including on a cold workspace.
    build = subprocess.run(
        ["cargo", "build", "--locked", "--features", "tools", "--bin", HARNESS,
         "--message-format=json-render-diagnostics"],
        cwd=COMPONENT, capture_output=True, encoding="utf-8", timeout=300, check=False,
    )
    assert build.returncode == 0, build.stderr  # nosec B101: pytest outcome assertion
    for line in build.stdout.splitlines():
        if not line.startswith("{"):
            continue
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact" and message["target"]["name"] == HARNESS:
            executable = message["executable"]
            if executable is not None:
                return Path(executable)
    pytest.fail(f"Cargo did not report the {HARNESS} executable")


def test_the_corpus_is_not_empty() -> None:
    assert SEEDS  # nosec B101: pytest outcome assertion


def test_seeds_replay_through_native_loaders(harness_executable: Path) -> None:
    result = subprocess.run(
        [str(harness_executable), str(CORPUS), "0"],
        cwd=COMPONENT, capture_output=True, text=True, timeout=60, check=False,
    )
    assert result.returncode == 0, result.stderr
    assert f"replayed {len(SEEDS)} seeds; 0 mutations; no panic" in result.stdout


def test_missing_corpus_refuses_success(tmp_path: Path, harness_executable: Path) -> None:
    result = subprocess.run(
        [str(harness_executable), str(tmp_path / "missing"), "0"],
        cwd=COMPONENT, capture_output=True, text=True, timeout=60, check=False,
    )
    assert result.returncode == 1
    assert "close-control-fuzz:" in result.stderr
