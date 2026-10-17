# Contributing

This exporter reads a Xero trial balance and writes a validated CSV. It stays read-only against the Xero API: no contribution should give it the ability to post, approve, or modify anything in an organisation.

## Data boundary

- Never commit credentials. The `.gitignore` blocks `.env*` except `.env.example`, plus `token.json`, its credential-free lock file, `*.json`, and the common export extensions. Keep `.env.example` to placeholder values.
- Keep exported trial balances, account lists and tenant identifiers from a real organisation out of the repository. Sample data belongs in `samples/`.
- On Windows, token-cache changes must preserve current-user DPAPI protection, the versioned envelope, UI-forbidden operation and atomic plaintext migration before any network call. Do not add a repository key or custom cryptography. On non-Windows systems the documented compatibility fallback stays plaintext and must remain mode `0600` where POSIX permissions are supported.

## Correctness rules worth preserving

- The export refuses to write when debits do not equal credits. A scheduled refresh reads the output path and ignores the exit code, so writing a wrong file causes more damage than writing nothing.
- Refresh tokens rotate and Xero accepts each one once. Validate the usable pair, then persist its DPAPI envelope on Windows (or the mode-`0600` fallback elsewhere) before using the new access token. Do not throw away a working rotated token over cosmetic response data or write plaintext token bytes to a Windows recovery temp.
- The exporter refuses a bare multi-organisation run on purpose. Picking the first connection can export the wrong entity.
- The trial balance is the year-to-date column pair. The Debit/Credit pair shows current-month movement.

## Local verification

Rust 1.99 stable. Cargo.lock pins the native dependencies.

```bash
cargo fmt --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
```

The offline tools and their tests read the shared contract at
`contracts/xero-trial-balance-v1/`. Run those checks from a full pipeline clone.
The two installed production commands have no sibling runtime dependency.

The differential oracle uses the immutable Python source at
`1af8a855578afaec97bd2e56836a8aa8fb15e729` and the hash-locked dependencies in
`tests/differential/requirements.lock`. Python 3.15 is required for this check.
The native commands do not require Python.

The OAuth source and README have canonical UTF-8/LF SHA-256 pins in
`tests/provenance.rs`. Review changes to that source and its claims before
updating the pins; the tests refuse BOMs, invalid UTF-8 and bare carriage returns.

## Pull requests

Say which failure mode your change closes and include the test that reproduces it. For anything touching the OAuth or token-cache flow, describe what happens when the token endpoint returns an error mid-rotation, DPAPI refuses a payload, or an atomic migration/write fails.

For a potential security vulnerability, follow [SECURITY.md](SECURITY.md) rather than opening an issue.
