#!/usr/bin/env bash
# Native binary smoke from an empty directory, with COMPONENT naming the source.
set -euo pipefail
[[ "$(review-ready --version)" = "review-ready 1.0.0" ]]

review-ready gate --profile bas --pack "${COMPONENT}/examples/bas-ready" --output pack
[[ -f pack/readiness-pack.json ]]
python -c "import json, os, sys; p = json.load(open(sys.argv[1]))['producer']; assert p['commit'] == os.environ['ACCOUNTING_REVIEW_SOURCE_COMMIT'], p" pack/readiness-pack.json
review-ready gate --profile bas --pack "${COMPONENT}/examples/bas-not-ready" --output pack-nr && nr=0 || nr=$?
[[ "$nr" = "2" ]]
review-ready gate --profile bas --pack "${COMPONENT}/examples/bas-blocked" --output pack-bl && bl=0 || bl=$?
[[ "$bl" = "2" ]]
python -c "import json; assert json.load(open('pack-bl/readiness-pack.json'))['overall_status']=='BLOCKED'"
review-ready view --pack-dir pack > ready-sheet.md
review-ready view --pack-dir pack-bl > blocked-sheet.md
review-ready compare --previous-pack-dir pack --current-pack-dir pack > comparison.json
review-ready gate --profile month_end --pack "${COMPONENT}/examples/month-end-ready" --output month-pack
review-ready gate --profile year_end --pack "${COMPONENT}/examples/year-end-ready" --output year-pack
review-ready intake --synthetic --source "${COMPONENT}/examples/document-intake/invoice.txt" --text "${COMPONENT}/examples/document-intake/invoice.txt" --text-origin manual --entity 'Cedar and Pine Consulting Pty Ltd' --period-end 2026-03-31 --output document
review-ready view-document --bundle document > document-sheet.md
