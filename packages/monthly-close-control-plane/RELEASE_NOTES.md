# v0.1.5

- Compare the acknowledgement effect against the writer's fixed sentence, so a
  rewritten or blank statement is refused rather than returned as agreed.
- Hold pack items to the writer's exact member set.
- Sum the four integrity totals with `Inexact` trapped. A file whose amounts
  would round is refused as malformed input with exit 1 instead of compared
  inexactly, so a $1.0000000000000000000000000001 debit no longer reads as
  balanced against a $1.00 credit.
- Read optional calculation evidence from disk and decide whether the close may
  rely on it. The package still performs no calculation, holds no credentials
  and contacts nothing; the evidence tests run with sockets blocked.
- Check supplied account mappings against explicit section and reporting-group
  pairs, retaining the review status and the original values.
- Record the controls not run for mapping, for subledger when rows are
  supplied, and for calculation evidence.
- Add close comparison, equity movements and evidence schedules, with a
  repository-owned driver for the close, forecast and provenance workflows.
- Rebuild the viewer's Scope, Exceptions and Human acknowledgement sections
  from the JSON, and settle a reconciliation suggestion's group label once so
  `reconciliation.json`, `review.html` and `suggestions.csv` name the same
  group.

# v0.1.4

- Include the disclaimer and documented examples in the source distribution.
- Preserve BLOCKED fixture results and reject unreviewed local workflow actions.
- Validate exact release triggers through the strict YAML loader.

# v0.1.3

First namespaced release from the maintained
`packages/monthly-close-control-plane` source in the Accounting Review
Pipeline. Project and component links now identify the canonical monorepo,
release guidance uses the protected component tag, and the fabricated local
close loop resolves the co-located Elizabeth Anne Alexander package. The
control engine, status and exit-code contracts, and fabricated fixtures are
unchanged from v0.1.2.

# v0.1.2

Changes since `v0.1.0`:

- bind close-pack provenance to the exact source bytes and report the physical CSV line for rejected records
- accept canonical code-less accounts while retaining the account-identity controls
- refuse unsafe source/output path collisions and keep surrogate characters out of published packs
- accept a UTF-8 BOM in review-note JSON
- neutralise every spreadsheet formula prefix (`=`, `+`, `-`, `@`) in CSV output cells
- add the local close workbench command and a read-only pack viewer that verifies status, source digests, boundary statement and exception reconciliation agree across the 3 artefacts before rendering anything, printing the SHA-256 of each displayed file; verification failures exit 1 without displaying
- quarantine the legacy `openaccountants-au` entry point: it prints a redirect on stderr and exits 2, and `llms.txt` documents it as quarantined rather than working
- adopt the shared release-policy workflow and publish the attested distribution to PyPI via trusted publishing
- add editorconfig, CODEOWNERS, mailmap, job timeouts, Dependabot pacing, `llms.txt`, project URLs and a DISCLAIMER, with documentation corrected so every claim matches the repository.

## Unreleased: Phase B viewer

The read-only viewer described above ships in this release. Its standard-library-only import surface remains enforced by tests that parse its AST.

## Review-first policy

Exceptions remain visible for a human reviewer. An acknowledgement records that review occurred; it does not approve a close or change a result to `PASS`. The package posts no journals, makes no payments, lodges no returns and locks no period.
