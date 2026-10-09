# Supplier information for a firm's AI register

The National AI Centre's Guidance for AI Adoption (October 2025) asks organisations to keep an AI register as part of its fourth practice, sharing essential information, and asks developers to share technical details, test results, limitations and risks with the organisations that deploy their systems. Items 4.1.1 and 4.3.2 of its [implementation guidance](https://www.ai.gov.au/staying-safe-and-responsible/essential-ai-practices/guidance-ai-adoption-implementation-guidance) set out both. The tables below give the supplier's side of that record for the three pipeline components a firm is most likely to list when it uses AI on accounting work, checked against the versions they name on 29 September 2026.

The first seven rows of each table follow the columns of the National AI Centre's [AI register template](https://www.ai.gov.au/staying-safe-and-responsible/essential-ai-practices/ai-systems-register), so a firm can start those columns from them; the other rows serve the rest of its register and its risk assessment. The firm adds the template's remaining columns (owner, status, purpose, registered date and screening outcome) and its own use case, deployment environment, risk and impact assessment, controls and review cycle. A firm may give a component its own register row or record it as a component or control of a larger AI-assisted workflow.

Each table describes the version named in its Name and version row, as that version's README and tests describe it; `tests/test_ai_register_entries.py` fails when a component's version moves without its table. Record the installed version in the firm's register, and recheck the table against the component's README when the two differ. Supplier information is not a certification, an approval of any use or a statement that a firm's use complies with anything. The firm decides what client data may be used, under its AI-use policy and its legal, professional, contractual and privacy obligations.

## ISO/IEC 42001

AS ISO/IEC 42001:2023 is the Australian identical adoption of ISO/IEC 42001:2023, a management system standard for organisations that provide or use AI systems. Certification against it is voluntary and is carried out by external certification bodies, not by ISO. These tables are supplier information a firm may use in its own AI register, risk assessment or other governance records. They are not a certification, an audit or a conformity assessment, and they do not establish that any organisation's AI management system conforms to the standard or that a component is certified, approved or assured under it.

## evatt

