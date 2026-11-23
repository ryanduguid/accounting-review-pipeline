# Releasing

From 1.0.0 evatt is a standalone Rust binary. The Python release caller
`release-evatt.yml` has been removed. No 1.0.0 release can be made until a Rust
release path is designed and approved, including build targets, signing,
attestation, sample and documentation delivery, and the disclosure-policy gate.
`cargo build --locked --release` builds the binary locally; it publishes nothing.
There is no Python import API or PyPI publication.

The recipe below verifies the historical 0.2.x Python release assets. It does
not authorise a 1.0.0 tag.

evatt has no standalone release history. It was written in this repository, and
every release it ever has will be a namespaced tag `evatt/vMAJOR.MINOR.PATCH`
published from the
[Accounting Review Pipeline release history](https://github.com/ryanduguid/accounting-review-pipeline/releases).
A separate changelog is intentionally not maintained; `RELEASE_NOTES.md` is the
record.

The removed `release-evatt.yml` built each 0.2.x release from an annotated tag on
the exact `main` commit: a GitHub release with the wheel, source distribution,
`SHA256SUMS`, an SPDX 2.3 SBOM and the provenance and SBOM attestations, and
nothing published to any index. A clean `verify` is not a reason to release,
and a release is not authority to disclose any client file.

## Verifying a release

```bash
tag=evatt/v0.2.0
repo=ryanduguid/accounting-review-pipeline
version="${tag#evatt/v}"
wheel="evatt-${version}-py3-none-any.whl"
release_commit="$(git ls-remote "https://github.com/$repo.git" "refs/tags/$tag^{}" | cut -f1)"
test -n "$release_commit"
policy_sha="$(gh api -H "Accept: application/vnd.github.raw+json" \
  "repos/$repo/contents/.github/workflows/release-evatt.yml?ref=$release_commit" \
  | sed -n 's#.*release-policy/\.github/workflows/[a-z-]*\.yml@\([0-9a-f]\{40\}\).*#\1#p')"
test -n "$policy_sha"
gh release download "$tag" -R "$repo" --dir "release-$tag"
cd "release-$tag"
sha256sum --check SHA256SUMS
gh attestation verify "$wheel" -R "$repo" \
  --source-digest "$release_commit" \
  --source-ref "refs/tags/$tag" \
  --signer-workflow ryanduguid/release-policy/.github/workflows/release-python.yml \
  --signer-digest "$policy_sha"
gh attestation verify "$wheel" -R "$repo" \
  --predicate-type https://spdx.dev/Document/v2.3 \
  --source-digest "$release_commit" \
  --source-ref "refs/tags/$tag" \
  --signer-workflow ryanduguid/release-policy/.github/workflows/release-python.yml \
  --signer-digest "$policy_sha"
gh release view "$tag" -R "$repo" --json isImmutable
gh release verify "$tag" -R "$repo"
gh release verify-asset "$tag" "$wheel" -R "$repo"
```

If any gate fails, inspect it before touching the tag or the draft. Never move a
published tag. Cut a new version rather than rewriting history.
