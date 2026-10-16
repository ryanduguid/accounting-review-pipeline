"""Replay the fuzz seed corpus so a broken harness fails ordinary CI."""

from __future__ import annotations

from pathlib import Path
import subprocess

COMPONENT = Path(__file__).resolve().parents[1] / "packages/monthly-close-control-plane"

SEEDS = sorted((Path(__file__).resolve().parents[1] / "fuzz" / "corpus").iterdir())


def test_the_corpus_is_not_empty() -> None:
    assert SEEDS  # nosec B101: pytest outcome assertion


def test_seeds_replay_through_native_loaders() -> None:
    result = subprocess.run(
        ["cargo", "run", "--quiet", "--locked", "--features", "tools", "--bin",
         "close-control-fuzz", "--", str(SEEDS[0].parent), "0"],
        cwd=COMPONENT, capture_output=True, text=True, timeout=60, check=False,
    )
    assert result.returncode == 0, result.stderr
    assert f"replayed {len(SEEDS)} seeds; 0 mutations; no panic" in result.stdout


def test_missing_corpus_refuses_success(tmp_path: Path) -> None:
    result = subprocess.run(
        ["cargo", "run", "--quiet", "--locked", "--features", "tools", "--bin",
         "close-control-fuzz", "--", str(tmp_path / "missing"), "0"],
        cwd=COMPONENT, capture_output=True, text=True, timeout=60, check=False,
    )
    assert result.returncode == 1
    assert "close-control-fuzz:" in result.stderr
