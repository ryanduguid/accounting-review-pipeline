from __future__ import annotations

import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EXAMPLES = ROOT / "examples"


def copy_example_pack(name: str, dest: Path) -> Path:
    """Copy one fabricated pack directory into dest, files only."""
    dest.mkdir(parents=True, exist_ok=True)
    for path in (EXAMPLES / name).iterdir():
        if path.is_file():
            shutil.copyfile(path, dest / path.name)
    return dest
