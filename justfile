# Development entrypoint for the whole repository.
#
# These recipes are a convenience, not a new authority. Each component is still
# checked and released from its own directory with its own documented commands,
# and CI runs those commands per component; the recipes here loop over the same
# commands so a fresh clone can be verified from the top.
#
# Requires just, uv and cargo:
#     uv tool install rust-just
#
# The two Python distributions are uv workspace members. The Xero Ledger Review
# Gate, evatt and Xero exporter are Rust crates, run with cargo from their directories. The Excel adapter and
# the Power BI application have no pyproject.toml, and the grant workpapers
# application is not a member; its workpaper is a Rust crate run with cargo from
# its directory. Their standard-library unittest suites run from
# the same .venv. Adding a component means adding it to
# the matching list below and to the root pyproject.toml workspace members.

# pytest components: directory:import package
pytest_components := trim(replace('''
packages/review-ready-gate:reviewready
packages/monthly-close-control-plane:closecontrol
''', "\n", " "))

# unittest components: directory only
unittest_components := trim(replace('''
adapters/accounting-excel-toolkit
apps/australian-accounting-power-bi
apps/grant-acquittal-workpapers
''', "\n", " "))

# cargo components: directory only
rust_components := "apps/australian-accounting-power-bi apps/grant-acquittal-workpapers packages/elizabeth-anne-alexander packages/evatt packages/xero-trial-balance-export"

# List the available recipes.
default:
    @just --list

# Install every Python component and the shared toolchain into one workspace .venv.
setup:
    uv sync

# Reproduce the fictional close pack outside this checkout. REVIEW preserves exit 2.
demo:
    uv run --locked close-control review --current packages/monthly-close-control-plane/examples/current_trial_balance.csv --prior packages/monthly-close-control-plane/examples/prior_trial_balance.csv --mapping packages/monthly-close-control-plane/examples/account_mapping.csv --subledger packages/monthly-close-control-plane/examples/subledger_balances.csv --absolute-threshold 10000 --percentage-threshold 0.10 --reconciliation-tolerance 0.01 --review-note packages/monthly-close-control-plane/examples/review_note.json --output ../close-control-demo

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
    for directory in {{ rust_components }}; do
        echo "==> ${directory}"
        (cd "${directory}" && cargo test --locked --all-features)
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
    for directory in {{ rust_components }}; do
        echo "==> ${directory}"
        (cd "${directory}" && cargo fmt --check && cargo clippy --locked --all-targets --all-features -- -D warnings)
    done

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

# Regenerate the root lock and each Python component's standalone release lock.
# Run it after any Python dependency change, including on a Dependabot
# branch before merging. Inside the workspace `uv lock` writes only the root lock,
# so each component is relocked from a copy outside the checkout, as CI checks it.
relock:
    #!/usr/bin/env bash
    set -euo pipefail
    echo "==> root"
    uv lock
    for lock in packages/*/uv.lock apps/*/uv.lock; do
        directory="$(dirname "${lock}")"
        [ -f "${directory}/pyproject.toml" ] || continue
        echo "==> ${directory}"
        standalone="$(mktemp -d)"
        tar -C "${directory}" --exclude=.venv --exclude=target -cf - . | tar -C "${standalone}" -xf -
        (cd "${standalone}" && uv lock)
        cp "${standalone}/uv.lock" "${lock}"
        if [ -f "${directory}/requirements.lock" ]; then
            (cd "${standalone}" && uv export --locked --no-dev --no-emit-project \
                --format requirements-txt --output-file requirements.lock >/dev/null)
            cp "${standalone}/requirements.lock" "${directory}/requirements.lock"
        fi
        rm -rf "${standalone}"
    done

# Not a CI equivalent. The component workflows also verify the component
# lockfile, run the exporter's pinned Python comparison, build each
# distribution, install and smoke-test the built wheel outside the checkout,
# run actionlint, validate the Power BI
# report with Microsoft's PBIR CLI, check the contract digests and run CodeQL.
# None of those run here. A green `just check` is not a green CI.

# The fast local pass: lint, type-check and test every component.
check: lint typecheck test
