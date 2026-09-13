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
