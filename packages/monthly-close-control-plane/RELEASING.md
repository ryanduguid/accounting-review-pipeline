# Releasing

Version 1.0.0 onwards ships the native `close-control` executable on an immutable
GitHub Release. The root caller
[release-monthly-close-control-plane.yml](../../.github/workflows/release-monthly-close-control-plane.yml)
calls Release Policy's `release-binary.yml` at a pinned commit on an annotated
`monthly-close-control-plane/vMAJOR.MINOR.PATCH` tag. Nothing is published to PyPI or crates.io
(`publish = false`). Never build, upload or replace release assets by hand, and
tag only with the owner's explicit approval for that release.

## Before tagging

1. In a reviewed pull request, set the same version in `VERSION`, the
   `[package] version` in `Cargo.toml` and `Cargo.lock`, and make the first line
   of `RELEASE_NOTES.md` exactly `# vMAJOR.MINOR.PATCH`. The whole file becomes
   the release body.
2. From this directory, run the component checks:

    ```bash
    cargo fmt --check
    cargo clippy --locked --all-targets --all-features -- -D warnings
    cargo test --locked --all-features
    cargo build --locked --release --bin close-control
    ```

3. Merge, then wait until every check the caller's `required-checks` names has
   succeeded in the push run of `main`'s head. The release refuses a tag on any
   other commit, or one whose checks are missing, skipped or failed.
4. Confirm release immutability from an operator session with repository
   Administration read access; the workflow token cannot read it:

    ```bash
    gh api -H "X-GitHub-Api-Version: 2026-03-10" repos/ryanduguid/accounting-review-pipeline/immutable-releases --jq .enabled
    ```

    Do not push the tag unless the output is exactly `true`.
5. Tag `main`'s head and push only that tag:

    ```bash
    git fetch origin main
    git tag -a monthly-close-control-plane/v1.0.0 -m "monthly-close-control-plane/v1.0.0" origin/main
    git push origin monthly-close-control-plane/v1.0.0
    ```

The workflow runs `cargo test --locked` and builds `close-control` with
`cargo-auditable` on each target, checks static linkage and `close-control --version`,
audits the embedded dependency list against RustSec, then publishes 9 assets:

| Asset | Contents |
| --- | --- |
| `monthly-close-control-plane-<version>.zip`, `.tar.gz` | deterministic source archives |
| `monthly-close-control-plane-<version>.spdx.json` | SPDX SBOM of the source |
| `monthly-close-control-plane-<version>-x86_64-unknown-linux-musl.tar.gz` | static Linux `close-control` with `LICENSE`, `NOTICE` and `README.md` |
| `monthly-close-control-plane-<version>-x86_64-unknown-linux-musl.spdx.json` | SBOM of the Linux executable |
| `monthly-close-control-plane-<version>-x86_64-pc-windows-msvc.zip` | Windows `close-control.exe` with the same documents |
| `monthly-close-control-plane-<version>-x86_64-pc-windows-msvc.spdx.json` | SBOM of the Windows executable |
| `SHA256SUMS` | digests of the 7 assets above |
| `monthly-close-control-plane-<version>.intoto.jsonl` | signed provenance bundle |

From 1.1.0, each release executable records the tagged commit in the
`producer` block of the packs it writes. The release build does not set
`ACCOUNTING_REVIEW_SOURCE_COMMIT`, so the command falls back to the
`GITHUB_SHA` GitHub Actions gives that build. The 1.0.0 executables, and a local
build without either variable, record `unknown`.

If any gate fails, inspect it before touching the tag or draft. An existing
release is never replaced; cut a new version instead.

## Verify a release

Download an explicit tag and target, never `latest`. The signer digest comes
from the caller at the tagged commit, so the recipe stays right when the pin moves:

```bash
tag=monthly-close-control-plane/v1.0.0
repo=ryanduguid/accounting-review-pipeline
version="${tag#monthly-close-control-plane/v}"
asset="monthly-close-control-plane-${version}-x86_64-unknown-linux-musl.tar.gz"  # or -x86_64-pc-windows-msvc.zip
release_commit="$(git ls-remote "https://github.com/$repo.git" "refs/tags/$tag^{}" | cut -f1)"
test -n "$release_commit"
policy_sha="$(gh api -H "Accept: application/vnd.github.raw+json" \
  "repos/$repo/contents/.github/workflows/release-monthly-close-control-plane.yml?ref=$release_commit" \
  | sed -n 's#.*release-policy/\.github/workflows/release-binary\.yml@\([0-9a-f]\{40\}\).*#\1#p')"
test -n "$policy_sha"
pins=(--repo "$repo" --source-ref "refs/tags/$tag" --source-digest "$release_commit"
      --signer-workflow ryanduguid/release-policy/.github/workflows/release-binary.yml
      --signer-digest "$policy_sha")
gh release download "$tag" -R "$repo" --pattern SHA256SUMS --pattern "$asset"
gh attestation verify SHA256SUMS "${pins[@]}"
sha256sum --check --ignore-missing SHA256SUMS
gh attestation verify "$asset" "${pins[@]}"
gh attestation verify "$asset" "${pins[@]}" --predicate-type https://spdx.dev/Document/v2.3
gh release view "$tag" -R "$repo" --json isImmutable --jq .isImmutable
gh release verify "$tag" -R "$repo"
gh release verify-asset "$tag" "$asset" -R "$repo"
```

