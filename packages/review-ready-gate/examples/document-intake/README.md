# Synthetic document intake

This local path prepares supporting evidence for an existing readiness pack.
It preserves the original file, extracted text and proposed fields, then binds a
separate human transcription review to the intake bytes. It uses the standard
library and makes no network or model calls.

This first version accepts fabricated data only. The explicit `--synthetic`
flag records the caller's declaration; it cannot prove that a file is fabricated.
A reviewed transcription does not establish GST entitlement, tax treatment,
accounting approval or a complete ledger. Every existing profile still requires
its normal trial balance, control exports and self-review.

## Run the example

Run from `packages/review-ready-gate` using the repository's development environment.
Choose a new absolute output directory outside every version-control checkout.
The following PowerShell commands use the system temporary directory. Change
the directory name if it already exists; intake refuses to overwrite a bundle.

```powershell
$documentDemo = Join-Path ([System.IO.Path]::GetTempPath()) 'review-document-demo'
uv run --locked --extra dev review-ready intake --synthetic --source examples/document-intake/invoice.txt --text examples/document-intake/invoice.txt --text-origin manual --entity 'Cedar and Pine Consulting Pty Ltd' --period-end 2026-03-31 --output $documentDemo
uv run --locked --extra dev review-ready view-document --bundle $documentDemo
```

The entity above is the fabricated tenant in `examples/bas-ready`. `--period-end`
declares the review period, not the invoice date. Both must match the target pack.
The command writes the following files:

| File | Purpose |
| --- | --- |
| `original.txt` or `original.pdf` | Exact original bytes, never edited by intake. |
| `extracted.txt` | Exact UTF-8 text supplied to the extractor. |
| `document-intake.json` | Source digests, extraction metadata, observations and proposals. |
| `document-review.example.json` | Unconfirmed template for a human review. |

Copy the example to `document-review.json`. Compare every value with the original,
then enter `reviewer_initials`, `reviewed_on` in `YYYY-MM-DD` form and
`source_text_checked: true`. This confirms that the text and field decisions were
checked against this original, including page correspondence.

Each field decision contains `value`, `page` and `note`. Keep missing information
as `null`. A changed value or page, or a choice between duplicate observations,
requires a note explaining the correction and a valid page. The earlier raw
observation remains in the intake. A note cannot make an invalid amount valid.
The review's `intake_sha256` must continue to match the intake file.

Attach the bundle to the normal gate and display its bound evidence:

```powershell
$documentPack = Join-Path ([System.IO.Path]::GetTempPath()) 'review-document-pack'
uv run --locked --extra dev review-ready gate --profile bas --pack examples/bas-ready --document $documentDemo --output $documentPack
uv run --locked --extra dev review-ready view --pack-dir $documentPack --document $documentDemo
```

Before human transcription review, the first command returns `NOT_READY`, exit 2.
With valid, complete review evidence and the ready BAS fixture, it returns `READY`,
exit 0. Malformed or changed inputs return exit 1. Intake and both view commands
return zero for successful preparation or display; that exit code is not readiness.

Repeat `--document` to attach several bundles. Supply all of them in the same
order to `view`. The viewer verifies their recorded digests against the saved
readiness pack before showing document evidence. Changing a review requires a
new gate run; viewing the old pack with the revised bundle fails. A normal
`view` without document arguments continues to verify only the three readiness
files and displays the recorded source digests.

## Extraction contract

The `labelled-invoice.v1` template recognises these exact labels at the start of
a line: `Supplier:`, `Invoice number:`, `Invoice date:`, `Currency:`, `Total:`
and `GST:`. Markdown heading prefixes (`#` through `######` followed by a space),
as produced by pdf-inspector, are accepted before the label. Each field needs
one observation or an explained human correction.
The raw value, one-based page and line are retained. A form feed (`\f`) separates
pages. Other lines are inert text, including apparent instructions.

