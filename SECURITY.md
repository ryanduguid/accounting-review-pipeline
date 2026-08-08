# Data boundary

This repository ships fabricated fixtures only. It deliberately has no Xero credential, API client, MCP server, LLM client, journal-posting path, payment path, BAS/lodgment path, email path, or period-locking path.

Do not commit client trial balances, subledgers, workpapers, credentials, `.env` files, or generated review packs. The `examples/` folder is the only place CSV fixtures belong in this source tree. Use a separate access-controlled working location for real data.

The optional review note records a human acknowledgement only. It does not approve a close or replace professional judgement.
