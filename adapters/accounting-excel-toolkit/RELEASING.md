# Releasing

Use [Accounting Review Pipeline releases](https://github.com/ryanduguid/accounting-review-pipeline/releases) for new component releases. Retain the [standalone release history](https://github.com/ryanduguid/accounting-excel-toolkit/releases), including immutable rollback release `v0.1.5`.

The active caller is the root [release-accounting-excel-toolkit.yml](../../.github/workflows/release-accounting-excel-toolkit.yml). It selects only `adapters/accounting-excel-toolkit/` through a namespaced `accounting-excel-toolkit/v*` tag. The nested `.github/` files are inert source history. This documentation migration does not bump the version or request an archive release.

Releases are built by GitHub Actions from an annotated tag on the exact `main` commit. Do not create or upload release assets by hand.

Before tagging:

1. Merge the release pull request and require every `main` check to pass.
2. Enable release immutability in the repository settings.
3. From an operator session authenticated with repository Administration read access, run:

    ```bash
    gh api -H "X-GitHub-Api-Version: 2026-03-10" repos/ryanduguid/accounting-review-pipeline/immutable-releases --jq .enabled
    ```

    Do not push the tag unless the output is exactly `true`. The Actions `GITHUB_TOKEN` cannot be granted repository Administration read access, so the tag workflow cannot perform this preflight itself.
4. Confirm the component's `VERSION` and `RELEASE_NOTES.md` describe the separately approved version. Keep the notes heading as `# vMAJOR.MINOR.PATCH`; the root caller supplies the tag namespace.
5. After separate release approval, create an annotated tag on the current remote `main` commit, for example `git tag -a accounting-excel-toolkit/v0.1.6 -m "accounting-excel-toolkit/v0.1.6"` (use `-s` instead of `-a` when a signing key is configured), then push only that tag.

The standalone repository published `v0.1.0`, `v0.1.2` and `v0.1.5`. Its protected `v0.1.1`, `v0.1.3` and `v0.1.4` tags retain failed-preflight history with no releases or assets. [Pilot run 31822769922](https://github.com/ryanduguid/accounting-excel-toolkit/actions/runs/31822769922) records the `v0.1.1` Administration-read failure. Do not move or delete those tags. `VERSION` and `RELEASE_NOTES.md` still describe `v0.1.5`; a future `accounting-excel-toolkit/v0.1.6` needs its own version-and-notes change and release approval.

The workflow reruns the regression suite, builds deterministic ZIP and tar.gz source archives, generates an SPDX 2.3 SBOM and `SHA256SUMS`, records GitHub provenance and SBOM attestations, then publishes a draft release only after every asset is uploaded. The archive helper fixes the timezone to UTC and Git text conversion to LF so the same tagged tree produces the same archive bytes on Linux and Windows. Existing releases are refused rather than overwritten.

After publication, download the assets and verify them:

```bash
gh release download v0.1.2 -R ryanduguid/accounting-excel-toolkit --dir release-v0.1.2
cd release-v0.1.2
sha256sum --check SHA256SUMS
gh attestation verify accounting-excel-toolkit-0.1.2.zip -R ryanduguid/accounting-excel-toolkit
gh attestation verify accounting-excel-toolkit-0.1.2.zip -R ryanduguid/accounting-excel-toolkit --predicate-type https://spdx.dev/Document/v2.3
gh release view v0.1.2 -R ryanduguid/accounting-excel-toolkit --json isImmutable
gh release verify v0.1.2 -R ryanduguid/accounting-excel-toolkit
gh release verify-asset v0.1.2 accounting-excel-toolkit-0.1.2.zip -R ryanduguid/accounting-excel-toolkit
```

Those commands preserve the consumer-owned signer identity of historical
release `v0.1.2`. Releases cut after the shared archive-policy migration use
the policy's internal publication workflow as the signer. For the next
release, update `tag` if the intended version changes and verify that exact
source and signer identity:

```bash
set -euo pipefail
tag=accounting-excel-toolkit/v0.1.6
repo=ryanduguid/accounting-review-pipeline
release_commit="$(git ls-remote "https://github.com/$repo.git" "refs/tags/$tag^{}" | cut -f1)"
test -n "$release_commit"
release_dir="$(mktemp -d)"
gh release download "$tag" -R "$repo" --dir "$release_dir"
cd "$release_dir"
sha256sum --check SHA256SUMS
for file in *; do
  gh attestation verify "$file" -R "$repo" \
    --source-digest "$release_commit" \
    --source-ref "refs/tags/$tag" \
    --signer-workflow ryanduguid/release-policy/.github/workflows/publish-archives.yml \
    --signer-digest 787db4590e725cfd37104c8a9dd9e75f7fd4c018
done
for archive in "accounting-excel-toolkit-${tag##*/v}.zip" "accounting-excel-toolkit-${tag##*/v}.tar.gz"; do
  gh attestation verify "$archive" -R "$repo" \
    --predicate-type https://spdx.dev/Document/v2.3 \
    --source-digest "$release_commit" \
    --source-ref "refs/tags/$tag" \
    --signer-workflow ryanduguid/release-policy/.github/workflows/publish-archives.yml \
    --signer-digest 787db4590e725cfd37104c8a9dd9e75f7fd4c018
done
```

Releases cut before the policy pin last moved verify against the
`--signer-digest` that the active caller carried at the tagged commit, not the one
above.

If any gate fails, leave the tag and any draft release untouched until the failure is understood. Never move an already published tag.

## Rollback

Rollback of the caller requires a reviewed pull request that repins the root caller to a reviewed 40-character commit in `ryanduguid/release-policy`. Preserve existing tags, releases and assets. The structural check in `tests/test_release_archives.py` validates the active caller's immutable pin and component boundary.
