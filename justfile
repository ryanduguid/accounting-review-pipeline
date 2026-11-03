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
# unittest components: directory only
unittest_components := trim(replace('''
adapters/accounting-excel-toolkit
apps/grant-acquittal-workpapers
''', "\n", " "))

# cargo components: directory only
rust_components := "adapters/accounting-excel-toolkit apps/australian-accounting-power-bi apps/grant-acquittal-workpapers packages/elizabeth-anne-alexander packages/evatt packages/xero-trial-balance-export packages/review-ready-gate packages/monthly-close-control-plane tests/joined-conformance"

# List the available recipes.
default:
    @just --list

# Install the shared development toolchain.
setup:
    uv sync --locked --group dev

# Reproduce the fictional close pack outside this checkout. The sample needs
# review, so the CLI exits 2; the recipe succeeds on that exit alone.
demo:
    #!/usr/bin/env bash
    set -uo pipefail
    cargo run --locked --release --manifest-path packages/monthly-close-control-plane/Cargo.toml --bin close-control -- review --current packages/monthly-close-control-plane/examples/current_trial_balance.csv --prior packages/monthly-close-control-plane/examples/prior_trial_balance.csv --mapping packages/monthly-close-control-plane/examples/account_mapping.csv --subledger packages/monthly-close-control-plane/examples/subledger_balances.csv --absolute-threshold 10000 --percentage-threshold 0.10 --reconciliation-tolerance 0.01 --review-note packages/monthly-close-control-plane/examples/review_note.json --output ../close-control-demo
    status=$?
    if [ "${status}" -ne 2 ]; then
        echo "demo: expected exit 2 (REVIEW), got ${status}" >&2
        exit 1
    fi

# Run every component's test suite; tests/joined-conformance, among the Rust
# components, holds the joined trial-balance and shared-block conformance checks.
test: setup
    #!/usr/bin/env bash
    set -euo pipefail
    for directory in {{ unittest_components }}; do
        echo "==> ${directory}"
        (cd "${directory}" && uv run --no-sync python -B -m unittest discover -s tests -v)
    done
    for directory in {{ rust_components }}; do
        echo "==> ${directory}"
        (cd "${directory}" && cargo test --locked --all-features)
    done

# Lint retained Python tooling and native components.
lint: setup
    #!/usr/bin/env bash
    set -euo pipefail
    uv run --no-sync ruff check .
    for directory in {{ rust_components }}; do
        echo "==> ${directory}"
        (cd "${directory}" && cargo fmt --check && cargo clippy --locked --all-targets --all-features -- -D warnings)
    done

# Type-check the standard library consumers with their component settings.
typecheck: setup
    #!/usr/bin/env bash
    set -euo pipefail
    python="$(uv run --no-sync python -c 'import sys; print(sys.executable)')"
    for directory in {{ unittest_components }}; do
        (cd "${directory}" && "${python}" -m mypy)
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
# lockfile, run the frozen Python comparisons, build and smoke-test native
# binaries outside the checkout,
# run actionlint, validate the Power BI
# report with Microsoft's PBIR CLI, check the contract digests and run CodeQL.
# None of those run here. A green `just check` is not a green CI.

# The fast local pass: lint, type-check and test every component.
check: lint typecheck test
