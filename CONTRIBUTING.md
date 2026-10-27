# Contributing

Each component is developed, checked and released from its own directory. The root holds
a development entrypoint so a fresh clone can be set up and verified in one command; it
does not change where a component is developed or released from.

## Quick start

From a fresh clone, with Rust and [uv](https://docs.astral.sh/uv/) installed:

```bash
uv sync --locked --group dev  # install retained Python development tooling
just test      # every component's suite, plus the joined conformance test
```

The root is a virtual workspace with no Python package members. Its `.venv`
holds the test, lint and type-check tools for retained Python consumers.
`just test` also runs each native crate's Cargo tests; each crate pins its
toolchain in `rust-toolchain.toml`.

`just` is optional tooling; install it with `uv tool install rust-just`. The recipes are
`setup`, `lint`, `typecheck`, `test` and `check` (the last 3 together). Each loops over
the per-component commands in the `AGENTS.md` command-routing table, which remain the
authority; `just` runs them from one place, it does not replace them.

`just check` runs Ruff, mypy, Cargo fmt/clippy/tests and retained unittest suites.
CI also checks frozen Python differentials, native release builds and smokes,
dependency advisories, workflow policy, Power BI structural validation and
contract digests. CodeQL covers Python and Actions here, not Rust. Follow the
`AGENTS.md` routing table for each component's complete checks.

The root `dev` group pins Python development tools; regenerate root `uv.lock`
after changing it. Native crates use their own `Cargo.lock`. Changes to root
development files select every component workflow.

## Pull request rules

Keep a change scoped to one component, to the data-only contract
`contracts/xero-trial-balance-v1/`, or to the root policy, workspace and workflow files. Run
the owning component's documented checks from its directory and include affected downstream
conformance evidence when a file contract changes. `AGENTS.md` lists the commands.

Component versions, lockfiles, publishers and release workflows stay independent. The root
`pyproject.toml`, `uv.lock` and `justfile` are a development entrypoint only: the root is a
virtual uv workspace with no package, no version, no runtime dependency and nothing
published from it. Do not add a root distribution, shared runtime package, unified version
or monorepo framework. Movement-only changes and behaviour changes go in separate pull
requests.

## Test fixtures

Use fabricated fixtures only. Do not add client exports, workpapers, credentials, tokens,
generated packs or screenshots containing client data. Keep fixture expectations explicit,
run the owning component's checks, and run joined conformance whenever the shared contract
changes.

## Data handling

Review components may not gain a network or Xero dependency or import the exporter or
a sibling component. Preserve their documented status and exit-code boundaries and
the native review commands' exact decimal arithmetic. Document native Excel and
Power BI numeric types separately. Only Xero Trial Balance Export may handle Xero
OAuth, HTTP, credentials or tokens. Keep client data and generated workpapers outside the
checkout.

For a potential security vulnerability follow `SECURITY.md`.

## Required CI results

Branch protection requires one aggregate per workflow: `pipeline-gates`,
`conformance-gates`, `standard-library-gates`, `public-fixtures-gates` and
`codeql-gates`, plus the `CodeQL` results check and `Attribution policy`.
`pipeline-gates` needs every `<component directory> / gates` result, so those
are not listed separately. Each aggregate fails if an expected job fails, is
cancelled or does not run, and fails when a job in its workflow is missing from
its `needs` or when a pull request removes a job without adding a
`# removed-jobs: <job>` comment to that workflow (`.github/ci/check_gates.py`).
Gate lineage combines the existing destination with the unique reported-context
source. Callable-only job names occupy a separate namespace from standalone jobs.
The reader resolves static matrix include rows and literal outcomes of matrix
equality conditionals;
other possible or ambiguous predecessors fail explicitly and need a reader update.
On a pull request, unchanged components skip their matrix jobs and
the final result accepts that only after successful path selection. A push to
`main` runs every job, so the release commit carries the component evidence the
release gate requires. File moves select both the source and destination
components.

Run `python -m unittest tests.test_ci_paths -v` for selector changes. The joined
and standard-library workflows run this regression suite before selecting work.
Keep existing required contexts until the replacement checks have reported on
a reviewed revision, then migrate branch protection in the same maintenance window.
