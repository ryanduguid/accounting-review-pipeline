# v0.1.11

- Withhold the output filenames from the checkout guard under `--quiet`. The
  refusal named every unignored basename, and the two `git check-ignore`
  failures named the path outright. The post-fetch call names the real
  destination, so a default filename put the organisation's name in the log the
  flag exists to keep it out of. The refusal now gives the count and the
  checkout path, and both git failures route through `shown_path`.
- Correct two v0.1.10 notes below. The unbalanced warning prints the difference,
  not no amounts, and `--quiet` did not reach the checkout refusal until this
  release. Neither correction changes what 0.1.10 does.

# v0.1.10

- Refuse a report whose header repeats a column title instead of exporting
  whichever cell came last. A header carrying `Debit` twice named 2 columns
  one record key, so the later amount silently won; the run now exits with the
  repeated titles named, before a row is read.
- Stop printing the client's debit and credit totals with the unbalanced
  WARNING. It names the difference and, where the caller supplies it, the number
  of accounts the totals were taken over. Tenant refusals name tenant ids and
  counts rather than tenant names.
- Add `--quiet`, which withholds the tenant line, the totals and the
  client-named default filename from the messages that pass through
  `shown_path`, including its failure paths. The containing directory is still
  shown, and the staged `tmp*` names are printed in full so a recovery
  instruction can be followed. The checkout refusal is not covered until
  v0.1.11.
- Refuse to write inside a git working tree unless the CSV, its manifest, the
  `.previous` file and the staged `.tmp` names are all ignored. The check runs
  before any credential is read.

# v0.1.9

- Write `<out>.manifest.json` beside the CSV once it is on disk: tenant id and name, as-at
  date, basis, the CSV filename, the SHA-256 of the bytes written and a UTC `generated_at`.
  A manifest an earlier run left at the same path is set aside before the write,
  removed once the new CSV is on disk and restored if the write fails; `--no-manifest`
  suppresses the file.
- Render the export in memory and write it whole, so the digest is of the payload itself.

# v0.1.8

- Apply both bounded retry policies when an HTTP 401 is followed by an HTTP 429.
- Reject C1 control characters in tenant metadata.
- Tighten money-property, inline-comment and release-workflow checks.

# v0.1.7

- Publish the wheel and source distribution to PyPI from the monorepo through
  the shared attested Python release policy, so `pip install
  xero-trial-balance-export` installs the maintained source and the PyPI page
  points here rather than at the archived source repository.
- Declare the version statically in `pyproject.toml` with a locked `dev` extra
  for the policy's test and build steps; `VERSION` is retired.
- Preserve the v0.1.6 source, OAuth, token-cache, export and command behaviour.

# v0.1.6

- Add the complete pinned `requirements-test.txt` manifest expected by the
  shared source-archive release policy.
- Supersede the protected v0.1.5 tag, whose workflow stopped before creating a
  draft or release because the clean test runner had no runtime dependencies.
- Preserve the v0.1.5 source, OAuth, token-cache, export and command behaviour.

# v0.1.5

- Move the maintained source into `packages/xero-trial-balance-export` in the
  Accounting Review Pipeline.
- Prepare source archives through the monorepo's namespaced, attested release
  workflow.
- Include the canonical `xero-tb-csv.v1` contract and its fabricated integrity
  evaluation in the release.
- Preserve the v0.1.4 OAuth, token-cache, export and command behaviour.

# v0.1.4

Changes since published `v0.1.3`:

- move the Xero token cache out of site-packages to a user-owned location and validate its path through one guard
- exercise the DPAPI token round-trip on the Windows CI leg
- let an explicit `--token-file` beat the `XERO_TOKEN_FILE` environment variable so scripts can pin their credentials source
- documentation corrections: the fact-check fixes, the sample organisation name, a DISCLAIMER linked from the README, the provisional-member CA ANZ designation, and retirement of the last codename from user-facing docs.

The annotated `v0.1.2` tag is protected and permanently records commit `bd4cd417b06fb9dba3d6b36fbedbe544b1e0fec7`. [Workflow run 31832080223](https://github.com/ryanduguid/xero-trial-balance-export/actions/runs/31832080223) passed its tests, archives, checksums and attestations, then stopped before draft creation because one step lacked `GH_TOKEN`. No v0.1.2 GitHub release or draft was created. The protected tag must never be moved, deleted or reused; v0.1.3 is the recovery release.

## Security boundary

The release contains source and fabricated samples only: no Xero credentials, tokens or client exports, and it remains read-only against Xero.

Carried from v0.1.3: the complete Windows token cache is protected at rest with current-user DPAPI; a valid legacy plaintext cache is migrated atomically, under the existing cross-process lock, before any Xero request; corrupt, malformed or unknown-version cache envelopes are rejected without a network call or rewrite; the explicit non-Windows compatibility fallback remains plaintext JSON with owner-only `0600` permissions. Current-user DPAPI is a same-user, same-machine control: it does not protect tokens from code already running as that user, an administrator controlling the machine, or a compromised user session.
