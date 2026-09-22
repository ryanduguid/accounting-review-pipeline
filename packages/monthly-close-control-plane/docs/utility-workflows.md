# Run the joined accounting examples

Use the current Git source checkouts for Accounting Review Pipeline, au-fpa-pack, australian-accounting and grant-acquittal-workpapers. The new commands are not established by older published packages. Install Python 3.11 or later, Git and uv. The driver uses Python 3.11 for each command; uv supplies it and dependencies if absent. Initial setup may download public dependencies. The calculations use local fabricated inputs.

The joined examples are available on all four repositories' `main` branches.
The grant route needs access to the private grant-acquittal-workpapers checkout.
The driver refuses an older WIP checkout without `examples/job_to_cash.py`
before creating outputs or environments.

For one-command setup, run this from the grant-acquittal-workpapers checkout:

```powershell
python setup_utility.py --workspace ../accounting-utility-demo
```

The setup command needs Python 3.11 or later, Git and uv on PATH. It clones the
three public companions at `main`, creates isolated environments and runs every
joined example. The workspace must be new and outside existing Git checkouts.
Existing checkouts are not updated. Results and their provenance manifest go
under `accounting-utility-demo/results`; a failed run leaves its workspace for
diagnosis. Retry with a new workspace path.

The companion setup also accepts `--replay-manifest path/to/manifest.json` to
fetch the recorded commits into a new workspace. It requires a complete
`utility-workflows.v2` manifest from clean source checkouts. For a failed
all-route run, use `results/replay.json`, which the runner saves before commands
start. It carries source revisions without claiming successful verification. The private grant
commit is fetched from the existing local grant repository, which is left
unchanged. Source commits and locks are pinned; operating systems and external
tool versions are not. Replay cannot recover uncommitted edits or unavailable
commits. It requires a runner revision with the fixture checks described below.

Within a uv workspace, `uv run --project` uses the workspace root lockfile. Close control therefore uses Accounting Review Pipeline's root `uv.lock`. A project outside a workspace uses its own lockfile. The manifest records the resolved lock path, scope and digest for each owner. Separate environment directories do not establish standalone component-lock compatibility; that requires a separate extracted-source or release check.

From a directory containing those 4 checkout folders:

```powershell
python accounting-review-pipeline/packages/monthly-close-control-plane/examples/utility_workflows.py --fpa au-fpa-pack --accounting australian-accounting --grants grant-acquittal-workpapers --output ../utility-results --environment-root ../utility-environments
```

Both destination directories must be new, separate and outside the source checkouts. Paths can contain spaces when quoted. No machine-specific user directory or private task harness is required. RTK is used when installed but is not a dependency. Omit `--environment-root` to use uv's ordinary project environments.

The default `all` run produces:

- Lumbridge opening balances, a verified close pack and its cash refresh, with shared source hashes checked.
- A three-month fabricated quarter, 3 verified close packs and 2 comparisons.
- Baseline and extra-completion-cost WIP packs, each consumed by the existing cash forecast.
- A two-grant workpaper and its restricted-cash handoff, preserving review findings and unresolved commitments.

Select `--workflow close-forecast`, `quarter`, `job-cash` or `grant-cash` to run one route. The first two need only `--fpa`; job cash also needs `--accounting`, and grant cash needs `--grants`.

Each owner runs in a separate process. With `--environment-root`, each owner also gets a fresh environment directory. Omitting that option uses uv's existing project or workspace environment. No engine imports a sibling repository. The driver preserves REVIEW exit 2 where the fabricated example expects it and verifies close packs through the existing viewer. Repeating a run requires a fresh output directory.

The fixed fixture checks require the opening quarter balance and all three
monthly periods before any period is used in a path. They verify the expected
cash balances, three close reviews, two comparisons, both thirteen-week WIP
scenarios and the grant's retained findings and unresolved commitments. They
also check command and runtime-probe coverage for the selected route. Empty,
missing or altered results fail even when their producer exits normally. These
are regression expectations for these fabricated examples, not rules for
assessing client accounts or configurable quarter inputs.

A successful run records `fixture_validation: passed` and writes `summary.md`
with coverage, cash results, retained review findings and source revisions.
The grant workflow appends it to the private Actions job summary. Failed
commands produce a failure summary; fixture validation errors retain
`failed-results.json` and do not produce a success manifest. Neither summary
publishes subprocess output. Review the retained diagnostics locally.

A successful run writes `manifest.json` with the 19 workflow commands, 4 runtime probes, expected and actual exits, elapsed times and output hashes. It records Git revisions, uncommitted file status, source hashes, authoritative lock hashes, Python versions, installed package versions and the uv version. Source hashes cover Git-listed Python, TOML, lock, JSON, CSV, YAML, Markdown, manifest and text files, excluding local environment files, secret directories, symlinks and ignored files. The driver compares source evidence before and after execution and refuses a success manifest if it changes.

Each command has a 300-second timeout, adjustable with `--timeout` up to 3,600 seconds. A timeout or interruption stops the command process tree. Launch errors, unexpected exits and timeouts retain `failed-calls.json` with completed steps, timings, source evidence and the failed command's output. No success manifest is written for that run. Review diagnostics locally before sharing them. The driver does not resume a partial run; correct the cause and choose a new output directory.

The grant repository's `Joined accounting examples` workflow runs the same setup
command on Linux and Windows for its PRs, pushes to `main`, manual runs and a
daily schedule. It uses the current grant checkout and the public companions'
latest `main` revisions. The manifest records the actual revisions. Companion
changes are caught by the next scheduled run, not by a check on their own PRs.
The daily schedule starts once the workflow is merged to the default branch.
Fabricated results and failure diagnostics stay in private workflow artefacts
for seven days; no extra cross-repository secret is required.

Review results retain their accounting limits; a successful integration does not approve a close, a grant acquittal or a funding decision. Component tests continue to run through their existing CI commands. Timings are measurements for comparison, not a claim of improved performance.
