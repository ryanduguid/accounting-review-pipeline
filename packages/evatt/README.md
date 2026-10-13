# evatt

| Binary | Version | Command |
| --- | --- | --- |
| `evatt` | `1.0.0` (unreleased) | `evatt` |

This checkout builds a standalone Rust binary. It needs no installed Python.
The Python import API and map-writing facility have been removed. Version
0.2.0 remains the prior Python release; nothing is published to PyPI.
There is no 1.0.0 release until a Rust release path is approved. See
[RELEASING.md](RELEASING.md) and [RELEASE_NOTES.md](RELEASE_NOTES.md).

## Scope and assurance boundary

evatt performs **pseudonymisation with local key retention**. It replaces
Australian structured identifiers and known named entities in markdown with
placeholders, so that a document can be handed to an external model without the
model receiving the identifiers.

Entity placeholders are stable across documents, because they come from the
map: `CLIENT_01` means the same client in every file redacted against it.
Structured placeholders are not. They are numbered per document, so `TFN_01` in
one workpaper and `TFN_01` in the next are 2 different tax file numbers.
Putting 2 redacted documents in front of one model is putting 2 meanings of
`TFN_01` in front of it.

**The key never leaves the local machine.** The entity map is a gitignored file
on disk. It is not transmitted, and no network client exists in this package.

**This does not take the data outside the Privacy Act.** Pseudonymised data
remains personal information in the hands of anyone holding the key. The
recipient does not receive the map needed to reverse the entity substitutions.

The **residual risk is contextual re-identification**. A document can identify
a client through surviving detail alone, such as an industry, a location, a
balance date and a turnover figure appearing together. evatt does not solve
this. A human check before sending is the only mitigation.

evatt is a control that makes a policy enforceable. It is not authority to
disclose anything.

Firms deciding where client data may go usually compare a model running inside
their Microsoft 365 tenant, a model hosted in Australian data centres and a
model running on their own hardware. evatt sits in front of any of these: it
reduces what leaves the machine whichever model receives the document, and it
does not decide which destination the firm's policy permits.

**No client data passes through evatt until the policy governing it is signed
off, and that includes testing.** A trial run is a disclosure of the document to
whatever the operator does with the output, and a control being evaluated is not
yet a control anyone has agreed to rely on. Use the fabricated files under
`evatt/samples/` until the sign-off exists.

Build and run from `packages/evatt/`:

```bash
cargo build --locked --release
./target/release/evatt --version
```

On Windows the binary is `target\release\evatt.exe`. Either put the binary
on `PATH` or use its full path in the examples below. Rust 1.99.0 builds the
crate; `git` on `PATH` is the only runtime program it calls.

```
evatt redact  --in notes.md --map entities.json --out build/notes.md
evatt verify  --in build/notes.md --map entities.json
evatt restore --in answer.md --map entities.json --out build/answer.md
```

## Worked example: a question for an outside research tool

A tax research question usually names the client. Redact it before it goes to
any outside AI research tool, and restore the reply on your own machine. Run the
fabricated samples from `packages/evatt/` in a checkout, where this repository's
`.gitignore` already covers the map and `build/`:

```
cp evatt/samples/entities.sample.json entities.json
evatt redact  --in evatt/samples/research-question.md --map entities.json --out build/question.md
evatt verify  --in build/question.md --map entities.json
evatt restore --in evatt/samples/research-answer.md --map entities.json --out build/answer.md
```

The fabricated samples stay at `evatt/samples/` in the source checkout. Copy
that folder into your scratch repository when running an installed binary,
and write the ignore rules below before trying a command.

`build/question.md` carries `CLIENT_01`, `ENTITY_01` and `PERSON_01` in place of
the names and `TFN_01` and `ABN_01` in place of the identifiers. That file, not
the original, is what the tool receives. `research-answer.md` stands in for the
tool's reply: `restore` puts the names back, and the identifiers stay as
placeholders because they are one-way. Read `build/question.md` before you send
it, because a year, an industry and an amount together can still identify a
client. The tool's own terms decide what happens to what you send, and evatt
cannot change them.

