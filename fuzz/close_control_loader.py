"""Fuzz Monthly Close Controls' CSV loaders with fabricated input files."""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path

# The fuzzing job installs only Atheris, so import the component from source.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "packages/monthly-close-control-plane"))

from closecontrol import loader  # noqa: E402
from closecontrol.errors import ControlInputError  # noqa: E402

SCRATCH = Path(tempfile.mkdtemp(prefix="closecontrol-fuzz-")) / "input.csv"
LOADERS = (
    loader.load_canonical_tb,
    loader.load_mapping,
    loader.load_mapping_policy,
    loader.load_balance_policy,
    loader.load_subledger,
)


def test_one_input(data: bytes) -> None:
    SCRATCH.write_bytes(data)
    for load in LOADERS:
        # Every malformed file must be refused with the component's input error,
        # which the command line reports; anything else escaping is a crash.
        try:
            load(SCRATCH)
        except ControlInputError:
            pass


if __name__ == "__main__":
    import atheris

    atheris.instrument_all()
    atheris.Setup(sys.argv, test_one_input)
    atheris.Fuzz()