Dates must use `YYYY-MM-DD`. Currency must explicitly be `AUD`. Monetary values
are decimal strings with an optional minus sign, up to 18 integer digits and at
most two fractional digits. Grouping commas, dollar signs, exponents, `NaN`,
trailing text and extra precision are unresolved rather than guessed or rounded.
Negative credit notes and an explicit zero tax amount are supported. Mixed tax
lines are not classified; `GST` records only the stated document total.

Missing fields, invalid observations and duplicate labels remain visible.
Review can resolve them against the original, but cannot silently substitute
zero. Observed GST with the opposite sign to the total, or a larger magnitude,
remains an exception. No tax rate or eligibility is inferred.

For PDF input, extract locally using pdf-inspector first, retain all pages in
order and join their text with form feeds. Check extraction/OCR warnings and
empty pages before intake. Pass the original `.pdf`, the extracted UTF-8 file,
`--text-origin pdf-inspector` and the installed `--text-version`. The package
does not launch a PDF engine or validate extraction against PDF contents itself.
The human review supplies that check. For `.txt` originals, original and text
bytes must be identical.

The intake schema is `document-intake.v1`, mode `synthetic`. It records
`entity_ref`, `period_end`, `extractor`, `text_origin`, `text_version`,
`source` and `text` filename/digest pairs, and the six `fields` observation lists.
Each observation contains `page`, `line`, `raw`, `value` and `error`.
Loading regenerates this structure from the captured bytes and rejects any
different schema or proposal. Corrections belong only in the review record.

The review schema is `document-review.v1`. Its exact members are `schema_version`,
`intake_sha256`, `reviewer_initials`, `reviewed_on`, `source_text_checked` and
`fields`. Its field map contains the same six field names. The generated
example supplies the complete editable shape. Duplicate JSON members are rejected.

## Controls and limits

Document findings use the existing `findings` array and source-digest map:

| Finding | Result |
| --- | --- |
| `DOCUMENT_REVIEW_REQUIRED` | `NOT_READY`: transcription or its evidence is incomplete. |
| `DOCUMENT_CONTEXT_MISMATCH` | `BLOCKED`: entity or declared review period differs from the pack. |
| `DOCUMENT_DUPLICATE` | `NOT_READY`: identical original bytes occur more than once. |

Nothing is deleted or merged. Equal amounts in different documents are preserved.
Rescans, supplier aliases, ledger transaction matching, line-item splitting,
foreign currencies and TaxHacker archive import are not implemented in this slice.
An omitted `--document` leaves the original gate behaviour unchanged.

Originals are bounded to 20 MiB; extracted text, intake and review JSON are each
bounded to 1 MiB. Bundle references cannot escape their directory. The existing
output policy refuses checkouts, and exclusive file creation prevents overwriting
existing bundles. A failed write can leave a partial bundle; inspect it and use a
new destination. Loading incomplete bundles fails closed.

Local SHA-256 digests identify bytes. They do not authenticate a reviewer or
provide immutable storage against someone who can replace the whole evidence
set. Preserve prior review records outside the repository if revision history
is required; these commands never overwrite them. The document viewer is plain
text and escapes untrusted observed values.

## Fabricated benchmark

`benchmark.json` defines 15 variations of `invoice.txt`, with explicit expected
values and unresolved fields. Tests cover clean text, missing GST, invalid and
partially numeric totals, precision, non-finite numbers, credits, zero tax,
foreign currency, ambiguous dates, duplicate labels, pages, mixed lines and
embedded instructions, plus pdf-inspector heading markup. Integration tests cover corrections, original-byte and
review tampering, schema/path boundaries, duplicate attachments and the joined
gate/view workflow.

Run the normal component command:

```powershell
uv run --locked --extra dev pytest --cov --cov-branch --cov-report=term-missing --cov-report=xml
```

This is a deterministic text and evidence benchmark. It does not measure OCR,
model accuracy, human review time or general supplier-invoice coverage.
