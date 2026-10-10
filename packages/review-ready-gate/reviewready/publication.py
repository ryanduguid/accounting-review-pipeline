"""Fixed pack publication for cooperating writers and the read-only viewer.

An unresolved process death keeps its evidence and refuses another publication.
This protocol provides atomic visibility of its state record, not power-loss durability.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import stat
import sys
import uuid
from collections.abc import Callable, Iterator, Sequence
from contextlib import contextmanager
from contextvars import ContextVar
from pathlib import Path
from typing import Any

CONTROL_NAME = ".reviewready"
NAMES = ("readiness-pack.json", "readiness-summary.md", "findings.csv")
_ID = re.compile(r"[0-9a-f]{32}\Z")
_OWNED = {f"{kind}-{index}" for kind in ("stage", "backup", "restore") for index in range(3)}


class PublicationCancelled(OSError):
    """A CLI cancellation observed at a safe publication checkpoint."""


class CleanupStateUncertain(OSError):
    """Cleanup could not confirm its final control-state replacement."""


_cancellation_check: ContextVar[Callable[[], None] | None] = ContextVar(
    "publication_cancellation_check", default=None
)


def _check_cancellation() -> None:
    check = _cancellation_check.get()
    if check is not None:
        check()


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate publication state key")
        result[key] = value
    return result


def _regular(path: Path, *, absent: bool = False) -> bool:
    try:
        info = path.lstat()
    except FileNotFoundError:
        if absent:
            return False
        raise
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
        raise OSError(f"publication refuses a non-regular or linked file: {path.name}")
    return True


def _directory(path: Path) -> None:
    info = path.lstat()
    if not stat.S_ISDIR(info.st_mode) or (
        getattr(info, "st_file_attributes", 0) & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0)
    ):
        raise OSError(f"publication refuses a non-directory or link: {path.name}")


def _read_state(control: Path) -> dict[str, Any]:
    _directory(control)
    _regular(control / "state.json")
    try:
        state = json.loads((control / "state.json").read_bytes(), object_pairs_hook=_unique_object)
    except (ValueError, UnicodeError) as exc:
        raise OSError("publication state is malformed; retain recovery evidence") from exc
    if (
        not isinstance(state, dict)
        or set(state) != {"format", "status", "revision", "transaction", "previous"}
        or type(state["format"]) is not int
        or state["format"] != 1
        or state["status"] not in ("settled", "in_progress")
        or not isinstance(state["revision"], str)
        or not _ID.fullmatch(state["revision"])
        or (state["transaction"] is not None and (
            not isinstance(state["transaction"], str)
            or not _ID.fullmatch(state["transaction"])
        ))
        or not isinstance(state["previous"], list)
        or len(state["previous"]) != 3
        or any(value is not None and (
            not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{64}", value)
        ) for value in state["previous"])
        or (state["status"] == "in_progress" and state["transaction"] is None)
    ):
        raise OSError("publication state is unsupported; retain recovery evidence")
    return state


def _write_state(control: Path, state: dict[str, Any]) -> None:
    temporary = control / "state.next"
    with temporary.open("xb") as stream:
        stream.write((json.dumps(state, sort_keys=True) + "\n").encode("utf-8"))
    os.replace(temporary, control / "state.json")


def snapshot_revision(output: Path) -> str | None:
    """Observe publication without creating or changing any file."""
    control = output / CONTROL_NAME
    try:
        control.lstat()
    except FileNotFoundError:
        return None
    state = _read_state(control)
    if state["status"] != "settled":
        raise OSError("pack publication is unresolved; retain recovery evidence")
    return str(state["revision"])


@contextmanager
def _admission(control: Path) -> Iterator[None]:
    try:
        control.mkdir()
    except FileExistsError:
        _read_state(control)
        _regular(control / "writer.lock")
    _directory(control)
    lock = control / "writer.lock"
    _regular(lock, absent=True)
    flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0)
    descriptor = os.open(lock, flags, 0o600)
    acquired = False
    try:
        if os.fstat(descriptor).st_size == 0:
            os.write(descriptor, b"1")
        os.lseek(descriptor, 0, os.SEEK_SET)
        try:
            if sys.platform == "win32":
                import msvcrt

                msvcrt.locking(descriptor, msvcrt.LK_NBLCK, 1)
            else:
                import fcntl

                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as exc:
            raise OSError("another pack writer holds admission") from exc
        acquired = True
        yield
    finally:
        try:
            if acquired:
                os.lseek(descriptor, 0, os.SEEK_SET)
                if sys.platform == "win32":
                    import msvcrt

                    msvcrt.locking(descriptor, msvcrt.LK_UNLCK, 1)
                else:
                    import fcntl

                    fcntl.flock(descriptor, fcntl.LOCK_UN)
        finally:
            os.close(descriptor)


def _validate_control(control: Path, state: dict[str, Any]) -> None:
    allowed = {"writer.lock", "state.json"}
    if state["transaction"] is not None:
        allowed.add("tx-" + state["transaction"])
    if {path.name for path in control.iterdir()} - allowed:
        raise OSError("unknown publication control entries; retain recovery evidence")
    _regular(control / "writer.lock")


def _cleanup(control: Path, state: dict[str, Any]) -> None:
    _validate_control(control, state)
    identity = state["transaction"]
    if identity is None:
        return
    transaction = control / ("tx-" + identity)
    try:
        transaction.lstat()
    except FileNotFoundError as exc:
        raise OSError(
            "referenced publication transaction is missing; retain control for review"
        ) from exc
    else:
        _directory(transaction)
        entries = list(transaction.iterdir())
        if {path.name for path in entries} - _OWNED:
            raise OSError("unknown transaction entries; retain recovery evidence")
        for path in entries:
            _regular(path)
        for path in entries:
            path.unlink()
        transaction.rmdir()
    try:
        _write_state(control, {**state, "transaction": None, "previous": [None] * 3})
    except OSError as exc:
        raise CleanupStateUncertain("cleanup state uncertain; retain control for review") from exc


def publish(output: Path, rendered: Sequence[tuple[Path, str, str, str | None]]) -> None:
    """Publish the three known slots; retain evidence when restoration is uncertain."""
    if tuple(path.name for path, *_ in rendered) != NAMES:
        raise ValueError("publication requires the three fixed pack slots")
    for name in NAMES:
        _regular(output / name, absent=True)
    control = output / CONTROL_NAME
    with _admission(control):
        if not (control / "state.json").exists():
            if {path.name for path in control.iterdir()} != {"writer.lock"}:
                raise OSError("uninitialised publication control; retain recovery evidence")
            _write_state(control, {
                "format": 1, "status": "settled", "revision": uuid.uuid4().hex,
                "transaction": None, "previous": [None] * 3,
            })
        _check_cancellation()
        state = _read_state(control)
        if state["status"] != "settled":
            raise OSError("unresolved pack publication; retain recovery evidence for review")
        _cleanup(control, state)
        _check_cancellation()
        identity = uuid.uuid4().hex
        state = {**state, "status": "in_progress", "transaction": identity, "previous": [None] * 3}
        transaction = control / ("tx-" + identity)
        backups_complete = False
        commit_revision = uuid.uuid4().hex
        try:
            transaction.mkdir()
            _write_state(control, state)
            _check_cancellation()
            previous: list[str | None] = []
            for index, name in enumerate(NAMES):
                destination = output / name
                if _regular(destination, absent=True):
                    backup = transaction / f"backup-{index}"
                    with destination.open("rb") as source, backup.open("xb") as sink:
                        shutil.copyfileobj(source, sink)
                    previous.append(hashlib.sha256(backup.read_bytes()).hexdigest())
                else:
                    previous.append(None)
                _check_cancellation()
            for index, (_path, text, encoding, newline) in enumerate(rendered):
                stage = transaction / f"stage-{index}"
                with stage.open("x", encoding=encoding, newline=newline):
                    pass
                stage.write_text(text, encoding=encoding, newline=newline)
                _check_cancellation()
            state = {**state, "previous": previous}
            _write_state(control, state)
            backups_complete = True
            _check_cancellation()
            for index, name in enumerate(NAMES):
                os.replace(transaction / f"stage-{index}", output / name)
                _check_cancellation()
            settled = {**state, "status": "settled", "revision": commit_revision}
            _write_state(control, settled)
            _check_cancellation()
        except BaseException as failure:
            try:
                observed = _read_state(control)
            except OSError as exc:
                raise OSError("publication state uncertain; retain recovery evidence") from exc
            if observed["status"] == "settled" and observed["revision"] == commit_revision:
                raise OSError("pack published; cleanup incomplete, inspect retained control before retry") from failure
            try:
                if backups_complete:
                    for index, name in enumerate(NAMES):
                        destination = output / name
                        if state["previous"][index] is None:
                            if _regular(destination, absent=True):
                                destination.unlink()
                        else:
                            backup = transaction / f"backup-{index}"
                            _regular(backup)
                            if hashlib.sha256(backup.read_bytes()).hexdigest() != state["previous"][index]:
                                raise OSError("recovery backup differs; retain recovery evidence")
                            if _regular(destination, absent=True) and hashlib.sha256(
                                destination.read_bytes()
                            ).hexdigest() == state["previous"][index]:
                                continue
                            restore = transaction / f"restore-{index}"
                            with backup.open("rb") as source, restore.open("xb") as sink:
                                shutil.copyfileobj(source, sink)
                            os.replace(restore, destination)
                settled = {**state, "status": "settled", "revision": uuid.uuid4().hex}
                _write_state(control, settled)
                _cleanup(control, settled)
            except BaseException as recovery_failure:
                raise OSError(
                    "publication or recovery incomplete; retain control and backups for review"
                ) from recovery_failure
            raise
        try:
            _cleanup(control, settled)
        except CleanupStateUncertain as exc:
            raise OSError("pack published; cleanup state uncertain, requires review") from exc
        except BaseException as exc:
            raise OSError("pack published; cleanup incomplete, inspect retained control before retry") from exc
        try:
            _check_cancellation()
        except PublicationCancelled as exc:
            raise OSError("pack published; cancellation after commit") from exc
