# v1.0.0

- Relicensed from Apache-2.0 to FSL-1.1-ALv2 (Functional Source License, with
  Apache-2.0 as the future licence). Released versions keep their licence.
- Replace the Python runtime and import API with the `review-ready` Rust binary.
  Preserve the five commands, readiness statuses, exit codes, pack schemas,
  decimal controls, document evidence and publication recovery protocol.
- `gate` takes a repeatable `--require-control NAME`. A named control that was
  skipped or failed raises a `NOT_READY` finding, `REQUIRED_CONTROL_SKIPPED` or
  `REQUIRED_CONTROL_FAILED`. An unknown name is exit 1. The option makes `--r` and
  `--re` ambiguous: they used to abbreviate `--review-note`, so write `--rev` or
  longer. A build older than this option refuses a flagged pack in `view`. Packs
  record each name and state in a new `required_controls` key, which
  appears only when the flag is used; runs without it keep their bytes.
- Add `--version` for native executable identity checks. Power BI records the
  native version and executable hash alongside the Python close-control identity.
- Use native help, usage and JSON syntax diagnostics. Limit JSON nesting to
  1,000 containers and decimal rendering to a 1 GiB resource ceiling.
- Decode publication control files as UTF-8. Unix SIGTERM is managed at writer
  checkpoints; SIGINT and Windows process termination can leave unresolved state
  that requires review before retrying.
- Record a `producer` block (component, version, source commit or `unknown`,
  pack contract `2`) in packs the command writes, repeated in the summary's
  Scope section and cross-checked by `view`. Set
  `ACCOUNTING_REVIEW_SOURCE_COMMIT` at build time to record the commit. Packs
  without the block still verify; a `view` from before it refuses packs that
  carry it. The library and the Python differential probe write none.
- Breaking: the `month_end` profile requires `bank_rec.csv`. A balanced pack
  without its bank reconciliation used to reach `READY`, with the control listed
  only under 'Controls not run'; it is now `NOT_READY` with `MISSING_ARTEFACT`.
- Breaking: each bank reconciliation `GLBalance` must agree with the current
  trial balance's `YTDDebit - YTDCredit` for its `AccountID`, within
  `--tieout-tolerance`. A reconciliation that agreed only with itself used to
  reach `READY`; it, and an account the trial balance does not hold, is now
  `NOT_READY` with `BANK_REC_BREAK`. Balances are debit-positive, so an
  overdrawn account's balances are negative.
- `year_end` takes an optional `fs_mapping.csv` (`StatementLine,AccountID,NormalBalance`)
  that computes each `TIED` line's `StatementAmount` from the trial balance,
  within `--tieout-tolerance`. A disagreeing line, a mapping to an account or line
  the pack does not have, and a trial-balance account with a balance on no line
  are `TIEOUT_BREAK`; a `TIED` line with no mapped account is
  `TIEOUT_UNSUPPORTED`. Without the file, `fs_mapping` is listed under
  'Controls not run' and `--require-control fs_mapping` makes it a condition.
- Breaking: in the `bas` profile, the GST control accounts must be in the
  trial balance, and their closing `YTDCredit - YTDDebit` must equal the GST
  control file's movement plus the balance they carry in, within
  `--tieout-tolerance`. The activity statement's optional `GST_OPENING` label
  declares the balance carried in from before the period and still held at
  `period_end`, such as an unpaid earlier BAS or the cents whole-dollar labels
  leave, but not an earlier BAS paid during the period; without it the balance
  carried in is taken as nil, and a break says so. A break, and the existing
  1A less 1B break, names a posting whose direction and size, or twice its
  size, explain the difference, or a difference divisible by 9 cents. The
  `bas-ready` and `bas-blocked` examples' trial balances now hold GST account
  820.
- Native release publishing is not configured. The release workflow fails
  explicitly and cannot publish Python or native artefacts.

# v0.3.0

