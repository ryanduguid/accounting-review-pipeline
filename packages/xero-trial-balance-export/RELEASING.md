# Releasing

The Accounting Review Pipeline
[releases](https://github.com/ryanduguid/accounting-review-pipeline/releases)
are canonical from `xero-trial-balance-export/v0.1.5` onward. Releases through
v0.1.4 remain in the
[source repository](https://github.com/ryanduguid/xero-trial-balance-export/releases).
A separate changelog is intentionally not maintained.

Releases are built by GitHub Actions from an annotated tag on the exact `main` commit. Do not create or upload release assets by hand.

## Protected v0.1.2 failed tag

The annotated `v0.1.2` tag is protected and permanently records commit `bd4cd417b06fb9dba3d6b36fbedbe544b1e0fec7`. [Release workflow run 31832080223](https://github.com/ryanduguid/xero-trial-balance-export/actions/runs/31832080223) completed the tests, deterministic archives, checksums and both attestation steps, then failed safely at the immediate remote recheck because that GitHub CLI step did not receive `GH_TOKEN`. The publication step was skipped, and the authenticated release inventory confirmed that no v0.1.2 release or draft exists.

Do not move, delete or reuse `v0.1.2`. The no-bypass tag ruleset prevents those operations; `v0.1.3` is the recovery version.

## Preserved squash-boundary releases

Two published tags point at pull-request-side commits that preceded their
squash merges to `main`. They are intentional historical exceptions outside
current `main` ancestry:

| Release | Tag object | Peeled commit |
| --- | --- | --- |
| `v0.1.1` | `aeee63b723fcf5276f9375769668c865b19ba8bb` | `d9b4cfd9ee8398c30dbe64b4ba2254aca900c006` |
| `v0.1.3` | `e52022b2e81c1920619d66e77b388b44876c8337` | `8586a960b4fd08dd0cd68be28fcac811a20a2e0c` |

Preserve those immutable tags exactly as published. Do not move, delete or
recreate them to make the history appear linear. Every future release tag must
point to a commit reachable from protected `main`.

Before tagging:

1. Merge the release pull request and require every `main` check to pass.
2. Enable release immutability in the repository settings.
3. From an operator session authenticated with repository Administration read access, run:

    ```bash
    gh api -H "X-GitHub-Api-Version: 2026-03-10" repos/ryanduguid/accounting-review-pipeline/immutable-releases --jq .enabled
    ```

    Do not push the tag unless the output is exactly `true`. The Actions `GITHUB_TOKEN` cannot be granted repository Administration read access, so the tag workflow cannot perform this preflight itself.
4. Confirm the active `Protect version tags` ruleset includes
   `refs/tags/xero-trial-balance-export/v*`, has no bypass actor, allows
   creation, and blocks tag updates and deletion:

    ```bash
    ruleset_id="$(gh api -H "X-GitHub-Api-Version: 2026-03-10" repos/ryanduguid/accounting-review-pipeline/rulesets --jq '.[] | select(.name == "Protect version tags" and .target == "tag" and .enforcement == "active") | .id')"
    test -n "$ruleset_id"
    gh api -H "X-GitHub-Api-Version: 2026-03-10" "repos/ryanduguid/accounting-review-pipeline/rulesets/$ruleset_id" --jq '{enforcement, bypass_actors, conditions, rules}'
    ```

    Stop unless the returned configuration has an empty `bypass_actors` array,
    includes the exact namespaced tag prefix, and contains active `update` and
    `deletion` rules but no `creation` rule. This protection is required because
    immutable-release protection begins only when a draft is published.
5. Confirm `VERSION` and the first line of `RELEASE_NOTES.md` match the intended tag.
6. Fetch current remote `main`, create a namespaced annotated tag on that exact
   commit, for example `git tag -a xero-trial-balance-export/v0.1.5 -m "xero-trial-balance-export v0.1.5"`
   (or `-s` when signing is configured), then push only that tag.

The workflow installs the hash-locked dependencies, runs the full offline suite and builds deterministic ZIP and tar.gz source archives. The archive helper fixes the timezone to UTC and Git text conversion to LF so the same tagged tree produces the same archive bytes on Linux and Windows. It adds an SPDX 2.3 SBOM, `SHA256SUMS`, GitHub provenance and an SBOM attestation before publishing the completed draft.

The authenticated release inventory must prove that no release or draft already uses the tag. The workflow creates the candidate as a draft, finds that draft through the all-releases API, and addresses it only by release ID. Before publication it verifies the exact notes, asset names and digests, then rechecks that the remote annotated tag and `main` still peel to the tested workflow commit. After publication it checks immutability, latest-release classification, digests and every release attestation. A failure after draft creation leaves that draft for deliberate inspection; an earlier failure may leave no draft, as v0.1.2 demonstrated. Query the authenticated release inventory, preserve the failed tag and do not replace or rerun blindly. The immediate pre-publication check narrows, but cannot make atomic, the residual race with a concurrent merge to `main`; do not merge other work during a release run, and rely on the no-bypass tag ruleset to prevent tag movement.

Verify the downloaded release with:

```bash
tag=xero-trial-balance-export/v0.1.5
repo=ryanduguid/accounting-review-pipeline
version="${tag#xero-trial-balance-export/v}"
release_commit="$(git ls-remote "https://github.com/$repo.git" "refs/tags/$tag^{}" | cut -f1)"
test -n "$release_commit"
release_dir="release-${tag//\//-}"
gh release download "$tag" -R "$repo" --dir "$release_dir"
cd "$release_dir"
sha256sum --check SHA256SUMS
for file in *; do
  gh attestation verify "$file" -R "$repo" \
    --source-digest "$release_commit" \
    --source-ref "refs/tags/$tag" \
    --signer-workflow ryanduguid/release-policy/.github/workflows/publish-archives.yml \
    --signer-digest 3ff09b654a17b9a3b55548e25e6108ee582b00c4
  gh release verify-asset "$tag" "$file" -R "$repo"
done
for archive in "xero-trial-balance-export-$version.tar.gz" \
               "xero-trial-balance-export-$version.zip"; do
  gh attestation verify "$archive" -R "$repo" \
    --predicate-type https://spdx.dev/Document/v2.3 \
    --source-digest "$release_commit" \
    --source-ref "refs/tags/$tag" \
    --signer-workflow ryanduguid/release-policy/.github/workflows/publish-archives.yml \
    --signer-digest 3ff09b654a17b9a3b55548e25e6108ee582b00c4
done
gh release view "$tag" -R "$repo" --json isImmutable,tagName \
  | jq -e --arg tag "$tag" \
      '.isImmutable == true and .tagName == $tag'
gh release verify "$tag" -R "$repo"
```

If any gate fails, inspect it before touching the draft. Never move, delete or reuse a protected release tag, whether or not publication completed.
