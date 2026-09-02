# Releasing

The repository's [GitHub Releases](https://github.com/ryanduguid/accounting-excel-toolkit/releases) page is the canonical release history. A separate changelog is intentionally not maintained.

Releases are built by GitHub Actions from an annotated tag on the exact `main` commit. Do not create or upload release assets by hand.

Before tagging:

1. Merge the release pull request and require every `main` check to pass.
2. Enable release immutability in the repository settings.
3. From an operator session authenticated with repository Administration read access, run:

    ```bash
    gh api -H "X-GitHub-Api-Version: 2026-03-10" repos/ryanduguid/accounting-excel-toolkit/immutable-releases --jq .enabled
    ```

    Do not push the tag unless the output is exactly `true`. The Actions `GITHUB_TOKEN` cannot be granted repository Administration read access, so the tag workflow cannot perform this preflight itself.
4. Confirm `VERSION` is the intended version and the first line of `RELEASE_NOTES.md` is the matching tag.
5. Create an annotated tag on the current remote `main` commit, for example `git tag -a v0.1.6 -m "v0.1.6"` (use `-s` instead of `-a` when a signing key is configured), then push only that tag.

Published releases are `v0.1.0`, `v0.1.2` and `v0.1.5`. The protected `v0.1.1`, `v0.1.3` and `v0.1.4` tags are unreleased failed-preflight history: each stopped before any build or publication step, so none has a release or assets. [Pilot run 31822769922](https://github.com/ryanduguid/accounting-excel-toolkit/actions/runs/31822769922) is the `v0.1.1` Administration-read failure. Do not move or delete any of those tags. `VERSION` and `RELEASE_NOTES.md` currently describe `v0.1.5`; the next intended tag is `v0.1.6`.

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
tag=v0.1.6
repo=ryanduguid/accounting-excel-toolkit
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
    --signer-digest fca32335275ee264799644ccd659b025358dd23c
done
gh attestation verify "accounting-excel-toolkit-${tag#v}.zip" -R "$repo" \
  --predicate-type https://spdx.dev/Document/v2.3 \
  --source-digest "$release_commit" \
  --source-ref "refs/tags/$tag" \
  --signer-workflow ryanduguid/release-policy/.github/workflows/publish-archives.yml \
  --signer-digest fca32335275ee264799644ccd659b025358dd23c
```

Releases cut before the policy pin last moved verify against the
`--signer-digest` that `release.yml` carried at the time, not the one
above.

If any gate fails, leave the tag and any draft release untouched until the failure is understood. Never move an already published tag.

## Rollback

Rollback of the caller is a reviewed pull request that repins `release-archive.yml` to the previous full 40-character commit SHA of this repository (or reverts to a reviewed local implementation). No workflow creates, moves or deletes tags, so rollback never touches published releases; existing tags and their assets stay exactly as they are. The structural test in `tests/test_release_archives.py` pins the expected SHA and must be updated in the same change.
