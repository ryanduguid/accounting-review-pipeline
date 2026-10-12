"""Pack writer helpers: special-value rendering, best-effort restore and the park-and-swap rollback."""
from __future__ import annotations

import os
from decimal import Decimal
from pathlib import Path

import pytest
from closecontrol import report


@pytest.mark.parametrize(("value", "rendered"), [(Decimal("Infinity"), "Infinity"), (Decimal("NaN"), "NaN")])
def test_money_renders_a_special_decimal_without_a_scale(value: Decimal, rendered: str) -> None:
    assert report._money(value) == rendered  # nosec B101: pytest outcome assertion


def _swap_fixture(tmp_path: Path) -> tuple[Path, Path]:
    destination = tmp_path / "close-summary.md"
    destination.write_text("previous run", encoding="utf-8")
    staged = tmp_path / "close-summary.md.staged.partial"
    staged.write_text("this run", encoding="utf-8")
    return staged, destination


def test_a_previous_file_that_cannot_be_parked_is_left_in_place(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    staged, destination = _swap_fixture(tmp_path)
    real_replace = os.replace

    def refuse_to_park(source, target):  # noqa: ANN001
        if Path(source) == destination:
            raise PermissionError("file is open in another program")
        return real_replace(source, target)

    monkeypatch.setattr(report.os, "replace", refuse_to_park)
    with pytest.raises(PermissionError):
        report._swap_into_place(staged, destination)
    assert destination.read_text(encoding="utf-8") == "previous run"  # nosec B101: pytest outcome assertion
    assert sorted(path.name for path in tmp_path.iterdir()) == [  # nosec B101: pytest outcome assertion
        "close-summary.md", "close-summary.md.staged.partial"]


def test_a_failed_move_restores_the_parked_previous_file(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    staged, destination = _swap_fixture(tmp_path)
    real_replace = os.replace

    def refuse_staged(source, target):  # noqa: ANN001
        if Path(source) == staged:
            raise PermissionError("destination is locked")
        return real_replace(source, target)

    monkeypatch.setattr(report.os, "replace", refuse_staged)
    with pytest.raises(PermissionError):
        report._swap_into_place(staged, destination)
    assert destination.read_text(encoding="utf-8") == "previous run"  # nosec B101: pytest outcome assertion
    assert sorted(path.name for path in tmp_path.iterdir()) == [  # nosec B101: pytest outcome assertion
        "close-summary.md", "close-summary.md.staged.partial"]


def test_a_restore_that_fails_is_best_effort(tmp_path: Path) -> None:
    destination = tmp_path / "close-summary.md"
    report._restore_quietly(tmp_path / "already-gone.partial", destination)
    assert not destination.exists()  # nosec B101: pytest outcome assertion


def test_a_destination_with_no_existing_ancestor_is_not_refused(monkeypatch: pytest.MonkeyPatch) -> None:
    # A Windows path on a drive letter that is not mapped has no ancestor to stat,
    # so every candidate is missing and the writer is left to report it in mkdir.
    def missing(self: Path, *args: object, **kwargs: object) -> os.stat_result:
        raise FileNotFoundError(2, "No such file or directory", str(self))

    monkeypatch.setattr(Path, "stat", missing)
    assert report._reject_unreachable(Path("unmapped") / "pack") is None  # nosec B101: pytest outcome assertion
