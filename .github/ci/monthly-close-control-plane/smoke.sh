#!/usr/bin/env bash
# Run the native commands against fabricated files from an empty directory.
set -euo pipefail
[[ "$(close-control --version)" = "close-control 1.0.0" ]]

review() {
  close-control "$1" --current "$COMPONENT/examples/current_trial_balance.csv" \
    --prior "$COMPONENT/examples/prior_trial_balance.csv" \
    --mapping "$COMPONENT/examples/account_mapping.csv" \
    --subledger "$COMPONENT/examples/subledger_balances.csv" \
    --absolute-threshold 10000 --percentage-threshold 0.10 \
    --review-note "$COMPONENT/examples/review_note.json" --output "$2" && code=0 || code=$?
  [[ "$code" = 2 ]]
}
review review pack
review workbench workbench
close-control view --pack-dir pack > review-sheet.md
close-control queue --pack-dir pack --pack-dir workbench --status REVIEW > queue.md
close-control compare --previous-pack pack --current-pack pack \
  --previous-tb "$COMPONENT/examples/current_trial_balance.csv" \
  --current-tb "$COMPONENT/examples/current_trial_balance.csv" > comparison.json
close-control reconcile --transactions "$COMPONENT/examples/clearing-june.csv" \
  --tenant Demo --account-id clearing --currency AUD --period-start 2026-06-01 \
  --period-end 2026-06-30 --opening-balance 0 --closing-balance 100 \
  --output clearing && code=0 || code=$?
[[ "$code" = 2 && -f clearing/review.html && -f clearing/carry-forward.json ]]
close-control drivers --pack-dir pack --transactions "$COMPONENT/examples/variance_transactions.csv" \
  --currency AUD --top 1 --output drivers && code=0 || code=$?
[[ "$code" = 2 && -f drivers/variance-drivers.csv ]]
close-control classify --original-transactions "$COMPONENT/examples/classification/transactions.csv" \
  --current-transactions "$COMPONENT/examples/classification/transactions.csv" \
  --coding-evidence "$COMPONENT/examples/classification/coding-evidence.json" \
  --currency AUD > classification.json && code=0 || code=$?
[[ "$code" = 2 ]]
for kind in expenses migration interentity; do
  close-control schedule --kind "$kind" --input "$COMPONENT/examples/schedules/$kind.json" > "$kind.json" && code=0 || code=$?
  [[ "$code" = 2 ]]
done
