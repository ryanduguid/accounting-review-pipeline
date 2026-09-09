from pathlib import Path

import evatt
from evatt.errors import EvattError, Halt

ROOT = Path(__file__).resolve().parents[1]


def test_version_is_a_three_part_string() -> None:
    parts = evatt.__version__.split(".")
    assert len(parts) == 3
    assert all(part.isdigit() for part in parts)


def test_halt_is_an_evatt_error_carrying_unknowns() -> None:
    error = Halt(("Jane Roe",))
    assert isinstance(error, EvattError)
    assert error.unknowns == ("Jane Roe",)


def test_gitignore_covers_the_real_map() -> None:
    lines = (ROOT / ".gitignore").read_text(encoding="utf-8").splitlines()
    ignored = [line.strip() for line in lines]
    assert "entities.json" in ignored
    assert "*.entities.json" in ignored
