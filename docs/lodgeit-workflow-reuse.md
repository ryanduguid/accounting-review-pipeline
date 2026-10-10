# Reusing LodgeiT workflow patterns in the review pipeline

Use the pipeline's existing import, mapping and evidence controls to make workpaper review easier to follow. The public LodgeiT documentation inspected on 9 October 2026 provides useful workflow examples; it does not establish tax treatment or interoperability with this repository.

## Patterns and current owners

| Public pattern | Existing pipeline owner | Practical use |
| --- | --- | --- |
| Import financials, then inspect classification | [Exporter](../packages/xero-trial-balance-export/README.md), [Excel adapter](../adapters/accounting-excel-toolkit/README.md) and [close controls](../packages/monthly-close-control-plane/README.md) | Preserve the source export and inspect unmapped accounts and incompatible review groups. |
| Link a workpaper to its supporting document | [Readiness gate](../packages/review-ready-gate/README.md) | Use the existing evidence checks and source digests; record missing support. |
| Show an exception next to the account it concerns | [Close workbench](../packages/monthly-close-control-plane/README.md#local-close-workbench) | Verify the existing pack before reading balances, exceptions or draft questions. |
| Separate preparation, review and later actions | [Status contract](../README.md#status-contract) and [ledger-review gate](../packages/elizabeth-anne-alexander/README.md) | Keep readiness, control results and recorded human decisions separate. |
| Retain the history behind a result | [Review-pack contract](../README.md#review-pack-contract) | Keep the related output files together and retain the exact input digests. |

LodgeiT's [mapping guide](https://help.lodgeit.net.au/support/solutions/articles/60000609338-chart-of-accounts-and-mapping) describes classification and tags, with review of automatic suggestions. Its [workpaper guide](https://help.lodgeit.net.au/support/solutions/articles/60000609509-using-workpapers) links evidence and reviewers to accounts and periods. Its [audit-trail guide](https://help.lodgeit.net.au/support/solutions/articles/60000609142-audit-trail-and-document-history) records who changed a form and when.

The pipeline already implements explicit mapping and balance policies. Its review groups are not LodgeiT tax categories. Reusing the workflow does not mean importing the vendor's taxonomy or inferring a tax tag from an account name.

## Preparation and review checklist

1. Establish the tenant, period, currency, source system and export method. Retain the input files in the approved location and identify the bytes assessed.
2. Run the existing readiness and close checks from their owning components using their documented commands. If an optional control was not supplied, retain that coverage gap when handing over the pack.
3. Inspect new, missing and unmapped accounts. If the firm supplies a mapping policy, inspect `mapping_compatibility`; if it supplies a balance policy, inspect `balance_policy`. The firm's explicit policy owns those expectations.
4. Open the pack with its existing verified viewer. Investigate each exception using the source account and supporting evidence. Keep draft client questions for human review; the pipeline sends nothing.
5. Retain the review decision and the evidence it refers to. `READY`, `PASS`, `REVIEW_READY` and `DECISION_RECORDED` have different owners and meanings. None authorises posting, payment, signing or lodgement.

## Improvements to consider next

The [dashboard guide](https://help.lodgeit.net.au/support/solutions/articles/60000609258-dashboard-work-status-filter-and-sort-team-form-period-client-group-) suggests useful queue filters: work type, period, owner and due date. The [local close review queue](../packages/monthly-close-control-plane/docs/review-queue.md) now lists explicitly supplied packs after verification, with filters for recorded period, state and reviewer. It labels missing metadata and omitted controls. Owners and due dates are absent from the pack schema and are not inferred from a reviewer acknowledgement.

The [rollover guide](https://help.lodgeit.net.au/support/solutions/articles/60000688784-rollover) describes selected prior-year information carrying forward. A future preparation helper should show the originating period and require current-period evidence. Carrying forward a template must not carry forward a review decision.

The [workpaper guide](https://help.lodgeit.net.au/support/solutions/articles/60000609509-using-workpapers) warns that switching between API and spreadsheet imports can change line-item identity and duplicate or disrupt work. A future importer should show its proposed identity mapping before replacing anything. Preserve zero-balance source accounts when the existing contract accepts them; the vendor's omission of zero-value accounts is not a rule for this pipeline.

The [form workflow](https://help.lodgeit.net.au/support/solutions/articles/60000609327-form-workflow-sequence-and-e-signature) and [queued-status guidance](https://help.lodgeit.net.au/support/solutions/articles/60001644142-form-status-lodging-pre-lodged-queued) describe operations outside this pipeline's authority. They are useful evidence that validation, signatures and confirmed submission are separate events. Do not copy the vendor's sign-and-lodge flow into the local review commands.

Rollover and import preview remain proposals. The queue retains the existing pack schema, states and review authority.

## ClientRelay preparation patterns

The separately hosted [ClientRelay help site](https://help.clientrelay.lodgeit.net.au/support/home) documents a client-management product alongside LodgeiT. Its [client and contact guide](https://help.clientrelay.lodgeit.net.au/support/solutions/articles/60001645837-understanding-the-difference-between-contacts-and-clients) distinguishes an entity receiving a service from a person associated with it. Preserve that distinction when recording a pack's owner or a proposed recipient; an entity match does not establish a person's signing authority.

Its [workflow guide](https://help.clientrelay.lodgeit.net.au/support/solutions/articles/60001645858-daisy-chaining-tasks-e-g-onboarding-proposals-questionnaire-) describes completion-triggered transitions. A local preparation workflow can identify the next missing input while retaining each component's status and the human decision. Transitioning a local record must not send a client message or create a signature, invoice or payment. ClientRelay's public app redirected to sign-in; account access and workflow execution were not tested.
