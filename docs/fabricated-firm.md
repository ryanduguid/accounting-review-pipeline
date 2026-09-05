# The Fabricated Firm: design

Design only, 5 September 2026. No dataset is generated until the owner
approves this document. It covers audit items 16 (inventory) and 17 (design).

Governance note: root `CONTRIBUTING.md` scopes a change to one component or
to the data-only contract `contracts/xero-trial-balance-v1/`. This document
fits neither, and the policy grants no exception for root documentation, so
merging it is the owner's explicit call (decision 7). The dataset it
proposes would be a second data-only contract directory, which the rule does
not cover yet either: `CONTRIBUTING.md`, `AGENTS.md` and
`joined-conformance.yml` all name the v1 directory literally and would be
amended in the change that creates the firm (section 3).

## 1. Inventory of fabricated entities today

Every name in the tables is invented. Two places name real parties and are
deliberately not repeated here: the `progress-claim-preparation` skill cites
a NSW Supreme Court security-of-payment judgment by its parties, which is a
citation, not a fixture; and `FireFalcon/examples/arb/` is a worked example
built from a listed company's published Appendix 4E figures. That example is
outside the firm and stays or goes on the owner's call; it is not a
fabricated entity.

| Entity | Files | Facts asserted | Consumers |
| --- | --- | --- | --- |
| Cedar and Pine Consulting Pty Ltd | `australian-accounting-skills/assets/readme/bas-workpaper-synthetic.svg` and `README.md` (preview); `packages/review-ready-gate/examples/bas-ready` and `bas-blocked` (`trial_balance.csv`, `activity_statement.csv`, `gst_control_gl.csv`, `open_items.csv`, `self_review.json`) and `bas-not-ready` (`prior_findings.csv` in place of `gst_control_gl.csv`); `tests/test_loader.py`, `tests/test_engine.py` | Two fact sets that do not agree. The preview: consulting company, cash basis, quarterly BAS, quarter ended 31 March 2026, identifier `SYNTH-0001`, ABN withheld, G1 48,400.00, 1A 4,400.00, 1B 1,210.00, net 3,190.00 tied to the GST control account, two carried exceptions. The gate examples: 1A 10,000.00, 1B 2,500.00 and a seven-account trial balance for the same quarter | Skills README preview; Workpaper Review Gate examples and tests; the site's refusals page (pending on site PR #69) cites the `bas-not-ready` and `bas-blocked` runs at `review-ready-gate/v0.1.3` |
| Harbour Light Pty Ltd | `FireFalcon/examples/harbour-light/` (`data/xero_pl.csv`, `xero_bs.csv`, `xero_pl_tracking.csv`, `harbour_model.py`, `run_harbour.py`, `.fpa/` intake, profile, source and mapping registries, `output/briefing.md`); `FireFalcon/tests/test_harbour_light.py` | Victorian lighting wholesaler, FY2027, AUD, quarterly BAS; North and South tracking (monthly GST-exclusive sales of 70,000 North, being 58,000 domestic and 12,000 GST-free, and 41,000 South); five VIC roles, 499,200 gross, super at 12 per cent, workers compensation 2 per cent, payroll tax nil under the Victorian threshold; opening cash 85,420; 13-week window from 1 October 2026 | FireFalcon pipeline and nine tests; `model_to_excel` verification |
| CivilCo and HaulCo (Group) | `PaciolisCube/model/dimensions/Entity.hierarchies/Entity.json`, `model/cubes/PnL.rules`, `examples/capex.csv`, `drivers.csv`, `pnl-direct.csv`, `revenue.csv`, `workforce.csv`, `docs/model-assumptions.md`, four test files | Two operating entities of a mining-services group rolling up to `Group`; FY2026-27 budget; CivilCo (civil earthworks) is the NSW group employer for payroll tax; SG rate 0.12 and maximum contribution base 270,830 as drivers; headcount, rates, fleet capex and asset lives per cost centre | PaciolisCube engine, CLI and CI recomputation |
| Varrock group (Varrock Ventures Pty Ltd, Draynor Produce Pty Ltd, Falador Freight Pty Ltd, Ardougne Holdings Trust) | `apps/australian-accounting-power-bi/samples/` (`sample-entities.csv`, `sample-chart-of-accounts.csv`, `sample-general-ledger.csv`, `sample-budgets.csv`, `sample-payroll-super.csv`, `sample-ato-benchmarks.csv`), `tools/generate_fixtures.py`, `tests/test_fixtures_balance.py`; Varrock Ventures is also the tenant in `packages/monthly-close-control-plane/examples/` and in Workpaper Review Gate `month-end-ready` and `year-end-ready` | ENT001 to ENT004 with checksum-valid ABNs, ACNs, tax structure, role and ANZSIC code; 30-account chart; balanced journals from 1 July 2024 with intercompany management fees and freight; monthly budgets; payroll events with remittance, fund receipt and statutory due dates and `ON_TIME` or `LATE_BREACH` status; opening balances per entity | Power BI model and tests (balance, intercompany, byte-for-byte regeneration); Monthly Close Controls examples; Workpaper Review Gate examples |
| Catherby Fisheries Pty Ltd | `contracts/xero-trial-balance-v1/fixtures/` (`passing.csv`, `failing_movement.csv`, `failing_ytd.csv`), `expected_results.json`, `SHA256SUMS`; `packages/xero-trial-balance-export/samples/sample-output.csv`, `assets/quick-proof.*`, exporter tests | Report date 30 June 2026, GUID `AccountID`s and text `AccountCode` (`090`); two accounts in the contract fixtures and ten in the exporter sample; movement 1,200.00 and YTD 15,234.50, one cent breaks in the two failing files; exit codes and output markers per scenario | Exporter evaluation and quick proof; conformance tests in all three review packages, the Excel adapter, Power BI and the root joined test; the site's exporter page and refusals page |

