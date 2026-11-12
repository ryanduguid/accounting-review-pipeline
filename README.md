<p align="center">
  <picture>
    <source media="(max-width: 600px) and (prefers-color-scheme: light)" srcset=".github/assets/banner-phone-light.svg">
    <source media="(max-width: 600px)" srcset=".github/assets/banner-phone.svg">
    <source media="(prefers-color-scheme: light)" srcset=".github/assets/banner-light.svg">
    <img src=".github/assets/banner.svg" width="100%" alt="Review Pipeline. Month-end close checks, ready for human sign-off. Pixel art of a magnifying glass over a ticked document beside the Review Pipeline wordmark in slab-serif block letters.">
  </picture>
</p>

<div align="center">

[![tests](https://github.com/ryanduguid/accounting-review-pipeline/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ryanduguid/accounting-review-pipeline/actions/workflows/ci.yml)
[![Codecov](https://codecov.io/gh/ryanduguid/accounting-review-pipeline/graph/badge.svg)](https://codecov.io/gh/ryanduguid/accounting-review-pipeline)
[![CodeQL](https://github.com/ryanduguid/accounting-review-pipeline/actions/workflows/codeql.yml/badge.svg?branch=main)](https://github.com/ryanduguid/accounting-review-pipeline/actions/workflows/codeql.yml)
[![SonarCloud](https://sonarcloud.io/api/project_badges/measure?project=ryanduguid_accounting-review-pipeline&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=ryanduguid_accounting-review-pipeline)
[![Codacy code quality](https://app.codacy.com/project/badge/Grade/361fa65bc2ac4e9eb812e761b0f4e227)](https://app.codacy.com/gh/ryanduguid/accounting-review-pipeline/dashboard)<br>
[![OpenSSF Scorecard](https://raw.githubusercontent.com/ryanduguid/.github/badges/ryanduguid/accounting-review-pipeline.svg)](https://scorecard.dev/viewer/?uri=github.com/ryanduguid/accounting-review-pipeline)
[![OpenSSF Best Practices](https://www.bestpractices.dev/projects/15430/badge)](https://www.bestpractices.dev/projects/15430)
[![SLSA provenance](.github/assets/badges/slsa.svg)](https://github.com/ryanduguid/accounting-review-pipeline/attestations)
[![VirusTotal](.github/assets/badges/virustotal.svg)](https://github.com/ryanduguid/accounting-review-pipeline/releases)<br>
[![licence: Ryan Duguid Public Attribution Licence 1.0](.github/assets/badges/licence.svg)](LICENSE)
[![Open in GitHub Codespaces](.github/assets/badges/github.svg)](https://codespaces.new/ryanduguid/accounting-review-pipeline)

[Install](#install) · [Sample pack](#reproduce-the-sample-pack) · [Review packs](#review-pack-contract) · [Statuses](#status-contract) · [Exit codes](#exit-code-contract) · [Release checks](#release-checks) · [Development](#development) · [Licence](#licence)

</div>

Accounting Review Pipeline is a set of 8 independently versioned components that
carry an Australian month end from a Xero export to a reviewable pack, for accountants
and reviewers in public practice.

The components are a read-only trial balance exporter, deterministic close controls, a
workpaper readiness gate, a ledger review gate, an Excel toolkit, a Power BI project,
grant acquittal workpapers and evatt, a local pseudonymisation tool. Local files and
commands join them, not a shared runtime, and each keeps its own distribution name,
version, licence and command.

The components produce exception reports and review packs for an accountant to work through.
Close-control components read local files and refuse rather than guessing: a trial balance
that does not tie is not exported, and an incomplete pack does not reach manager
review. The Xero trial-balance exporter is the exception: it pulls through Xero OAuth/API
before writing its CSV (see its [README](packages/xero-trial-balance-export/README.md)).
None of them approves a close, locks a period or lodges anything.

For a Xero Activity Statement transaction GST review, use the
[BAS skill and transaction procedure](https://github.com/ryanduguid/australian-accounting-skills/blob/main/docs/bas-walkthrough.md#transaction-gst-review-in-codex-chatgpt-or-copilot).
Account for supplied rows and establish supply treatment, buyer entitlement and
attribution separately. The BAS readiness gate checks pack evidence and the GST
control tie-out; it does not classify transaction tax treatment. Equal ledger
and statement errors can still reconcile. Carry unresolved transaction findings
to the authorised reviewer alongside the gate result.

**Start here:** [review-pack contract](#review-pack-contract) ·
[status meanings](#status-contract) ·
[Monthly Close Controls](packages/monthly-close-control-plane/README.md) ·
[Workpaper Review Gate](packages/review-ready-gate/README.md) ·
[Xero trial balance export](packages/xero-trial-balance-export/README.md)

Synthetic examples. A human decides whether the close is acceptable.

**Input:** the supplied current and prior trial balances, account mapping and subledger. The creditors reconciliation differs by $250.

**Output:** `REVIEW`, 8 exceptions, exit 2. [Read the example in your browser](https://duguid.com.au/tools/monthly-close-controls/#worked-example) before installing anything.

| Finding | Evidence | Human decision |
| --- | --- | --- |
| Creditors reconciliation | $250 difference | Trace and explain the difference before sign-off. |
| Financial-year reset | 30 June compared with 31 July | Reconsider P&L YTD comparisons across the reset. |

## Install

Download `close-control` (Monthly Close Controls) or `review-ready` (Workpaper Review Gate) from its component's [GitHub Release](https://github.com/ryanduguid/accounting-review-pipeline/releases), tagged `monthly-close-control-plane/v<version>` or `review-ready-gate/v<version>`. Each release holds one executable for each of 2 platforms:

| Command | Linux x86_64 (static, musl) | Windows x86_64 |
| --- | --- | --- |
| `close-control` | `monthly-close-control-plane-<version>-x86_64-unknown-linux-musl.tar.gz` | `monthly-close-control-plane-<version>-x86_64-pc-windows-msvc.zip` |
| `review-ready` | `review-ready-gate-<version>-x86_64-unknown-linux-musl.tar.gz` | `review-ready-gate-<version>-x86_64-pc-windows-msvc.zip` |

Download an explicit tag, never `latest`, and verify the archive against the release's signed `SHA256SUMS` and attestations before extracting it, using the recipe in [Monthly Close Controls](packages/monthly-close-control-plane/RELEASING.md#verify-a-release) or [Workpaper Review Gate](packages/review-ready-gate/RELEASING.md#verify-a-release). The archive holds the executable with its `LICENSE`, `NOTICE` and `README.md`; put the executable on your PATH.

For another platform, or for the Xero exporter and the other native commands, which have no release, build from a checkout with Rust. From `packages/monthly-close-control-plane`, run `cargo install --locked --path . --bin close-control`. Each component's README gives its own command.

To have an AI agent run the close checks for you, copy [`packages/monthly-close-control-plane/skills/close-review`](packages/monthly-close-control-plane/skills/close-review/SKILL.md) into your agent's skills directory (for Claude Code, `~/.claude/skills/`). The skill runs the installed `close-control` on files you name, checks the pack with `close-control view` and answers only from `close-review-pack.json` and `exceptions.csv`. It runs no server, uses no network, never approves or posts anything and stops on an exit code it does not recognise.

## Reproduce the sample pack

From the repository root, with Rust and [just](https://github.com/casey/just) installed, run:

```bash
just demo
```

This calls the existing `close-control review` command with the supplied fictional trial balances, mapping, subledger and review note. It writes `../close-control-demo/`, outside the checkout, and replaces any files with the same output names. Keep this directory for the fictional demo. Open `../close-control-demo/close-summary.md` to read the findings.

The CLI exits 2 because the sample needs review. `just demo` accepts that exit alone: it succeeds on exit 2 and fails on any other status, including 0. The command does not record a new human decision. The [demo recipe](justfile) shows every input and threshold.

Check that the 4 result files agree before reviewing the pack:

```bash
cargo run --locked --release --manifest-path packages/monthly-close-control-plane/Cargo.toml --bin close-control -- view --pack-dir ../close-control-demo
```

The viewer exits 0 after verification or 1 if the files disagree. Check its list of controls not run before assessing the pack's coverage. It does not re-read the input trial balances. Keep all 4 result files together and record each review decision in your workpaper. The [reviewer quick start](packages/monthly-close-control-plane/docs/reviewer-quick-start.md) explains installation from source.

The [full command and all 8 exceptions](packages/monthly-close-control-plane/README.md#worked-example) remain available without just. The [GitHub Codespaces](https://codespaces.new/ryanduguid/accounting-review-pipeline) configuration includes Rust and the Python development tools; use the full command there if just is unavailable. Codespaces usage counts against your own GitHub quota. The `devcontainer` workflow builds the container and runs the fabricated close review inside it on every push to `main`.

[Read the 5-minute close case](packages/monthly-close-control-plane/docs/manager-case-study.md) · [Workpaper Review Gate](packages/review-ready-gate/README.md)

Every invented entity the components share, and the proposal to consolidate them into one fabricated firm, is recorded in [docs/fabricated-firm.md](docs/fabricated-firm.md).

A firm keeping an AI register can draw on the supplier information for evatt, the Xero Ledger Review Gate and the Workpaper Review Gate in [docs/ai-register-entries.md](docs/ai-register-entries.md).

<details>
<summary>Setup, component identities, releases and the exporter data contract</summary>

This repository brings together Monthly Close Controls and its related tools. Its canonical GitHub repository
is [`ryanduguid/accounting-review-pipeline`](https://github.com/ryanduguid/accounting-review-pipeline).
It holds 8 independently versioned components joined only by local files and commands:

| Component | Directory | Identity | Source version | Latest release |
|---|---|---|---|---|
| Xero Trial Balance Export | `packages/xero-trial-balance-export/` | native Rust commands `export-tb` and `xero-tb-auth`; the only OAuth, Xero and network producer | 1.0.0 (unreleased) | 0.1.15, Python, PyPI |
| Workpaper Review Gate | `packages/review-ready-gate/` | native Rust command `review-ready`; Python runtime and import API removed | 1.0.1 (unreleased) | 1.0.0, native, GitHub release |
| Monthly Close Controls | `packages/monthly-close-control-plane/` | native Rust command `close-control`; Python runtime and import API removed | 1.1.0 (unreleased) | 1.0.0, native, GitHub release |
| Xero Ledger Review Gate | `packages/elizabeth-anne-alexander/` | Rust binary `elizabeth-anne-alexander`; the released Python distribution imports `elizabeth_anne_alexander` | 1.0.0 (unreleased) | 0.2.6, Python, PyPI |
| Accounting Excel Toolkit | `adapters/accounting-excel-toolkit/` | source-archive adapter `accounting-excel-toolkit` (Power Query and VBA) | 1.0.0 | 1.0.0, GitHub release archive |
| Australian Accounting Power BI | `apps/australian-accounting-power-bi/` | PBIP reference application | 1.0.0 | none |
| Grant Acquittal Workpapers | `apps/grant-acquittal-workpapers/` | Rust command `grant-workpaper`, built with cargo | 0.2.0 | none |
| evatt | `packages/evatt/` | Rust binary `evatt`; a local pseudonymisation boundary with no network step | 1.0.0 (unreleased) | 0.2.0, GitHub release only |

Data flows in one direction: the exporter (or a manual Excel export) produces the 10-column
Xero trial-balance file, the readiness gate decides whether a pack reaches review, monthly
close surfaces exceptions, and the ledger-review boundary or Power BI consumes the result.
evatt works separately: it pseudonymises Markdown locally before an operator hands it to an
external model. It does not read trial balances.
Only the exporter may touch OAuth, Xero, HTTP or credentials. Every other component is
offline and ships fabricated data only. Native review commands preserve the original
exact decimal money rules. The Excel adapter and Power BI application use their native
numeric types, including VBA `Double`, so that Decimal guarantee does not cover them.

Directory names follow each component's normalised distribution name so that the reviewed
Release Policy identity gate (directory leaf, `tag-prefix` and distribution name must agree)
can release one component per namespaced tag. That is why Monthly Close Controls lives at
`packages/monthly-close-control-plane/` rather than the migration plan's
`packages/monthly-close-controls/`; see `AGENTS.md` and `IMPORTS.md`.

Each component keeps its own package identity, version, lockfile, licence, commands,
documentation and release cadence. There is no root runtime package, shared library or
unified version. The root `pyproject.toml`, `uv.lock` and `justfile` are a development
entrypoint only: `uv sync` then `just test` sets up and verifies a fresh clone from the top
(see `CONTRIBUTING.md`). Run a component's own checks from its directory with its documented
commands. Only the root `.github/workflows/` are active. `IMPORTS.md` records source identities, tree digests
and import records. The source repositories are retired, so their GitHub releases and tags
are no longer hosted; some of those versions remain on PyPI. Later releases use
namespaced tags in this repository.

## Releases

Each active release caller uses its own namespaced annotated tag, `<component>/vMAJOR.MINOR.PATCH`,
through a root caller pinned to a Release Policy commit. `accounting-excel-toolkit/v*` calls the
source-archive workflow at the independently reviewed commit
`ca5492bd09693be9e1fd47d980e2740ef8daf023`. `monthly-close-control-plane/v*` and
`review-ready-gate/v*` call the native binary workflow at
`1058bbf7a3f22f3c5c421676cf5f89a14389100c`, which publishes the command for Linux x86_64
(musl) and Windows x86_64 with source archives, SBOMs, `SHA256SUMS` and signed provenance.
One tag publishes exactly one component; the
identity gate refuses a tag whose prefix does not equal the component directory leaf and its
distribution name or archive stem. Each component's `RELEASING.md` describes its preflight; the tag name
is the only difference. `IMPORTS.md` lists the callers and publisher environments.
To check a downloaded release's checksum and SLSA provenance, follow [Verify a release](https://github.com/ryanduguid/release-policy/blob/main/docs/verify-a-release.md).

The Rust Xero exporter, Xero Ledger Review Gate and evatt 1.0.0 have no release callers
until their Rust release paths are approved. The PyPI releases in the component table
(Xero exporter 0.1.15, Workpaper Review Gate 0.3.0, Monthly Close Controls 0.2.0 and the
ledger review gate 0.2.6) are the earlier Python distributions; they predate the native
commands documented here. evatt 0.2.0 was released on GitHub only, never on PyPI.

## Contract

`contracts/xero-trial-balance-v1/` is the data-only authority for the exporter-owned
`xero-tb-csv.v1` corpus: the exact 10-column schema, 3 fabricated fixtures, the expected
accept and reject results and `SHA256SUMS`. It is test and data input only and adds no shared
runtime package. The `tests/joined-conformance` crate and the
`joined-conformance.yml` workflow run the exporter runner and all 3 offline review
implementations against it. [`tests/joined-conformance/fixtures/`](tests/joined-conformance/fixtures/README.md)
adds fabricated vectors for quoted CSV, leading-zero codes, Unicode text and numeric
spellings, and records where the consumers differ.

</details>

## Review-pack contract

The [LodgeiT workflow reuse note](docs/lodgeit-workflow-reuse.md) maps public
import, workpaper and review patterns to Accounting Review Pipeline's components,
with a checklist for using their local evidence outputs.

Review packs are deterministic evidence for a human reviewer; they do not approve a close,
post a journal, make a payment, lodge a return or lock a period. Each pack is a directory whose
files must agree with each other: the JSON file is the machine-readable source of truth, Markdown
is its human-readable summary, and CSV exposes the findings or exceptions as rows. The
monthly-close pack carries a second CSV holding the questions for the client that those exceptions
raise. The producing command writes every file in one pass; the component's `view` command
consumes and cross-checks an existing pack without changing it.

Packs written by the native commands carry a `producer` block naming the component, its version,
the source commit (or `unknown`) and the pack contract, `2`; the summary's Scope section repeats it,
and `view` refuses a pack whose 2 copies disagree. It holds no time, host or path, so the same
build and inputs still write byte-identical packs. A pack without the block is contract 1, written
before it existed; `view` still reads one, but a `view` from before the block refuses packs that
carry it. Building with `ACCOUNTING_REVIEW_SOURCE_COMMIT` set to a full lowercase commit SHA
records that commit; any other value fails the build. Without it, a full lowercase
`GITHUB_SHA`, as in a GitHub Actions build, is recorded instead, else `unknown`.

### Readiness-gate output

`review-ready gate`, called with `--profile <profile> --pack <directory> --output <directory>`, produces
`readiness-pack.json`, `readiness-summary.md` and `findings.csv`.

| JSON field | Meaning |
|---|---|
| `overall_status` | The separate readiness result: whether configured evidence permits the pack to enter human review. |
| `engagement_type` | The selected `bas`, `month_end` or `year_end` evidence profile. |
| `period_end` | The reporting date declared by the preparer's self-review. |
| `findings` | Missing, incomplete or integrity conditions found by the configured gates. |
| `source_sha256` | Filenames and SHA-256 digests identifying the exact input byte snapshots assessed. |
| `thresholds` | The tie-out tolerance applied during the run. |
| `review_boundary` | The fixed statement that readiness is not approval, advice or lodgement authority. |
| `preparer_initials` | The initials on the preparer's self-review; empty when none was loaded. |
| `controls_not_run` | Each optional slot with no usable input, with its filename and reason. `READY` says nothing about the controls listed here. |
| `required_controls` | With `--require-control`: each slot this run required and its state, `ran`, `skipped` or `failed`. One that did not run is a `NOT_READY` finding. |
| `acknowledgement` | Optional evidence of a later human review; it cannot change readiness or approve a file. |
| `producer` | The component, version, commit or `unknown`, and pack contract that wrote the pack; absent from contract 1 packs. |

### Monthly-close pack

`close-control review`, called with `--current <csv> --prior <csv> --output <directory>`, produces
`close-review-pack.json`, `close-summary.md` and `exceptions.csv`, together with
`client-queries.csv`.

| JSON field | Meaning |
|---|---|
| `overall_status` | The aggregate Monthly Close `PackState` produced by the configured controls. |
| `current_report_dates` / `prior_report_dates` | Report dates read from the current and prior validated trial-balance exports. |
| `exceptions` | Material variances, integrity failures and other conditions requiring attention. |
| `client_queries` | Draft questions derived from the exceptions only a client can answer, with the evidence each one asks for. Nothing is sent, and answering them approves nothing. |
| `source_sha256` | SHA-256 digests identifying the exact current, prior and optional supporting inputs. |
| `thresholds` | The absolute, percentage and reconciliation tolerances used to classify exceptions. |
| `controls_not_run` | The optional controls whose input was not supplied, so they did not run. `PASS` or `REVIEW` says nothing about them. |
| `required_controls` | With `--require-control`: each control this run required and its state, `ran`, `skipped` or `failed`. One that did not run is a `REVIEW` exception. |
| `acknowledgement` | Optional evidence of human review; it cannot change a state or approve or close a period. |
| `producer` | The component, version, commit or `unknown`, and pack contract that wrote the pack; absent from contract 1 packs. |

### Ledger-review evidence

The ledger-review boundary is not a review pack and does not use `PackState`. It writes a model
result, reviewer evidence and a digest-bound receipt. The receipt records a supplied human
decision without making that decision for the reviewer.

## Status contract

In this repository, pack state means the Monthly Close `PackState`; its value is exactly `PASS`,
`REVIEW` or `BLOCKED`. Review Ready Gate emits a separate `ReadinessStatus`, and the ledger
gateway uses result and receipt statuses rather than pack states.

| Output | Status domain | Value | Meaning |
|---|---|---|---|
| Review Ready Gate | `ReadinessStatus` | `READY` | Configured evidence permits the pack to enter manager review; a human still decides. |
| Review Ready Gate | `ReadinessStatus` | `NOT_READY` | Required evidence or preparation is incomplete. |
| Review Ready Gate | `ReadinessStatus` | `BLOCKED` | Integrity or safety evidence prevents review. |
| Monthly Close Control Plane | `PackState` | `PASS` | No configured exception requires review. |
| Monthly Close Control Plane | `PackState` | `REVIEW` | One or more bounded exceptions need human review. |
| Monthly Close Control Plane | `PackState` | `BLOCKED` | An integrity or input condition prevents a reliable result. |
| Elizabeth Anne Alexander model result | gateway result status | `REVIEW_READY` | Bounded evidence is ready for a human decision. |
| Elizabeth Anne Alexander receipt | decision-receipt status | `DECISION_RECORDED` | Every supplied finding decision has been recorded. |
| Elizabeth Anne Alexander receipt | decision-receipt status | `PARTIAL_DECISION_RECORDED` | At least one finding still has no supplied human decision. |

## Exit-code contract

For `review-ready gate`, exit `0` means `READY`, exit `2` means `NOT_READY` or `BLOCKED`,
and exit `1` means malformed input or an operational error. For `close-control review` and
`workbench`, exit `0` means `PASS`, exit `2` means `REVIEW` or `BLOCKED`, and exit `1`
means malformed input, invalid configuration or an unwritable output. The read-only
`close-control view` exits `0` only after verified display and `1` on verification failure.
The exporter and ledger-review commands document their command-specific exits in their
component READMEs.

The read-only [close review queue](packages/monthly-close-control-plane/docs/review-queue.md)
also exits `0` after verified display and `1` for invalid arguments or verification
failure. It filters supplied packs by recorded period, state and reviewer without
changing their status.

## Release checks

The release caller names the component checks that must have succeeded in the
push run for the exact release commit on `main`. It advances the policy SHA,
`required-checks` and `actions: read` together. Skipped, missing, cancelled or
failed checks block publication, and an aggregate gates job cannot replace them,
so every push to `main` runs every component's gates; only pull requests filter
by path. Tag the current `main` commit once its run is green; a successful run
for an older commit is not evidence for the release. Tags and publication still
require explicit approval.

## Alternatives

Accounting Review Pipeline's close controls read local trial balance exports and, for `drivers`,
`classify` and `reconcile`, supplied transactions. None of those 3 commands is a journal-level
population test. The Xero trial-balance exporter is the exception to local files, because it pulls
through Xero OAuth/API. These cover nearby ground (the 3 projects as read on 5 November 2026):

- Xero's built-in [General Ledger Exceptions report](https://central.xero.com/0/article/General-Ledger-Exceptions-report)
  (read 12 November 2026) lists transaction lines that are 2 or more standard deviations above or
  below the average line in their account over the life of the organisation, or that carry a GST
  treatment other than the account's default, and marks unexpected debits and credits. It excludes
  conversion balance lines, and it can be exported as PDF or a spreadsheet, or published.
  `close-control` works on the trial balance instead: it checks debit and credit totals, YTD
  movements against set thresholds, accounts that appear, disappear or change, and supplied
  mapping, subledger and GST inputs ([Controls](packages/monthly-close-control-plane/README.md#controls)).
  It exits 0 for `PASS` and 2 for `REVIEW` or `BLOCKED`, so a script can act on the result,
  publishes how many seeded cases each control detects and misses
  ([Detection benchmark](packages/monthly-close-control-plane/README.md#detection-benchmark)), and writes
  its pack as local files.
- [RecHound AI Workflows](https://rechound.com/features/ai-connections) (read 12 November 2026):
  an existing Claude, ChatGPT, Copilot, Gemini CLI or Cursor assistant connects to RecHound's server
  URL and reads the user's reconciliations, workpaper support and attachments for companies synced
  from Xero. Access is read-only: it cannot reconcile, edit support, sign off or post to Xero. What
  the assistant reads goes to that AI provider.

- [adoptai/cpa-skills](https://github.com/adoptai/cpa-skills): agent skills and scripts
  for close, tax and audit work in US practice, which refuse to deliver a workpaper that does not tie.
- [divyanshus2404/journal-entry-testing](https://github.com/divyanshus2404/journal-entry-testing):
  a PySpark journal entry testing pipeline on Databricks.
- [XeroAPI/xero-mcp-server](https://github.com/XeroAPI/xero-mcp-server): the MCP server from
  Xero's developer organisation, for agents working directly with Xero.

## Development

From a fresh clone, with Rust, [uv](https://docs.astral.sh/uv/) and just installed, `uv sync --locked --group dev` then `just test` runs every component's suite and the joined conformance test. [CONTRIBUTING.md](CONTRIBUTING.md) covers pull requests, and the command routing in [AGENTS.md](AGENTS.md#command-routing) lists each component's complete checks.

## Licence

All 8 components are licensed under the Ryan Duguid Public Attribution Licence 1.0, the FSL-1.1-ALv2 text
with a public attribution condition, as is the repository's [LICENSE](LICENSE).
Each component directory carries its own `LICENSE` and `NOTICE`.

<sub>Review aids only: not tax, financial, audit or legal advice, not a tax or BAS agent service and not an audit or review engagement. A human reviewer decides. Component DISCLAIMER.md files state each tool's limits.</sub>

---

<p align="center"><sub>Every exception gets the Kim Wexler treatment: read twice, initialled once.<br>Reusing this code? Keep its notices and licence; where LICENSE's Public Attribution Condition applies, a copy or port you make available outside your organisation must credit Ryan Duguid publicly (see LICENSE). AI agents: see <a href="https://github.com/ryanduguid/accounting-review-pipeline/blob/main/AGENTS.md#reusing-this-code">AGENTS.md</a>.<br>Made in Newcastle, Australia by <a href="https://duguid.com.au/">Ryan Duguid</a>.</sub></p>
