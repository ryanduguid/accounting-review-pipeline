# Unreleased Rust tooling

- Replace `tools/xero_aged_receivables.py`, `tools/debtor_evidence.py` and
  `tools/replay_aged_review.py` with `xero-aged-receivables`, `debtor-evidence`
  and `replay-aged-review`; their Python import APIs are removed. Build with
  `cargo build --locked --release --bins` and use the same flags and subcommands.
  Install replay beside the verifier. Preserve signed four-place amounts,
  logical records, evidence bytes, statuses and the 12 fabricated replay cases.
  Native help, usage, malformed JSON, OS and replay failure diagnostics can
  differ; literal Unicode JSON output requires encodable scalar values.
  JSON parsing limits are 1,000 nested containers and 4,300 integer digits.
  New evidence files are created exclusively; a later write failure can leave
  partial evidence. The source archive remains version 0.1.8.
- Keep `tools/benchmark_aged_review.py` measuring the frozen Python reference.
  Schema 1's Python allocation tracing cannot measure the Rust commands.
- Replace the Python encoding guard with `check-vba-encoding`. Its source lookup
  follows the executable's installed or standard Cargo layout. Custom target
  layouts need explicit file arguments or an installed copy. Native path, OS
  and directory-listing diagnostics can differ.
- Replace `tools/xero_account_transactions.py` with `xero-account-transactions`;
  its Python import API is removed. Preserve exact amounts, source hashes,
  logical-row mappings, inspection JSON and clearing CSV. Native help, usage,
  JSON parser and OS diagnostics can differ; invalid amounts use JSON quotation,
  and text option values require valid Unicode scalar values. See the
  [migration commands](docs/account-transactions-import.md).
- Require native Windows/Linux checks and frozen Python comparisons alongside
  the retained source-archive release gates. No release is created by this port.

# v0.1.8

- The aged payables and receivables queries keep genuine `Total` contacts and signed amounts, require an explicit `HasSummaryFooter` role for an ambiguous terminal row, and refuse amount-bearing rows without a contact label (#304).
- `tools/debtor_evidence.py` drafts a source-only manifest, verifies an aged receivables summary in Decimal arithmetic and prepares evidence requests; a summary `PASS` leaves each debtor decision at `REVIEW`. Replay and benchmark tools come with fabricated cases, and every output must sit outside the repository checkout (#304).
- The README's aged review commands write to the temporary directory instead of the checkout's parent (#332).
- Development checks need Python 3.11 or newer (#318).

# v0.1.7

- Xero aged payables keep their source section labels on request, so supplier and expense-claim balances reconcile separately; the default output is unchanged (#163).
- The aged payables and receivables parsers refuse a record whose width differs from the header instead of reading a malformed CSV, and the aged summary queries no longer describe their output as verified (#246).
- `Xero.TrialBalance` buffers its file like the other three queries, so header measurement and parsing read the same bytes, and its final parse refuses an extra value.
- The workpaper header macro no longer stops with run-time error 13 when cell A3 already holds an error value.
- `tools/xero_account_transactions.py` inspects exported Xero account-transaction reports offline against their documented shapes, with a fabricated sample, and keeps row observations apart from clearing-item identity (#215).
- `Fx.AUFinancialYear` documents how to let a western or central state's local calendar decide the financial year.

# v0.1.6

- Require all 6 ageing bucket headers and retain fixed-decimal currency columns.
- Reject excess decimal scale and embedded amount errors in Payday imports.
- Limit fixture exceptions to the 6 reviewed files.

# v0.1.5

The `v0.1.1`, `v0.1.3` and `v0.1.4` tags are retained as unreleased failed-preflight tags. All 3 workflows stopped before any build or publication step (v0.1.3's release.yml pinned release-policy to a SHA orphaned by a history rewrite; v0.1.4's VERSION file was not bumped alongside RELEASE_NOTES.md), so none has a release or assets, and GitHub's tag-protection rule blocks deleting or moving any of them.

Changes since `v0.1.2`:

- add contract-checked Power Query parsers for Xero Aged Receivables and Aged Payables summary exports, including the documented MYOB Business scoping
- add the Payday Super close-input contract
- pin the accounting number format across VBA modules, size the workpaper header border from the used range instead of `A5:H5`, and make `ApplyWorkpaperHeader` idempotent so a second run is a no-op
- make the release archives cross-platform reproducible (UTC timestamps, LF text) and lint workflows inside verify, with the caller rollback documented
- document platform caveats, the test coverage map, the financial-year timezone note and plain-language setup
- refresh documentation so every claim matches the repository, including striking the bucket tie-out promise the queries never implemented and retiring the retired codename.

No client data or binary workbook is included. The CSV fixtures are fabricated.
