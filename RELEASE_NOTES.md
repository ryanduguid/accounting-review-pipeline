# v0.1.1

Changes since `v0.1.0`:

- bind close-pack provenance to the exact source bytes and report the physical CSV line for rejected records;
- accept canonical code-less accounts while retaining the account-identity controls;
- refuse unsafe source/output path collisions and keep surrogate characters out of published packs; and
- add workflow-built wheel and source distribution artefacts, SHA-256 checksums, an SPDX SBOM and GitHub build attestations.

## Unreleased: Phase B viewer

- add `close-control view --pack-dir <dir>`: a read-only display of an existing
  review pack that verifies the three artefacts agree before rendering anything
  (status, source digests, boundary statement, exception-by-exception CSV/JSON
  reconciliation) and prints the SHA-256 of each displayed file; verification
  failures exit 1 without displaying; and
- the viewer imports nothing beyond `csv`, `hashlib`, `io`, `json`, `re`,
  `decimal` and `pathlib`, enforced by tests that parse its AST.


The package remains a review-first local control aid. It does not post journals, approve close decisions or lock accounting periods.
