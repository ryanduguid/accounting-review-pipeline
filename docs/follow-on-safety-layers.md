# Follow-on safety layers

The close-control engine is deliberately a local review-pack generator. It should not grow into a broad accounting-system integration simply because the next work involves AI or source monitoring. The next layers are separate repositories with their own boundary contracts.

## ElizabethAnneAlexander

The intended next component is [`ElizabethAnneAlexander`](https://github.com/ryanduguid/ElizabethAnneAlexander), a fixed-policy, synthetic-data demonstration of how a future AI assistant can receive a bounded variance-review result without controlling Xero.

```text
Authorised human-operated read-only export
                |
                v
validated canonical TB + source manifest
                |
                v
allowlisted review operation and redacted model result
                |
                v
human reviewer evidence and acknowledgement/escalation
```

The gateway must not accept a natural-language tool request, an arbitrary Xero/MCP command, a token, or an accounting-system mutation. It has no authority to create a journal, payment, invoice, BAS, contact, or period lock. Its model-facing result excludes tenant names, account names/codes, source paths, and unbounded source text.

## AU Tax Change Impact Monitor

The intended next component is [`au-tax-change-impact-monitor`](https://github.com/ryanduguid/au-tax-change-impact-monitor), a provenance-first synthetic demonstration that turns version metadata into a technical-review queue.

```text
Reviewed source-index metadata + reviewed Register observation
                         |
                         v
explicit status classification
                         |
                         v
exact source-to-workflow mapping
                         |
                         v
human technical-tax review
```

Its output must distinguish a superseded compilation, a current version without a published compilation, a source that is no longer in force, and a failed lookup. No state establishes legal effect by itself. The monitor never automatically changes a skill, gives tax advice, updates a client workpaper, sends a notification, or lodges anything.

## Shared rules

- Every initial demonstration uses fabricated inputs only and marks outputs as `synthetic`.
- Source hashes and versioned JSON contracts make each run reproducible.
- A human decision record can document acknowledgement or escalation, but cannot cause the program to claim `approved`, `resolved`, `posted`, `paid`, `lodged`, or `locked`.
- A future live integration needs a fresh, explicitly authorised design for access control, privacy, retention, audit logging, and user authority. A local JSON file split is not a production security boundary.
