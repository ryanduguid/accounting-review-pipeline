# evatt

| Install distribution | Python import | Command |
| --- | --- | --- |
| `evatt` | `evatt` | `evatt` |

## Scope and assurance boundary

evatt performs **pseudonymisation with local key retention**. It replaces
Australian structured identifiers and known named entities in markdown with
stable placeholders, so that a document can be handed to an external model
without the model receiving the identifiers.

**The key never leaves the local machine.** The entity map is a gitignored file
on disk. It is not transmitted, and no network client exists in this package.

**This does not take the data outside the Privacy Act.** Pseudonymised data
remains personal information in the hands of anyone holding the key. The claim
this package supports is narrower: the recipient cannot re-identify, because
the recipient never receives the map.

The **residual risk is contextual re-identification**. A document can identify
a client through surviving detail alone, such as an industry, a location, a
balance date and a turnover figure appearing together. evatt does not solve
this. A human check before sending is the only mitigation.

evatt is a control that makes a policy enforceable. It is not authority to
disclose anything.

```
evatt redact  --in notes.md --map entities.json --out build/notes.md
evatt verify  --in build/notes.md --map entities.json
evatt restore --in answer.md --map entities.json --out build/answer.md
```

## Requirements

Python 3.10 or later, and `git` on `PATH`. Every command asks git whether it
would let you commit the map, and refuses to run when the answer is yes or
unclear. That question has no answer outside a work tree, so the map, and
therefore the run, must live inside a repository that ignores it. The rule
this package ships covers `entities.json`, `*.entities.json`, `*.triage.md`
and `*.tmp`.

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
line and context. Classify each one into the map or confirm it is noise, then
run again.

This is the point of the tool. A detector that silently passes what it does not
understand is the failure that leaks.

A halt also deletes any output and manifest already sitting at `--out` from an
earlier run. "Nothing was written" has to be true of the directory the operator
is about to send from, not only of the run that just stopped.

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
- **`verify` prints the values it found to the terminal.** That is what makes
  it useful in front of someone else, and it also means a full tax file number
  can land in a shell recording, a scrollback buffer or a CI log. Run it where
  the output is as private as the document.
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
  back with the ending its source carried, so a file with one ending
  throughout round trips byte for byte. A file that mixes endings is
  normalised to its dominant one.

Contextual re-identification is not addressed by any of this. Read the
document before you send it.

## The entity map

One JSON file, `schema_version` and `entries`, each entry holding a real value,
its placeholder, a kind (`client`, `person`, `staff` or `entity`) and the date
it was added. Placeholders are stable and are never reissued, so a placeholder
already written into a redacted document cannot come to mean somebody else.
`evatt/samples/entities.sample.json` is a fabricated map of the right shape.

## Documents

- `DATA-FLOW.md`, the zero-network boundary and the three passes.
- `DISCLAIMER.md`, what this package does not decide.
- `CONTRIBUTING.md`, the checks, the fixture rule and the claim rule.
- `SECURITY.md`, the security boundary and how to report a vulnerability.

## Licence

MIT. See `LICENSE`.