Smaller fixtures that a canonical group would also replace or leave:

| Name | Where | Note |
| --- | --- | --- |
| Demo Entity Pty Ltd | `packages/elizabeth-anne-alexander/.../samples/inputs/sample-tb-*.csv`, its tests | Two-month trial balances in the v1 header |
| Yanille Trading Pty Ltd | `adapters/accounting-excel-toolkit/samples/` and `tools/native_excel_acceptance.ps1` | Xero report-shaped CSVs (title rows above the table) that Power Query parses |
| Example Principal Pty Ltd | `australian-accounting/packages/the-wip-tally/examples/sample_contracts.csv` | Customer on the sample contract register |
| Acme, Acme Holdings, HoldingCo, the bakery | exporter README, an `export_tb.py` docstring and tests, The Exchequer Tally tests, MCP tests, ATO benchmark example | Unit-test and docstring strings and a single-purpose example; not group members |

## 2. The canonical group

Reuse the Varrock group. It already has the richest facts (ABNs that pass
the modulus 89 check, ANZSIC codes, a chart, balanced journals, budgets and
payroll events) and it is already the tenant in two review packages. Add one
member and three columns:

| ID | Entity | Role | State | GST | Carries |
| --- | --- | --- | --- | --- | --- |
| ENT001 | Varrock Ventures Pty Ltd | Parent, advisory | NSW | cash, quarterly | The BAS pack story that Cedar and Pine tells today |
| ENT002 | Draynor Produce Pty Ltd | Trading subsidiary, wholesale | VIC | accruals, quarterly | The Harbour Light story: North and South tracking, five Victorian roles, nil payroll tax |
| ENT003 | Falador Freight Pty Ltd | Logistics subsidiary | NSW | accruals, monthly | HaulCo: fleet, fuel, haulage rates |
| ENT004 | Ardougne Holdings Trust | Property unit trust | NSW | accruals, quarterly | Rent to the group, no employees |
| ENT005 | Lumbridge Civil Pty Ltd (new) | Civil earthworks for mining services | NSW | accruals, monthly | CivilCo: the contract register, WIP schedule, progress claims, retentions, Coal LSL and fuel tax credit inputs; NSW group employer for payroll tax |

New entity-master columns: `State`, `GSTBasis`, `BASCycle`. The Power BI
`Dim_Entity` expression declares `Columns=10` and
`test_powerquery_m.py` holds every sample row to that width, so the
expression and the table definition change with the master. Place names
follow the existing convention.
Customer, supplier and employee names stay obviously synthetic.

## 3. The spine and the layout

`contracts/xero-trial-balance-v1/` stays byte-identical: its digests are
cited in exporter release evidence and `expected_results.json` names product
release `v0.1.6`. The firm is a second data-only contract that reuses the v1
header for every trial balance and the v1 rules (exact `Decimal`, movement
and YTD both balance, `Tenant` plus `AccountID` as identity, `AccountCode` as
text):

