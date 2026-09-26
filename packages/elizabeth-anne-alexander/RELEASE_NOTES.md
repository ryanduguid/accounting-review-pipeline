# Unreleased

- Amount strings now carry at least 2 decimal places. A whole-dollar source emitted a
  `delta` of `"200"`, which equals account code `"200"` and raised a disclosure error on a
  correct pack. Values are padded, never rounded, and `finding_id` is unchanged.

- Document the actual exit codes in the README: `0` for `DECISION_RECORDED` and
  `PARTIAL_DECISION_RECORDED`, `2` for every `GatewayError` including malformed input.
- Known item, for the owner to decide: those codes diverge from the repository convention of
  `1` for malformed input and `2` for a non-passing status, and this command has no `1`, so a
  parse failure and a refusal are indistinguishable by exit code. The codes are unchanged.

- `validate` replaces an existing output when the optional `model-result.json` is absent; the protected-input check now skips an input that does not exist instead of refusing the write.

# v0.2.4

- `validate` writes its output on Windows: where Python has no `dir_fd` support, a path-based fallback with the same protected-input check runs instead of failing every run as `blocked:`.
- Refuse a validation output that names a review input.
- Sum trial-balance totals exactly or refuse the file, and set the decimal context at the gate entry points.
- A `windows-latest` CI job runs the suite alongside the Linux jobs.
- Close the documentation findings from the September fact check.

# v0.2.3

- Include the data-flow, disclaimer and release notes in the source distribution so its shipped documentation tests can run.

- Reject re-sealed evidence with invalid account identity, non-finite monetary values or incorrect source references.
- Keep repository-policy packaging checks in the source checkout while preserving installed-wheel checks.

# v0.2.2

First maintained release from `packages/elizabeth-anne-alexander` in the
Accounting Review Pipeline. Project, citation, security and workflow links
now identify the canonical monorepo, and the namespaced release uses the
shared attested Python policy. The no-network claim applies to this package,
not to the monorepo's separate exporter. Runtime behaviour, persisted schema
identifiers, source contracts and fabricated fixtures are unchanged from v0.2.1.

# v0.2.1

The `v0.2.0` tag is retained as an unreleased failed-preflight tag: its release.yml pinned release-policy to a SHA orphaned by a history rewrite, the workflow stopped before publishing, and GitHub's tag-protection rule blocks deleting or moving it.

Changes since `v0.1.1`:

- rename the Python distribution and command to `elizabeth-anne-alexander`, and the import package to `elizabeth_anne_alexander`
- make the identity change a clean break, without aliases for the former distribution, command or import package
- retain the exact persisted schemas and source contracts, including `xero-source-manifest.v1` and the `xero-trial-balance-export` source-system value.

The review policy and safety boundary are unchanged. The package remains a synthetic, local review-boundary demonstration. It has no Xero mutation adapter and does not approve, post, pay, lodge or lock anything.

# v0.1.1

Changes since `v0.1.0`:

- bind review provenance and run identifiers to the exact trial-balance source bytes
- align canonical account identity and accepted timestamp contracts across supported Python versions
- refuse extreme magnitudes safely and keep truncated human-review decisions incomplete
- add workflow-built wheel and source distribution artefacts, SHA-256 checksums, an SPDX SBOM and GitHub build attestations.

The package remains a synthetic, local review-boundary demonstration. It has no Xero mutation adapter and does not approve, post, pay, lodge or lock anything.
