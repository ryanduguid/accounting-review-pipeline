# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

"""The approved identifier-only delta from the frozen 73f2a438 Python oracle.

Hashes bind both sides. No expected bytes come from the application under test,
so a damaged current fixture cannot become its own regeneration expectation.
"""

import hashlib
import importlib.util
from pathlib import Path

GENERATOR_SHA256 = "f248d725405ac65963b56965aa9d90ea0c003d13fc99b246b8a2996f966f539f"
TEST_SHA256 = {
    "test_fixtures_balance": "015e1e0a4247d7baf37ba42bb72123d0d0cb9e99d4ac6388574b5a4d3466c0dc",
    "test_model_bpa_rules": "ba3924328189411dfc459c0f1352942f5a89a956c641d1d7057b4a253258248d",
    "test_payday_super_rules": "2876efaf37e9634809b1e0b8f9589b944945a6f84a4f904a5bd8e282ba018018",
    "test_pbip_structure": "a3cf71223b43c1ad482e9e1a3eb28d6e12da395ec2a85a839c6a857b82487e0e",
    "test_powerquery_m": "3ce04847576adb858878f800d887d2de128778e4c5665b997f5f66b6b40fa415",
    "test_tmdl_integrity": "d263843a03b837af9942dab35bd284715c8c70c6bec5821f947e84010eafd3c0",
    "test_xero_trial_balance_contract": "14a8867ff882dcd74555e0a505d8a21ad595800776c2db391cebc08271a0813f",
}
LEGACY_SHA256 = {
    "sample-entities.csv": "8a53134cb8205ef868df627d9348523056eeda5891883ba16bd8f6ada187530e",
    "sample-chart-of-accounts.csv": "a7dd1c84db220a8990deb4b85a9203e65c4b29ab6043b45b870ec122272634c6",
    "sample-general-ledger.csv": "1e4e6d9f72ef7602e8ee0aceb3b38ec2ca33524af59b0dc23e236a82e7b162a1",
    "sample-budgets.csv": "3c6aeba76a83014c3fefab57680682362dd7dc02730da44e8300e44d9899cf1c",
    "sample-payroll-super.csv": "c914894cd7ce27b9df9448807950ab7df9e280b705f059f27e46aafd569d5532",
    "sample-ato-benchmarks.csv": "13c58ee6f0faed16b1af4aa631c2798b1540dfc3cf4a65d6ea565ca270333744",
}
CURRENT_SHA256 = LEGACY_SHA256 | {
    "sample-entities.csv": "2c38e72f7930edf6cc6cbe33a0eb491c7c754bb510483226a40cc529aa328775",
    "sample-payroll-super.csv": "b069307b6b002d107ee313873c4bcb47f9b7b83d800169c8e3324825eaa5652d",
}
REPLACEMENTS = {
    "sample-entities.csv": (
        (b",51824753556,123456789,", b",99900000180,,", 1),
        (b",82147609350,234567890,", b",82147609350,,", 1),
        (b",33087542991,345678901,", b",33087542991,,", 1),
    ),
    "sample-payroll-super.csv": (
        (b",19323234999001,AustralianSuper,", b",DEMO-FUND-001,Varrock Sample Super,", 156),
        (b",16457520308001,ART Super,", b",DEMO-FUND-002,Draynor Sample Super,", 78),
        (b",60905115063001,Hostplus,", b",DEMO-FUND-003,Falador Sample Super,", 78),
    ),
}


def require_hash(data: bytes, expected: str, label: str) -> None:
    if hashlib.sha256(data).hexdigest() != expected:
        raise ValueError(f"Unrecognised {label}; expected SHA-256 {expected}")


def load_generator(script: Path):
    require_hash(script.read_bytes(), GENERATOR_SHA256, "frozen Python generator")
    spec = importlib.util.spec_from_file_location("_frozen_generate_fixtures", script)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def migrate_samples(samples: Path) -> None:
    migrated = {}
    for name, expected in LEGACY_SHA256.items():
        data = (samples / name).read_bytes()
        require_hash(data, expected, f"legacy {name}")
        for old, new, count in REPLACEMENTS.get(name, ()):
            if data.count(old) != count:
                raise ValueError(f"Unexpected identifier occurrence count in {name}")
            data = data.replace(old, new)
        require_hash(data, CURRENT_SHA256[name], f"migrated {name}")
        migrated[name] = data
    # Validate every output before replacing any scratch file.
    for name in REPLACEMENTS:
        (samples / name).write_bytes(migrated[name])