Verify `SHA256SUMS` before trusting a digest it lists, expect `true` from the
immutability check, and fail closed on any mismatch. A checksum alone, or an
attestation check without the pins, is not enough.

Version 1.0.0 replaces the Python runtime and import API with the close-control
Rust binary. Existing Python tags and artefacts remain historical records.
A PASS review pack does not approve accounting or authorise a software release.

## Historical Python release procedure

The following procedure documents earlier Python releases and does not apply to
version 1.0.0 or authorise a new publication.

The repository's [GitHub Releases](https://github.com/ryanduguid/accounting-review-pipeline/releases) page is the canonical release history. A separate changelog is intentionally not maintained.

Releases are built by GitHub Actions from an annotated tag on the exact `main` commit. Do not build or upload package assets by hand.

Before tagging:

1. Merge the release pull request and require every `main` check to pass.
2. Enable release immutability in the repository settings.
3. From an operator session authenticated with repository Administration read access, run:

    ```bash
    gh api -H "X-GitHub-Api-Version: 2026-03-10" repos/ryanduguid/accounting-review-pipeline/immutable-releases --jq .enabled
    ```

    Do not push the tag unless the output is exactly `true`. The Actions `GITHUB_TOKEN` cannot be granted repository Administration read access, so the tag workflow cannot perform this preflight itself.
4. Confirm the versions in `pyproject.toml` and `uv.lock` match the `RELEASE_NOTES.md` heading.
5. Create the annotated component tag on current remote `main`, for example `git tag -a monthly-close-control-plane/v0.2.0 -m "monthly-close-control-plane/v0.2.0"` (or `-s` when signing is configured), then push only that tag.

The workflow runs the locked tests, builds the wheel and source distribution once, generates an SPDX 2.3 SBOM for the wheel and `SHA256SUMS`, records GitHub provenance and an SBOM attestation, then publishes the completed draft. An existing release is never overwritten.

Verify the downloaded release with:

```bash
tag=v0.1.2
repo=ryanduguid/accounting-review-pipeline
wheel="monthly_close_control_plane-${tag#v}-py3-none-any.whl"
# The commit the v0.1.2 attestations were issued for. Read the discrepancy note
# below before substituting anything a fresh ls-remote returns.
release_commit=623f0a3f8c4832c92bf0fc1a2a215441808b15b4
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

The `v0.1.2` certificate retains the repository's historical
`monthly-close-control-plane` source identity, so its attestation checks are
owner-scoped and then bound to the exact source digest, source ref, workflow
and signer digest.

Its source digest is pinned rather than read from the tag, because the two
disagree. The certificate attests `623f0a3f8c4832c92bf0fc1a2a215441808b15b4`,
while the annotated `v0.1.2` tag in this repository peels to
`8aabc23648ed922103e7860a4c6abbc89e0c5af1`. Both commits carry the same tree,
`2cfbb3043353680205ebf0cefabd1df38b1dd660`, and different parents, so no file
differs between them. When and why the tag came to name the second commit is
not recorded here, and this guide does not treat it as settled. The two
`gh attestation verify` commands above therefore establish the historical
attestations only. `gh release verify` and `gh release verify-asset` fail for
this tag, because no attestation is bound to the current tag object; run them
and record the failure rather than reading it as a passing check. Never move a
published tag to make these agree. Releases cut after the rename and shared Python-policy
migration use the current repository identity and hardened policy digest. For
the next release, update `tag` if the intended version changes and run these
checks after downloading the assets and checking `SHA256SUMS`:

```bash
tag=monthly-close-control-plane/v0.2.0
repo=ryanduguid/accounting-review-pipeline
version="${tag#monthly-close-control-plane/v}"
wheel="monthly_close_control_plane-${version}-py3-none-any.whl"
release_commit="$(git ls-remote "https://github.com/$repo.git" "refs/tags/$tag^{}" | cut -f1)"
test -n "$release_commit"
policy_sha="$(gh api -H "Accept: application/vnd.github.raw+json" \
  "repos/$repo/contents/.github/workflows/release-monthly-close-control-plane.yml?ref=$release_commit" \
  | sed -n 's#.*release-policy/\.github/workflows/[a-z-]*\.yml@\([0-9a-f]\{40\}\).*#\1#p')"
test -n "$policy_sha"
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
```

If any gate fails, inspect it before touching the tag or draft. Never move a
published tag. It behaves like a boulder in a corridor: once it is rolling the
only direction is forward, so cut a new version rather than try to get behind it.
