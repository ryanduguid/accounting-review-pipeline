# Accounting Review Pipeline

Local monorepo assembly anchored in Monthly Close Controls. The repository has not been
renamed on GitHub; it remains `ryanduguid/monthly-close-controls` until a separately
authorised cutover. It holds six independently versioned components joined only by local
files and commands:

| Component | Directory | Identity | Version |
|---|---|---|---|
| Xero Trial Balance Export | `packages/xero-trial-balance-export/` | distribution `xero-trial-balance-export`, commands `export-tb` and `xero-tb-auth`; the only OAuth, Xero and network producer | 0.1.4 |
| Workpaper Review Gate | `packages/review-ready-gate/` | distribution `review-ready-gate`, import `reviewready`, command `review-ready` | 0.1.2 |
| Monthly Close Controls | `packages/monthly-close-control-plane/` | distribution `monthly-close-control-plane`, import `closecontrol`, commands `close-control` and `openaccountants-au` | 0.1.2 |
| Xero Ledger Review Gate | `packages/elizabeth-anne-alexander/` | distribution `elizabeth-anne-alexander`, import `elizabeth_anne_alexander`, command `elizabeth-anne-alexander` | 0.2.1 |
| Accounting Excel Toolkit | `adapters/accounting-excel-toolkit/` | source-archive adapter `accounting-excel-toolkit` (Power Query and VBA) | 0.1.5 |
| Australian Accounting Power BI | `apps/australian-accounting-power-bi/` | PBIP reference application, no release | none |

Data flows in one direction: the exporter (or a manual Excel export) produces the ten-column
Xero trial-balance file, the readiness gate decides whether a pack reaches review, monthly
close surfaces exceptions, and the ledger-review boundary or Power BI consumes the result.
Only the exporter may touch OAuth, Xero, HTTP or credentials. Every other component is
offline, exact-Decimal and fabricated-data-only.

Directory names follow each component's normalised distribution name so that the reviewed
Release Policy identity gate (directory leaf, `tag-prefix` and distribution name must agree)
can release one component per namespaced tag. That is why Monthly Close Controls lives at
`packages/monthly-close-control-plane/` rather than the migration plan's
`packages/monthly-close-controls/`; see `AGENTS.md` and `IMPORTS.md`.

Each component keeps its own package identity, version, lockfile, licence, commands,
documentation and release cadence. There is no root runtime package, shared library or
unified version. Run checks from the owning component directory with its documented
commands. Only the root `.github/workflows/` are active; nested `.github/` directories are
inert records of the imported sources. `IMPORTS.md` records source identities, tree digests
and import records. Historical releases and tags remain owned by the source repositories.

## Releases

Each component releases on its own namespaced annotated tag, `<component>/vMAJOR.MINOR.PATCH`,
through a root caller pinned to the independently reviewed Release Policy commit
`6ad53a7b030da22fc299cee704c37ba7550ea1d7`: `monthly-close-control-plane/v*`,
`review-ready-gate/v*`, `elizabeth-anne-alexander/v*`, `xero-trial-balance-export/v*` and
`accounting-excel-toolkit/v*`. One tag publishes exactly one component; the identity gate
refuses a tag whose prefix does not equal the component directory leaf and its distribution
name or archive stem. Each component's `RELEASING.md` describes its preflight; the tag name
is the only difference. `IMPORTS.md` lists the callers and publisher environments.
