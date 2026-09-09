# Security boundary

This v0.1 package is a local, offline pseudonymisation boundary. It has no OAuth, API client, HTTP client, MCP server, LLM client, credential store, or accounting-system write path. It sends nothing anywhere; the operator carries the sanitised file to whatever reads it next.

The entity map is the key, and it stays on the local disk. Do not place a real entity map, real workpapers, client data, tokens or `.env` files in this repository. Every command refuses to read a map that git tracks or would not ignore, and the shipped rules cover the map, its `.tmp` and any triage file; those rules are a guard against a mistake, not a substitute for keeping real client data out of a public repository.

Pseudonymisation reduces what a recipient receives. It is not a compliance determination, and the residual risk is contextual re-identification. See `DISCLAIMER.md`.

## Reporting a vulnerability

Report a suspected vulnerability privately through [GitHub's advisory form](https://github.com/ryanduguid/accounting-review-pipeline/security/advisories/new), or by email to ryan@duguid.com.au. Do not open a public issue for one.

Include the identifier, entity or path you believe survives redaction, or the guard you believe is bypassed, and the artefacts that reproduce it. Fabricated data only, as everywhere else in this repository.
