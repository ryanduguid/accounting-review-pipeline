# Contributing

Keep a change scoped to one component, or to the data-only contract once it exists. Run the
owning component's documented checks from its directory and include affected downstream
conformance evidence when a file contract changes. `AGENTS.md` lists the commands.

Use fabricated fixtures only. Do not add client exports, workpapers, credentials, tokens,
generated packs or screenshots containing client data. Review components may not gain a
network or Xero dependency, may not import the exporter or a sibling component, and must
keep exact `Decimal` arithmetic and their documented status and exit-code boundaries.

Component versions, lockfiles, publishers and release workflows stay independent. Do not add
a root package manager, shared runtime package, unified version or monorepo framework.
Movement-only changes and behaviour changes go in separate pull requests.

For a potential security vulnerability follow `SECURITY.md`.
