# Security policy

Report a suspected vulnerability through this repository's private vulnerability-reporting
feature. Do not open a public issue containing exploit details, credentials or client data.
Include the component directory, the gate or refusal you believe is bypassed and a fabricated
reproduction.

Only `packages/xero-trial-balance-export/` may contain OAuth, Xero API, HTTP-client, token or
Xero-credential handling, and only its own release path may ever reference publishing
secrets. Every review package, the Excel adapter and the Power BI application consume
fabricated local files and must remain offline. Root review workflows run with
`contents: read` and receive no Xero credentials.

Do not commit client trial balances, ledgers, workpapers, generated review artefacts,
credentials, `.env` files, tokens, private keys or live-system screenshots.
