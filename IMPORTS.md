# Import manifest

Captured: 2026-09-02 (Australia/Sydney).

This repository is assembled from freshly fetched default branches. "Git tree" is the
commit's `HEAD^{tree}` object. "Tracked-tree SHA-256" is SHA-256 over the exact
NUL-delimited bytes of `git ls-tree -r --full-tree -z HEAD`. Historical tags remain
authoritative in the source repositories; none is copied here.

## Path decision (owner decision D1)

The migration plan's path table named `packages/monthly-close-controls`,
`packages/workpaper-review-gate` and `packages/xero-ledger-review-gate`. The reviewed
Release Policy identity gate (`gate_component_identity` in `scripts/gates.sh` at
`6ad53a7b030da22fc299cee704c37ba7550ea1d7`) requires a nested release's directory leaf,
`tag-prefix` and normalised distribution name (or `artifact-stem`) to be identical. Those
plan paths cannot satisfy it for the distributions `monthly-close-control-plane`,
`review-ready-gate` and `elizabeth-anne-alexander`, so the coordinator directed
distribution-named directories:

| Plan path | Actual directory | Reason |
|---|---|---|
| `packages/monthly-close-controls/` | `packages/monthly-close-control-plane/` | distribution `monthly-close-control-plane` |
| `packages/workpaper-review-gate/` | `packages/review-ready-gate/` | distribution `review-ready-gate` |
| `packages/xero-ledger-review-gate/` | `packages/elizabeth-anne-alexander/` | distribution `elizabeth-anne-alexander` |
| `packages/xero-trial-balance-export/` | unchanged | archive stem `xero-trial-balance-export` |
| `adapters/accounting-excel-toolkit/` | unchanged | archive stem `accounting-excel-toolkit` |
| `apps/australian-accounting-power-bi/` | unchanged | no release |

No distribution name, import package, command or version changed.

## Source identities

| Component | Source and selected commit | Git tree | Tracked-tree SHA-256 | Latest release | Destination |
|---|---|---|---|---|---|
| Monthly Close Controls (anchor) | `https://github.com/ryanduguid/monthly-close-controls.git` at `06a9bfec0d52baceb3a15f1d5ec5afde8850df42` | `96b3fea5234a891d2f3d53f4dfca646436463235` | `883a499040ad2b413e75a348e211bb58f4ca4107f0d2a415f0143b37e0f33e8b` | `v0.1.2` | `packages/monthly-close-control-plane/` |
| Xero Trial Balance Export | `https://github.com/ryanduguid/xero-trial-balance-export.git` at `2a0966e89e5f8daa587be8466f988d9adc16003a` | `0b35ee4de1a71a5cb04fa00e27a41c130f1a7563` | `72d773eafccf558d832ab7b7df2604d3a6f7a8d1fd06d6da3acceb1f7c2e623b` | `v0.1.4` | `packages/xero-trial-balance-export/` |
| Workpaper Review Gate | `https://github.com/ryanduguid/workpaper-review-gate.git` at `e2a01292b9782dc086595865bb80516c81fcb70e` | `f5fe55d4a2fcd5b49952d3de2583a863aa4dad45` | `851eccd592fa0edcb60becc8087f37f236cfe030867e662933336cda634e72cc` | `v0.1.1` | `packages/review-ready-gate/` |
| Xero Ledger Review Gate | `https://github.com/ryanduguid/xero-ledger-review-gate.git` at `a3df72bfefc94c2a4b5e6fa01fe54aec21200d1f` | `82ae98af57db154c19556f7bfe0eeeaad77bdf60` | `679976f85bb5731782d68e527f5fa3cd9680744bb46a373393f1f8cbec5efb83` | `v0.2.1` | `packages/elizabeth-anne-alexander/` |
| Accounting Excel Toolkit | `https://github.com/ryanduguid/accounting-excel-toolkit.git` at `5c2a779d316d2e0338f2189f3c98f0add4e7cbea` | `25af282d275b3e03c93bb23eab05144453d8d89b` | `0afbb0584b8f80444880ccc5d0b19c5c29d98294e9c8adae5a9a99e9dde28ad7` | `v0.1.5` | `adapters/accounting-excel-toolkit/` |
| Australian Accounting Power BI | `https://github.com/ryanduguid/australian-accounting-power-bi.git` at `9aab858e5e099b44ae828ac4793ca47d25cf5675` | `37db7e334d30fd7a8c1573967f9d11b5fe7c82cc` | `6fa498f69b8c0af07c5f3b6d7c8362b482eeaadd582e7dffaf9be318bb6f41ad` | no GitHub release | `apps/australian-accounting-power-bi/` |

Every source default branch is `main`. Each clone was clean at the recorded commit.

## Fresh-head reconciliation

The plan's audited references were compared with the fetched heads before selection:

- Monthly Close Controls advanced from `f31572d6a5a7a8897b169e9b5e12fa3f24cfa7f0` by one commit, "Use pinned setup-uv action (#76)", touching `.github/workflows/ci.yml` and one line of `AGENTS.md`.
- Xero Trial Balance Export advanced from `ffffd05a9da2965075d409b8f8c986f1a8458c0b` by two commits, "Add reproducible trial balance proof (#73)" and "Add a reusable Power Query sample for Power BI (#75)"; the exporter-owned `xero-tb-csv.v1` corpus did not change.
- Workpaper Review Gate advanced from `6c733e27e4db027176775fdf30f399dd1144a254` by two commits, "Use pinned setup-uv action (#15)" and "Name the canonical host and drop the em dashes from the README (#16)"; the distribution stays `review-ready-gate` 0.1.2.
- Xero Ledger Review Gate advanced from `562e6c6204a558b9161a83e9bacae338a4e02115` by one commit, "Use pinned setup-uv action (#61)".
- Accounting Excel Toolkit still equals the audited `5c2a779d316d2e0338f2189f3c98f0add4e7cbea`.
- Australian Accounting Power BI advanced from `22029eb8b0c7f5df887ef026ae599a7883078ed2` by two commits, "Add Dependabot for GitHub Actions (#9)" and "build(deps): bump actions/setup-node from 6.5.0 to 7.0.0 (#10)".

