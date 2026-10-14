# Python compatibility oracle

`xero-tb-differential` compares the native probe and installed command behaviour
with CPython 3.15 running the original exporter at
`1af8a855578afaec97bd2e56836a8aa8fb15e729`. The commands use fabricated data
and remove Xero credentials from each oracle process.

From this component directory:

```bash
baseline="$(mktemp -d)"
git archive 1af8a855578afaec97bd2e56836a8aa8fb15e729:packages/xero-trial-balance-export | tar -xf - -C "$baseline"
python3.15 -m venv "$baseline/venv"
"$baseline/venv/bin/python" -m pip install --require-hashes -r tests/differential/requirements.lock
cargo build --locked --features probe
target/debug/xero-tb-differential "$baseline/venv/bin/python" "$baseline" target/debug/xero-tb-probe 1 10000
```

On Windows use `venv/Scripts/python.exe` and the `.exe` binaries. CI runs this
comparison on Linux and Windows. The corpus includes captured report inputs,
generated malformed responses, decimal boundaries, Unicode, callback URLs,
byte-for-byte CSV and manifest output, and command arguments at four terminal
widths. It also verifies that Python and Rust exclude each other from the same
fabricated token-cache lock and release it successfully. A mismatch or process
deadline exits non-zero.

The comparison accounts for the documented migration changes: recovery guidance
uses native command names, token-file help names the executable-adjacent legacy
cache, and an unpaired Unicode surrogate in CSV output produces the native UTF-8
refusal. It does not normalise other value, output or exit-code differences.
Python remains a test oracle only.
