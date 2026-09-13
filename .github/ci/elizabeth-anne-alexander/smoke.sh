#!/usr/bin/env bash
# Wheel smoke for elizabeth-anne-alexander. ci-package.yml runs this from an
# empty directory with the installed wheel's console scripts on PATH and
# COMPONENT pointing at the checked-out component, so it proves the wheel
# carries its package data and the documented exit codes hold.
set -euo pipefail

elizabeth-anne-alexander evaluate \
  --context samples/contexts/sample-monthly-variance.context.json \
  --request samples/requests/sample-revenue-variance.request.json \
  --policy policy/demo-policy-v1.json \
  --out build/demo
elizabeth-anne-alexander validate-review \
  --evidence build/demo/reviewer-evidence.json \
  --receipt build/demo/receipt.json \
  --decision samples/decisions/sample-review-decision.json
test -f build/demo/receipt.json
