# Security policy

## Supported versions

Security fixes are applied to the latest version on the default branch.

## Reporting a vulnerability

Please use this repository's private vulnerability-reporting feature. Do not
open a public issue for a suspected security vulnerability. Include a clear
description, reproduction steps, impact, and any suggested mitigation.

We will acknowledge a valid report within seven days and will coordinate a fix
and disclosure timeline with the reporter.

## Data boundary

This repository ships fabricated fixtures only. It deliberately has no Xero
credential, API client, MCP server, LLM client, journal-posting path, payment
path, BAS/lodgment path, email path, or period-locking path.

Do not commit client trial balances, subledgers, workpapers, credentials,
`.env` files, or generated readiness packs. The `examples/` folder (fabricated
fixtures) and the `schemas/` folder (header-only schema references) are the
only places CSVs belong in this source tree. Use a separate access-controlled
working location for real data.

The optional review note records a human acknowledgement only. It does not
approve a file or replace professional judgement.
