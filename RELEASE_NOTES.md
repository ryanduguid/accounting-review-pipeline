# v0.1.5

The `v0.1.1`, `v0.1.3` and `v0.1.4` tags are retained as unreleased failed-preflight tags. All three workflows stopped before any build or publication step (v0.1.3's release.yml pinned release-policy to a SHA orphaned by a history rewrite; v0.1.4's VERSION file was not bumped alongside RELEASE_NOTES.md), so none has a release or assets, and GitHub's tag-protection rule blocks deleting or moving any of them.

Changes since `v0.1.2`:

- add contract-checked Power Query parsers for Xero Aged Receivables and Aged Payables summary exports, including the documented MYOB Business scoping;
- add the Payday Super close-input contract;
- pin the accounting number format across VBA modules, size the workpaper header border from the used range instead of `A5:H5`, and make `ApplyWorkpaperHeader` idempotent so a second run is a no-op;
- make the release archives cross-platform reproducible (UTC timestamps, LF text) and lint workflows inside verify, with the caller rollback documented;
- document platform caveats, the test coverage map, the financial-year timezone note and plain-language setup; and
- refresh documentation so every claim matches the repository, including striking the bucket tie-out promise the queries never implemented and retiring the retired codename.

No client data or binary workbook is included. The CSV fixtures are fabricated.
