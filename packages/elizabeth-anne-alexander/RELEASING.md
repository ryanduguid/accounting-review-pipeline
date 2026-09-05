# Releasing

Standalone releases through `v0.2.1` remain in the
[source repository](https://github.com/ryanduguid/xero-ledger-review-gate/releases).
Maintained releases starting with `elizabeth-anne-alexander/v0.2.2` use the
[Accounting Review Pipeline release history](https://github.com/ryanduguid/accounting-review-pipeline/releases).
A separate changelog is intentionally not maintained. The failed standalone
`v0.2.0` tag remains historical and must not be moved or reused.

Releases are built by GitHub Actions from an annotated tag on the exact `main` commit. Do not build or upload package assets by hand.

## Publisher setup

Before creating the first namespaced tag, read back the GitHub environment
and existing PyPI project's trusted publisher with exactly these values:

| Field | Value |
| --- | --- |
| PyPI project name | `elizabeth-anne-alexander` |
| Owner | `ryanduguid` |
| Repository name | `accounting-review-pipeline` |
| Workflow filename | `release-elizabeth-anne-alexander.yml` |
| Environment name | `pypi-elizabeth-anne-alexander` |

Keep the old standalone publisher during the observation period. The new
environment permits `elizabeth-anne-alexander/v*` tags. Do not create the tag
until both subjects have been verified; never repair a failed publication by
moving or reusing a protected tag.

## Before tagging

1. Merge the release pull request and require every `main` check to pass.
2. Enable release immutability in the repository settings.
3. From an operator session authenticated with repository Administration read access, run:

    ```bash
    gh api -H "X-GitHub-Api-Version: 2026-03-10" repos/ryanduguid/accounting-review-pipeline/immutable-releases --jq .enabled
    ```

    Do not push the tag unless the output is exactly `true`. The Actions `GITHUB_TOKEN` cannot be granted repository Administration read access, so the tag workflow cannot perform this preflight itself.
4. Confirm `elizabeth_anne_alexander/version.py` matches the `RELEASE_NOTES.md` heading and `uv lock --check` passes.
5. Create the annotated component tag on current remote `main`, for example `git tag -a elizabeth-anne-alexander/v0.2.2 -m "elizabeth-anne-alexander/v0.2.2"` (or `-s` when signing is configured), then push only that tag.

The workflow runs the locked tests, builds the wheel and source distribution once, generates an SPDX 2.3 SBOM for the wheel and `SHA256SUMS`, records GitHub provenance and an SBOM attestation, then publishes the completed draft. An existing release is never overwritten.

## Historical v0.2.1 verification

Verify the standalone rollback release with:

```bash
tag=v0.2.1
repo=ryanduguid/xero-ledger-review-gate
wheel="elizabeth_anne_alexander-${tag#v}-py3-none-any.whl"
release_commit="$(git ls-remote "https://github.com/$repo.git" "refs/tags/$tag^{}" | cut -f1)"
test -n "$release_commit"
gh release download "$tag" -R "$repo" --dir "release-$tag"
cd "release-$tag"
sha256sum --check SHA256SUMS
gh attestation verify "$wheel" --owner ryanduguid \
  --source-digest "$release_commit" \
  --source-ref "refs/tags/$tag" \
  --signer-workflow ryanduguid/release-policy/.github/workflows/release-python.yml \
  --signer-digest 1ef826004b2dfa7a886e9415156a48ed675a8ca5
gh attestation verify "$wheel" --owner ryanduguid \
  --predicate-type https://spdx.dev/Document/v2.3 \
  --source-digest "$release_commit" \
  --source-ref "refs/tags/$tag" \
  --signer-workflow ryanduguid/release-policy/.github/workflows/release-python.yml \
  --signer-digest 1ef826004b2dfa7a886e9415156a48ed675a8ca5
gh release view "$tag" -R "$repo" --json isImmutable
gh release verify "$tag" -R "$repo"
gh release verify-asset "$tag" "$wheel" -R "$repo"
```

The `v0.2.1` certificate retains the repository's historical
`xero-ai-review-gateway` source identity, so its attestation checks are
owner-scoped and then bound to the exact source digest, source ref, workflow
and signer digest. Releases cut after the rename and shared Python-policy
migration use the current repository identity and hardened policy digest.

## Current namespaced release verification

Update `tag` if the intended version changes, then download and verify the
exact namespaced release:

```bash
tag=elizabeth-anne-alexander/v0.2.2
repo=ryanduguid/accounting-review-pipeline
version="${tag#elizabeth-anne-alexander/v}"
wheel="elizabeth_anne_alexander-${version}-py3-none-any.whl"
release_commit="$(git ls-remote "https://github.com/$repo.git" "refs/tags/$tag^{}" | cut -f1)"
test -n "$release_commit"
gh release download "$tag" -R "$repo" --dir "release-$tag"
cd "release-$tag"
sha256sum --check SHA256SUMS
gh attestation verify "$wheel" -R "$repo" \
  --source-digest "$release_commit" \
  --source-ref "refs/tags/$tag" \
  --signer-workflow ryanduguid/release-policy/.github/workflows/release-python.yml \
  --signer-digest 787db4590e725cfd37104c8a9dd9e75f7fd4c018
gh attestation verify "$wheel" -R "$repo" \
  --predicate-type https://spdx.dev/Document/v2.3 \
  --source-digest "$release_commit" \
  --source-ref "refs/tags/$tag" \
  --signer-workflow ryanduguid/release-policy/.github/workflows/release-python.yml \
  --signer-digest 787db4590e725cfd37104c8a9dd9e75f7fd4c018
gh release view "$tag" -R "$repo" --json isImmutable
gh release verify "$tag" -R "$repo"
gh release verify-asset "$tag" "$wheel" -R "$repo"
```

## Historical v0.1.1 verification

The immutable `v0.1.1` release predates the identity change. Its asset names remain `xero_ai_review_gateway-*`; verify those historical bytes against the renamed repository rather than relabelling them:

```bash
gh release download v0.1.1 -R ryanduguid/xero-ledger-review-gate --dir release-v0.1.1
cd release-v0.1.1
sha256sum --check SHA256SUMS
historical_source=9079691d9e20f4af9f1b2583110b0a85d680d690
gh attestation verify xero_ai_review_gateway-0.1.1-py3-none-any.whl \
  --owner ryanduguid \
  --source-digest "$historical_source" \
  --source-ref refs/tags/v0.1.1 \
  --signer-workflow ryanduguid/xero-ai-review-gateway/.github/workflows/release.yml \
  --signer-digest "$historical_source"
gh attestation verify xero_ai_review_gateway-0.1.1-py3-none-any.whl \
  --owner ryanduguid \
  --predicate-type https://spdx.dev/Document/v2.3 \
  --source-digest "$historical_source" \
  --source-ref refs/tags/v0.1.1 \
  --signer-workflow ryanduguid/xero-ai-review-gateway/.github/workflows/release.yml \
  --signer-digest "$historical_source"
gh release view v0.1.1 -R ryanduguid/xero-ledger-review-gate --json isImmutable
```

Release `v0.1.1` predates release-level attestations. Its checksum and exact
asset attestations remain verifiable, but `gh release verify` and
`gh release verify-asset` correctly report that no tag attestation exists.

If any gate fails, inspect it before touching the tag or draft. Never move a published tag.
