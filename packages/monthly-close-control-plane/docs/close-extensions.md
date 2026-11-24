# Close Residuals, Audit Binders, Flux Checks and Rule Compiler

These deterministic library modules extend `monthly-close-control-plane` without
changing existing CLI schemas, exit codes or frozen baseline outputs. All money
and ratio logic uses exact `Decimal` arithmetic. None of these modules approves
a close, posts a journal, makes a payment or locks a period.

## 1. Named reconciliation residuals (`reconciliation::residuals`)

`classify_pack(&pack, &ResidualPolicy)` inspects the `outstanding` rows of a
`clearing-reconciliation-v1` pack and classifies every open residual into one of
six deterministic classes (`clearing-residuals-v1`):

- `fx`: opposing legs sharing a reference with an FX cue within `fx_ratio_limit`,
  or a standalone FX adjustment.
- `bank_fee`: opposing legs or standalone items carrying a fee cue within
  `bank_fee_limit`.
- `partial`: opposing legs sharing a reference that leave a non-zero balance
  outside FX and fee criteria.
- `duplicate`: two or more same-side items with equal exact amount and matching
  key within `duplicate_window_days`.
- `timing`: unmatched item dated within `timing_window_days` of `period_end`.
- `unexplained`: fail-closed default when no deterministic pattern matches.

## 2. Tamper-evident close binder (`publication::binder`)

`export_binder(&BinderRequest)` verifies a close review pack via
`viewer::verify_pack`, checks the target path is outside version control, and
writes a deterministic stored ZIP archive sealing:

- the four verified pack artefacts (`close-review-pack.json`, `close-summary.md`,
  `exceptions.csv`, `client-queries.csv`),
- optional validated attachments (such as residuals or compiled rules),
- an optional hash-chained `audit-chain.jsonl` verified from `GENESIS_HASH`, and
- `manifest.json` (`close-binder-manifest-v1`) recording byte lengths, SHA-256
  digests, source hashes and the optional audit chain head.

`verify_binder` and `verify_binder_bytes` require an independently stored
`BinderAnchor` (`manifest_sha256` and optional `audit_chain_head`) and validate
ZIP structure, CRC-32
checksums, exact manifest inventory, SHA-256 digests, embedded pack metadata and
the audit hash chain. Any modification fails closed.

## 3. Flux anomaly checks (`drivers::flux`)

`evaluate_flux(&current, &prior, &FluxPolicy)` runs two deterministic checks
(`flux-checks-v1`):

- `sudden_zero_recurring_cost`: flags recurring expense accounts whose current
  period charge dropped to `0.00`, suppressing one-off expenses, seasonal items,
  cleanly reclassified costs and expenses whose linked prepaid asset unwound.
- `normal_balance_sign_flip`: flags closing balances sitting opposite their
  expected normal debit or credit sign above `min_sign_flip_amount`, respecting
  declared contra and either-sign overrides.

## 4. Deterministic override-to-rule compiler (`queue::rule_compiler`)

- `compile_rules(&records, &CompilePolicy)` groups human review dispositions and
  promotes only unanimous patterns meeting `min_agreements` (at least 2; across
  distinct periods by default) into SHA-256-sealed `close-override-rules-v1`
  rules, serialisable to canonical JSON or TOML.
- `backtest_rules(&ruleset, &held_out)` replays compiled rules against held-out
  dispositions with zero model calls and demotes any rule with a conflict.
- `replay_rules(&ruleset, &open_items)` verifies `ruleset_sha256` before replay,
matches patterns on alphanumeric token boundaries (preventing substring
collisions)
  and keeps unmatched or multi-rule conflicting items in `exceptions_remaining`.