The fetched heads above are the selected import snapshots.

## Component identities and boundaries

| Component | Public identity | Lock and verification | Licence and publisher boundary |
|---|---|---|---|
| Monthly Close Controls | distribution `monthly-close-control-plane` 0.1.2; import `closecontrol`; commands `close-control`, `openaccountants-au`; version in `pyproject.toml` | `uv.lock`; pytest, build, Ruff, mypy, clean-wheel smoke | MIT; PyPI environment `pypi`; local, offline review package |
| Xero Trial Balance Export | distribution `xero-trial-balance-export` 0.1.4; modules `export_tb`, `xero_client`, `auth`, `token_store`; commands `export-tb`, `xero-tb-auth`; version in `VERSION` | hash-locked `requirements.lock`; unittest, Ruff, mypy | MIT; GitHub source-archive release with `artifact-stem: xero-trial-balance-export`; the only component allowed OAuth, Xero API, HTTP, tokens or Xero credentials |
| Workpaper Review Gate | distribution `review-ready-gate` 0.1.2; import `reviewready`; command `review-ready`; version in `pyproject.toml` | `uv.lock`; pytest, build, wheel smoke, `uv lock --check`, Ruff, mypy | MIT; PyPI; local, offline and fabricated-only |
| Xero Ledger Review Gate | distribution `elizabeth-anne-alexander` 0.2.1; import `elizabeth_anne_alexander`; command `elizabeth-anne-alexander`; version in `elizabeth_anne_alexander/version.py` (`version-parser: python-literal`) | `uv.lock`; pytest, build, wheel demo, Ruff, mypy | MIT; PyPI; zero-network synthetic demonstration |
| Accounting Excel Toolkit | source adapter 0.1.5; version in `VERSION`; Power Query and VBA source, no Python distribution | pinned actionlint and ShellCheck; unittest; optional native Excel acceptance | MIT; GitHub source-archive release with `artifact-stem: accounting-excel-toolkit`; local-file adapters only |
| Australian Accounting Power BI | PBIP/PBIR reference application; no release and no version | unittest plus Microsoft Power BI report-authoring CLI 0.1.4 | MIT; no publisher; fabricated local model and report |

Imported workflows remain nested under their component paths and are not active root
workflows. They were read before any component command ran. Review jobs receive no Xero,
OAuth or publishing credentials. Production packages do not import sibling packages.

## Anchor remote preflight (read-only)

The anchor remains public, active and named `monthly-close-controls`; its default branch is
`main`. Branch protection on `main` requires the status checks `package`, `test (3.10)`,
`test (3.11)`, `test (3.12)`, `test (3.13)` and `Analyze Python` with strict up-to-date
enforcement, so the root `ci.yml` keeps the workflow name `tests` with job ids `test`,
`package` and `lint`, and `codeql.yml` keeps its job name `Analyze Python`. The repository
has exactly one environment, `pypi`. The latest anchor release is `v0.1.2`. No remote
setting was changed.

## Release Policy prerequisite

The reviewed Release Policy extension (`source-directory`, `tag-prefix`, `version-parser`,
`version-file` and `upload-dist-artifact` inputs) is the independently approved commit
`6ad53a7b030da22fc299cee704c37ba7550ea1d7`. At assembly time it exists only as a local
commit in the coordinator's `release-policy` clone; remote `ryanduguid/release-policy` main
was observed at `5707d96f6fb0f5ba368df34a3500b54299c2ec44`. Every root release caller pins
exactly `6ad53a7b030da22fc299cee704c37ba7550ea1d7`; the callers cannot run until that commit
is pushed, which is expected.

## Import records

Each `git subtree add --squash` record is appended immediately after its local import
commits are created. The imported source commit and tree must continue to match the source
table above.
- Xero Trial Balance Export: source `2a0966e89e5f8daa587be8466f988d9adc16003a`; squash commit `cf537fe42493406fa0147e9274b1c77d88aaec9a`; subtree merge commit `fe5192336da3d0ccc52aa169bdec86e8fb141095`; destination `packages/xero-trial-balance-export/`; imported tree `0b35ee4de1a71a5cb04fa00e27a41c130f1a7563` equals the source tree.
- Workpaper Review Gate: source `e2a01292b9782dc086595865bb80516c81fcb70e`; squash commit `6e2db45c6c6dad68987a783886e0f2e00d415d24`; subtree merge commit `3063b1df80418f4b4672acc85703720be1eecace`; destination `packages/review-ready-gate/` (distribution name, owner decision D1); imported tree `f5fe55d4a2fcd5b49952d3de2583a863aa4dad45` equals the source tree.
- Xero Ledger Review Gate: source `a3df72bfefc94c2a4b5e6fa01fe54aec21200d1f`; squash commit `de7d3bab8156ec7678ad6f6d9ef6f383104a5058`; subtree merge commit `8841889c8a91508a4d81ec6835ff35fc2d903c9e`; destination `packages/elizabeth-anne-alexander/` (distribution name, owner decision D1); imported tree `82ae98af57db154c19556f7bfe0eeeaad77bdf60` equals the source tree.
