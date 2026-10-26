# v1.0.0 (unreleased)

- evatt is a standalone Rust binary, built with `cargo build --locked --release`.
  It uses local files and git only, with no installed Python and no network
  dependency. The Python release caller has been removed; a Rust release path
  must be approved before 1.0.0 can be released.
- The command names, flags, abbreviations, help, exit codes, redacted documents,
  manifests, triage files and disclosure records retain the Python 0.2.0 CLI
  contract. The compatibility reference is CPython 3.14.8 and Unicode 16.0.0.
  Unicode predicates, regex IGNORECASE equivalence and full casefold are frozen
  separately; NFC pins unicode-normalization 0.1.24 (Unicode 16).
- The email matcher uses a bounded scanner in place of the equivalent regex.
  The regex remains in the test-only probe for comparison. Long failing local
  parts such as `9-9-9-...` otherwise retry each suffix in quadratic time.
  Numeric and name scans retain their existing cursor and retry rules. Masking
  writes one `#` per code point and maps byte offsets back explicitly; findings
  on a line share their context. Map size and sorting are outside the bounded
  matcher-work claims.

Deliberate differences:

- `--version` and newly written disclosure records carry `1.0.0`. A record
  carrying `0.2.0` therefore fails `disclosure-check` under 1.0.0.
- The Python import API is removed, including `entities.save`, `entities.assign`
  and the public disclosure functions. evatt never writes a map or a map
  temporary. The `disclosure-record` and `disclosure-check` algorithms remain
  internal and are ported in full. Keep existing `*.tmp` ignore rules.
- Failures that escaped Python as a traceback exit 1 with one `error:` line.
  This includes a restored lone surrogate that UTF-8 cannot encode, a path
  stdout cannot encode in its native code page, unbuffered command diagnostics
  that cannot be written, oversized
  JSON integers and excessive JSON nesting. Restoration preserves the empty
  output left by Python's failed encoding. JSON nesting is capped at 10,000
  levels to protect the worker stack; ordinary record JSON remains capped at
  4,096 bytes. CPython's own depth limit follows its C stack and can be lower,
  so a map nested deeper than CPython parses can reach evatt's field
  validation, which still refuses it, because every map and record field is a
  string or an integer. These are failure paths, not successful output
  transformations.
  A shutdown stream-flush failure retains CPython's exit 120 and emits an
  `error:` line instead of its ignored-exception diagnostic. Argparse's
  unbuffered help/version writes still ignore an I/O error, as CPython does.
  A closed stderr cannot receive that diagnostic; the failure exit is retained.
- evatt refuses to start, with one `error:` line and exit 1, when CPython 3.14
  would write its streams in a codec this binary does not implement or would
  decode arguments and paths with a non-UTF-8 locale. The accepted stream
  codecs are UTF-8, UTF-8 with signature, ASCII, Latin-1 and Windows-1252,
  set through `PYTHONIOENCODING` or, on Windows, the active code page; any
  other Windows code page is refused unless the console, UTF-8 mode or
  `PYTHONIOENCODING` selects an accepted codec. POSIX locales must resolve to
  UTF-8 (a UTF-8 codeset, C, POSIX or UTF-8 mode). CPython would run in some
  of these environments (for example `PYTHONIOENCODING=cp1251`, a Latin-1
  locale or code page 1251) and there it can write redacted documents,
  manifests and records that evatt 1.0.0 does not. It stops at start-up in
  the others, as evatt does: an unknown codec, a `PYTHONUTF8` other than 0 or
  1, or either variable holding a value the system cannot decode. On Windows
  evatt also refuses a non-empty `PYTHONLEGACYWINDOWSFSENCODING` or
  `PYTHONLEGACYWINDOWSSTDIO`, which change how CPython selects UTF-8 mode and
  the console code page; CPython runs under them, including with an otherwise
  invalid `PYTHONUTF8` beside the legacy file system setting. A codec named in
  `PYTHONIOENCODING` must be an accepted one even when every stream is a
  console that CPython would write in UTF-8 regardless. A codec name holding
  any non-ASCII character is refused, as is an error handler other than
  strict, surrogateescape, surrogatepass, ignore, replace or backslashreplace
  (CPython also accepts xmlcharrefreplace and namereplace, and looks an
  unknown name up only when it needs it). On POSIX,
  `PYTHONCOERCECLOCALE=warn` is refused in every locale, because CPython can
  then write locale warnings to stderr; it writes none under a UTF-8 locale,
  where the refusal is an extra one. A non-Unicode `PATHEXT` on Windows makes git
  unavailable rather than guessed at; POSIX ignores `PATHEXT`, as CPython
  does.