## Requirements

The standalone binary and `git` on `PATH`. Every command asks git whether it
would let you commit the map, and refuses to run when the answer is yes or
unclear. A halt asks the same question about the triage path before it writes
one. That question has no answer outside a work tree, so the map, and therefore
the run, must live inside a repository that ignores it.

The rules covering `entities.json`, `*.entities.json`, `*.triage.md` and
`*.disclosure.json` live in this repository's own `.gitignore`. That file
is not part of the binary. **If you installed evatt, you must write your own**
rules in the repository your map and your run live in.
Three kinds of path are guarded, and a rule has to cover every one of them:

```
entities.json
*.entities.json
*.triage.md
*.disclosure.json
```

The first 2 are the map itself, under whichever of the two names you use.
The triage rule covers the worklist a halt writes beside `--out`. The
disclosure rule covers local evidence records, whose metadata and map digest
must also stay local. evatt reads the map and never writes it.

Leave any of them out and the run fails closed, naming the path and the rule it
wants, rather than writing the file.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Clean |
| 2 | Halted on candidates, verify found something, or disclosure evidence was refused |
| 1 | Malformed input |

A usage error from the command line is a 1, not argparse's usual 2, so a
mistyped option is never mistaken for a document that needs triage.

## Halting

The residual sweep stops the run when it finds a candidate neither earlier pass
recognised. Nothing is written, and a triage file lists each candidate with its
line and context. Classify each candidate, then run again. To clear a
false positive, map that phrase as an `entity`; it will also be replaced and
restored. There is no separate noise allowlist or command-line bypass.

An unclassified numeric token with 8 or more digits halts with kind `number`.
Rewrite a TFN, ABN, ACN or Medicare number in a recognised spelling so pass one
replaces it one-way. Map any other number, such as an account or invoice
reference, as an `entity` using its workpaper spelling. Numeric-only map values
are accepted; restore writes the spelling held in the map. An entity match must
cover every suspicious numeric token it overlaps in full. Mapping just `456`
does not clear `123.456.782`; the complete number still halts.

The number sweep classifies whole tokens, joining digits across at most 2
spaces, tabs, hyphens, Unicode dashes, slashes, dots or commas, within one line.
It removes a leading calendar-valid `yyyy-mm-dd` or `d/m/yyyy` date before
classifying the remainder. A token with exactly one dot and no other separator,
or comma thousands groups with an optional decimal part, is treated as an
amount. Such an amount halts only if its 9, 10 or 11 digits pass a TFN, ACN,
Medicare or ABN check. Other tokens of 8 or more digits halt, including dotted
identifiers, IPv4 addresses and long version strings. Shorter runs, including
6 and 7 digit account numbers, remain outside this sweep. Ordinary long
references can halt too; this policy has no established workpaper false
positive rate. The fabricated `negatives.md` sample now halts on its 8-digit
statutory reference.

This is the point of the tool. A detector that silently passes what it does not
understand is the failure that leaks.

A halt also deletes any output and manifest already sitting at `--out` from an
earlier run, and a clean run deletes any triage file left there by an earlier
one. What the operator sees is a directory, not one run's exit code, and a
sanitised document sitting beside a worklist that names a real person is what
gets sent.

The triage path is checked against git before it is written, the same question
every command asks about the map. It quotes whole residual lines about a real
document at a path `--out` derived rather than one you typed, so a halt that
cannot write it safely writes nothing and exits 1.

## Structured identifiers are one-way

`restore` reverses named entities only. A TFN, ABN, ACN, BSB or Medicare number
replaced by `redact` is gone, and nothing records its value.

## What the operator has to know

Detection is regular expressions and check digits. It has no idea what a
document is about, and the trade-offs below all favour over-detection, because
an extra placeholder costs a triage decision while a miss leaks.

