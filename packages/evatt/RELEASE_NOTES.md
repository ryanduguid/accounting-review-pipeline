# Unreleased

- Match a mapped name through underscore and asterisk emphasis, `<br>` line breaks, blockquote continuations and zero-width joiners, and write its placeholder inside the original emphasis so it restores. Report `__John Smith__` and other emphasised unmapped names in the residual sweep.
- Keep every placeholder already in the text, underscore-wrapped ones included, out of pass two's reach; halt on a placeholder the input carries inside emphasis; and refuse map values made only of placeholder parts or differing from another entry only in the separators the matcher joins.
- Detect identifiers whose digit groups are separated by up to 2 spaces or hyphens (a bare TFN's 2 gaps may differ), the `tax file no.` label, and labels and qualifiers with emphasis edges such as `__TFN__:` and `**ABN no.**:`.
- Skip a map value with no token, such as `*`, and match joiner runs atomically, so a value holding a partial `<br` tag no longer searches in cubic time.
- Refuse a map value that starts or ends with whitespace or markup, such as `Jane Roe ` or `<br>Jane`. Pass two replaces only a value's words, so restore would write the edge a second time; remove it from the map entry.
- Name the shapes still unsupported in the README and DECISIONS.md ruling 42.

# v0.1.2

- Detect labelled identifiers in padded code spans containing literal emphasis markers. Preserve the surrounding markup, line endings and replacement counts.
- Detect labelled identifiers across complete emphasis and inline-code delimiter runs, including four or more backticks and mixed nested markup. Preserve digit spans, replacement counts and bounded scan growth.

# v0.1.1

- Compose input to NFC before scanning, and admit markdown between a label and its digits so a bolded or linked identifier is still caught.
- Resolve `git` to an absolute path before the entity-map guard spawns it, so a same-named executable on the search path cannot answer the git-ignore check.
- Package the archive inputs, label the illustrative report timings and widen the trial-balance overflow probe.
- Remove the shipped plan and design documents from the package.

# v0.1.0

First release of `evatt` from `packages/evatt` in the Accounting Review
Pipeline. There is no earlier standalone history.

A local, zero-network pseudonymisation boundary for Australian client data in
markdown. `redact` replaces structured identifiers one way and known entities
from a local map, `verify` re-scans an output, and `restore` reverses named
entities only. The entity map is the key, it stays on disk, and every command
refuses to run when git would let you commit it.

`redact` halts rather than guessing. A candidate neither earlier pass
recognised stops the run, writes a triage worklist and leaves nothing to send.
Under-detection is the failure that matters, so every detection trade-off in
this release favours an extra placeholder over a miss.

What this release does not do:

- It does not take the data outside the Privacy Act. Pseudonymised data remains
  personal information in the hands of anyone holding the key.
- It does not address contextual re-identification. Surviving detail can
  identify a client with no identifier present, and a human check before
  sending is the only mitigation.
- It does not detect an ATO client reference. No single published fixed format
  exists for one, so a pattern would be guesswork producing either noise or
  false confidence. The gap is a named limit for the human check.
- It does not authorise disclosure of anything to anyone, and no client data
  should pass through it before the policy governing that data is signed off.

Nothing is published to PyPI. `release-evatt.yml` deliberately carries no
publish job while the disclosure policy this package enforces is unsigned;
`RELEASING.md` records what to add when that changes.

Review corrections included in this candidate:

- Suppress matched values in verification output and inspect the actual map repository after removing inherited Git location variables.
- Correct unsupported BSB and anonymity claims.
