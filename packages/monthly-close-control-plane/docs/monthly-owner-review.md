# Prepare a monthly owner review

Use the existing close and forecast examples to prepare a dated review pack,
then add debtor evidence and a cash decision table. Keep unresolved findings
visible when a calculation or package check succeeds.

This recipe uses fabricated examples. For an authorised engagement, store
exports and outputs in the firm's approved location outside every version-control
checkout. Establish entity, currency, period, basis and filters before comparing
sources. A review pack does not approve a close or authorise collection messages.

## Run the maintained examples

Use current source checkouts and the locked environments described in
[utility workflows](utility-workflows.md). From the directory holding both
public repositories, run each route into a new output and environment directory:

```powershell
python accounting-review-pipeline/packages/monthly-close-control-plane/examples/utility_workflows.py --fpa au-fpa-pack --workflow close-forecast --output ../owner-close-results --environment-root ../owner-close-environments
python accounting-review-pipeline/packages/monthly-close-control-plane/examples/utility_workflows.py --fpa au-fpa-pack --workflow quarter --output ../owner-quarter-results --environment-root ../owner-quarter-environments
```

The first route shares source hashes between the Lumbridge close and forecast.
The second retains three close packs and two comparisons. Read each `summary.md`
and `manifest.json`, then inspect the accounting findings in the packs. A successful
fixture assertion confirms these supplied examples; it does not resolve their
`REVIEW` findings or establish a live client's completeness.

| Review question | Existing evidence | Acceptance check |
|---|---|---|
| Are the close and forecast using the same source? | Lumbridge handoff, close pack and forecast review | Source hashes agree; every expected command ran |
| What changed across the quarter? | Three close packs and two comparisons | Dates and opening balance are present; unresolved findings remain visible |
| Which debtors need investigation? | [Xero debtor review recipe](https://github.com/ryanduguid/australian-accounting-skills/tree/main/.claude/skills/xero-exports/references/debtor-review.md) | Signed credits, bucket differences, control differences and missing evidence are retained |
| What happens if receipts slip or payroll increases? | [Ridgeline decisions](https://github.com/ryanduguid/au-fpa-pack/tree/main/examples/ridgeline) | Baseline and changed assumptions use the same metric definitions and forecast horizon |

## Check the installed package separately

Run the documented [package smoke outside the checkout](../AGENTS.md#package-smoke-outside-the-checkout).
Build one wheel into a fresh directory, install it in a fresh environment, and
run `close-control` from outside the source checkout. The fabricated close
returns `REVIEW` with exit `2`; require the review JSON and verify the pack with
the installed viewer. Check the installed `drivers` command and `review
--balance-policy` option when assessing 0.1.8 capability claims.

Keep the wheel version, wheel hash, command exits and output checks beside the
example manifest. Source tests, an installed wheel, native Excel and hosted CI
are separate evidence. Record which ran; do not infer the others.

## Hand the review pack to its owner

Record the cut-off date, source manifest, completed checks, retained differences,
owner and next action for each exception. Compare a proposed cash decision with
the baseline and state whether any receipt moved outside the thirteen-week
horizon. The authorised reviewer decides what to accept and what to investigate.
