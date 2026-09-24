# Missing evidence evaluation

## Accounting problem

A month-end pack can look finished while evidence is absent, empty, for the
wrong period, or edited after the gate ran. This evaluation records what the
gate does in each of those cases, so a reviewer can compare it with their own
checklist before trusting the status.

## Intended reviewer

This pack is for a practitioner or manager checking whether the gate's result
matches what they would expect from the same evidence. It is not a client
workpaper and it does not assess a client's accounting records.

## Fabricated inputs

Every pack under `packs/` is a copy of the fabricated `examples/month-end-ready`
pack with one change. `expected_results.json` names that change, and
`tests/test_missing_evidence_evaluation.py` fails if a pack differs from the
example in any other file. No pack contains client data.

## Expected results

| Scenario | Change to the ready pack | Status | Finding | Controls not run |
| --- | --- | --- | --- | --- |
| `absent_prior_trial_balance` | `prior_trial_balance.csv` removed | `NOT_READY` | `MISSING_ARTEFACT` | `prior_findings` |
| `no_bank_reconciliation` | `bank_rec.csv` removed | `READY` | none | `bank_rec`, `prior_findings` |
| `empty_bank_reconciliation` | `bank_rec.csv` left with no bytes | `NOT_READY` | `EMPTY_ARTEFACT` | `bank_rec`, `prior_findings` |
| `wrong_period` | self-review declares 31 May for a 30 June trial balance | `BLOCKED` | `PERIOD_ORDER` | `prior_findings` |

`no_bank_reconciliation` is the case to read twice. The bank reconciliation is
optional in the month-end profile, so a balanced pack without it reaches
`READY`. The status is true of the controls that ran; the summary's "Controls
not run" section says the bank control did not run. A reviewer who reads only
the status misses that.

`prior_findings` appears in every row because none of these packs carries a
prior review's findings, so the repeat-finding control has nothing to compare.

### Edited after the gate

The gate writes `readiness-pack.json`, `readiness-summary.md` and
`findings.csv`. If the summary is edited afterwards (here, the preparer
initials changed from `CD` to `XY`), `review-ready view` refuses the pack and
exits 1 with `preparer initials disagrees`. The viewer checks that the three
files agree; it does not re-read the source files, so rerun the gate when a
source may have changed since the pack was written.

## Reproduce the result

```bash
uv sync --locked --all-extras
uv run review-ready gate --profile month_end --pack evaluation/missing_evidence/packs/no_bank_reconciliation --output ../../../review-ready-demo/missing-evidence
uv run pytest tests/test_missing_evidence_evaluation.py -q
```

Run both from the component directory. `--output` points outside the checkout
because the command refuses an output directory inside a version-control
checkout.

## Human decision still required

READY means no configured gate tripped; it does not mean every control ran, and it is not approval. Read the controls not run before relying on the status. A human remains accountable for professional judgement and approval.

## Practitioner review

Pending. No external practitioner has yet confirmed that these are the results
they would expect. Until one does, treat the table above as the tool's recorded
behaviour, not as a benchmark.

## Product and fixture version

Product release `0.1.7`; fixture version `1`.

## Limitations and non-claims

This is a deterministic evaluation of configured gates against fabricated
inputs. It does not test the BAS or year-end profiles, and it is not approval,
advice or a conclusion that a pack is correct.