- **`BSB` fires on any hyphenated 3-three digit pair**, and accepts a space or
  tab between the groups when labelled, as in `BSB 062 000`. There is no check
  digit for a BSB. An Australian
  general ledger account code written `410-100` is therefore replaced with a
  BSB placeholder, and a real workpaper contains a great many of them. The
  manifest counts will look wrong until you read them that way.
- **`verify` prints finding kinds and locations, without detected values.**
  Inspect the source document privately to review each finding.
- **`verify` re-runs the same detection over the output.** It is not a second,
  independent detector. It catches redaction applied wrongly, a file redacted
  against a different map, a half-redacted file and an output edited by hand.
  It cannot catch a detection bug: what the detector could not see going in it
  cannot see coming out. A clean `verify` says the output agrees with the
  detector, not that the output is clean.
- **Replacement can add a space at a touching junction.** A punctuation-starting
  phone beside a name becomes `PERSON_01 PHONE_01`; two touching mapped values
  become separate placeholders. Restore retains that space. Existing emphasis
  between accepted matches keeps its spacing.
- **Some mapped junctions remain unsupported.** A punctuation-edged value after
  a word character, such as `(John Smith)` after `0412 345 678`, can be missed.
  Different mapped values sharing an emphasis delimiter can overlap, leaving
  the later value unmatched, as in `*(Jane Roe)*(John Smith)`. A placeholder-like
  map value such as `XTFN_01` can also rewrite structured output inside a longer
  word. Avoid such map values and inspect the whole output before sending;
  `verify` shares these detection limits.
- **Unmapped names can escape the residual sweep.** It looks for 2 or
  3 capitalised Latin-1 tokens and filters statutory vocabulary. Lower-case
  names, single names, other scripts and names containing statutory words can
  pass unnoticed, and so can a name whose words are split by markup rather than
  spaces, such as `__John__ __Smith__`, `John<br>Smith` or a blockquote wrap.
  Dates and addresses also have limited format coverage.
  Seed known values in the map and inspect the whole output before sending.
- **evatt does not detect an ATO client reference.** No single fixed published
  format exists for one, so a pattern would be guesswork producing either noise
  or false confidence, and a detector nobody can calibrate is worse than a
  documented gap. A document carrying a client reference is the human check's
  job, not the tool's.
- **`restore --out` is not gitignore-guarded.** It writes real names to a path
  you choose, anywhere, so the tool cannot know what rule would cover it. The
  map, the disclosure record and the triage path are guarded,
  because those are either the key or a path evatt derived rather than one you
  typed. Where the restored answer lands is yours to keep out of a commit.
- **A labelled identifier is admitted on the label alone**, with no check
  digit, because something wrote 'TFN' next to those digits and a typo in a
  real tax file number is still a real tax file number. Only a bare digit run
  has to pass a check digit to be replaced.
- **The triage file carries whole source lines** as context, taken from the
  redacted text rather than the input. It still names every candidate it wants
  classified, so it is gitignored as `*.triage.md` and is not a file to attach
  to anything.
- **`--out` refuses to equal the map or the input**, including the manifest and
  triage paths it derives from `--out`. The map is the only copy of the key,
  and one mistyped path used to overwrite it with redacted markdown.
  The redacted document, manifest and triage paths must also refer to separate
  files, including through symbolic or hard links. A collision returns exit 1
  before any file is written or removed, preserving the previous files.
- **CRLF input is normalised to LF for detection.** The CLI writes the file
  back with the ending its source carried. Restoration uses the map's spelling
  and whitespace, so case variants and wrapped names do not round trip byte
  for byte. A file that mixes endings is normalised to its dominant one.
- **Input is composed to Unicode NFC for detection**, and the sanitised file
  is written composed. A name the map holds as one code point and a document
  spells with a combining mark are one name to both `redact` and `verify`.
