# v1.0.0 (unreleased)

Breaking changes:

- The gate is a standalone Rust binary. The `elizabeth_anne_alexander` Python import is removed; call the `elizabeth-anne-alexander` command, whose subcommands and flags are unchanged.
- There is no release workflow for 1.0.0 yet, so it cannot be installed with `pip`. Build it with `cargo build --release`. PyPI keeps 0.2.6.

Unchanged: exit codes, stdout and stderr lines, output file bytes, and the `run_id`, `finding_id` and receipt digests, so packs written by 0.2.6 still validate. The receipt's `code_version` is now `1.0.0`. `tests/differential/differential.py` runs the 0.2.6 package and the binary on the same generated inputs; against Python 3.15 it found no differences in 1,412 cases on Linux and 1,412 on Windows, and the decimal module matched Python 3.11, 3.13, 3.14 and 3.15 on more than 1.2 million operations.

Differences from 0.2.6:

- The policy and sample files are built into the binary. Messages that printed the installed package directory now print `<bundled>`, for example `context must stay within <bundled>/samples.` A relative path that climbs above the bundled data is reported as missing, and an absolute path can no longer name a bundled file.
- Where Python versions disagreed, the binary follows Python 3.15: command-line errors suggest a close subcommand ("maybe you meant"), a `24:00` time is accepted as the next midnight, output is UTF-8 on Windows, and Unicode rules come from Python 3.15 (Unicode 17).
- Where 0.2.6 stopped with a Python traceback (exit `1`), the binary exits `1` with one `internal error:` line: a JSON integer over 4,300 digits, JSON nested more than 10,000 levels deep, and a zero amount whose fixed-point form would need over 1 GiB of digits.
- Operating-system error text inside a refusal message (the part after the path) can be worded differently, particularly on Windows.


# v0.2.6

- `write_evaluation` stages each run's JSON files in its own temporary directory under the validated output directory. It used three fixed `.partial` paths, so one run could overwrite or remove unrelated files, or another run's staging files, at those paths. Cleanup touches only that run's directory. Output names, JSON formatting, error mapping and receipt validation are unchanged.
- The README says what Xero's developer terms allow for exported API data.

# v0.2.5

- Amount strings now carry at least 2 decimal places. A whole-dollar source emitted a
  `delta` of `"200"`, which equals account code `"200"` and raised a disclosure error on a
  correct pack. Values are padded, never rounded, and `finding_id` is unchanged. The disclosure check now
  holds the 4 numeric model fields to their number formats instead of comparing them with source
  values, so an account code equal to an amount no longer refuses a pack.
- `evaluate` accepts a trial balance whose account name matches its section, such as an account
  called `Revenue` in section `Revenue`. The disclosure check refused these ordinary packs; it now
  skips `section`, which the model result is meant to carry.
- `evaluate` states when findings stop at `max_results` and gives the total finding count.

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
