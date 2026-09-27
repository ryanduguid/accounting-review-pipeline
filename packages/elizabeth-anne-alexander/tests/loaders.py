"""Snapshot-discarding loaders for tests that only need the parsed value."""

from __future__ import annotations

from pathlib import Path
from typing import Any

from elizabeth_anne_alexander.gateway import (
    BalanceRow,
    Source,
    _load_context_snapshot,
    _load_policy_snapshot,
    _load_request_snapshot,
    _load_tb_snapshot,
)
from elizabeth_anne_alexander.util import snapshot_file


def _load_tb(path: Path) -> tuple[BalanceRow, ...]:
    return _load_tb_snapshot(snapshot_file(path, label="source CSV"))


def _load_context(path: Path) -> tuple[Source, Source]:
    current, prior, _snapshot = _load_context_snapshot(path)
    return current, prior


def _load_policy(path: Path) -> dict[str, Any]:
    policy, _snapshot = _load_policy_snapshot(path)
    return policy


def _load_request(path: Path, policy: dict[str, Any]) -> dict[str, Any]:
    request, _snapshot = _load_request_snapshot(path, policy)
    return request
