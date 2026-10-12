"""Publication control paths: admission locking on each platform and recovery refusals."""

from __future__ import annotations

import os
import sys
import types
from pathlib import Path

import pytest
from reviewready import publication
from reviewready.report import PACK_FILE_NAMES, write_review_pack
from tests.test_gate import _not_ready_pack, _ready_pack
from tests.test_publication import Cancelled, assert_clean, payloads


def test_control_without_state_is_not_treated_as_absent(tmp_path: Path) -> None:
    (tmp_path / publication.CONTROL_NAME).mkdir()
    with pytest.raises(FileNotFoundError):
        publication.snapshot_revision(tmp_path)


def test_control_that_is_a_file_is_refused(tmp_path: Path) -> None:
    (tmp_path / publication.CONTROL_NAME).write_text("not a directory", encoding="utf-8")
    with pytest.raises(OSError, match="refuses a non-directory or link: .reviewready"):
        publication.snapshot_revision(tmp_path)


def test_live_writer_in_this_process_refuses_a_second_admission(tmp_path: Path) -> None:
    output = tmp_path / "pack"
    write_review_pack(_ready_pack(), output)
    previous = payloads(output)
    control = output / publication.CONTROL_NAME
    with publication._admission(control):
        with pytest.raises(OSError, match="another pack writer holds admission"):
            write_review_pack(_not_ready_pack(), output)
    assert payloads(output) == previous  # nosec B101: pytest outcome assertion
    write_review_pack(_not_ready_pack(), output)
    assert_clean(output)


def _fake_lock_modules(monkeypatch: pytest.MonkeyPatch, platform: str, *, held: bool) -> list:
    """Stand in for msvcrt and fcntl, recording each lock call for one platform."""
    calls: list = []

    def locking(descriptor: int, mode: str, size: int) -> None:
        calls.append(("msvcrt", mode, size))
        if held and mode == "LK_NBLCK":
            raise OSError("locked")

    def flock(descriptor: int, operation: int) -> None:
        calls.append(("fcntl", operation))
        if held and operation == 2 | 4:
            raise OSError("locked")

    msvcrt = types.SimpleNamespace(LK_NBLCK="LK_NBLCK", LK_UNLCK="LK_UNLCK", locking=locking)
    fcntl = types.SimpleNamespace(LOCK_EX=2, LOCK_NB=4, LOCK_UN=8, flock=flock)
    monkeypatch.setitem(sys.modules, "msvcrt", msvcrt)
    monkeypatch.setitem(sys.modules, "fcntl", fcntl)
    monkeypatch.setattr(publication, "sys", types.SimpleNamespace(platform=platform))
    return calls


@pytest.mark.parametrize(
    "platform,expected",
    [
        ("win32", [("msvcrt", "LK_NBLCK", 1), ("msvcrt", "LK_UNLCK", 1)]),
        ("linux", [("fcntl", 2 | 4), ("fcntl", 8)]),
    ],
)
def test_admission_locks_and_releases_with_the_platform_primitive(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, platform: str, expected: list
) -> None:
    calls = _fake_lock_modules(monkeypatch, platform, held=False)
    control = tmp_path / publication.CONTROL_NAME
    with publication._admission(control):
        assert calls == expected[:1]  # nosec B101: pytest outcome assertion
    assert calls == expected  # nosec B101: pytest outcome assertion
    assert (control / "writer.lock").read_bytes() == b"1"  # nosec B101: pytest outcome assertion


@pytest.mark.parametrize("platform", ["win32", "linux"])
def test_held_lock_refuses_admission_without_releasing_it(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, platform: str
) -> None:
    calls = _fake_lock_modules(monkeypatch, platform, held=True)
    with pytest.raises(OSError, match="another pack writer holds admission"):
        with publication._admission(tmp_path / publication.CONTROL_NAME):
            pass
    assert len(calls) == 1  # nosec B101: pytest outcome assertion


def test_cancelled_install_restores_a_slot_that_was_absent(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    output = tmp_path / "pack"
    write_review_pack(_ready_pack(), output)
    (output / PACK_FILE_NAMES[0]).unlink()
    previous = payloads(output)
    replace = os.replace

    def cancel_first_install(source, destination):
        if Path(source).name == "stage-0":
            raise Cancelled()
        replace(source, destination)

    monkeypatch.setattr(publication.os, "replace", cancel_first_install)
    with pytest.raises(Cancelled):
        write_review_pack(_not_ready_pack(), output)
    assert payloads(output) == previous  # nosec B101: pytest outcome assertion
    assert not (output / PACK_FILE_NAMES[0]).exists()  # nosec B101: pytest outcome assertion


def test_unrecorded_transaction_that_cannot_be_removed_is_retained(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    output = tmp_path / "pack"
    write_review_pack(_ready_pack(), output)
    previous = payloads(output)
    control = output / publication.CONTROL_NAME
    replace = os.replace

    def fail_recording(source, destination):
        if Path(source).name == "state.next" and b'"in_progress"' in Path(source).read_bytes():
            stray = next(control.glob("tx-*")) / "stray"
            stray.write_bytes(b"")
            raise OSError("state write denied")
        replace(source, destination)

    monkeypatch.setattr(publication.os, "replace", fail_recording)
    with pytest.raises(OSError, match="unrecorded transaction remains"):
        write_review_pack(_not_ready_pack(), output)
    assert payloads(output) == previous  # nosec B101: pytest outcome assertion
    assert len(list(control.glob("tx-*/stray"))) == 1  # nosec B101: pytest outcome assertion


def test_unreadable_state_during_recovery_keeps_evidence(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    output = tmp_path / "pack"
    write_review_pack(_ready_pack(), output)
    control = output / publication.CONTROL_NAME
    replace = os.replace

    def corrupt_state(source, destination):
        if Path(source).name == "stage-0":
            (control / "state.json").write_bytes(b"{")
            raise Cancelled()
        replace(source, destination)

    monkeypatch.setattr(publication.os, "replace", corrupt_state)
    with pytest.raises(OSError, match="publication state uncertain; retain recovery evidence"):
        write_review_pack(_not_ready_pack(), output)
    assert len(list(control.glob("tx-*/backup-*"))) == 3  # nosec B101: pytest outcome assertion
