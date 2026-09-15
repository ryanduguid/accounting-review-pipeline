"""Select workflow jobs from a Git diff without losing the source of a rename."""

from __future__ import annotations

import fnmatch
import os
import subprocess


def changed_paths(base: str, head: str) -> list[str]:
    result = subprocess.run(
        ["git", "diff", "--name-only", "--no-renames", "-z", base, head],
        check=True,
        capture_output=True,
        text=True,
    )
    return [path for path in result.stdout.split("\0") if path]


def matches(paths: list[str], patterns: list[str]) -> bool:
    return any(
        fnmatch.fnmatchcase(path, pattern) for path in paths for pattern in patterns
    )


def main() -> None:
    base = os.environ.get("CI_BASE", "")
    head = os.environ.get("CI_HEAD", "HEAD")
    patterns = os.environ["CI_PATHS"].splitlines()
    if not any(patterns):
        raise ValueError("CI_PATHS must include at least one path pattern")
    known_base = (
        bool(base)
        and subprocess.run(
            ["git", "cat-file", "-e", f"{base}^{{commit}}"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        ).returncode
        == 0
    )
    selected = not known_base or matches(changed_paths(base, head), patterns)
    print(f"run={str(selected).lower()}")


if __name__ == "__main__":
    main()
