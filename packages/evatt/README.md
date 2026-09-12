# evatt

| Install distribution | Python import | Command |
| --- | --- | --- |
| `evatt` | `evatt` | `evatt` |

## Scope and assurance boundary

evatt performs **pseudonymisation with local key retention**. It replaces
Australian structured identifiers and known named entities in markdown with
placeholders, so that a document can be handed to an external model without the
model receiving the identifiers.

Entity placeholders are stable across documents, because they come from the
map: `CLIENT_01` means the same client in every file redacted against it.
Structured placeholders are not. They are numbered per document, so `TFN_01` in
one workpaper and `TFN_01` in the next are two different tax file numbers.
Putting two redacted documents in front of one model is putting two meanings of
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

**No client data passes through evatt until the policy governing it is signed
off, and that includes testing.** A trial run is a disclosure of the document to
whatever the operator does with the output, and a control being evaluated is not
yet a control anyone has agreed to rely on. Use the fabricated files under
`evatt/samples/` until the sign-off exists.

```
evatt redact  --in notes.md --map entities.json --out build/notes.md
evatt verify  --in build/notes.md --map entities.json
evatt restore --in answer.md --map entities.json --out build/answer.md
```

## Requirements

Python 3.10 or later, and `git` on `PATH`. Every command asks git whether it
would let you commit the map, and refuses to run when the answer is yes or
unclear. A halt asks the same question about the triage path before it writes
one. That question has no answer outside a work tree, so the map, and therefore
the run, must live inside a repository that ignores it.

The rules covering `entities.json`, `*.entities.json`, `*.triage.md` and `*.tmp`
live in this repository's own `.gitignore`, which ships in neither the wheel nor
the source distribution. **If you installed evatt as a package, you have none of
them and must write your own**, in the repository your map and your run live in.
Three kinds of path are guarded, and a rule has to cover every one of them:

```
entities.json
*.entities.json
*.tmp
*.triage.md
```

The first two are the map itself, under whichever of the two names you use. The
`*.tmp` rule covers the temporary `save` writes beside the map before renaming
it over the top, which is named `<map>.tmp.<random>.tmp` and holds the same real
values as the map; a hard kill can leave one behind. The last is the triage
worklist a halt writes beside `--out`.

Leave any of them out and the run fails closed, naming the path and the rule it
wants, rather than writing the file.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Clean |
| 2 | Halted on unclassified candidates, or verify found something |
| 1 | Malformed input |

A usage error from the command line is a 1, not argparse's usual 2, so a
mistyped option is never mistaken for a document that needs triage.

## Halting

The residual sweep stops the run when it finds a candidate neither earlier pass
recognised. Nothing is written, and a triage file lists each candidate with its
line and context. Add each candidate to the map, then run again. To clear a
false positive, map that phrase as an `entity`; it will also be replaced and
restored. There is no separate noise allowlist or command-line bypass.

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

- **`BSB` fires on any hyphenated three-three digit pair.** There is no check
  digit for a BSB, so the hyphen is the only evidence there is. An Australian
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
- **Unmapped names can escape the residual sweep.** It looks for two or
  three capitalised Latin-1 tokens and filters statutory vocabulary. Lower-case
  names, single names, other scripts and names containing statutory words can
  pass unnoticed. Dates and addresses also have limited format coverage.
  Seed known values in the map and inspect the whole output before sending.
- **evatt does not detect an ATO client reference.** No single fixed published
  format exists for one, so a pattern would be guesswork producing either noise
  or false confidence, and a detector nobody can calibrate is worse than a
  documented gap. A document carrying a client reference is the human check's
  job, not the tool's.
- **`restore --out` is not gitignore-guarded.** It writes real names to a path
  you choose, anywhere, so the tool cannot know what rule would cover it. The
  map, every temporary written beside it and the triage path are guarded,
  because those are either the key or a path evatt derived rather than one you
  typed. Where the restored answer lands is yours to keep out of a commit.
- **The map's temporary is created exclusively, at a name that does not already
  exist.** `save` writes `<map>.tmp.<random>.tmp` and renames it over the map.
  The name is fresh each time and the file is created with `O_CREAT | O_EXCL`,
  so a symbolic or hard link left at that path is never followed or truncated,
  and a temporary left behind by a hard kill cannot block the next save. On
  Windows `O_NOFOLLOW` does not exist and is not used; exclusive creation is
  what refuses a link on both platforms, because a link is a name that already
  exists and creating one therefore fails. What Windows does not honour is the
  POSIX file mode, so there the temporary is readable by whoever can read the
  directory rather than by its owner alone.
- **A labelled identifier is admitted on the label alone**, with no check
  digit, because something wrote "TFN" next to those digits and a typo in a
  real tax file number is still a real tax file number. Only a bare digit run
  has to pass a check digit to be replaced.
- **The triage file carries whole source lines** as context, taken from the
  redacted text rather than the input. It still names every candidate it wants
  classified, so it is gitignored as `*.triage.md` and is not a file to attach
  to anything.
- **`--out` refuses to equal the map or the input**, including the manifest and
  triage paths it derives from `--out`. The map is the only copy of the key,
  and one mistyped path used to overwrite it with redacted markdown.
- **CRLF input is normalised to LF for detection.** The CLI writes the file
  back with the ending its source carried. Restoration uses the map's spelling
  and whitespace, so case variants and wrapped names do not round trip byte
  for byte. A file that mixes endings is normalised to its dominant one.

Contextual re-identification is not addressed by any of this. Read the
document before you send it.

## The entity map

One JSON file, `schema_version` and `entries`, each entry holding a real value,
its placeholder, a kind (`client`, `person`, `staff` or `entity`) and the date
it was added. Keep the map append-only: never delete an entry or change what
its placeholder means. Assignment uses the highest remaining ordinal plus one;
deleting the highest entry would allow its placeholder to be reissued.
`evatt/samples/entities.sample.json` is a fabricated map of the right shape.

## Documents

- `DATA-FLOW.md`, the zero-network boundary and the three passes.
- `DISCLAIMER.md`, what this package does not decide.
- `CONTRIBUTING.md`, the checks, the fixture rule and the claim rule.
- `SECURITY.md`, the security boundary and how to report a vulnerability.

## Licence

MIT. See `LICENSE`.
