# Workpaper Review Gate

Workpaper Review Gate checks whether an Australian accounting workpaper pack is
ready for manager review. Version 1.0.0 provides the `review-ready` Rust binary.

From this component directory, install with Rust stable and run the fabricated demo:

```bash
cargo install --path . --locked
review-ready gate --profile bas --pack examples/bas-ready --output ../../../review-ready-demo/bas-ready
```

The demo exits `0` for `READY`. A human still decides whether the pack may proceed.
Python imports and the Python runtime were removed in this major version.
Native release publishing is not configured; see [RELEASING.md](RELEASING.md).

```
+----------------------------------------------------------------------+
|                        Workpaper Review Gate                         |
+----------------------------------------------------------------------+
|     Stop incomplete workpapers reaching manager review               |
+----------------------------------+-----------------------------------+
| DR  what it gives you            | CR  what it needs                 |
+----------------------------------+-----------------------------------+
| READY / NOT_READY / BLOCKED      | a pack directory of artefacts     |
| cover sheet for the reviewer     | a self-review JSON from the prep  |
| repeat-finding flags             | -                                 |
+----------------------------------+-----------------------------------+
```

<div align="center">

[![tests](https://img.shields.io/github/actions/workflow/status/ryanduguid/accounting-review-pipeline/ci.yml?branch=main&label=tests&labelColor=000000&style=flat)](https://github.com/ryanduguid/accounting-review-pipeline/actions/workflows/ci.yml)<br>
[![Rust stable](https://img.shields.io/badge/rust-stable-66023C?logo=rust&labelColor=000000&style=flat&logoColor=E5E4E2)](https://www.rust-lang.org/)
[![License: Ryan Duguid Public Attribution Licence 1.0](https://img.shields.io/badge/License-Public%20Attribution%201.0-66023C?labelColor=000000&style=flat)](LICENSE)

</div>

[Browser evaluation](https://duguid.com.au/evaluate/manager-review-gate/) · [Reproduce locally](evaluation/manager_review_gate/README.md) · [Quick demo](#quick-demo) · [Release notes](RELEASE_NOTES.md)

The maintained source is under
[`packages/review-ready-gate`](https://github.com/ryanduguid/accounting-review-pipeline/tree/main/packages/review-ready-gate)
in the Accounting Review Pipeline. The `review-ready` command retains the 5
commands, flags, readiness statuses and payload formats of the Python baseline.
There is no supported `reviewready` Python import API.

A local, **review-first readiness gate** for Australian public-practice packs. You point it at a folder of workpapers from a junior, an offshore team or an AI agent. It tells you whether that folder is allowed to enter manager review.

A public evaluation pack reproduces the v0.1.5 manager-review result on fabricated BAS fixtures. It is a local review aid, not an approval system.

## Fabricated proof

| Pack | Result | What the gate reports |
| --- | --- | --- |
| `examples/bas-ready` | `READY` | No configured findings. A human still decides whether the pack may proceed. |
| `examples/bas-not-ready` | `NOT_READY` | Missing GST control export, incomplete self-review and a blocking open item. |

Open the [manager-facing browser evaluation](https://duguid.com.au/evaluate/manager-review-gate/) or reproduce the [versioned evaluation pack](evaluation/manager_review_gate/README.md) from the fabricated fixtures.

It is the missing upstream step in this stack:

```text
Incomplete pack
      |
      v
review-ready-gate      <-- maintained monorepo package
      |
      |  READY
      v
Manager review (judgement, risk, client impact)
      |
      v
Monthly Close Controls / payday-super-checker / other engines
```

[Monthly Close Controls](https://github.com/ryanduguid/accounting-review-pipeline) answers 'what material exceptions exist on these trial balances?'. This tool answers a prior question: **'is the pack even allowed onto the review desk?'** A file can have material variances and still be READY, because the variances are documented. A file with a missing GST control export is NOT_READY even if the numbers look tidy.

It does **not** connect to Xero, store OAuth tokens, write journals, lodge BAS, lock a period, call an LLM or claim that a file is correct. A `READY` result means no configured gate tripped on the files that were present; a human still decides.

## Why this exists

Preparation got cheaper. Review did not. Software, offshore capacity and AI all increase the number of files that hit the review desk. The scarce resource in a firm is the manager who can actually sign.

Missing tie-outs, open questions in email, an unbalanced trial balance and unresolved findings from last period send a pack back for more preparation. This gate checks for those gaps before the pack reaches the manager.

## Quick demo

The repository contains fabricated data only. Do not commit client workpapers.

For optional supporting documents, the [synthetic document intake example](examples/document-intake/README.md)
preserves original bytes, proposed fields and human corrections. It adds document checks to an
existing pack without replacing its trial balance or changing the readiness output schema.

For the manager-facing, reproducible BAS evaluation, see the [manager review gate evaluation pack](evaluation/manager_review_gate/README.md).
The [missing evidence evaluation](evaluation/missing_evidence/README.md) shows what the gate
does when a month-end source is absent, empty, for the wrong period or edited after the gate
ran, including a balanced pack that is `NOT_READY` without its bank reconciliation.

[`examples/`](examples/README.md) is the assault course: every move the tool has, run against
fabricated data, with nothing at stake. Learn the flags here before pointing
it at a real pack.

```bash
cargo install --path . --locked

review-ready gate \
  --profile bas \
  --pack examples/bas-ready \
  --output ../../../review-ready-demo/bas-ready
```

The ready demo exits `0` and writes 3 files:

- `readiness-summary.md`: cover sheet a manager reads top to bottom
- `findings.csv`: one row per finding, for Excel or Power BI
- `readiness-pack.json`: structured evidence, source hashes and any supplied acknowledgement

The output also reserves `.reviewready` for writer admission and publication state.
Keep it with the pack when copying a directory. The 3 returned payload paths and
their contents retain their existing format. Use a private output directory with
cooperating writers; protection against hostile pathname replacement is outside
this protocol.

```bash
review-ready gate \
  --profile bas \
  --pack examples/bas-not-ready \
  --output ../../../review-ready-demo/bas-not-ready
```

The not-ready demo exits `2`. The GST control export is missing, the preparer has not certified the pack, a blocking open item is still OPEN and the same missing artefact was OPEN on the prior pack, so it is flagged as a repeat.

```bash
review-ready view --pack-dir ../../../review-ready-demo/bas-ready
```

`--output` points outside this checkout, and it has to: the command refuses an
output directory inside a version-control checkout, exiting `1` without
creating anything. A readiness pack names a client's file, its workpaper
references and every finding standing between it and manager review, and
inside a checkout it is one `git add -A` away from a history that every clone
copies. A `.gitignore` entry is a convention the next commit can waive, and it
does nothing about the copy sitting in the working tree meanwhile. A checkout
is any directory at or above the output holding `.git`, `.hg`, `.svn` or
`.bzr`, because the harm is the pack going under version control and every one
of those copies a committed pack to each clone. A directory inside the work
tree named by `GIT_WORK_TREE` counts too, since Git can hold its metadata
elsewhere and leave a tracked tree carrying no marker to find. A work tree
selected some other way, by `--work-tree` on another process's git invocation
or by `core.worktree` in a repository this command never opens, cannot be
discovered from here: the check is a backstop for the location you chose, not
a proof that a directory is untracked. An output the command cannot examine,
such as one behind a symbolic-link loop or an unreadable parent, is refused on
the same grounds rather than assumed safe. The refusal applies to
writing only: `review-ready view` still opens a pack wherever it already is.

Use exit code `0` only for `READY`, `2` for `NOT_READY` or `BLOCKED`, and `1` for a malformed file, an invalid command, an `--output` path inside a version-control checkout, or an `--output` path that cannot be written.

To run the gate on a schedule in CI, copy [examples/github-actions-readiness-check.yml](examples/github-actions-readiness-check.yml) into `.github/workflows/`.
It runs against a repo-stored synthetic pack, fails the job when the pack is `BLOCKED` and reports `NOT_READY` for a human. A `READY` result is still not an approval.

## What gets gated

| Profile | Required artefacts | Extra controls |
| --- | --- | --- |
| `bas` | trial balance, activity statement, GST control GL, open items, self-review | 1A less 1B ties to GST control movement; the GST control accounts roll forward to the TB; no GST posting dated after `period_end`, or before `period_start` where the self-review declares one |
| `month_end` | current TB, prior TB, bank rec, open items, self-review | bank rec statement against its GL balance, and that GL balance against the current TB; prior date earlier; same tenant |
| `year_end` | current TB, prior TB, tie-out matrix, open items, self-review | no `UNSUPPORTED` statement lines; with `fs_mapping.csv`, each `TIED` line computed from the current TB |

Optional in every profile: `prior_findings.csv`. An OPEN prior finding that is still present is marked `repeat`. Optional in `year_end`: `fs_mapping.csv`, the [computed statement tie-out](#computed-statement-tie-out). Optional in `bas`: `bank_rec.csv`, which runs the bank reconciliation described next; use `--require-control bank_rec` when its absence must prevent `READY`.

The bank reconciliation is tied twice for each account. Its `StatementBalance`
must agree with its `GLBalance`, and that `GLBalance` must agree with the current
trial balance's `YTDDebit - YTDCredit` for the same `AccountID`, each within
`--tieout-tolerance`. `StatementBalance` and `GLBalance` therefore use that signed
convention: debit balances positive, credit balances negative, so an overdrawn
account is negative. A reconciliation prepared against an earlier ledger state,
or for an account the trial balance does not hold, is a `BANK_REC_BREAK`.

The GST control file is checked against the period the self-review declares. A
posting dated after `period_end` is a `PERIOD_ORDER` finding, and so is one dated
before `period_start` when a `bas` self-review declares it. One limit remains:
`period_start` is optional, so that packs prepared without it still load, and
without it a file from an earlier quarter passes the date check. The pack does
not record whether it was declared: check the self-review file that the pack's
`self_review` sha256 identifies.

A `READY` status means no configured control tripped, and only the controls that
ran can trip. Every optional slot with no usable input is therefore listed under
'Controls not run' in the pack JSON and the summary, and counted in the summary's
scope block. An otherwise compliant pack can still reach `READY` without
`prior_findings.csv`, with the repeat-finding control not run, which is a
different claim from `READY` with every control run. The `month_end` profile
is for entities with bank accounts and requires their bank reconciliation.
Entities without a bank account are outside this profile and cannot reach
`READY`; it has no applicability waiver. An optional file that exists but
holds no bytes is a finding
as well, because an empty file is not evidence; its digest is still recorded, so a
reviewer can see which bytes the finding is about.

`--require-control NAME` names a control this pack needs, so a script reading the
exit code cannot take `READY` for coverage it did not get. Repeat it for more than
one. `NAME` is a slot name, the name 'Controls not run' uses: `prior_findings` is
the one every profile can skip. A control counts as having run when its file was
in the pack, loaded and held at least one row. Each named control ends in one of 3
states:

| State | Meaning | Effect |
| --- | --- | --- |
| `ran` | The file loaded. Anything wrong with it is in its own findings. | None. |
| `skipped` | No file was supplied, or the `--profile` has no such slot. | `NOT_READY` finding, `REQUIRED_CONTROL_SKIPPED`, exit `2`. |
| `failed` | The file is empty, or holds a header and no rows. | `NOT_READY` finding, `REQUIRED_CONTROL_FAILED`, exit `2`. |

A file that cannot be loaded stops the gate before any pack is written, with exit
`1`. The gate's 3 statuses are unchanged, and an unmet requirement can never give
`READY` or exit `0`. A name that is no profile's slot is exit `1` with no pack,
because a mistyped requirement would otherwise require nothing. A header-only
`open_items.csv` or `prior_findings.csv` still gives `READY` when its control is not
required; requiring the control is what asks for rows. The command prints each
state. The pack records them under `required_controls` as `name` and `state` pairs,
sorted and without repeats, and in the summary's scope block. `view` refuses a pack
whose states disagree with its findings, or that names a control twice or names none
it should. A build older than this option refuses a flagged pack, because
`required_controls` is a member it does not know. `compare` reports a change under
`scope_changes`. A run without the flag prints and writes none of this.

Filenames inside the pack directory are fixed. Header-only CSV schemas live under `schemas/`, together with the JSON Schema for `self_review.json`.

### Self-review

`self_review.json` is part of the pack, not a courtesy. Exact keys, exact assertion names, JSON booleans only:

```json
{
  "preparer_initials": "AB",
  "prepared_on": "2026-04-10",
  "engagement_type": "bas",
  "period_end": "2026-03-31",
  "assertions": {
    "pack_complete": true,
    "tie_outs_done": true,
    "open_items_listed": true,
    "variances_explained": true,
    "self_reviewed": true
  }
}
```

`engagement_type` must match `--profile`. `prepared_on` must not be earlier than `period_end`. Any assertion that is not `true` is `NOT_READY`. The assertions are necessary, not sufficient: a preparer who ticks `pack_complete` whilst the GST control file is missing still gets `MISSING_ARTEFACT`. The JSON Schema is [schemas/self_review.json](schemas/self_review.json).

A `bas` self-review may also declare the first day of its period, such as `"period_start": "2026-01-01"`. A GST control posting dated before it is then `NOT_READY` with `PERIOD_ORDER`, which is what stops an earlier quarter's file. It must be an ISO date not later than `period_end`. The other profiles read no period start and refuse the key.

### Open items

```text
ItemID,Severity,Owner,DueDate,Status,Description,Resolution
```

`Severity` is `BLOCKING`, `EXPLAIN` or `TRIVIAL`. `Status` is `OPEN` or `CLEARED`. A `BLOCKING` item that is still `OPEN` with no resolution blocks the gate. A `CLEARED` item with no resolution text also fails: cleared means someone wrote down what happened.

### Trial balance

The 10-column canonical CSV from [xero-trial-balance-export](https://github.com/ryanduguid/accounting-review-pipeline/tree/main/packages/xero-trial-balance-export):

```text
ReportDate,Tenant,Section,AccountID,AccountName,AccountCode,Debit,Credit,YTDDebit,YTDCredit
```

One tenant, one report date, unique `Tenant`+`AccountID`. Movement debit must equal movement credit, and YTD debit must equal YTD credit, or the pack is `BLOCKED`.

### BAS tie-out

If both the activity statement and the GST control GL are present:

- labels `1A` and `1B` are required
- `1A - 1B` is compared with GST-control `sum(Credit) - sum(Debit)`
- a difference beyond `--tieout-tolerance` (default `$0.01`) is `NOT_READY`
- when `1A` and `1B` are both whole dollars, as the ATO requires on a lodged statement, any difference under `$1` also ties: the ATO drops the cents from each label, so the statement can sit up to 99 cents either side of the control movement. A statement whose labels carry cents gets no allowance

If the GST control file is present, its accounts also roll forward to the trial
balance:

- every `AccountID` in the GST control file must be in the trial balance
- those accounts' closing balance in the trial balance, `YTDCredit - YTDDebit`, is compared with the balance they carry in plus the file's `sum(Credit) - sum(Debit)` for the period; a difference beyond `--tieout-tolerance` is `NOT_READY`, with no 99-cent allowance, since all the figures are ledger amounts
- the balance carried in is the activity statement's optional `GST_OPENING` label: what the accounts carry in from before the period and still hold at `period_end`, which is the period-start balance less any settlement of it posted during the period, since the GST control file holds only the period's GST. It is signed like the closing balance, so a payable is positive and a refund owed is negative. Declare, for example, an earlier BAS still unpaid under a payment plan, or the cents left behind when whole-dollar 1A and 1B were paid. An earlier BAS paid during the period is not carried in: declare nothing for it. A malformed or repeated `GST_OPENING` is refused like any other activity-statement amount
- without `GST_OPENING` the balance carried in is taken as nil, and a break says so. With every earlier period's GST settled through the control accounts, they close at the period's movement
- the gate does not verify `GST_OPENING`, and a `READY` pack does not show it: the pack records only the activity statement's sha256. The reviewer checks the declared figure against the earlier BAS or the ATO account

A GST break names the likeliest cause it can see: a posting in the file that the
statement or trial balance lacks, or holds on the other side (half the
difference), or a difference in whole cents divisible by 9, which transposed
digits produce. A posting is named only when its direction explains the
difference, not merely its size. The hint is a starting point, not a diagnosis.

This is a cash-style control-account tie-out on the files you supply. It is not a lodgement, not a cash-versus-accruals bridge, and not a substitute for the `bas-preparation` skill.

### Computed statement tie-out

The tie-out matrix records a `Status` the preparer asserts. With `fs_mapping.csv`
beside it, the gate also computes each `TIED` line's `StatementAmount` from the
current trial balance:

```text
StatementLine,AccountID,NormalBalance
```

- each row places one trial-balance `AccountID` on one `StatementLine` of the matrix. A line may take several accounts, and an account may sit on more than one line, such as a subtotal
- a `DEBIT` line is the sum of its accounts' `YTDDebit - YTDCredit`, and a `CREDIT` line the sum of `YTDCredit - YTDDebit`, so trade creditors with a credit balance show positive on a `CREDIT` line and a loss shows negative on a `CREDIT` profit line. Every row for one line must give the same `NormalBalance`
- a computed amount that differs from `StatementAmount` by more than `--tieout-tolerance` is `TIEOUT_BREAK`. There is no rounding allowance of its own: a statement presented in whole dollars needs a run tolerance that covers its rounding
- a `TIED` line no row maps is `TIEOUT_UNSUPPORTED`. A row naming an account the trial balance does not hold or a line the matrix does not have, and a trial-balance account with a balance that no row maps, are `TIEOUT_BREAK`
- `EXCEPTION` and `UNSUPPORTED` lines keep their own findings and are not computed

Without the file the computed tie-out is listed under 'Controls not run', and a
`READY` year-end pack rests on the statuses the preparer asserted. Name it with
`--require-control fs_mapping` to make the computed tie-out a condition of
`READY`. The figures come from the matrix: the gate reads no PDF or signed
statement.

## Human acknowledgement

Optional `--review-note` JSON:

```json
{
  "reviewer_initials": "RD",
  "reviewed_on": "2026-04-12",
  "comment": "Reviewed fabricated demo findings only; no client file was approved by this example."
}
```

`reviewed_on` must not be earlier than `period_end`. An acknowledgement is evidence of a human action only. It **never** changes `NOT_READY` or `BLOCKED` to `READY`.

## Viewing an existing pack

`review-ready view` is the read-only half of the gate: it loads a generated pack, proves the 3 files still agree with each other and prints the cover sheet. It never writes, renames or deletes anything, and it cannot change what the engine computed.

```bash
review-ready view --pack-dir ../../../review-ready-demo/bas-ready
```

Before displaying anything it fails closed on: a missing artefact; JSON that is not valid UTF-8, not valid JSON, or carries unknown, missing or duplicated top-level members; a threshold or nested source digest that no longer parses as the writer rendered it; a `readiness-summary.md` whose overall status, source-evidence digests or review-boundary statement disagree with the JSON (including a second, conflicting status line); and a `findings.csv` whose header, row count or any cell disagrees with the JSON findings, honouring the writer's formula-injection guard exactly. On success the sheet ends with the SHA-256 of each artefact's exact bytes, so the displayed evidence can itself be archived. Exit code is 0 when a pack was verified and shown, 1 when verification failed.

Packs `review-ready gate` writes record a `producer` block: the component, its version, the source commit or `unknown`, and pack contract `2`. The Scope section repeats it, and `view` refuses a pack whose block is malformed or disagrees with that line. A pack without the block (contract 1) still verifies.

Payload entries must be regular files with a single hard link. The viewer rejects
links, directories and special files before reading them. For a pack carrying
`.reviewready`, it also refuses unresolved publication and a revision change whilst
reading. Legacy packs without that directory retain schema and cross-file checks;
they have no publication revision witness.

The CLI observes its default SIGTERM at safe checkpoints between bounded writer
operations. Before commit, recovery restores previous payload bytes and absence
where possible. It does not preserve filesystem metadata. After commit, an error
identifies the pack as published. A retry can finish ordinary cleanup, but a
cleanup-state error requires review. A missing referenced transaction also refuses
the next writer, including failure after transaction removal but before clearing
its state reference.

If publication is unresolved or recovery evidence is retained, stop writers and
preserve the complete output directory, including `.reviewready`. Do not delete
control files or retry into that directory without investigating the error. To
regenerate from the same inputs whilst retaining evidence, choose a new output
directory. Process death is detected when its state remains unresolved; physical
power-loss durability is unverified.

## Listing many packs

`review-ready queue` verifies every pack you name, with the same checks as `view`, then lists them in one table: each pack's profile, period end, status, finding count, finding codes and the controls that did not run. It writes nothing.

```bash
review-ready queue --pack-dir outputs/varrock --pack-dir outputs/falador --pack-dir outputs/cedar-and-pine --summary
```

Repeat `--pack-dir` for each pack; the queue reads only those and does not look in neighbouring directories. `--status NOT_READY` shows only packs with that status. `--summary` adds counts across the displayed packs: how many are `READY`, `NOT_READY` and `BLOCKED` and which clients they are, how many packs and findings carry each finding code, and which clients did not run each control. A pack written before control coverage was recorded shows `not recorded` and is named in a note rather than counted.

A pack records no client name, so the queue labels each one by the name of the directory its path resolves to; a symbolic link shows its target's name. Name each output directory after its client; a misnamed directory shows under the wrong label. The full path and the SHA-256 of each verified file follow the table.

Every pack is verified before any filter runs. One pack that fails verification fails the whole command with exit `1` and prints no table, even when `--status` would have hidden it. Exit `0` means every pack verified and the list printed, whatever their statuses; it is not a readiness verdict.

## Comparing 2 runs of a pack

A pack sent back to the preparer is gated again. `review-ready compare` verifies both runs with the same checks as `view`, then prints JSON showing what moved between them: the overall status, each source file's digest, and each group of findings by code and slot. It writes nothing.

```bash
review-ready compare --previous-pack-dir outputs/bas-first-run --current-pack-dir outputs/bas-second-run
```

Each finding group is `NEW`, `RECURRING`, `CHANGED`, `NOT_RAISED` or `NOT_COMPARABLE`. `NOT_RAISED` means absent from the later run, not resolved. A changed tolerance or control coverage is listed under `scope_changes`, and a finding absent after a tolerance change, from a slot the later run did not check, or from a later pack that states no control coverage, is `NOT_COMPARABLE`. Tolerances compare by value, so `0.01` and `0.010` are the same setting. Exit `0` means both packs verified and the comparison printed; it is not a readiness verdict. A pack that fails verification, or 2 packs for different engagement types or periods, exit `1`. Packs carry no wall-clock time, so the reviewer records who made each change and why.

The comparison records changes to the supplied documents as `document_coverage`
under `scope_changes`. It matches original files by their hashes and retains
duplicate counts. Reordering documents, correcting intake metadata or recording a
human review preserves that coverage. A finding absent after its document is
removed becomes `NOT_COMPARABLE`; duplicate copies match in their original order.
Findings still group by code and slot, so reordering can move a finding between
groups. The source rows show the changed file hashes in each slot.
Each recorded document slot must identify exactly one original file. Missing or
duplicate original-file evidence prevents comparison and exits `1`.
Document filenames establish the slots even when evidence labels change. A
numbered document label that contradicts its filename also prevents comparison.

## Design

- Decimal arithmetic preserves money values and uses a 28-digit, half-even context for control calculations. The gate refuses trial-balance totals or BAS tie-out amounts that would round.
- Schema, duplicate-key, date and numeric gates fail closed: a malformed file is exit `1` and writes no pack.
- Missing or empty required artefacts are findings, not crashes, so the cover sheet can tell the preparer what to send back.
- Source SHA-256 digests travel with the pack. Each digest is taken from the same immutable byte snapshot the loader parsed.
- Spreadsheet-facing finding text whose first non-whitespace character is `=`, `+`, `-` or `@` is prefixed with an apostrophe.
- The 3 payloads are staged before replacement under exclusive writer admission. The supported viewer refuses an unresolved publication; failed restoration retains its backups for review.
- No wall-clock timestamps in the pack.

## Data boundary

- Use a separate, access-controlled working directory for client source files and outputs.
- A generated pack cannot be written into a version-control checkout. The writer walks up from the resolved `--output` directory and refuses if any level holds `.git`, `.hg`, `.svn` or `.bzr`, before it creates anything; a `.git` file counts as well as a directory. A level it cannot examine is refused.
- Keep this checkout limited to fabricated fixtures. Its `.gitignore` blocks CSVs outside `examples/`, `schemas/` and `evaluation/missing_evidence/packs/`, and blocks all 3 generated pack files by name. That stays as a second line: it catches a pack copied in by hand, which no guard on the writer can see.

## Related

- [Monthly Close Controls](https://github.com/ryanduguid/accounting-review-pipeline/tree/main/packages/monthly-close-control-plane) - exception pack once a trial balance is allowed onto the review desk
- [Xero Ledger Review Gate](https://github.com/ryanduguid/accounting-review-pipeline/tree/main/packages/elizabeth-anne-alexander) - zero-network variance boundary for AI-assisted TB review
- [australian-accounting-skills](https://github.com/ryanduguid/australian-accounting-skills) - `workpaper-tie-out` and `bas-preparation` workflows this gate enforces mechanically
- [DrDebits](https://github.com/ryanduguid/DrDebits) - APES 110 / TPB guardrails for any LLM sitting *after* a READY pack

## Development

Use Rust stable and the committed Cargo lockfile:

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --locked --release
```

The test suite covers schema gates, fabricated packs for all 3 engagement profiles, empty and incomplete artefacts, GST, GST roll-forward, bank-rec and computed statement breaks, unsupported tie-outs, acknowledgement parsing, deterministic pack generation, fail-closed pack viewing and the command-line exit contract.

CI runs Cargo format, lint and tests on Linux and Windows, builds the release
binary and exercises it from an empty directory. The feature-gated differential
tool compares fabricated cases with Python source frozen at commit
`8061079d0b4206d064c54751aa047af8588fa265`; Python is needed only for that oracle.
CodeQL analyses the Rust implementation.
See [CONTRIBUTING.md](CONTRIBUTING.md) for verification and data handling.

### Deliberate major-version differences

- Help, usage and JSON syntax diagnostics have native wording. Exit codes retain their command meaning.
- `review-ready --version` prints the native package version for producer identity checks.
- JSON nesting is bounded at 1,000 containers. Decimal rendering refuses an output above its 1 GiB resource ceiling.
- SIGTERM recovery is supported on Unix. Windows process termination and SIGINT can stop the process abruptly; retained publication state prevents a mixed pack from being accepted. Preserve the complete directory and investigate before retrying.
- Publication control state is decoded as UTF-8, including an optional BOM. Its writer emits UTF-8; alternate UTF-16 or UTF-32 control files are refused.
- A `bas` pack's GST control accounts must be in the trial balance and roll forward to it, and a GST break carries a difference hint. The baseline tied only 1A less 1B to the control file. The differential's `bas` examples hold the GST control account and tie, so it compares them unchanged.
- A `bas` self-review may declare `period_start`, and a GST control posting dated before it is a finding. The baseline refuses the key. The differential's self-reviews do not declare it, so it compares them unchanged.
- A `year_end` pack lists `fs_mapping` under 'Controls not run' when the file is absent, and computes `TIED` lines from the trial balance when it is present. The baseline has no such slot. For the `year-end-ready` example the differential takes exactly that entry out of the native result, its pack JSON, the summary's line and count, and the printed list of controls not run, then compares everything else byte for byte, including the exit code, standard error and `findings.csv`; a result without exactly that entry is an unexplained difference.
- A `bas` pack lists `bank_rec` under 'Controls not run' when the file is absent, and runs the bank reconciliation when it is present. The baseline has no such slot. For every case that reviews a `bas` example the differential takes exactly that entry out of the native result, as it does for `fs_mapping`: its pack JSON, the summary's line and count, and the printed list of controls not run. Where that leaves no control, it writes what the renderer writes for none. It compares everything else byte for byte, and a result without exactly that entry is an unexplained difference. A `month_end` pack that supplies `prior_findings.csv` keeps the native output for no control not run in the comparison.
- `queue` is new: the baseline has no command that lists several packs.
- A bank reconciliation's `GLBalance` is tied to the trial balance, and an account the trial balance does not hold is a finding. The baseline compared it only with `StatementBalance`. The differential's packs tie on both, so it compares them unchanged.

## Licence

Licensed under the [Ryan Duguid Public Attribution Licence 1.0](LICENSE), the FSL-1.1-ALv2 text with a public attribution condition. You may use, modify and redistribute it for any purpose except a Competing Use as defined in LICENSE (making it available to others in a competing commercial product or service), and each version becomes available under Apache-2.0 2 years after it is made available. For versions made available from 4 November 2026, LICENSE's Public Attribution Condition applies to the FSL grant: anyone who makes a copy, port or derivative available outside their organisation must credit Ryan Duguid publicly, with a link to this repository. Internal use is exempt, and the condition does not apply to each version's Apache-2.0 licence. See [LICENSE](LICENSE) and [NOTICE](NOTICE). Keep the copyright notices in [NOTICE](NOTICE) when you redistribute or adapt it.

<sub>Not tax, financial, audit or legal advice. Boundary statement: [DISCLAIMER.md](DISCLAIMER.md).</sub>