- Compatibility scope: the reference is Python evatt 0.2.0 run by a working
  CPython 3.14.8 with default interpreter settings. evatt reads these
  environment variables that CPython also reads: `PYTHONIOENCODING`,
  `PYTHONUTF8`, `PYTHONLEGACYWINDOWSFSENCODING`, `PYTHONLEGACYWINDOWSSTDIO`,
  `PYTHONCOERCECLOCALE`, `PYTHONUNBUFFERED`, `PYTHON_COLORS`, `NO_COLOR`,
  `FORCE_COLOR`, `TERM`, `COLUMNS` and `LINES`, plus the locale;
  `PYTHONUNBUFFERED` and the two legacy Windows variables are read with
  CPython's integer flag rule. `PATH`, `PATHEXT` and
  `NoDefaultCurrentDirectoryInExePath` locate git, and the git subprocess
  inherits the remaining environment apart from the six `GIT_*` location
  overrides evatt removes, as the Python release's subprocess did. Settings
  that only configure the interpreter itself, such as `PYTHONHOME`,
  `PYTHONPATH`, `PYTHONDEVMODE`, `PYTHONWARNINGS` or `-X` options, have no
  meaning for a standalone binary and are ignored, including where they
  would stop or change CPython.
- A regex compilation or execution error, including exhaustion of its 100,000,000-step
  backtrack budget, refuses the run with an `error:` line and exit 1. Every
  detector, mapped value and restore pattern uses the same budget. It never
  becomes a missing match or permits a sendable output to be written. Extremely
  long inputs that CPython can scan may therefore be refused. This follows the
  port's explicit fail-closed regex-error rule.

The test-only probe is behind Cargo's `probe` feature and is absent from the
normal binary build. It compares individual pattern captures before arbitration,
name retries, the email scanner, structured spans, number candidates, mapped
values, folding, residual findings, redaction and verification against the
preserved Python oracle. The CLI differential compares temporary worktrees,
including ignore rules, links, collisions and leftover files. All fixtures are
fabricated. Windows execution remains a separate verification gate.

# Also in v1.0.0: Python changes made after v0.2.0

- Licence: FSL-1.1-ALv2 (Functional Source License, with Apache-2.0 as the
  future licence) replaces MIT. Anyone who redistributes this package or a
  derivative must keep the copyright notices in `NOTICE`. Versions already
  released stay under MIT.
- Halt on TFN, ABN, ACN and Medicare digits that continue into a next-line
  numbered-list marker, including short fragments and candidates hidden by
  trailing digit pins. Keep genuine hard wraps and index ambiguous `verify`
  locations once. Document the accepted next-line Medicare reference limit in
  README.md and DECISIONS.md rulings 47 and 48.
- Tighten `redact` and `verify` for dotted, comma-separated and Unicode-dash
  labelled identifiers, compound labels, labelled space-separated BSBs and
  Medicare individual reference numbers. Unclassified numeric tokens of 8 or
  more digits now halt; ordinary references may need mapping as entities.
  Keep bare dotted/comma fragments out of one-way replacement.
- `verify` is stricter. Reverify files that version 0.2.0 verified clean. Where
  findings appear, re-redact from the originals and recreate any disclosure
  records for the changed output. Review the number sweep's amount/date
  exclusions and remaining detection limits in the README before upgrading.
- Replace both identifiers when an email is written straight against a phone number, ABN or other identifier, so `(02) 9876 5432jane@example.com` becomes `PHONE_01EMAIL_01`, where it had become `PHONE_01jane@example.com`. Whichever started second was dropped, and the rest of it stayed in the sanitised output with no halt.

# v0.2.0

- Breaking: requires Python 3.11 or later. CPython 3.10 reached end of life on 1 October 2026; 0.1.4 remains the last release that runs on Python 3.10 ([#318](https://github.com/ryanduguid/accounting-review-pipeline/pull/318)).
- New `evatt disclosure-record` and `evatt disclosure-check` bind a payload to its exact bytes, validated entity map, destination, decision reference and tool version. Both rescan with the current verifier and require ignored, untracked local records. A record checks consistency only; the sender still authenticates the external decision ([#300](https://github.com/ryanduguid/accounting-review-pipeline/pull/300)).
- Keep names detectable when a following phone's opening punctuation is replaced,
  and separate touching mapped placeholders so both restore. Existing emphasis
  between accepted matches keeps its spacing. Restore retains any added space.
- Document the remaining punctuation-edge, shared-emphasis and longer-word
  placeholder limitations in the README and DECISIONS.md rulings 43 and 44 ([#305](https://github.com/ryanduguid/accounting-review-pipeline/pull/305)).

# v0.1.4

- The redact command refuses to run when its document, manifest or triage paths refer to the same file through a symbolic or hard link, and checks each output against those already examined before any write or removal. A manifest hard-linked to the output could replace the sanitised document with JSON while the command reported success. All three outputs now need separate files, including triage hard links that happened to work before; the refusal exits 1 and leaves the previous files in place.
- The README walks through redacting a fabricated research question before it goes to an outside AI research tool and restoring the reply locally, with the two samples in `evatt/samples/`.

# v0.1.3

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
