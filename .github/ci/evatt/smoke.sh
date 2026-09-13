#!/usr/bin/env bash
# Wheel smoke for evatt. ci-package.yml runs this from an empty directory with
# the installed wheel's console scripts on PATH and COMPONENT pointing at the
# checked-out component, so it proves the wheel carries its package data and
# the documented exit codes hold.
set -euo pipefail

# samples/ ships as package data, so the demo inputs are read out
# of the installed wheel rather than the checkout: a wheel that
# drops them produces a console script whose demo cannot run.
# Every evatt command asks git whether it would let you commit the
# map, so the demo directory has to be a work tree with a covering
# ignore rule.
git init --quiet .
printf 'entities.json\n*.triage.md\n' > .gitignore
samples=$(python -c "import evatt, pathlib; print(pathlib.Path(evatt.__file__).parent / 'samples')")
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
test -f build/out.md.manifest.json
cat build/out.md.manifest.json
# A file that uses CRLF throughout comes back with CRLF intact.
awk '{ printf "%s\r\n", $0 }' in.md > crlf.md
evatt redact --in crlf.md --map entities.json --out build/crlf.md
evatt restore --in build/crlf.md --map entities.json --out build/crlf-back.md
cmp crlf.md build/crlf-back.md
# verify exits 2 on a file that is not ready. Its output is
# captured to check the diagnostic. Findings identify kinds and
# line numbers without printing matched values.
evatt verify --in identifiers.md --map entities.json > verify.log 2>&1 && vf=0 || vf=$?
test "$vf" = "2"
grep -q "not ready to send" verify.log
# A halt exits 2, writes a triage file, and clears any stale output
# already sitting at that path. The clean redact above it is what
# puts a stale output there to be cleared.
evatt redact --in in.md --map entities.json --out build/halt.md
test -f build/halt.md
evatt redact --in halt.md --map entities.json --out build/halt.md && ht=0 || ht=$?
test "$ht" = "2"
test ! -f build/halt.md
test ! -f build/halt.md.manifest.json
test -f build/halt.md.triage.md
# --out refuses to be the map or the input, and refuses through the
# 2 paths it derives as well. A map named map.triage.md is
# covered by the same ignore rule as a triage file, and --out map
# derives that exact path.
before=$(cksum < entities.json)
cp entities.json map.triage.md
evatt redact --in in.md --map entities.json --out entities.json && mp=0 || mp=$?
evatt redact --in in.md --map entities.json --out in.md && ip=0 || ip=$?
evatt redact --in in.md --map map.triage.md --out map && dv=0 || dv=$?
test "$mp" = "1"
test "$ip" = "1"
test "$dv" = "1"
test "$before" = "$(cksum < entities.json)"
# Nothing the demo produced left the map committable.
git add -A
test -z "$(git ls-files entities.json)"
