# Contributing to evatt

Run every check from this directory.

```bash
uv lock --check
uv run --locked --extra dev pytest
uv run --locked --extra dev ruff check evatt tests
uv run --locked --extra dev mypy evatt
uv run --locked --extra dev python -m build
```

## Fixture rule

**Never create a fixture by redacting a real document.** Every file under
`evatt/samples/` is written from nothing. Sanitising a real document with an
imperfect first pass is how real data reaches a public repository.

Sample entity values carry the marker `Sample `, except the reserved
placeholder person names. Sample email addresses use RFC 2606 reserved
domains. `tests/test_packaging.py` enforces both.

Check-digit test vectors are documented ATO and ASIC test identifiers. Most
live in `tests/test_patterns.py`, but `evatt/samples/identifiers.md` ships 4
of the same kind, because the sample sheet exists to show the labelled patterns
firing and an empty one would show nothing: TFN `123 456 782`, ACN
`123 456 780`, Medicare `2123 45670 1` and BSB `062-000`. None belongs to
anybody. The ABN and the phone number in that file are chosen the other way, an
ABN whose check digit fails and an ACMA fiction range mobile, and
`test_no_sample_ships_a_real_identifier` pins both.

`062-000` is the exception worth knowing about. No reserved or fictitious BSB
range is published, so there is no safe pair to use: a BSB is an allocated bank
branch code and any well-formed one names a real branch. `062-000` is a
Commonwealth Bank code. It stays because it identifies a branch and not a
person, an account or a balance, and because the alternative is guessing at an
unallocated pair and possibly landing on somebody else's branch. Never pair it
with an account number in a sample.

## Claim rule

`tests/test_assurance_claims.py` pins what the documentation may claim. If a
change to the README fails that test, the test is right. Pseudonymisation is a
risk reduction, not a compliance conclusion.

The retired list is the half that matters. It holds substrings rather than
whole sentences, so it catches the family of wordings, and nothing the
documented position needs is caught by any of them.

## Demo rule

Every command asks git whether it would let you commit the map, so a scratch
directory used to try the tool has to be a work tree with a rule covering the
map. `git init` and a one-line `.gitignore` are enough. The component workflow
does exactly that before it runs the clean-wheel demo.

## Prose rule

Australian English. No em dashes.
