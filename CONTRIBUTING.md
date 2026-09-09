# Contributing

Each component is developed, checked and released from its own directory. The root holds
a development entrypoint so a fresh clone can be set up and verified in one command; it
does not change where a component is developed or released from.

## Quick Start

From a fresh clone, with [uv](https://docs.astral.sh/uv/) installed:

```bash
uv sync        # install every Python component and the shared toolchain
just test      # every component's suite, plus the joined conformance test
```

`uv sync` creates one `.venv` at the root and installs the four Python distributions into it
as editable workspace members. The Excel adapter and the Power BI application have no
`pyproject.toml`; their standard-library unittest suites run from the same `.venv`.

`just` is optional tooling; install it with `uv tool install rust-just`. The recipes are
`setup`, `lint`, `typecheck`, `test` and `check` (the last three together). Each loops over
the per-component commands in the `AGENTS.md` command-routing table, which remain the
authority; `just` runs them from one place, it does not replace them.

`just check` is the fast local pass, not a CI equivalent. It runs Ruff, mypy, pytest and
unittest. CI additionally runs the tests with branch coverage on Python 3.10, 3.12 and
3.13, verifies each component lockfile, audits each locked environment with pip-audit,
installs the exporter's hash-locked requirements, builds each distribution and imports it
from a clean environment, validates
the Power BI report with Microsoft's PBIR CLI, checks the contract digests and runs CodeQL.
The shared component gates are defined once in `.github/workflows/ci-package.yml`; the
`AGENTS.md` command-routing table lists them. A green `just check` is not a green CI.

Every member's `dev` extra and the root `dev` group carry the same exact pins for `ruff`,
`mypy`, `pytest`, `pytest-cov` and `coverage`, so one workspace resolution holds them all
and the versions `just` runs are the versions CI runs. Move a pin in all five places
together, then regenerate the root and component lockfiles.

Two consequences of the workspace are worth knowing before you run a component's own
commands:

- `uv run --locked` from a component directory now validates the root `uv.lock`, not the
  component's. Regenerate it with `uv lock` at the root after changing any component's
  dependencies, and commit it. The component's own `uv.lock` stays the authority for
  building and releasing that component alone, so update both.
- A change to the root `pyproject.toml`, `uv.lock` or `justfile` runs every component
  workflow, for the same reason.

## Pull Request Rules

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

## Test Fixtures

Use fabricated fixtures only. Do not add client exports, workpapers, credentials, tokens,
generated packs or screenshots containing client data. Keep fixture expectations explicit,
run the owning component's checks, and run joined conformance whenever the shared contract
changes.

## Data Handling

Review components may not gain a network or Xero dependency, may not import the exporter or
a sibling component, and must keep exact `Decimal` arithmetic and their documented status
and exit-code boundaries. One shared `.venv` makes every Python component importable from
every other; that does not relax the rule. Only Xero Trial Balance Export may handle Xero
OAuth, HTTP, credentials or tokens. Keep client data and generated workpapers outside the
checkout.

For a potential security vulnerability follow `SECURITY.md`.