- **Markdown between a label and its digits does not break the label.**
  `**TFN**: 123 456 783`, `__TFN__: 123 456 783`, `**ABN no.**: 51 824 753 557`,
  `TFN: **123 456 783**`, `` TFN: `123 456 783` `` and a table cell
  `| TFN | 123 456 783 |` are all labelled identifiers. Emphasis around a
  qualifier alone, as in `Medicare __card__ number:`, breaks the label.
- **Labelled TFNs, ABNs, ACNs and Medicare numbers accept dots, commas and
  Unicode dashes between digits**, as well as up to 2 spaces, tabs or hyphens.
  Existing wrapped identifiers still work. Labels include `tax file no.`,
  `tax file number (TFN)`, `tax file no. (TFN)`, `Australian business number
  (ABN)` and `Australian company number (ACN)`, with `no.` also accepted in the
  last 2. An optional `is`, `was` or `of` can precede the digits.
  Bare identifiers retain space/hyphen grouping; dotted and comma-grouped
  fragments are never removed as bare TFNs, ABNs, ACNs, Medicare numbers or BSBs.
  Slash and Unicode-dash numeric continuations likewise prevent a partial
  one-way replacement; `123456782/00` reaches numeric triage as a whole token.
- **TFN, ABN, ACN and Medicare digit continuations that look like numbered
  list markers halt.** `TFN: 12345678` followed by `1. Next item`, or
  `TFN: 1234567` followed by `82) next`, reaches numeric triage. Longer list
  indices and Unicode spaces around the marker also reach triage. The ambiguity
  halts when each line has fewer than 8 digits or an entity map targets
  the label or digits. Genuine hard wraps such as `TFN: 123` followed by
  `456 782` still replace. This guard covers these four identifier families;
  phone numbers retain their existing detection rules.
- **Medicare replaces the card and an optional individual reference number
  together on the same line.** A reference from 1 to 9 joined by up to 2 spaces,
  tabs, slashes or hyphens is consumed, as in `2123 45670 1/1`. The ten-digit card
  core determines its check digit and placeholder, so the same card with and
  without a reference gets one placeholder. A malformed reference reaches triage.
  **Accepted limit:** an individual reference written alone on the next line
  remains in the output. `Medicare: 2123456701` followed by a line containing `1`
  becomes `Medicare: MEDICARE_01` followed by that same `1`. The ten-digit card
  is replaced; the reference alone is below the number sweep's 8-digit floor.
  Ryan accepted this limit on 11 October 2026 (DECISIONS.md ruling 47).
- **Three or more spaces between digit groups remain unsupported.** Short
  groups split that way can still pass without a halt. A labelled number with
  another digit group up to 2 spaces after it, as in `TFN: 123456783 2026`, can fail
  structured detection and now reaches the number halt when the full token
  meets its rules. `Client TFN is 123 456 789` replaces the TFN but still halts
  on the name candidate `Client TFN`; `Tax File Number (TFN)` can likewise
  report `File Number`. These name false positives need a mapped entity.
- **A mapped name is matched through the markdown most documents put in it.**
  `_Jane Roe_`, `**Jane Roe**`, `Jane<br>Roe` in a table cell, a name wrapped
  inside a blockquote, `Jane_Roe` and a zero-width joiner between the words are
  all replaced, in upper, lower or mixed case, and the placeholder keeps the
  emphasis around it. These shapes are not reliably redacted, and most pass
  without a halt: first and last names in separate table columns, initials such as
  `J. Roe`, `Jane&nbsp;Roe`, `Jane <em>Roe</em>`, a backslash hard break between
  the words, a name split across code spans and a zero-width character inside a
  word. Read the whole output for them. Matching through markup also reaches
  text that is not a name: a mapped `Net Profit` replaces `` `net*profit` `` in
  a code span, and a name split by a blank line or a `***` rule is replaced as
  one.

Contextual re-identification is not addressed by any of this. Read the
document before you send it.

## The entity map