- Breaking: requires Python 3.14 or later. 0.2.0 remains the last release for Python 3.11 to 3.13 ([#321](https://github.com/ryanduguid/accounting-review-pipeline/pull/321)).
- Licence: Apache-2.0 replaces MIT, with a `NOTICE` file whose attribution anyone redistributing the package or a derivative must keep. Versions already released stay under MIT ([#358](https://github.com/ryanduguid/accounting-review-pipeline/pull/358)).
- The BAS tie-out accepts labels 1A and 1B reported in whole dollars: when both are integral, a difference under $1 from the cents-level GST control movement no longer breaks the tie-out, and a failing finding names the allowance ([#341](https://github.com/ryanduguid/accounting-review-pipeline/pull/341)). Real activity statements used to return `NOT_READY` almost every time.
- `review-ready gate` publishes its three pack files through a `.reviewready` control directory inside `--output`, so an interrupted or concurrent write can no longer leave a mixed pack that the viewer accepts. `verify_pack` refuses a pack whose revision changes while it is read ([#340](https://github.com/ryanduguid/accounting-review-pipeline/pull/340)).
- An oversized CSV header or field is reported as a `SchemaError` with the source path and exit 1, and no pack JSON is written ([#353](https://github.com/ryanduguid/accounting-review-pipeline/pull/353)).

# v0.2.0

- Breaking: requires Python 3.11 or later. CPython 3.10 reached end of life on 1 October 2026; 0.1.10 remains the last release that installs on Python 3.10 ([#318](https://github.com/ryanduguid/accounting-review-pipeline/pull/318)).
- The self-review and review acknowledgement loaders refuse a JSON member stated twice at any depth, so a conflicting second value can no longer replace the first ([#307](https://github.com/ryanduguid/accounting-review-pipeline/pull/307), [#309](https://github.com/ryanduguid/accounting-review-pipeline/pull/309)).
- `load_self_review` and the viewer report JSON nested too deeply to parse, or holding an over-long integer, as `SchemaError` instead of a traceback; `review-ready gate` used to end in a `RecursionError` traceback on 100,000 nested arrays ([#309](https://github.com/ryanduguid/accounting-review-pipeline/pull/309)).

# v0.1.10

- `review-ready compare` tracks document coverage between readiness runs. Removing an unreviewed supporting document used to leave the earlier finding labelled `NOT_RAISED` with no change in scope; that finding is now `NOT_COMPARABLE`, and the comparison reports the change as `document_coverage` under `scope_changes`. Document slots come from filenames and need exactly one original file each; changed evidence labels are accepted. Readiness statuses, pack verification and finding groups are unchanged.
- The internal `TieOutRow.statement_amount` field, which was parsed and never read, is gone; the package exports only `ReadinessPack` and `review_pack`. The loader still validates that column.

# v0.1.9

- Flag GST control postings dated after the self-review's `period_end` as
  `NOT_READY` (`PERIOD_ORDER`, slot `gst_control_gl`). The BAS tie-out summed
  every GST row, so a GST file and activity statement from the next quarter tied
  and read `READY` for the declared quarter. Only the upper bound is checked;
  a lower bound needs a period start the self-review does not carry yet.
- State two limits in the README: a bank reconciliation is checked against the
  GL balance it records itself, not the pack's trial balance, and the GST
  control file only for postings after `period_end`.
- Add `review-ready compare`, which verifies two runs of one pack and reports
  status, source digest and finding-group changes as `NEW`, `RECURRING`,
  `CHANGED`, `NOT_RAISED` or `NOT_COMPARABLE`.
- Link DrDebits at its renamed repository, `llm-tax-guardrails`.

# v0.1.8

- Add local synthetic document intake with a separate human transcription
  review and optional evidence controls. The original, extraction and review
  bytes are bound to the saved pack and the verified view. A missing pack input
  still reports `NOT_READY`; a genuine context mismatch blocks. Existing
  invocations and schemas are unchanged, and there is no new runtime
  dependency.
- Accept only the extended ISO date the loader requires in the readiness pack.
- Add a missing-evidence evaluation: fabricated month-end packs that each
  remove or empty one artefact, plus an edited-artefact case.

# v0.1.7

- Bind the trial balance to the declared period and check the acknowledgement effect the pack records, not the one the preparer asserts.
- Hold pack items to the writer's exact member set.
- Sum trial-balance totals exactly or refuse the file, and set the decimal context at every gate entry point.
- Close the viewer, parser, Power BI and documentation findings from the September fact check.
- Require the component's own checks to pass before a release is published.

# v0.1.6

- Return NOT_READY for exception tie-outs and report unreadable inputs as input errors.
- Verify the complete canonical summary and derive viewer status from its findings.
- Prevent a preparer from acknowledging their own pack.

# v0.1.5

The `review-ready-gate/v0.1.4` tag was pushed on 7 September 2026 and its
release workflow failed at startup: the shared policy commit it called,
`787db459`, had been left unreachable by a later release-policy history
rewrite, so no job ran and no release or PyPI upload was created. A pushed tag
is never moved, so the same tree ships as `v0.1.5` with every root release
caller repointed to `fcf25e5`, the on-`main` commit whose tree is identical to
the dead pin.

Discovery copy leads with the public name, Workpaper Review Gate, and the
sibling links point at the components' maintained homes inside the Accounting
Review Pipeline. The release workflow calls the shared Python release policy at
a commit reachable from that repository's `main`, and the component's CI now
runs the shared gates:
its own lockfile check, ruff, mypy, pytest with branch coverage on Python 3.10,
3.12 and 3.13, pip-audit over the locked environment and an installed-wheel
smoke, with the check tools pinned in the `dev` extra. The published schema
identity, runtime behaviour and fabricated fixtures are unchanged from v0.1.3.

# v0.1.3

First maintained release from `packages/review-ready-gate` in the Accounting
Review Pipeline. Project, citation and evaluation links now point to the
canonical monorepo, and the namespaced release publishes through the shared
attested Python policy. The published schema identity, runtime behaviour and
fabricated fixtures are unchanged from v0.1.2.

# v0.1.2

Correct source-distribution evidence packaging and immutable documentation
metadata. Product runtime behaviour and fabricated fixtures are unchanged.

Local CLI that gates BAS, month-end, and year-end workpaper packs before they
reach manager review. Fabricated fixtures only. Not advice.
