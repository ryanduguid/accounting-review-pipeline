# v0.1.1

Changes since `v0.1.0`:

- bind close-pack provenance to the exact source bytes and report the physical CSV line for rejected records;
- accept canonical code-less accounts while retaining the account-identity controls;
- refuse unsafe source/output path collisions and keep surrogate characters out of published packs; and
- add workflow-built wheel and source distribution artefacts, SHA-256 checksums, an SPDX SBOM and GitHub build attestations.

The package remains a review-first local control aid. It does not post journals, approve close decisions or lock accounting periods.
