# Manager review gate evaluation

## Accounting problem

Before a BAS pack reaches a manager, a preparer needs a repeatable check that the configured completeness and blocking-item gates have not tripped. This evaluation records the expected behaviour for two fabricated BAS packs.

## Intended reviewer

This pack is for a manager evaluating whether the product's configured gate behaves as declared. It is not a client workpaper and it does not assess a client's accounting records.

## Fabricated inputs

Both inputs are fabricated repository fixtures: `examples/bas-not-ready` and `examples/bas-ready`. They contain no client data.

## Reproduce the result

```bash
uv sync --locked --all-extras
uv run review-ready gate --profile bas --pack examples/bas-not-ready --output outputs/evaluation-not-ready
uv run review-ready gate --profile bas --pack examples/bas-ready --output outputs/evaluation-ready
uv run pytest tests/test_evaluation_pack.py -q
```

## Expected findings

The `bas-not-ready` gate command exits 2 with `MISSING_ARTEFACT` for `gst_control_gl`, `SELF_REVIEW_INCOMPLETE` and `OPEN_ITEM_BLOCKING`.

## Corrected pack

The corrected pack exits 0 with `READY` and no configured findings.

## Human decision still required

READY means no configured gate tripped; it is not approval, not a tax or BAS agent service, not advice and not lodgment authority. A human remains accountable for professional judgement, approval, client impact and lodgment decisions.

## Primary sources and review date

Reviewed 2026-08-26.

- [Australian Taxation Office, Business activity statements](https://www.ato.gov.au/businesses-and-organisations/preparing-lodging-and-paying/business-activity-statements-bas)
- [Australian Taxation Office, Monthly GST reporting](https://www.ato.gov.au/businesses-and-organisations/gst-excise-and-indirect-taxes/gst/lodging-your-bas-or-annual-gst-return/options-for-reporting-and-paying-gst/monthly-gst-reporting)

## Product and fixture version

Product release `0.1.2`; fixture version `1`.

## Limitations and non-claims

This is a deterministic evaluation of configured gates against fabricated inputs. It is not approval, tax advice, lodgment authority or a conclusion that a pack is correct.
