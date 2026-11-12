# Releasing

Version 1.0.0 onwards ships the native `review-ready` executable on an immutable
GitHub Release. The root caller
[release-review-ready-gate.yml](../../.github/workflows/release-review-ready-gate.yml)
calls Release Policy's `release-binary.yml` at a pinned commit on an annotated
`review-ready-gate/vMAJOR.MINOR.PATCH` tag. Nothing is published to PyPI or crates.io
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
    cargo build --locked --release --bin review-ready
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
    git tag -a review-ready-gate/v1.0.0 -m "review-ready-gate/v1.0.0" origin/main
    git push origin review-ready-gate/v1.0.0
    ```

The workflow runs `cargo test --locked` and builds `review-ready` with
`cargo-auditable` on each target, checks static linkage and `review-ready --version`,
audits the embedded dependency list against RustSec, then publishes 9 assets:

| Asset | Contents |
| --- | --- |
| `review-ready-gate-<version>.zip`, `.tar.gz` | deterministic source archives |
| `review-ready-gate-<version>.spdx.json` | SPDX SBOM of the source |
| `review-ready-gate-<version>-x86_64-unknown-linux-musl.tar.gz` | static Linux `review-ready` with `LICENSE`, `NOTICE` and `README.md` |
| `review-ready-gate-<version>-x86_64-unknown-linux-musl.spdx.json` | SBOM of the Linux executable |
| `review-ready-gate-<version>-x86_64-pc-windows-msvc.zip` | Windows `review-ready.exe` with the same documents |
| `review-ready-gate-<version>-x86_64-pc-windows-msvc.spdx.json` | SBOM of the Windows executable |
| `SHA256SUMS` | digests of the 7 assets above |
| `review-ready-gate-<version>.intoto.jsonl` | signed provenance bundle |

If any gate fails, inspect it before touching the tag or draft. An existing
release is never replaced; cut a new version instead.

## Verify a release

Download an explicit tag and target, never `latest`. The signer digest comes
from the caller at the tagged commit, so the recipe stays right when the pin moves:

```bash
tag=review-ready-gate/v1.0.0
repo=ryanduguid/accounting-review-pipeline
version="${tag#review-ready-gate/v}"
asset="review-ready-gate-${version}-x86_64-unknown-linux-musl.tar.gz"  # or -x86_64-pc-windows-msvc.zip
release_commit="$(git ls-remote "https://github.com/$repo.git" "refs/tags/$tag^{}" | cut -f1)"
test -n "$release_commit"
policy_sha="$(gh api -H "Accept: application/vnd.github.raw+json" \
  "repos/$repo/contents/.github/workflows/release-review-ready-gate.yml?ref=$release_commit" \
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

A `READY` pack is a review aid. It does not approve a client file or authorise a
software release.

## Historical Python releases

Version 1.0.0 replaced the Python distribution with the `review-ready` Rust
binary. The Python tags, PyPI artefacts and release history remain historical
records. Do not move or reuse them. Standalone releases through `v0.1.1` came
from the retired `workpaper-review-gate` repository; namespaced Python releases
came from Accounting Review Pipeline. The protected `v0.1.0` tag failed before
publication and must not be reused. Once published, the native 1.0.0 release
supersedes 0.3.0, the last Python release on PyPI.