```text
contracts/fabricated-firm-v1/
  README.md                 what it is, what it is not, how to regenerate
  firm.json                 manifest: dataset_version, generator commit, entity list,
                            period coverage, file list with roles, consumers
  SHA256SUMS                every file above and below except itself
  entities.csv              Power BI Dim_Entity shape plus State, GSTBasis, BASCycle
  chart-of-accounts.csv     Power BI shape, one chart for the group
  general-ledger.csv        Power BI Fact_GeneralLedger shape; the single source
  budgets.csv               Power BI shape
  payroll-register.csv      Power BI Fact_PayrollSuper shape (pay date, remittance,
                            fund receipt, due date per event)
  contract-register.csv     The WIP Tally input columns, ENT005 only
  progress-claims.csv       claim, certified, uncertified, retention per contract per month
  trial-balances/ENT00n/YYYY-MM-DD.csv   v1 header, one per entity per month end
  packs/bas/ENT00n/YYYY-MM-DD/           Workpaper Review Gate BAS pack shape, with
                            prior_findings.csv carried from the prior quarter
  packs/close/ENT00n/YYYY-MM-DD/         Monthly Close Controls input shape
  xero-reports/ENT00n/YYYY-MM-DD/        Xero report-shaped CSVs for the Excel adapter
  scenarios/<id>/           overlay files plus expected outcome, one per refusal
  expected_results.json     the outcome every consumer must reproduce
```

Creating the directory also amends the three files that name the v1
directory literally: the scope rule in `CONTRIBUTING.md`, the contract rule
in `AGENTS.md`, and `joined-conformance.yml`, which gains the new path in
its triggers and a second `sha256sum --check --strict` step.

The generator is a stdlib-only script at the repository root,
`tools/fabricated_firm.py`, beside the root `tests/`: not a package, not
imported by any component, so the contract directory stays data-only and no
shared runtime package appears. The Power BI byte-for-byte test then
compares its samples with the contract copies instead of importing a
generator.

Identity: `AccountID` is a GUID-shaped deterministic string,
`00000000-0000-0000-000n-000000000ccc` for entity `n` and account code
`ccc`, following the Catherby pattern, so the v1 identity rule holds without
inventing real Xero identifiers. `Tenant` is the legal name.

Coverage: 1 July 2024 to 30 June 2027, as the Power BI ledger already runs,
with the walk-through anchored at 30 September 2026 so that Payday Super
events fall after the 1 to 28 July 2026 transition window that makes the
checker stop for confirmation.

## 4. Cross-consistency rules

One generator, one seed, one ledger. Every other file is derived from
`general-ledger.csv` and the two registers, so agreement is by construction
and a test proves it:

1. Each trial balance equals the ledger aggregated by entity and account:
   movement for the month, YTD from 1 July. Both pairs balance.
2. Payroll: wages, PAYG withheld and super expense posted per pay run equal
   the payroll register per entity per month; super payable clears on each
   `RemittanceDate`; `FundReceiptDate` drives the Payday Super verdicts.
3. BAS: `activity_statement.csv` labels are computed from the GST control
   accounts and revenue accounts for the quarter on the entity's basis;
   `gst_control_gl.csv` is the ledger detail for those accounts; the pack's
   `open_items.csv`, `prior_findings.csv` and `self_review.json` are
   authored per pack, and `prior_findings.csv` is what makes a repeat
   finding `NOT_READY`.
4. Contracts: the WIP schedule the engine computes from
   `contract-register.csv` at each month end equals the contract asset,
   contract liability, retention and revenue balances in ENT005's trial
   balance; certified billings equal ledger billings.
5. Intercompany lines net to zero across the group (the Power BI test).
6. Budgets for ENT003 and ENT005 may later be pinned to PaciolisCube's
   computed budget by hash; that coupling is optional and separate.

Versioning and hashes follow the v1 contract: the directory leaf carries the
schema major; `firm.json` carries `dataset_version` (integer, bumped on any
byte change) and the generator commit; `SHA256SUMS` is checked with
`sha256sum --check --strict` in the amended joined conformance workflow; a test
regenerates the dataset from the generator and compares bytes, as the Power
BI fixtures do today. A published version is never edited in place.

## 5. The walk-through it enables

All local, all fabricated, one entity at a time, in the order the pipeline
runs:

1. Export. The exporter's balance check runs offline over each
   `trial-balances/` file, as the evaluation pack does, and accepts every
   one. No Xero call: the tenant does not exist.
2. Gate. `review-ready gate --profile bas --pack packs/bas/ENT001/2026-09-30
   --output out` returns `READY`; the `scenarios/` overlays reproduce
   `NOT_READY` (a repeat finding from `prior_findings.csv` and a missing
   artefact) and `BLOCKED` with the same status lines the refusals page
   shows.
3. Close. Monthly Close Controls over ENT001 September 2026 returns `REVIEW`
   with its exceptions listed (exit 2; the engine returns `PASS` only when
   there is no exception); an overlay with one debit raised by 100.00
   returns `BLOCKED` on `trial_balance_integrity`.
