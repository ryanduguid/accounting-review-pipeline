# Development entrypoint for the whole repository.
#
# These recipes are a convenience, not a new authority. Each component is still
# checked and released from its own directory with its own documented commands,
# and CI runs those commands per component; the recipes here loop over the same
# commands so a fresh clone can be verified from the top.
#
# Requires just and uv:
#     uv tool install rust-just
#
# The five Python distributions are uv workspace members. The Excel adapter and
# the Power BI application have no pyproject.toml; their standard-library
# unittest suites run from the same .venv. Adding a component means adding it to
# the matching list below and to the root pyproject.toml workspace members.

# pytest components: directory:import package
pytest_components := trim(replace('''
packages/review-ready-gate:reviewready
packages/monthly-close-control-plane:closecontrol
packages/elizabeth-anne-alexander:elizabeth_anne_alexander
packages/evatt:evatt
''', "\n", " "))

# unittest components: directory only
unittest_components := trim(replace('''
packages/xero-trial-balance-export
adapters/accounting-excel-toolkit
apps/australian-accounting-power-bi
''', "\n", " "))

# The exporter is flat modules, not a package; its lint and mypy targets are files.
exporter_modules := "xero_client.py export_tb.py auth.py token_store.py"

# List the available recipes.
default:
    @just --list

# Install every Python component and the shared toolchain into one workspace .venv.
setup:
    uv sync

# Run every component's test suite, then the joined trial-balance conformance
# test and the root check on the blocks the two review packages each copy.
test: setup
    #!/usr/bin/env bash
    set -euo pipefail
    # No -q here: each component's pyproject addopts already supplies it, and a
    # second one is -qq, which drops the "N passed" summary.
    for entry in {{ pytest_components }}; do
        directory="${entry%%:*}"
        echo "==> ${directory}"
        (cd "${directory}" && uv run --no-sync pytest)
    done
    for directory in {{ unittest_components }}; do
        echo "==> ${directory}"
        (cd "${directory}" && uv run --no-sync python -B -m unittest discover -s tests -v)
    done
    echo "==> joined conformance"
    uv run --no-sync python -B -m unittest tests.test_xero_trial_balance_contract -v
    echo "==> shared blocks"
    uv run --no-sync python -B -m unittest tests.test_shared_blocks -v

# Lint every Python component with Ruff.
lint: setup
    #!/usr/bin/env bash
    set -euo pipefail
    for entry in {{ pytest_components }}; do
        directory="${entry%%:*}"
        package="${entry##*:}"
        echo "==> ${directory}"
        (cd "${directory}" && uv run --no-sync ruff check "${package}" tests)
    done
    echo "==> packages/xero-trial-balance-export"
    (cd packages/xero-trial-balance-export && uv run --no-sync ruff check {{ exporter_modules }})

# Type-check every Python component with mypy.
typecheck: setup
    #!/usr/bin/env bash
    set -euo pipefail
    for entry in {{ pytest_components }}; do
        directory="${entry%%:*}"
        package="${entry##*:}"
        echo "==> ${directory}"
        (cd "${directory}" && uv run --no-sync mypy "${package}")
    done
    echo "==> packages/xero-trial-balance-export"
    (cd packages/xero-trial-balance-export && uv run --no-sync mypy --ignore-missing-imports --pretty {{ exporter_modules }})

# Not a CI equivalent. The component workflows also verify the component
# lockfile, install the exporter's hash-locked requirements, build each
# distribution, install and smoke-test the built wheel outside the checkout,
# run actionlint, validate the Power BI
# report with Microsoft's PBIR CLI, check the contract digests and run CodeQL.
# None of those run here. A green `just check` is not a green CI.

# The fast local pass: lint, type-check and test every component.
check: lint typecheck test