One JSON file, `schema_version` and `entries`, each entry holding a real value,
its placeholder, a kind (`client`, `person`, `staff` or `entity`) and the date
it was added. Keep the map append-only: never delete an entry or change what
its placeholder means. Allocate a new ordinal above the highest one ever used
for that kind; deleting the highest entry can lead to its placeholder being reissued.
`evatt/samples/entities.sample.json` is a fabricated map of the right shape.

## Disclosure evidence

`disclosure-record` records local evidence for an intended disclosure.
`disclosure-check` checks that evidence against independently supplied context.
Neither command authenticates a human decision or grants permission to send.

```bash
evatt disclosure-record --in build/notes.md --map entities.json \
  --destination model:sample-tenant:sample-project --decision-ref sample-decision:42 \
  --out build/notes.disclosure.json
evatt disclosure-check --in build/notes.md --map entities.json \
  --destination model:sample-tenant:sample-project --decision-ref sample-decision:42 \
  --record build/notes.disclosure.json
```

Both commands read the file once as immutable bytes and require UTF-8 without
a byte order mark, including empty input. The existing verifier scans decoded
text using its documented CRLF and Unicode NFC normalisation. The payload digest
hashes the original bytes unchanged, including line endings, Unicode composition
and trailing whitespace. Detection retains its documented limits.
The existing replacement-count manifest stays unchanged.

The separate JSON record has exactly `schema`, `output_sha256`,
`entity_map_sha256`, `tool_version`, `destination` and `decision_ref`.
The schema is `evatt.disclosure-evidence/v1`. Its map digest hashes
`evatt.entity-map/v1` followed by a zero byte and the loaded map's JSON: ASCII
escapes, sorted object keys, separators `,` and `:`, `schema_version`, and every
entry's `value`, `placeholder`, `kind` and `added`. Entry order is retained;
map-file formatting is ignored. Only the digest is stored. Candidate maps can
be hashed and compared, so this record remains sensitive metadata.

Destination IDs use 1 to 128 lower case ASCII letters, digits, dots, underscores,
colons or hyphens, starting with a letter or digit. Decision references use the
same form but also permit upper case letters. Inputs are compared exactly;
evatt does not trim them, resolve them or convert them to URLs. Use opaque IDs
that distinguish the actual tenant, account and resource. Never put names,
credentials or bearer tokens in either field.
Use non-secret IDs: command-line arguments can appear in local shell history
and process inspection.

Checking rejects changed bytes, map entries, destination, decision reference or
tool version. It also rejects unknown schemas, duplicate or unknown fields,
incorrect types and hashes, and records larger than 4,096 bytes. Both identifiers
are required on each invocation; do not copy the expected context from the record.

Creation refuses existing paths, including links, and the input's reserved
manifest and triage paths. Checking refuses linked or non-regular record files.
Record paths must not contain links. Both commands require ignored, untracked
records at the time they run. Use a trusted local worktree: these checks do not
confine a process that can change its directories concurrently. Failed writes
can leave an incomplete record, which checking rejects. On Windows, directory
permissions control access; POSIX creation uses owner-only file permissions.

The CLI operates on one immutable snapshot of the supplied payload bytes.
The Python library API is removed in 1.0.0; use the two commands above.

**Matching evidence is not permission.** The record is unsigned and can be edited
consistently. An outer sender must independently authenticate the external human
decision for the exact payload and actual destination, then send only the checked
bytes. Reopening the file, re-encoding it, adding message text or attaching another
file needs separate validation. evatt sends nothing and cannot prevent manual
copying or authenticate the external decision.

## Documents

- `DATA-FLOW.md`, the zero-network boundary and the 3 passes.
- `DISCLAIMER.md`, what this package does not decide.
- `CONTRIBUTING.md`, the checks, the fixture rule and the claim rule.
- `SECURITY.md`, the security boundary and how to report a vulnerability.

## Licence

Apache-2.0. See [LICENSE](LICENSE). If you redistribute this work or build on it, keep the attribution in [NOTICE](NOTICE).
