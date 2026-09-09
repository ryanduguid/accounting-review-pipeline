# Releasing

evatt has no standalone release history. It was written in this repository, and
every release it ever has will be a namespaced tag `evatt/vMAJOR.MINOR.PATCH`
published from the
[Accounting Review Pipeline release history](https://github.com/ryanduguid/accounting-review-pipeline/releases).
A separate changelog is intentionally not maintained; `RELEASE_NOTES.md` is the
record.

Releases are built by GitHub Actions from an annotated tag on the exact `main`
commit. Do not build or upload package assets by hand. Do not tag until you
intend to publish. A clean `verify` is not a reason to release, and a release is
not authority to disclose any client file.

## PyPI is deliberately out of scope

`release-evatt.yml` has no `pypi` job. Its four siblings publish through trusted
publishing; this one does not, because a public release of the tool is out of
scope until the disclosure policy the tool enforces is signed off. An absent job
is a better guard than a job that would fire on a stray tag.

A tag therefore produces a GitHub release with the built wheel, source
distribution, `SHA256SUMS`, an SPDX 2.3 SBOM and the provenance and SBOM
attestations, and publishes nothing to any index.

When a PyPI release is actually wanted, the one-time setup is the sibling
pattern and nothing else. Create the GitHub Actions environment `pypi-evatt`
with the URL `https://pypi.org/p/evatt`, register a PyPI trusted publisher
while the project does not yet exist, with exactly these values:

| Field | Value |
| --- | --- |
| PyPI project name | `evatt` |
| Owner | `ryanduguid` |
| Repository name | `accounting-review-pipeline` |
| Workflow filename | `release-evatt.yml` |
| Environment name | `pypi-evatt` |

then add the caller-side `pypi` job. Read both identities back exactly before
the first tag that would use them.

## Before tagging

1. Merge the release pull request and require every `main` check to pass.
2. Enable release immutability in the repository settings.
3. From an operator session authenticated with repository Administration read
   access, run:

    ```bash
    gh api -H "X-GitHub-Api-Version: 2026-03-10" \
      repos/ryanduguid/accounting-review-pipeline/immutable-releases --jq .enabled
    ```

    Do not push the tag unless the output is exactly `true`. The Actions
    `GITHUB_TOKEN` cannot be granted repository Administration read access, so
    the tag workflow cannot perform this preflight itself.
4. Confirm `evatt/version.py` matches the `RELEASE_NOTES.md` heading and that
   `uv lock --check` passes.
5. Confirm the source distribution still carries `DISCLAIMER.md`,
   `DATA-FLOW.md`, `SECURITY.md` and `CONTRIBUTING.md`. `README.md` is the long
   description and points a reader at all four, and `MANIFEST.in` is the only
   thing putting them in the artefact:

    ```bash
    uv run --locked --extra dev python -m build
    python -m tarfile -l dist/evatt-"$(python -c 'import evatt; print(evatt.__version__)')".tar.gz
    ```

6. Create the annotated component tag on current remote `main`, for example
   `git tag -a evatt/v0.1.0 -m "evatt v0.1.0"` (or `-s` when signing is
   configured), then push only that tag.

The workflow runs the locked tests, builds the wheel and source distribution
once, generates an SPDX 2.3 SBOM for the wheel and `SHA256SUMS`, records GitHub
provenance and an SBOM attestation, then publishes the completed draft. An
existing release is never overwritten.

## Verifying a release

```bash
tag=evatt/v0.1.0
repo=ryanduguid/accounting-review-pipeline
version="${tag#evatt/v}"
wheel="evatt-${version}-py3-none-any.whl"
release_commit="$(git ls-remote "https://github.com/$repo.git" "refs/tags/$tag^{}" | cut -f1)"
test -n "$release_commit"
gh release download "$tag" -R "$repo" --dir "release-$tag"
cd "release-$tag"
sha256sum --check SHA256SUMS
gh attestation verify "$wheel" -R "$repo" \
  --source-digest "$release_commit" \
  --source-ref "refs/tags/$tag" \
  --signer-workflow ryanduguid/release-policy/.github/workflows/release-python.yml \
  --signer-digest fcf25e532e9eb60056ae6e5c819cf3125c4f4b91
gh attestation verify "$wheel" -R "$repo" \
  --predicate-type https://spdx.dev/Document/v2.3 \
  --source-digest "$release_commit" \
  --source-ref "refs/tags/$tag" \
  --signer-workflow ryanduguid/release-policy/.github/workflows/release-python.yml \
  --signer-digest fcf25e532e9eb60056ae6e5c819cf3125c4f4b91
gh release view "$tag" -R "$repo" --json isImmutable
gh release verify "$tag" -R "$repo"
gh release verify-asset "$tag" "$wheel" -R "$repo"
```

If any gate fails, inspect it before touching the tag or the draft. Never move a
published tag. Cut a new version rather than rewriting history.
