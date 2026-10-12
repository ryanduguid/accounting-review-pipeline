#!/usr/bin/env bash
# Release-binary smoke for elizabeth-anne-alexander. ci.yml's rust job runs this
# from an empty directory with the release binary on PATH, so it proves the
# binary carries its bundled data and the documented exit codes hold.
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
