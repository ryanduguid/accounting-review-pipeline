# AI register entries

The National AI Centre's Guidance for AI Adoption (October 2025) asks organisations to keep an AI register as part of its fourth practice, sharing essential information. The entries below describe the three pipeline components a firm is most likely to list when it uses AI on accounting work. Copy the relevant entry into the firm's own register and complete the fields marked for the firm.

Each entry describes the component as its README and tests do at this revision. An entry is not a certification, an approval of any use or a statement that a firm's use complies with anything. The firm's own AI-use policy decides what may be done with client data.

## evatt

| Field | Entry |
| --- | --- |
| System | [evatt](../packages/evatt/README.md), local pseudonymisation of Australian client data in markdown |
| Contains an AI model | No. It prepares documents for an external model and restores named entities in the answer. |
| Role in an AI-assisted workflow | Runs before a document is sent to a model, replacing structured identifiers and mapped named entities with placeholders |
| Data handled | Client markdown; structured identifiers such as TFN, ABN, ACN, BSB and Medicare numbers (replaced one-way); named entities from a local, gitignored map |
| Network access | None. The package has no network client, and the map stays on the local machine. |
| Privacy position | Pseudonymised data remains personal information for anyone holding the key; this does not take the data outside the Privacy Act |
| Where a person decides | The firm signs off the policy before any client data passes through evatt, including testing. The operator checks each document for contextual re-identification before sending it. |
| Known limits | Contextual re-identification is the residual risk; there is no ATO client reference detector; general ledger codes shaped like a BSB are replaced; unmapped names can escape the residual sweep; `verify` re-runs the same detection rather than a second detector |
| Evidence of testing | The component's test suite and CI gates, run on fabricated samples only |
| For the firm to complete | Owner, approved uses, policy sign-off date, approved model destinations, next review date |

## Xero Ledger Review Gate

| Field | Entry |
| --- | --- |
| System | [Xero Ledger Review Gate](../packages/elizabeth-anne-alexander/README.md) (`elizabeth-anne-alexander`), a fixed-policy review boundary for AI-assisted trial-balance variance review |
| Contains an AI model | No. It contains no LLM client; it produces a bounded, redacted result that a model or a person can review. |
| Role in an AI-assisted workflow | Turns synthetic Xero-shaped trial balances into a variance result that carries no tenant name, account name, account code or source text |
| Data handled | Synthetic Xero-shaped trial-balance fixtures only; it is a design demonstration and does not accept client exports |
| Network access | None, and no write operation on any accounting system |
| Where a person decides | A human decision file records the reviewer's decision, and `validate-review` checks it against the run's artefacts and receipt |
| Known limits | Synthetic-only; the receipt is an unkeyed local checksum, so anyone who can replace the files can replace it, and it proves no authorship, origin or time |
| Evidence of testing | The component's test suite and CI gates on Linux and Windows |
| For the firm to complete | Owner, whether any non-synthetic use is approved, next review date |

## Workpaper Review Gate

| Field | Entry |
| --- | --- |
| System | [Workpaper Review Gate](../packages/review-ready-gate/README.md) (`review-ready-gate`), a readiness gate for workpaper packs before manager review |
| Contains an AI model | No. A firm lists it as a control on workpapers, whether a person or an AI tool prepared them. |
| Role in an AI-assisted workflow | Stops an incomplete, untied or unbalanced pack reaching manager review, however it was prepared |
| Data handled | Workpaper artefacts (CSV and JSON) in a separate working directory; a generated pack cannot be written into a version-control checkout |
| Network access | None |
| Where a person decides | `READY` only admits a pack to manager review. The reviewer decides whether the work is correct, and an acknowledgement never changes a computed status. |
| Known limits | `READY` means no configured control tripped, not that every control ran; the pack lists the optional controls that had no input. Tie-out tolerance is an input to each gate run. |
| Evidence of testing | The component's test suite, CI gates and fabricated evaluation packs; practitioner review of the evaluation is pending |
| For the firm to complete | Owner, engagement types it gates, tolerance policy, next review date |
