# Releasing

Standalone releases through `v0.1.1` remain in the
[source repository](https://github.com/ryanduguid/workpaper-review-gate/releases). Maintained
releases starting with `review-ready-gate/v0.1.3` are published from the Accounting Review
Pipeline repository. A separate changelog is intentionally not maintained.

Releases are built by GitHub Actions from an annotated tag on the exact `main`
commit. Do not build or upload package assets by hand. Do not tag until you
intend to publish. A `READY` result from this tool is not a reason to release,
and a release is not an approval of any client file.

The imported package currently uses version `0.1.2`; the first replacement release will use
version `0.1.3`. The protected standalone `v0.1.0` tag failed its
release-notes-header gate before any GitHub Release, asset or PyPI project was
created. Never move or reuse that tag. The published `v0.1.1` recovery remains
historical and must not be moved or reused.

## One-time setup before the first tag

1. Create the GitHub Actions environment `pypi-review-ready-gate` on
   `ryanduguid/accounting-review-pipeline` (Settings → Environments). Set its URL to
   `https://pypi.org/p/review-ready-gate`.
2. Register a PyPI trusted publisher (Account → Publishing → "Add a new
   pending publisher" while the project does not exist) with exactly these
   values:

| Field | Value |
| --- | --- |
| PyPI project name | `review-ready-gate` |
| Owner | `ryanduguid` |
| Repository name | `accounting-review-pipeline` |
| Workflow filename | `release-review-ready-gate.yml` |
| Environment name | `pypi-review-ready-gate` |

Until both exist, the `pypi` job fails closed after the GitHub Release is published. Do not
create a namespaced release tag until both identities have been read back exactly. Never
retag; inspect and rerun a failed protected job, or cut a new version when correction is
required.

## Before tagging

1. Merge the release pull request and require every `main` check to pass.
2. Enable release immutability in the repository settings.
3. From an operator session authenticated with repository Administration read
   access, run against the repository name currently in force:

    ```bash
    gh api -H "X-GitHub-Api-Version: 2026-03-10" \
      repos/ryanduguid/accounting-review-pipeline/immutable-releases --jq .enabled
    ```

    Do not push the tag unless the output is exactly `true`. The Actions
    `GITHUB_TOKEN` cannot be granted repository Administration read access, so
    the tag workflow cannot perform this preflight itself.
4. Confirm the versions in `pyproject.toml` and `uv.lock` match the
   `RELEASE_NOTES.md` heading.
5. Create an annotated namespaced tag on current remote `main`, for example
   `git tag -a review-ready-gate/v0.1.3 -m "review-ready-gate v0.1.3"` (or `-s`
   when signing is configured), then
   push only that tag.

The workflow runs the locked tests, builds the wheel and source distribution
once, generates an SPDX 2.3 SBOM for the wheel and `SHA256SUMS`, records GitHub
provenance and an SBOM attestation, then publishes the completed draft. An
existing release is never overwritten.

Verify the downloaded release with:

```bash
tag=review-ready-gate/v0.1.3
repo=ryanduguid/accounting-review-pipeline
version="${tag#review-ready-gate/v}"
wheel="review_ready_gate-${version}-py3-none-any.whl"
release_commit="$(git ls-remote "https://github.com/$repo.git" "refs/tags/$tag^{}" | cut -f1)"
test -n "$release_commit"
gh release download "$tag" -R "$repo" --dir "release-$tag"
cd "release-$tag"
sha256sum --check SHA256SUMS
gh attestation verify "$wheel" -R "$repo" \
  --source-digest "$release_commit" \
  --source-ref "refs/tags/$tag" \
  --signer-workflow ryanduguid/release-policy/.github/workflows/release-python.yml \
  --signer-digest 3ff09b654a17b9a3b55548e25e6108ee582b00c4
gh attestation verify "$wheel" -R "$repo" \
  --predicate-type https://spdx.dev/Document/v2.3 \
  --source-digest "$release_commit" \
  --source-ref "refs/tags/$tag" \
  --signer-workflow ryanduguid/release-policy/.github/workflows/release-python.yml \
  --signer-digest 3ff09b654a17b9a3b55548e25e6108ee582b00c4
gh release view "$tag" -R "$repo" --json isImmutable
gh release verify "$tag" -R "$repo"
gh release verify-asset "$tag" "$wheel" -R "$repo"
```

If any gate fails, inspect it before touching the tag or draft. Never move a
published tag. Cut a new version rather than rewriting history.
