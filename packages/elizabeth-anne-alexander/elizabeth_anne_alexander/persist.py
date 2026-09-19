"""Atomic write of one evaluation pack (model, evidence, receipt)."""
from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any

from .errors import GatewayError
from .util import build_root, path_within


def _write_json(path: Path, payload: dict[str, Any]) -> None:
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _replace(source: Path, destination: Path) -> None:
    os.replace(source, destination)


def write_validation(payload: dict[str, Any], output: Path, protected: tuple[Path, ...]) -> None:
    """Write validation output without following a changed destination pathname."""
    try:
        output.parent.mkdir(parents=True, exist_ok=True)
        directory_fd = os.open(output.parent, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    except OSError as exc:
        raise GatewayError(f"validation output cannot be written to {output}: {exc}.") from exc

    temporary_name = f".{output.name}.partial"
    temporary_fd = None
    temporary_created = False
    try:
        # O_EXCL prevents a pre-existing pathname (including a symlink) from
        # being used as the staging file.
        temporary_fd = os.open(temporary_name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600, dir_fd=directory_fd)
        temporary_created = True
        with os.fdopen(temporary_fd, "w", encoding="utf-8") as stream:
            temporary_fd = None
            stream.write(json.dumps(payload, indent=2, sort_keys=True) + "\n")
            stream.flush()
            os.fsync(stream.fileno())

        destination_stat = None
        try:
            destination_stat = os.stat(output.name, dir_fd=directory_fd, follow_symlinks=True)
        except FileNotFoundError:
            pass
        if destination_stat is not None:
            for input_path in protected:
                try:
                    input_stat = input_path.stat()
                except OSError as exc:
                    raise GatewayError(f"validation output cannot be checked against {input_path}: {exc}.") from exc
                if (destination_stat.st_dev, destination_stat.st_ino) == (input_stat.st_dev, input_stat.st_ino):
                    raise GatewayError(f"validation output must not overwrite a review input: {input_path}.")
        os.replace(temporary_name, output.name, src_dir_fd=directory_fd, dst_dir_fd=directory_fd)
    except GatewayError:
        raise
    except OSError as exc:
        raise GatewayError(f"validation output cannot be written to {output}: {exc}.") from exc
    finally:
        if temporary_fd is not None:
            os.close(temporary_fd)
        if temporary_created:
            try:
                os.unlink(temporary_name, dir_fd=directory_fd)
            except OSError:
                pass
        os.close(directory_fd)


def write_evaluation(model: dict[str, Any], evidence: dict[str, Any], receipt: dict[str, Any], output_dir: Path) -> dict[str, Path]:
    """Stage all 3 artefacts, then move them into place, receipt last.

    The 3 files describe one run. Writing them straight into the output
    directory means an interrupted second run can leave a truncated file, or a
    new model-result.json beside the previous run's evidence and receipt. Each
    file is written under a temporary name first, and nothing is moved until
    all 3 staged files exist.

    Three separate moves are not one atomic step, so a failure between them can
    still leave one new file beside 2 old ones. The receipt seals both the
    evidence and the model result and is moved last, so validate_review refuses
    every such mixed pack rather than reporting a decision against artefacts
    that came from 2 different runs.
    """
    output_dir = path_within(output_dir, build_root(), label="output directory", require_exists=False)
    paths = {"model": output_dir / "model-result.json", "evidence": output_dir / "reviewer-evidence.json", "receipt": output_dir / "receipt.json"}
    staged = {key: path.with_name(path.name + ".partial") for key, path in paths.items()}
    try:
        output_dir.mkdir(parents=True, exist_ok=True)
        for key, payload in (("model", model), ("evidence", evidence), ("receipt", receipt)):
            _write_json(staged[key], payload)
        for key in ("model", "evidence", "receipt"):
            _replace(staged[key], paths[key])
    except OSError as exc:
        for temporary in staged.values():
            try:
                temporary.unlink()
            except OSError:
                pass
        raise GatewayError(f"run output cannot be written to {output_dir}: {exc}.") from exc
    return paths
