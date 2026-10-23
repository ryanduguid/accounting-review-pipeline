# Run the joined accounting examples

Use current Git checkouts of Accounting Review Pipeline, au-fpa-pack and australian-accounting; the grant workpapers live in this repository at `apps/grant-acquittal-workpapers/`. Install Python 3.14 or later, Git, uv and Rust. The retained Python driver runs Python companions through uv and builds the close command, the grant command and the native WIP example through Cargo. Initial setup may download public dependencies. The calculations use local fabricated inputs.

The joined examples are available on all three repositories' `main` branches.
The driver refuses a WIP checkout without its job-to-cash example
(`examples/job_to_cash.rs` for the native crate, `examples/job_to_cash.py` for
the earlier Python package) before creating outputs or environments.

For one-command setup, run this from `apps/grant-acquittal-workpapers` in a
pipeline checkout:

```powershell
python setup_utility.py --workspace ../../../accounting-utility-demo
```

The setup command needs Python 3.14 or later, Git, uv and cargo on PATH; cargo
builds the native close and grant commands, and the native WIP example, from
their component lockfiles. It clones the
two companions at `main`, uses the enclosing pipeline checkout for the close
controls, the runner and the grant workpapers, creates isolated environments and
runs every joined example. The workspace must be new and outside existing Git checkouts.
Existing checkouts are not updated. Results and their provenance manifest go
under `accounting-utility-demo/results`; a failed run leaves its workspace for
diagnosis. Retry with a new workspace path.

Setup also accepts `--replay-manifest path/to/manifest.json` to
fetch the recorded commits into a new workspace. It requires a complete
`utility-workflows.v2` manifest from clean source checkouts. For a failed
all-route run, use `results/replay.json`, which the runner saves before commands
start. It carries source revisions without claiming successful verification. The pipeline
commit, which holds both the close controls and the grant workpapers, is fetched
from the local pipeline checkout, which is left unchanged. Manifests recorded by
the separate grant repository before its move are refused. Source commits and locks are pinned; operating systems and external
tool versions are not. Replay cannot recover uncommitted edits or unavailable
commits. It requires a runner revision with the fixture checks described below.

Close control uses its component `Cargo.lock`; its manifest records project lock
scope. The native WIP tally uses its package `Cargo.lock` and records project
lock scope. Python companions use their own lock, or the workspace root lock when
they belong to a uv workspace. The manifest records each resolved lock path,
scope and digest. Separate environment directories do not establish standalone
component-lock compatibility.

From a directory containing those 3 checkout folders:

```powershell
python accounting-review-pipeline/packages/monthly-close-control-plane/examples/utility_workflows.py --fpa au-fpa-pack --accounting australian-accounting --grants accounting-review-pipeline/apps/grant-acquittal-workpapers --output ../utility-results --environment-root ../utility-environments
```

Both destination directories must be new, separate and outside the source checkouts. Paths can contain spaces when quoted. No machine-specific user directory or private task harness is required. RTK is used when installed but is not a dependency. Omit `--environment-root` to use uv's ordinary project environments.

The default `all` run produces:

- Lumbridge opening balances, a verified close pack and its cash refresh, with shared source hashes checked.
- A three-month fabricated quarter, 3 verified close packs and 2 comparisons.
- Baseline and extra-completion-cost WIP packs, each consumed by the existing cash forecast.
- A two-grant workpaper and its restricted-cash handoff, preserving review findings and unresolved commitments.

Select `--workflow close-forecast`, `quarter`, `job-cash` or `grant-cash` to run one route. The first two need only `--fpa`; job cash also needs `--accounting`, and grant cash needs `--grants`.

Each owner runs in a separate process. With `--environment-root`, Python owners
get fresh environment directories; native commands use their Cargo build
directories. Omitting that option uses uv's existing environments. No engine
imports a sibling repository. The driver preserves REVIEW exit 2 where the
fabricated example expects it and verifies close packs through the viewer.
Repeating a run requires a fresh output directory.

The fixed fixture checks require the opening quarter balance and all three
monthly periods before any period is used in a path. They verify the expected
cash balances, three close reviews, two comparisons, both thirteen-week WIP
scenarios and the grant's retained findings and unresolved commitments. They
also check command and runtime-probe coverage for the selected route. Empty,
missing or altered results fail even when their producer exits normally. These
are regression expectations for these fabricated examples, not rules for
assessing client accounts or configurable quarter inputs.

A successful run records `fixture_validation: passed` and writes `summary.md`
with `execution_mode: source-project` in its manifest. It runs the source
projects through their locked environments or native builds; installed-release evidence is separate.
The summary records coverage, cash results, retained review findings and source revisions.
The daily joined workflow appends it to its Actions job summary with the setup
`--summary` option. Failed
commands produce a failure summary; fixture validation errors retain
`failed-results.json` and do not produce a success manifest. Neither summary
publishes subprocess output. Review the retained diagnostics locally.

A successful all-route run writes `manifest.json` with 19 workflow commands,
4 runtime probes, expected and actual exits, elapsed times and output hashes.
It records Git revisions, uncommitted status, source and lock hashes, the native
close and WIP versions, Python companion versions and the uv version. Source hashes cover
Git-listed Rust, Python, TOML, lock, JSON, CSV, YAML, Markdown, manifest and text
files, excluding environments, secret directories, symlinks and ignored files.
The driver compares source evidence before and after execution and refuses a
success manifest if it changes.

Each command has a 300-second timeout, adjustable with `--timeout` up to 3,600 seconds. A timeout or interruption stops the command process tree. Launch errors, unexpected exits and timeouts retain `failed-calls.json` with completed steps, timings, source evidence and the failed command's output. No success manifest is written for that run. Review diagnostics locally before sharing them. The driver does not resume a partial run; correct the cause and choose a new output directory.

This repository's `Joined accounting examples` workflow
(`.github/workflows/joined-latest-main.yml`) runs the same setup command on Linux
and Windows daily and on manual runs. It uses the pipeline commit under test and
the companions' latest `main` revisions, and the manifest records the actual
revisions, so it checks the joined routes as companion branches advance.
Fabricated results and failure diagnostics stay in workflow artefacts for seven
days; no cross-repository secret is required.

The three repositories also define `joined-fixtures.yml` for their own
pull requests, pushes to `main` and manual runs, on Linux and Windows. Each workflow
checks out its proposed change and pins the other two repositories to full commit
SHAs. All three run `close-forecast`, `quarter` and `job-cash`, and this
repository also runs `grant-cash`. These routes need no private repository or
secret.
Update companion pins deliberately when adopting a compatible change.

The public jobs create fresh output and environment directories outside all
checkouts. Each operating system's artefact retains the fabricated results, failure
diagnostics, escaped summary, success manifest and replay record for seven days.
The existing fixture assertions check cash values, command coverage and retained
REVIEW findings. The manifests identify the actual source and dependency versions.
The Linux check keeps its `public-fixtures` name and Windows adds
`public-fixtures (Windows)`. Each workflow's `public-fixtures-gates` job needs
both, and branch protection in the three public repositories requires it.

Review results retain their accounting limits; a successful integration does not approve a close, a grant acquittal or a funding decision. Component tests continue to run through their existing CI commands. Timings are measurements for comparison, not a claim of improved performance.
