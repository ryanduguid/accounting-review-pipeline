"""Replay the fuzz seed corpus so a broken harness fails ordinary CI."""

from __future__ import annotations

from pathlib import Path
from unittest import mock

import pytest

from fuzz import close_control_loader

SEEDS = sorted((Path(__file__).resolve().parents[1] / "fuzz" / "corpus").iterdir())


def test_the_corpus_is_not_empty() -> None:
    assert SEEDS  # nosec B101: pytest outcome assertion


@pytest.mark.parametrize("seed", SEEDS, ids=lambda seed: seed.name)
def test_seed_replays_cleanly(seed: Path) -> None:
    close_control_loader.test_one_input(seed.read_bytes())


def test_unexpected_loader_errors_escape() -> None:
    broken = mock.Mock(side_effect=TypeError("bug"))
    with mock.patch.object(close_control_loader, "LOADERS", (broken,)):
        with pytest.raises(TypeError, match="bug"):
            close_control_loader.test_one_input(b"")
