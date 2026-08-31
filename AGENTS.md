# Agent instructions

This repository is a local, deterministic review-pack generator. Preserve these
accounting and human-review boundaries:

- Preserve exactly the `PASS`, `REVIEW`, and `BLOCKED` pack states. For `review` and
  `workbench`, exit `0` only for `PASS`, exit `2` for `REVIEW` or `BLOCKED`, and exit
  `1` for malformed input, invalid command configuration or an unwritable output.
  The read-only `view` command exits `0` after verified display and `1` on verification
  failure.
- An acknowledgement records a human action only. It never changes a control status,
  approves or signs off a close, or proves that a period was closed.
- Keep client source files, workpapers, review notes and generated packs in a separate,
  access-controlled directory outside the checkout. Repository fixtures must remain
  fabricated.
- Parse and calculate money, balances, thresholds and tolerances with exact `Decimal`
  arithmetic, never binary floating point. Preserve fail-closed schema and integrity gates.
- Do not add network or live Xero access, credential or token handling, journal, payment
  or report mutation, approval or sign-off authority, period locking, or tax lodgement.
- Route release work through [RELEASING.md](RELEASING.md) and the existing GitHub Actions
  workflows. Never build or upload release assets by hand, and do not tag or publish
  without explicit action-time approval.

## Repository map

- [README.md](README.md) owns the review-pack, status and exit-code contracts.
- [CONTRIBUTING.md](CONTRIBUTING.md) owns fixture, data-handling and pull-request rules.
- [RELEASING.md](RELEASING.md) owns release preflight, tagging and verification.

## CI gates

The fenced list records the unique single-line commands in
`.github/workflows/ci.yml`. The multiline package-smoke gate is explained and
matched semantically below without duplicating its shell body:

```bash
python -m pip install "uv==0.12.0"
uv run --locked --extra dev pytest -q
uv run --locked --extra dev --python 3.12 python -m build
uv run --locked --extra dev ruff check closecontrol tests
uv run --locked --extra dev mypy closecontrol
```

## Package smoke outside the checkout

The CI smoke uses `/tmp`; on Windows use separate fresh system temporary artifact
and smoke directories. Fail immediately if a native build or install command fails,
install only the one wheel produced by that build, and always restore the caller's
location and remove both temporary directories. The fabricated demo deliberately
returns `REVIEW` exit `2`, which is the accepted smoke result. This proves the
installed wheel, not a checkout import, and does not publish anything:

```powershell
$ErrorActionPreference = "Stop"
$repoRoot = (Get-Location).Path
$artifactDir = Join-Path ([System.IO.Path]::GetTempPath()) ("monthly-close-wheel-build-" + [guid]::NewGuid().ToString("N"))
$smokeDir = Join-Path ([System.IO.Path]::GetTempPath()) ("monthly-close-wheel-smoke-" + [guid]::NewGuid().ToString("N"))
try {
    New-Item -ItemType Directory -Path $artifactDir | Out-Null
    uv run --locked --extra dev --python 3.12 python -m build --outdir "$artifactDir"
    if ($LASTEXITCODE -ne 0) { throw "wheel build failed with exit $LASTEXITCODE" }
    $wheels = @(Get-ChildItem -LiteralPath $artifactDir -Filter "*.whl" -File)
    if ($wheels.Count -ne 1) { throw "expected exactly one built wheel" }
    $wheel = $wheels[0].FullName
    python -m venv "$smokeDir\venv"
    if ($LASTEXITCODE -ne 0) { throw "venv creation failed with exit $LASTEXITCODE" }
    & "$smokeDir\venv\Scripts\python.exe" -m pip install --no-index "$wheel"
    if ($LASTEXITCODE -ne 0) { throw "wheel install failed with exit $LASTEXITCODE" }
    Push-Location $smokeDir
    try {
        & "$smokeDir\venv\Scripts\close-control.exe" review --current "$repoRoot\examples\current_trial_balance.csv" --prior "$repoRoot\examples\prior_trial_balance.csv" --output pack
        if ($LASTEXITCODE -ne 2) { throw "expected REVIEW exit 2, got $LASTEXITCODE" }
        if (-not (Test-Path "pack\close-review-pack.json")) { throw "smoke pack missing" }
    }
    finally {
        Pop-Location
    }
}
finally {
    Remove-Item -LiteralPath $smokeDir -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $artifactDir -Recurse -Force -ErrorAction SilentlyContinue
}
```
