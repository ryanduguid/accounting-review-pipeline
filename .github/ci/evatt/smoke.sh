#!/usr/bin/env bash
# Standalone binary smoke, run from an empty directory with the release binary
# on PATH and COMPONENT pointing at this component's fabricated source samples.
set -euo pipefail

# Every command asks git whether it would let you commit the map.
git init --quiet .
printf 'entities.json\n*.triage.md\n*.disclosure.json\n' > .gitignore
samples="${COMPONENT:?COMPONENT must name the checkout}/evatt/samples"
cp "${samples}/entities.sample.json" entities.json
cp "${samples}/entities-only.md" in.md
cp "${samples}/identifiers.md" identifiers.md
cp "${samples}/unmapped-name.md" halt.md
# A redact, verify and restore round trip, byte for byte, with a
# manifest beside the redacted output.
evatt redact --in in.md --map entities.json --out build/out.md
evatt verify --in build/out.md --map entities.json
evatt restore --in build/out.md --map entities.json --out build/back.md
cmp in.md build/back.md
[[ -f build/out.md.manifest.json ]]
cat build/out.md.manifest.json
# Local evidence matches only the exact bytes and supplied context. No sender
# is invoked, and a matching record does not authenticate disclosure permission.
evatt disclosure-record --in build/out.md --map entities.json \
  --destination model:sample-tenant:sample-project --decision-ref sample-decision:42 \
  --out build/out.disclosure.json
evatt disclosure-check --in build/out.md --map entities.json \
  --destination model:sample-tenant:sample-project --decision-ref sample-decision:42 \
  --record build/out.disclosure.json
evatt disclosure-check --in build/out.md --map entities.json \
  --destination model:sample-other --decision-ref sample-decision:42 \
  --record build/out.disclosure.json && dc=0 || dc=$?
[[ "$dc" = "2" ]]
cp build/out.md build/changed.md
printf '\n' >> build/changed.md
evatt disclosure-check --in build/changed.md --map entities.json \
  --destination model:sample-tenant:sample-project --decision-ref sample-decision:42 \
  --record build/out.disclosure.json && bc=0 || bc=$?
[[ "$bc" = "2" ]]
# A file that uses CRLF throughout comes back with CRLF intact.
awk '{ printf "%s\r\n", $0 }' in.md > crlf.md
evatt redact --in crlf.md --map entities.json --out build/crlf.md
evatt restore --in build/crlf.md --map entities.json --out build/crlf-back.md
cmp crlf.md build/crlf-back.md
# verify exits 2 on a file that is not ready. Its output is
# captured to check the diagnostic. Findings identify kinds and
# line numbers without printing matched values.
evatt verify --in identifiers.md --map entities.json > verify.log 2>&1 && vf=0 || vf=$?
[[ "$vf" = "2" ]]
grep -q "not ready to send" verify.log
# A halt exits 2, writes a triage file, and clears any stale output
# already sitting at that path. The clean redact above it is what
# puts a stale output there to be cleared.
evatt redact --in in.md --map entities.json --out build/halt.md
[[ -f build/halt.md ]]
evatt redact --in halt.md --map entities.json --out build/halt.md && ht=0 || ht=$?
[[ "$ht" = "2" ]]
[[ ! -f build/halt.md ]]
[[ ! -f build/halt.md.manifest.json ]]
[[ -f build/halt.md.triage.md ]]
# --out refuses to be the map or the input, and refuses through the
# 2 paths it derives as well. A map named map.triage.md is
# covered by the same ignore rule as a triage file, and --out map
# derives that exact path.
before=$(cksum < entities.json)
cp entities.json map.triage.md
evatt redact --in in.md --map entities.json --out entities.json && mp=0 || mp=$?
evatt redact --in in.md --map entities.json --out in.md && ip=0 || ip=$?
evatt redact --in in.md --map map.triage.md --out map && dv=0 || dv=$?
[[ "$mp" = "1" ]]
[[ "$ip" = "1" ]]
[[ "$dv" = "1" ]]
[[ "$before" = "$(cksum < entities.json)" ]]
# Nothing the demo produced left the map committable.
git add -A
[[ -z "$(git ls-files entities.json)" ]]
[[ -z "$(git ls-files build/out.disclosure.json)" ]]
