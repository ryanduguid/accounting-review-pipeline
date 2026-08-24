# v0.1.2

Changes since `v0.1.0`:

- bind close-pack provenance to the exact source bytes and report the physical CSV line for rejected records;
- accept canonical code-less accounts while retaining the account-identity controls;
- refuse unsafe source/output path collisions and keep surrogate characters out of published packs;
- accept a UTF-8 BOM in review-note JSON;
- neutralise every spreadsheet formula prefix (`=`, `+`, `-`, `@`) in CSV output cells;
- add the local close workbench command and a read-only pack viewer that verifies status, source digests, boundary statement and exception reconciliation agree across the three artefacts before rendering anything, printing the SHA-256 of each displayed file; verification failures exit 1 without displaying;
- quarantine the legacy `openaccountants-au` entry point: it prints a redirect on stderr and exits 2, and `llms.txt` documents it as quarantined rather than working;
- adopt the shared release-policy workflow and publish the attested distribution to PyPI via trusted publishing; and
- add editorconfig, CODEOWNERS, mailmap, job timeouts, Dependabot pacing, `llms.txt`, project URLs and a DISCLAIMER, with documentation corrected so every claim matches the repository.

## Unreleased: Phase B viewer

The read-only viewer described above ships in this release. Its standard-library-only import surface remains enforced by tests that parse its AST.

## Review-first policy

Exceptions remain visible for a human reviewer. An acknowledgement records that review occurred; it does not approve a close or change a result to `PASS`. The package posts no journals, makes no payments, lodges no returns and locks no period.