4. BAS skill. `bas-preparation` takes the ENT001 pack, the prior quarter's
   labels and the basis, and produces the workpaper the skills README
   previews today. The firm adopts the computed label set, so the preview is
   regenerated from it and the two Cedar and Pine fact sets collapse into
   one.
5. Payroll. `payday-super-check payroll-register.csv --as-at 2026-09-30`
   with a mapping file gives the verdict counts recorded in
   `expected_results.json`; `month-end-close` step 2 reconciles the same
   register to the PAYG and super payable balances.
6. WIP schedule. `wip-tally schedule contract-register.csv --as-at
   2026-09-30` produces the schedule whose totals tie to ENT005's trial
   balance; `wip-over-under-billing` reviews it against the ledger.
7. Power BI. The six sample files are byte copies of the firm files with
   the same headers; after the `Dim_Entity` width change in section 2 the
   structural, balance and intercompany tests pass, the regeneration test
   becomes the drift test against the contract copies, and the report shows
   the five entities with eliminations.
8. Excel adapter and FireFalcon. The Xero report-shaped CSVs feed the Power
   Query import; ENT002's July 2026 profit and loss and balance sheet feed
   the FireFalcon pipeline in place of Harbour Light.

## 6. Migration path per existing entity

| Today | Becomes | How | Constraint |
| --- | --- | --- | --- |
| Cedar and Pine Consulting | ENT001 Varrock Ventures | Regenerate the skills README preview from the ENT001 September 2026 pack; add firm-based Workpaper Review Gate examples beside the existing three | The existing `bas-*` examples are pinned by `test_loader.py`, `test_engine.py` and the site's refusals page at `v0.1.3`; remove them only in a later minor release, in their own pull request |
| Harbour Light | ENT002 Draynor Produce, VIC | Regenerate `data/xero_pl.csv`, `xero_bs.csv` and `xero_pl_tracking.csv` from the ENT002 July 2026 ledger; rename the example directory, intake, profile and registries; update the nine tests to the firm figures | FireFalcon is source-only, so no distribution identity changes; keep the 13-week window and the 28 October BAS due date |
| CivilCo and HaulCo | ENT005 Lumbridge Civil and ENT003 Falador Freight | Rename the `Entity` dimension elements and every rule and example row; recompute; `Group` stays | The model is recomputed in CI, so every expected figure moves with the rename in one change |
| Varrock group (Power BI) | The base of the firm | Retire `generate_fixtures.py` in favour of the root generator; the six samples become copies with a drift test against `contracts/fabricated-firm-v1/`; add ENT005 and the three columns; update the `Dim_Entity` width | `Fx_ValidateABN` must keep passing, so ENT005 needs a checksum-valid ABN |
| Varrock Ventures (Monthly Close, Workpaper Review Gate month-end and year-end) | ENT001 | Add firm-based examples beside the existing ones | The Monthly Close README pins `REVIEW; 8 exception(s)` and the refusals page pins `BLOCKED; 10 exception(s)` with its totals at `monthly-close-control-plane/v0.1.3`, so the existing examples go only in a later minor release, as for the `bas-*` examples above; exit codes and status vocabularies never change |
| Catherby Fisheries | Unchanged | v1 stays the byte-pinned integrity corpus | Its digests are release evidence |
| Demo Entity, Yanille Trading | ENT001 | Regenerate from the firm trial balances and Xero report shapes | The ledger-review gate's tests pin its sample outcomes; update them in the same change |
| Example Principal, Acme, HoldingCo, the bakery | Left alone | Unit-test strings and single-purpose examples |  |

## 7. Decisions for the owner

1. The fifth entity's name (`Lumbridge Civil Pty Ltd` proposed) and the
   state and basis columns in section 2.
2. Home: `contracts/fabricated-firm-v1/` in this repository, or a
   repository of its own. The contract directory keeps the governance
   rule intact and puts the hashes under the joined conformance workflow.
3. Whether the five ABNs are checked by hand against ABR Lookup so that no
   synthetic checksum-valid number coincides with a registered one. The
   Power BI fixtures carry the same exposure today; the site withholds ABNs.
4. Whether the PaciolisCube budget coupling in rule 6 is in scope.
5. Coverage from 1 July 2024 (three years, matching the ledger) or FY2027
   only (smaller, but no prior-period comparisons for the close and BAS).
6. The generator's home: the root `tools/fabricated_firm.py` proposed in
   section 3, or inside the Power BI component, which would make one
   component the author of a contract three review packages consume.
7. Whether this document may merge as a root document at all, or whether
   `CONTRIBUTING.md` is first amended to admit root documentation, since the
   scope rule as written admits neither this file nor a second contract.

Generation waits on these answers.
