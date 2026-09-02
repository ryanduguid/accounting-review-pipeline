# Releasing

The repository's [GitHub Releases](https://github.com/ryanduguid/workpaper-review-gate/releases) page is the canonical release history. A separate changelog is intentionally not maintained.

Releases are built by GitHub Actions from an annotated tag on the exact `main`
commit. Do not build or upload package assets by hand. Do not tag until you
intend to publish. A `READY` result from this tool is not a reason to release,
and a release is not an approval of any client file.

This release uses version `0.1.2`. The protected `v0.1.0` tag failed its
release-notes-header gate before any GitHub Release, asset or PyPI project was
created. Never move or reuse that tag. The published `v0.1.1` recovery remains
historical and must not be moved or reused.

## One-time setup before the first tag

1. Create the GitHub Actions environment `pypi` on
   `ryanduguid/workpaper-review-gate` (Settings → Environments). Set its URL to
   `https://pypi.org/p/review-ready-gate`.
2. Register a PyPI trusted publisher (Account → Publishing → "Add a new
   pending publisher" while the project does not exist) with exactly these
   values:

| Field | Value |
| --- | --- |
| PyPI project name | `review-ready-gate` |
| Owner | `ryanduguid` |
| Repository name | `workpaper-review-gate` |
| Workflow filename | `release.yml` |
| Environment name | `pypi` |

Until both exist, the `pypi` job fails closed after the GitHub Release is
published. Once the publisher exists, backfill with `workflow_dispatch` on
`release.yml` and the existing tag. Do not retag.

## Before tagging

1. Merge the release pull request and require every `main` check to pass.
2. Enable release immutability in the repository settings.
3. From an operator session authenticated with repository Administration read
   access, run:

    ```bash
    gh api -H "X-GitHub-Api-Version: 2026-03-10" \
      repos/ryanduguid/workpaper-review-gate/immutable-releases --jq .enabled
    ```

    Do not push the tag unless the output is exactly `true`. The Actions
    `GITHUB_TOKEN` cannot be granted repository Administration read access, so
    the tag workflow cannot perform this preflight itself.
4. Confirm the versions in `pyproject.toml` and `uv.lock` match the
   `RELEASE_NOTES.md` heading.
5. Create an annotated tag on current remote `main`, for example
   `git tag -a v0.1.2 -m "v0.1.2"` (or `-s` when signing is configured), then
   push only that tag.

The workflow runs the locked tests, builds the wheel and source distribution
once, generates an SPDX 2.3 SBOM for the wheel and `SHA256SUMS`, records GitHub
provenance and an SBOM attestation, then publishes the completed draft. An
existing release is never overwritten.

Verify the downloaded release with:

```bash
tag=v0.1.2
repo=ryanduguid/workpaper-review-gate
wheel="review_ready_gate-${tag#v}-py3-none-any.whl"
release_commit="$(git ls-remote "https://github.com/$repo.git" "refs/tags/$tag^{}" | cut -f1)"
test -n "$release_commit"
gh release download "$tag" -R "$repo" --dir "release-$tag"
cd "release-$tag"
sha256sum --check SHA256SUMS
gh attestation verify "$wheel" -R "$repo" \
  --source-digest "$release_commit" \
  --source-ref "refs/tags/$tag" \
  --signer-workflow ryanduguid/release-policy/.github/workflows/release-python.yml \
  --signer-digest 8b4de1ed339f1358b5f3e850b63412d8717d01da
gh attestation verify "$wheel" -R "$repo" \
  --predicate-type https://spdx.dev/Document/v2.3 \
  --source-digest "$release_commit" \
  --source-ref "refs/tags/$tag" \
  --signer-workflow ryanduguid/release-policy/.github/workflows/release-python.yml \
  --signer-digest 8b4de1ed339f1358b5f3e850b63412d8717d01da
gh release view "$tag" -R "$repo" --json isImmutable
gh release verify "$tag" -R "$repo"
gh release verify-asset "$tag" "$wheel" -R "$repo"
```

If any gate fails, inspect it before touching the tag or draft. Never move a
published tag. Cut a new version rather than rewriting history.
