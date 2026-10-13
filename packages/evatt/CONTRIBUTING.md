# Contributing to evatt

Run every check from this directory.

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
cargo build --locked --release
```

Rust 1.99.0 and Cargo are pinned in `rust-toolchain.toml`. Keep functions within
50 lines, enforced by Clippy. The default build produces only the `evatt`
binary; `evatt-probe` requires the test-only `probe` feature.

The compatibility reference is CPython 3.14 with Unicode 16.0.0. Python is
needed only to regenerate the checked-in tables and run differential tools:

```bash
python3.14 tools/gen_unicode_tables.py --test
python3.14 tools/gen_unicode_tables.py > /tmp/evatt-tables.rs
cmp src/tables.rs /tmp/evatt-tables.rs
python3.14 tools/gen_patterns.py > /tmp/evatt-regexes.rs
cmp src/regexes.rs /tmp/evatt-regexes.rs
python3.14 tools/gen_unicode_cases.py > /tmp/evatt-unicode.json
cmp tests/fixtures/unicode-16.json /tmp/evatt-unicode.json
cargo build --locked --features probe
python3.14 tests/differential/detectors.py --python-root /path/to/pristine-0.2.0 \
  --probe target/debug/evatt-probe --seed 1 --cases 2000
python3.14 tests/differential/cli.py --python-root /path/to/pristine-0.2.0 \
  --binary target/debug/evatt --seed 1 --cases 300
python3.14 tests/differential/arguments.py --python-root /path/to/pristine-0.2.0 \
  --binary target/debug/evatt --seed 1 --cases 1000
```

Repeat all three differentials with seed 2. The harness uses fabricated temporary
worktrees and compares file bytes as well as output and exit status. It never
supplies client data. Use a pristine copy of the repository snapshot's Python
component as the oracle; the Rust program never invokes it.

## Fixture rule

**Never create a fixture by redacting a real document.** Every file under
`evatt/samples/` is written from nothing. Sanitising a real document with an
imperfect first pass is how real data reaches a public repository.

Sample entity values carry the marker `Sample `, except the reserved
placeholder person names. Sample email addresses use RFC 2606 reserved
domains. `tests/docs.rs` enforces both.

Check-digit test vectors are documented ATO and ASIC test identifiers. Most
live in `tests/fixtures/python-0.2.0.json`, but `evatt/samples/identifiers.md` ships 4
of the same kind, because the sample sheet exists to show the labelled patterns
firing and an empty one would show nothing: TFN `123 456 782`, ACN
`123 456 780`, Medicare `2123 45670 1` and BSB `062-000`. None belongs to
anybody. The ABN and the phone number in that file are chosen the other way, an
ABN whose check digit fails and an ACMA fiction range mobile, and
`tests/docs.rs` pins both.

`062-000` is the exception worth knowing about. No reserved or fictitious BSB
range is published, so there is no safe pair to use: a BSB is an allocated bank
branch code and any well-formed one names a real branch. `062-000` is a
Commonwealth Bank code. It stays because it identifies a branch and not a
person, an account or a balance, and because the alternative is guessing at an
unallocated pair and possibly landing on somebody else's branch. Never pair it
with an account number in a sample.

## Claim rule

`tests/docs.rs` pins what the documentation may claim. If a
change to the README fails that test, the test is right. Pseudonymisation is a
risk reduction, not a compliance conclusion.

The retired list is the half that matters. It holds substrings rather than
whole sentences, so it catches the family of wordings, and nothing the
documented position needs is caught by any of them.

## Demo rule

Every command asks git whether it would let you commit the map, so a scratch
directory used to try the tool has to be a work tree with rules covering the
map, triage files and any disclosure records. Initialise Git and add the rules
listed in the README. The component workflow
does exactly that before it smokes the release binary.

## Prose rule

Australian English. No em dashes.
