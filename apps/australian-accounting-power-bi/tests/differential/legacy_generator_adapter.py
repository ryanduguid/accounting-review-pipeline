# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

"""Staged as generate_fixtures.py beside the unchanged, pinned Python source."""

import importlib.util
from pathlib import Path

_spec = importlib.util.spec_from_file_location(
    "_fixture_migration", Path(__file__).with_name("legacy_fixture_migration.py"))
_migration = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_migration)
_legacy = _migration.load_generator(Path(__file__).with_name("_frozen_generate_fixtures.py"))
# Retain the frozen functions and mutable rate/calendar objects used by its suites.
globals().update({name: value for name, value in vars(_legacy).items() if not name.startswith("_")})


def generate_fixtures():
    _legacy.SAMPLES_DIR = SAMPLES_DIR
    _legacy.generate_fixtures()
    _migration.migrate_samples(SAMPLES_DIR)


if __name__ == "__main__":
    generate_fixtures()
