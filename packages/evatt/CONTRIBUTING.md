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

Check-digit test vectors are documented ATO and ASIC test identifiers. They
live in `tests/test_patterns.py` only and are not shipped under `samples/`.

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