| Field | Entry |
| --- | --- |
| Name and version | [evatt](../packages/evatt/README.md) 0.2.0, local pseudonymisation of Australian client data in markdown |
| Source and updates | Ryan Duguid, the sole maintainer, under the MIT licence, with no warranty or support agreement. Each release has [release notes](../packages/evatt/RELEASE_NOTES.md); an installed version changes only when the firm upgrades it. |
| Intended use cases | Runs before a document is sent to a model, replacing structured identifiers and mapped named entities with placeholders |
| Known limitations and prohibited use | Not for client data before the firm signs off the policy that governs it, and not authority to disclose anything. Contextual re-identification is the residual risk; there is no ATO client reference detector; general ledger codes shaped like a BSB are replaced; unmapped names can escape the residual sweep; `verify` re-runs the same detection rather than a second detector |
| Foreseeable misuse and failure | A client identifier or name reaches the model provider. evatt replaces structured identifiers one-way and mapped entities reversibly, and halts before writing any output when its residual sweep finds a candidate neither earlier pass recognised, listing each one for triage. Re-identification from surviving detail, such as an industry, a location and a turnover figure together, stays with the operator's check before sending. |
| Data sources and type | Client markdown (text); structured identifiers such as TFN, ABN, ACN, BSB and Medicare numbers (replaced one-way); named entities from a local, gitignored map |
| Key stakeholders affected | May include the firm's clients and the people named in their documents, whose personal information it is meant to keep from the model provider |
| Contains an AI model | No. It prepares documents for an external model and restores named entities in the answer. |
| Datasets and training | Tested on fabricated sample documents only. It trains, fine-tunes and evaluates no model. |
| Technical requirements | Python 3.14 or later and `git` on `PATH`. The map and the run must sit in a git work tree that ignores the map, or every command refuses to run. |
| Network access | None. The package has no network client, and the map stays on the local machine. |
| Privacy position | Pseudonymised data remains personal information for anyone holding the key; this does not take the data outside the Privacy Act |
| Where a person decides | The firm signs off the policy before any client data passes through evatt, including testing. The operator checks each document for contextual re-identification before sending it. |
| Acceptance and testing | A change reaches `main` only when the required CI checks pass, and they run the component's test suite on fabricated samples |
| Independent assurance | None. No external audit, certification or practitioner review. |
| Report a problem | A vulnerability through the repository's [security policy](../SECURITY.md); a wrong result as an issue reproduced with fabricated data, never client data |
| For the firm to complete | Owner, status, purpose and business goals, registered date and screening outcome (the template's firm columns); installed version, approved uses, policy sign-off date, approved model destinations, impact and risk assessment outcome and treatment, any audit requirement, next review date |

## Xero Ledger Review Gate

| Field | Entry |
| --- | --- |
| Name and version | [Xero Ledger Review Gate](../packages/elizabeth-anne-alexander/README.md) (`elizabeth-anne-alexander`) 0.2.6, a fixed-policy review boundary for AI-assisted trial-balance variance review |
| Source and updates | Ryan Duguid, the sole maintainer, under the MIT licence, with no warranty or support agreement. Each release has [release notes](../packages/elizabeth-anne-alexander/RELEASE_NOTES.md); an installed version changes only when the firm upgrades it. |
| Intended use cases | Turns synthetic Xero-shaped trial balances into a variance result that carries no tenant name, account name or account code. Each finding keeps its trial-balance section, such as Revenue, which is source text passed to the model. |
| Known limitations and prohibited use | Not for client exports or evidence from Xero. Synthetic-only; the receipt is an unkeyed local checksum, so anyone who can replace the files can replace it, and it proves no authorship, origin or time |
| Foreseeable misuse and failure | Ledger detail reaches a model, or a movement is misread. The model result never carries a tenant name, account name, account code or free text copied from the source; accounts are joined by stable ID, and a comparison across the 1 July reset, or an account changing section, is refused. Using it on real client data is outside its design. |
| Data sources and type | Synthetic Xero-shaped trial-balance fixtures (CSV) only; it is a design demonstration and does not accept client exports |
| Key stakeholders affected | None in its synthetic-only design; a firm that adapted the design to real ledgers would affect the clients whose accounts are reviewed |
| Contains an AI model | No. It contains no LLM client; it produces a bounded, redacted result that a model or a person can review. |
| Datasets and training | Tested on the fabricated `xero-tb-csv.v1` contract corpus, whose digests every consumer's tests check. It trains, fine-tunes and evaluates no model. |
| Technical requirements | Python 3.14 or later, run locally |
| Network access | None, and no write operation on any accounting system |
| Where a person decides | A human decision file records the reviewer's decision, and `validate-review` checks it against the run's artefacts and receipt |
| Acceptance and testing | A change reaches `main` only when the required CI checks pass, and they run the component's test suite on Linux and Windows |
| Independent assurance | None. No external audit, certification or practitioner review. |
| Report a problem | A vulnerability through the repository's [security policy](../SECURITY.md); a wrong result as an issue reproduced with fabricated data, never client data |
| For the firm to complete | Owner, status, purpose and business goals, registered date and screening outcome (the template's firm columns); installed version, whether any non-synthetic use is approved, impact and risk assessment outcome and treatment, any audit requirement, next review date |

## Workpaper Review Gate

| Field | Entry |
| --- | --- |
| Name and version | [Workpaper Review Gate](../packages/review-ready-gate/README.md) (`review-ready-gate`) 0.2.0, a readiness gate for workpaper packs before manager review |
| Source and updates | Ryan Duguid, the sole maintainer, under the MIT licence, with no warranty or support agreement. Each release has [release notes](../packages/review-ready-gate/RELEASE_NOTES.md); an installed version changes only when the firm upgrades it. |
| Intended use cases | Stops an incomplete, untied or unbalanced pack reaching manager review, however it was prepared |
| Known limitations and prohibited use | Not tax, financial, audit or legal advice, and not a sign-off. `READY` means no configured control tripped, not that every control ran; the pack lists the optional controls that had no input. Tie-out tolerance is an input to each gate run. The bank reconciliation is checked against the GL balance it records itself, not the pack's trial balance, and the GST control file only for postings after `period_end`. The command refuses to write a pack inside a version-control checkout it can detect, but a work tree chosen by `--work-tree` or `core.worktree` in another repository cannot be detected, so the check is a backstop, not proof that a directory is untracked. |
| Foreseeable misuse and failure | A reviewer reads `READY` as "correct", or signs off a pack whose missing control never ran. The status only admits a pack to review, the pack and its summary list every control that had no input, and an empty file is a finding rather than evidence. Whether the work is right stays with the reviewer. |
| Data sources and type | Workpaper artefacts (CSV and JSON) in a separate working directory and, with `--document`, an original PDF or text document with its extracted text and an intake manifest of proposed fields and human corrections |
| Key stakeholders affected | May include the clients whose workpapers pass through the gate, and the reviewers who rely on its status |
| Contains an AI model | No. A firm lists it as a control on workpapers, whether a person or an AI tool prepared them. |
| Datasets and training | Tested on fabricated example and evaluation packs only. It trains, fine-tunes and evaluates no model. |
| Technical requirements | Python 3.14 or later, run locally, with packs written to a working directory outside any version-control checkout |
| Network access | None |
| Where a person decides | `READY` only admits a pack to manager review. The reviewer decides whether the work is correct, and an acknowledgement never changes a computed status. |
| Acceptance and testing | A change reaches `main` only when the required CI checks pass, and they run the component's test suite and its fabricated evaluation packs |
| Independent assurance | None. No external audit, certification or practitioner review. |
| Report a problem | A vulnerability through the repository's [security policy](../SECURITY.md); a wrong result as an issue reproduced with fabricated data, never client data |
| For the firm to complete | Owner, status, purpose and business goals, registered date and screening outcome (the template's firm columns); installed version, engagement types it gates, tolerance policy, impact and risk assessment outcome and treatment, any audit requirement, next review date |
