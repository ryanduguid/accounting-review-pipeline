# Contributing

Power Query functions and VBA modules for Australian month-end work. Everything lives as text so a reviewer can read your change in a diff. Ship the source as text, and use a workbook only to demonstrate it.

## Data boundary

Keep client data out. The `.gitignore` blocks `input/`, `output/`, `clients/`, `exports/` and the common export extensions, including `.aba`, `.myox`, `.ofx` and `.qif`. Fabricate any sample you need and keep it under `samples/`.

## Format rules

- Export VBA as text under `vba/`. The VBE expects CRLF on import, so `.gitattributes` marks `.bas`, `.cls` and `.frm` as `-text whitespace=cr-at-eol`, which keeps git from touching their line endings and stops a trailing CR being reported as a whitespace error. That means `git diff --check` will not flag a renormalised blob. `tools/check_vba_encoding.py` is the check that does, and CI runs it.
- Keep Power Query M under `powerquery/` as plain text.
- Do not commit `.xlsm` or `.xlsx` as the source of truth for a function or module.

## Traps already found here

- `Csv.Document` infers width from the first row, so a one-field title row collapses the parse. Pass `Columns`, and put a caveat in a full-width trailing row.
- Typed M parameters such as `(x as date)` reject the shapes Excel hands over. Widen the type and use `Date.From(d, "en-AU")` so financial-year results stop following the machine locale.
- CSV formula injection: guard `=` in every case, and `+`, `-`, `@` only when the remainder is not a plain code. Otherwise account codes like `-00123` stop joining back to payroll. An A1-style reference such as `+A1` starts a formula, so an "inert remainder" test has to be narrower than "anything alphanumeric".
- `IsNumeric(True)` returns `True` in VBA, so Booleans sum as -1. `Activate` on a hidden sheet activates the visible neighbour. `Scripting.Dictionary` runs on Windows Excel only.
- `Trim$` misses non-breaking spaces, tabs and CRLF in reconciliation keys.

## Local verification

Python 3.10 or newer drives the test suite.

```bash
python -B -m unittest discover -s tests -v
```

On Windows with Windows PowerShell 5.1 or newer and desktop Excel installed,
run the checked-in acceptance checks in Excel itself:

```powershell
powershell -NoProfile -File tools/native_excel_acceptance.ps1
```

Use `-RepositoryRoot D:\src\accounting-excel-toolkit` to test a different
checkout. The runner requires Excel's Power Query engine and the
`Microsoft.Mashup.OleDb.1` provider. It exercises exactly 72 real-engine cases:
both fabricated Xero layouts, the fabricated Payday Super producer contract,
period and YTD selection, malformed exports, lazy evaluation, AU financial-year
boundaries, ABN validation and header promotion. That is every function except
the two aged summary parsers: `Xero.AgedReceivables` and `Xero.AgedPayables`
are loaded into the workbook with every other `.pq` file, but no check calls
either one, so neither is exercised in the engine yet.

The default `All` mode runs the 46 core checks and 26 Payday Super checks in
fresh child PowerShell and Excel processes. Within the Payday child, each of
20 queries reads only one of 19 fabricated files. This keeps Excel's
cross-source privacy/firewall composition state from masking adapter behaviour
while preserving the exact 72-check result contract. One check preserves a
quoted multiline field; three others materialise fabricated reports containing
500, 5,000 and 10,000 contribution
rows and print their measured refresh times.

The runner does not write to repository sources or samples. It creates its
workbook and adverse fixtures in a GUID-named system temporary directory,
closes the workbook, quits and releases every Excel COM reference in reverse
order, and removes the directory in its `finally` block.

VBA is not imported by automation because doing so depends on Excel's
machine-wide **Trust access to the VBA project object model** policy. Test VBA
in a disposable workbook by importing both `.bas` files, running
`ApplyWorkpaperHeader` against an entity name beginning with `=`, and running
`CompareKeyedRanges` against fabricated two-column ranges with a leading-zero
key. Confirm the header is text rather than a formula, existing rows move down,
and `Recon Result` carries the expected difference. Do not weaken the Office
trust policy solely to run this check.

## Pull requests

Say which function or module you changed and show the fixture that exercises it. For an M change, state the locale and column shape you tested against.

For a potential security vulnerability, follow [SECURITY.md](SECURITY.md) rather than opening an issue.
