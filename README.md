# Accounting Review Pipeline: Xero month-end close controls

Synthetic examples. Review aid, not professional advice; a human decides whether the close is acceptable.

**Input:** the supplied current and prior trial balances, account mapping and subledger. The creditors reconciliation differs by $250.

From a clone, with [uv](https://docs.astral.sh/uv/) installed:

```bash
cd packages/monthly-close-control-plane
uv run --locked --extra dev close-control review --current examples/current_trial_balance.csv --prior examples/prior_trial_balance.csv --mapping examples/account_mapping.csv --subledger examples/subledger_balances.csv --absolute-threshold 10000 --percentage-threshold 0.10 --reconciliation-tolerance 0.01 --review-note examples/review_note.json --output ../../../close-control-demo
```

**Output:** `REVIEW`, eight exceptions, exit 2. Open `../../../close-control-demo/close-summary.md`.

| Finding | Evidence | Human decision |
| --- | --- | --- |
| Creditors reconciliation | $250 difference | Trace and explain the difference before sign-off. |
| Financial-year reset | 30 June compared with 31 July | Reconsider P&L YTD comparisons across the reset. |

[Read the five-minute close case](packages/monthly-close-control-plane/docs/manager-case-study.md) · [Inspect all eight exceptions](packages/monthly-close-control-plane/README.md#worked-example)

Every invented entity the components share, and the proposal to consolidate them into one fabricated firm, is recorded in [docs/fabricated-firm.md](docs/fabricated-firm.md).

<details open>
<summary>Setup, component identities, file contracts and reference</summary>

This repository brings together Monthly Close Controls and its related tools. Its canonical GitHub repository
is [`ryanduguid/accounting-review-pipeline`](https://github.com/ryanduguid/accounting-review-pipeline).
It holds seven independently versioned components joined only by local files and commands:

| Component | Directory | Identity | Version |
|---|---|---|---|
| Xero Trial Balance Export | `packages/xero-trial-balance-export/` | distribution `xero-trial-balance-export`, commands `export-tb` and `xero-tb-auth`; the only OAuth, Xero and network producer | 0.1.8 |
| Workpaper Review Gate | `packages/review-ready-gate/` | distribution `review-ready-gate`, import `reviewready`, command `review-ready` | 0.1.6 |
| Monthly Close Controls | `packages/monthly-close-control-plane/` | distribution `monthly-close-control-plane`, import `closecontrol`, commands `close-control` and `openaccountants-au` | 0.1.4 |
| Xero Ledger Review Gate | `packages/elizabeth-anne-alexander/` | distribution `elizabeth-anne-alexander`, import `elizabeth_anne_alexander`, command `elizabeth-anne-alexander` | 0.2.3 |
| Accounting Excel Toolkit | `adapters/accounting-excel-toolkit/` | source-archive adapter `accounting-excel-toolkit` (Power Query and VBA) | 0.1.6 |
| Australian Accounting Power BI | `apps/australian-accounting-power-bi/` | PBIP reference application, no release | none |
| evatt | `packages/evatt/` | distribution `evatt`, import `evatt`, command `evatt`; a local pseudonymisation boundary, no network of any kind | 0.1.0 |

Data flows in one direction: the exporter (or a manual Excel export) produces the ten-column
Xero trial-balance file, the readiness gate decides whether a pack reaches review, monthly
close surfaces exceptions, and the ledger-review boundary or Power BI consumes the result.
evatt works separately: it pseudonymises Markdown locally before an operator hands it to an
external model. It does not read trial balances.
Only the exporter may touch OAuth, Xero, HTTP or credentials. Every other component is
offline and ships fabricated data only. Python review packages use exact `Decimal`
arithmetic for money. The Excel adapter and Power BI application use their native
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
and import records. Historical releases and tags remain owned by the source repositories.

## Review-pack contract

Review packs are deterministic evidence for a human reviewer; they do not approve a close,
post a journal, make a payment, lodge a return or lock a period. Each pack is a directory whose
files must agree with each other: the JSON file is the machine-readable source of truth, Markdown
is its human-readable summary, and CSV exposes the findings or exceptions as rows. The
monthly-close pack carries a second CSV holding the questions for the client that those exceptions
raise. The producing command writes every file in one pass; the component's `view` command
consumes and cross-checks an existing pack without changing it.

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
| `acknowledgement` | Optional evidence of a later human review; it cannot change readiness or approve a file. |

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
| `acknowledgement` | Optional evidence of human review; it cannot change a state or approve or close a period. |

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

## Releases

Each component releases on its own namespaced annotated tag, `<component>/vMAJOR.MINOR.PATCH`,
through a root caller pinned to the independently reviewed Release Policy commit
`fcf25e532e9eb60056ae6e5c819cf3125c4f4b91`: `monthly-close-control-plane/v*`,
`review-ready-gate/v*`, `elizabeth-anne-alexander/v*`, `xero-trial-balance-export/v*`,
`accounting-excel-toolkit/v*` and `evatt/v*`. One tag publishes exactly one component; the
identity gate refuses a tag whose prefix does not equal the component directory leaf and its
distribution name or archive stem. Each component's `RELEASING.md` describes its preflight; the tag name
is the only difference, except that `release-evatt.yml` has no `pypi` job at all, so an
`evatt/v*` tag produces GitHub release assets and publishes to no index while the disclosure
policy that package enforces is unsigned. `IMPORTS.md` lists the callers and publisher
environments.

## Contract

`contracts/xero-trial-balance-v1/` is the data-only authority for the exporter-owned
`xero-tb-csv.v1` corpus: the exact ten-column schema, three fabricated fixtures, the expected
accept and reject results and `SHA256SUMS`. It is test and data input only and adds no shared
runtime package. `tests/test_xero_trial_balance_contract.py` and the
`joined-conformance.yml` workflow run the exporter runner and all three offline review
implementations against it.

</details>
