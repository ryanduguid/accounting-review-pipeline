#!/usr/bin/env bash
# Wheel smoke for review-ready-gate. ci-package.yml runs this from an empty
# directory with the installed wheel's console scripts on PATH and COMPONENT
# pointing at the checked-out component, so it proves the wheel carries its
# package data and the documented exit codes hold.
set -euo pipefail

review-ready gate --profile bas --pack "${COMPONENT}/examples/bas-ready" --output pack
test -f pack/readiness-pack.json
review-ready gate --profile bas --pack "${COMPONENT}/examples/bas-not-ready" --output pack-nr && nr=0 || nr=$?
test "$nr" = "2"
review-ready gate --profile bas --pack "${COMPONENT}/examples/bas-blocked" --output pack-bl && bl=0 || bl=$?
test "$bl" = "2"
python -c "import json; assert json.load(open('pack-bl/readiness-pack.json'))['overall_status']=='BLOCKED'"
