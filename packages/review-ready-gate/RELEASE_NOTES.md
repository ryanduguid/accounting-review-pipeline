# v0.1.6

- Return NOT_READY for exception tie-outs and report unreadable inputs as input errors.
- Verify the complete canonical summary and derive viewer status from its findings.
- Prevent a preparer from acknowledging their own pack.

# v0.1.5

The `review-ready-gate/v0.1.4` tag was pushed on 7 September 2026 and its
release workflow failed at startup: the shared policy commit it called,
`787db459`, had been left unreachable by a later release-policy history
rewrite, so no job ran and no release or PyPI upload was created. A pushed tag
is never moved, so the same tree ships as `v0.1.5` with every root release
caller repointed to `fcf25e5`, the on-`main` commit whose tree is identical to
the dead pin.

Discovery copy leads with the public name, Workpaper Review Gate, and the
sibling links point at the components' maintained homes inside the Accounting
Review Pipeline. The release workflow calls the shared Python release policy at
a commit reachable from that repository's `main`, and the component's CI now
runs the shared gates:
its own lockfile check, ruff, mypy, pytest with branch coverage on Python 3.10,
3.12 and 3.13, pip-audit over the locked environment and an installed-wheel
smoke, with the check tools pinned in the `dev` extra. The published schema
identity, runtime behaviour and fabricated fixtures are unchanged from v0.1.3.

# v0.1.3

First maintained release from `packages/review-ready-gate` in the Accounting
Review Pipeline. Project, citation and evaluation links now point to the
canonical monorepo, and the namespaced release publishes through the shared
attested Python policy. The published schema identity, runtime behaviour and
fabricated fixtures are unchanged from v0.1.2.

# v0.1.2

Correct source-distribution evidence packaging and immutable documentation
metadata. Product runtime behaviour and fabricated fixtures are unchanged.

Local CLI that gates BAS, month-end, and year-end workpaper packs before they
reach manager review. Fabricated fixtures only. Not advice.
