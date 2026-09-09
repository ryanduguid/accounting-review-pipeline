# Client data pseudonymisation gateway

Date: 2026-09-09
Status: design approved, not yet implemented
Repository: `accounting-review-pipeline`, new package `evatt`

## 1. Problem

Working notes and analysis in markdown contain employer client data. Handing those
files to a capable model (Fable 5.1 in Claude Code, Astra 6 in Codex) would be
useful, but the files carry client names, entity identifiers, contact details
and financials that are subject to APES 110 confidentiality and the Privacy
Act.

The originally proposed design was a corpus of Australian privacy and data
legislation used to generate a prompt corpus, with Copilot performing the
deidentification before the sanitised file reached a stronger model. That
design is rejected for three reasons.

**The sanitiser would be a cloud service.** Sending unsanitised client data to
Copilot so that Copilot can sanitise it is itself the disclosure. Swapping one
vendor for another adds a hop without closing the gap. The sanitiser must run
locally, before anything is transmitted.

**It inverts the trust ordering.** Deidentification is a recall problem, where
a single miss leaks. Placing the least capable model in the chain at the
highest-stakes checkpoint is the wrong way round.

**Legislation does not generate redaction rules.** The Privacy Act 1988, the
Australian Privacy Principles, the TFN Rule, the Notifiable Data Breaches
scheme and the state health records Acts set obligations. None of them
enumerate what to strip from a markdown file. Redaction is driven by an entity
taxonomy plus a policy decision about how far to go. A general-purpose model
already knows the APPs at the level this task needs.

Separately, the identifiers that matter most are deterministic. TFN, ABN, ACN,
BSB and Medicare numbers all carry check digits. A script catches every
well-formed instance; a model catches most of them.

## 2. Scope decisions

Settled during design:

- **Data in scope:** employer client data.
- **Authorisation state:** the firm has no AI use policy. None exists and none has
  been requested.
- **Tool scope:** built for one user now, with clean enough boundaries that it
  could be adopted firm-wide without a rewrite.
- **Location:** a new package inside `accounting-review-pipeline`, alongside
  `elizabeth-anne-alexander`, reusing the monorepo's package layout, per-package
  CI and zero-network design language.

Two consequences follow and are binding on the whole design.

**Perfect deidentification does not grant permission to disclose.** The data
belongs to the firm and to its clients. The tool is a control that makes a
policy enforceable; it is not authority to send anything. No client data passes
through the tool before the policy is signed off, including for testing.

**The claim must be stated narrowly.** Reversible pseudonymisation does not
take data outside the Privacy Act. Pseudonymised data remains personal
information in the hands of anyone holding the key. The defensible claim is
that the recipient cannot re-identify because the key never leaves the local
machine. The words "anonymised" and "Privacy Act compliant" must not appear as
claims about this tool.

## 3. Architecture

A local, zero-network package exposing three commands.

| Command | Purpose |
| --- | --- |
| `redact` | Raw markdown to sanitised markdown, plus a manifest |
| `restore` | Reverses the map over a model's answer, locally |
| `verify` | Re-scans a sanitised file and asserts zero findings |

`verify` exists to be demonstrated. It is the artefact that accompanies the
policy when it goes to a partner.

### Components

**`patterns.py`.** Copied from
`au-tax-legislation-corpus/fadden/pii_patterns.py` with a provenance note, then
extended with ABN (modulus 89), ACN, BSB, Medicare check digit and ATO client
reference.

Copied rather than shared. A common package spanning two repositories for
roughly 200 lines of regular expressions costs more machinery than the drift it
prevents. The provenance note names the origin file so a future reader can
diff them.

The copied module already carries expensive knowledge that must survive the
copy: the ATO TFN check digit weighting, the catastrophic-backtracking fix in
the TFN label separator, the U+2019 curly apostrophe used in Register text, and
name tokens that accept all-caps runs, internal capitals and hyphens.

**`entities.json`.** The map. Local, gitignored, plain JSON. Each record holds
the real value, a stable placeholder, a kind (`client`, `person`, `staff`,
`entity`) and the date added. Seeded from the firm's client list.

Placeholders are stable and typed (`CLIENT_01`, `PERSON_03`, `ENTITY_A`) rather
than a uniform `[REDACTED]`. Stable placeholders preserve the recipient model's
ability to reason across a document and let `restore` put real names back into
its answer. Uniform redaction destroys both.

Placeholder assignment is deterministic on seed order, not on dictionary
iteration or hash order, so the same input and map always produce byte-identical
output.

**`redact.py`.** Four passes.

1. Structured identifiers by regular expression plus checksum, replaced with
   typed placeholders.
2. Known entities from the map, replaced with their assigned placeholders.
3. Residual sweep for anything name-shaped, address-shaped or date-shaped that
   neither earlier pass recognised.
4. Emit the sanitised markdown and a `manifest.json` recording counts by type
   and no values.

**`restore.py`.** Reverses the map over a model's answer, locally.

## 4. Data flow

```
raw .md
  -> redact          (local, no network)
  -> sanitised .md + manifest.json
  -> Fable 5.1 / Astra 6
  -> answer carrying placeholders
  -> restore         (local, no network)
  -> answer carrying real names
```

`entities.json` never leaves the disk. That single property is what the policy
claim rests on, and it is the property the tests must pin.

## 5. The safety property: halt on unknown

Pass three halts. On encountering a candidate it does not recognise, the tool
writes no output file and instead emits a triage file listing each unknown with
surrounding context lines. The operator classifies every unknown into the map
or marks it noise, then re-runs.

A detector that silently passes what it does not understand is the failure mode
that leaks. One that stops and asks is defensible even when its detection is
crude, which is what makes a regular-expression-grade tool something that can be
put in front of a partner.

The tool also refuses to run when `entities.json` is not covered by
`.gitignore`.

## 6. Testing

Synthetic fixtures only, matching the `elizabeth-anne-alexander` posture. Four
test modules following the package's existing conventions.

**`test_patterns.py`.** Checksum tests for TFN, ABN, ACN and Medicare, valid
and invalid. Ports the negative cases the source module already paid for:
eight-digit statutory references, comma-grouped dollar amounts, bare years and
spaced appropriation amounts must not trigger.

**`test_redact.py`.** Four properties.

- Round trip: `restore(redact(x)) == x` for every fixture.
- Halt on unknown: a fixture containing an unmapped name-shaped token must
  raise and the output file must not exist. Asserting only that it raised is
  insufficient.
- Determinism: identical output across two separate processes with
  `PYTHONHASHSEED` unset.
- No leakage: `verify` returns zero findings on every redacted fixture.

**`test_assurance_claims.py`.** Reuses the existing pattern from
`packages/elizabeth-anne-alexander/tests/test_assurance_claims.py`. Pins the
required README wording, and blocks the retired claims "anonymised",
"de-identified data is not personal information", "Privacy Act compliant" and
"safe to send". This test is what prevents a later README edit from quietly
upgrading the claim.

**`test_packaging.py`.** House convention, plus assertions that the map file is
gitignored and that no fixture contains a checksum-valid TFN or ABN.

### Fixtures

Generated, never produced by redacting a real document. That rule belongs in
CONTRIBUTING, because sanitising a real file with an imperfect first pass is
how real data reaches a public repository.

Coverage: a clean file; one containing every structured identifier type; one
containing an unmapped name to exercise the halt path; one containing the
negative cases (statutory references, dollar amounts, years); and one containing
curly apostrophes and all-caps names.

## 7. Out of scope for v1

- **A local model for name detection.** The seeded client list plus the
  halt-on-unknown rule covers the job without adding a dependency and a new
  failure mode.
- **An audit log.** Add it when the policy goes firm-wide and someone other
  than the operator needs to evidence usage.
- **Encryption of the map.** The raw source file sits in the same directory.
  Encrypting the key while the plaintext is beside it is theatre.
- **A Claude Code hook.** Add it after enough manual CLI runs to know the real
  halt rate. A hook that stops a session before the operator understands why
  will simply be disabled.
- **A PyPI release.** Internal until the policy is signed off.
- **GUI, daemon, watch mode.**

## 8. The policy document

The policy matters more than the code. Six short parts.

1. What the control is and what it is not. Pseudonymisation with local key
   retention. Explicitly not anonymisation, not a substitute for client consent
   or an engagement-letter term, and not something that removes the data from
   the Privacy Act.
2. Named scope: one user, one machine, one class of work, an enumerated list of
   permitted destinations.
3. What must never be sent even after redaction: anything where the client is
   identifiable from context alone, health information, and anything under a
   specific confidentiality undertaking.
4. The halt rule, and who is permitted to classify unknowns.
5. Review trigger and review date.
6. Who signs off, and what changes require re-approval.

It must include the honest statement that the residual risk is contextual
re-identification, that the tool does not solve it, and that a human check
before sending is the only mitigation.

## 9. Sequencing and gates

1. **Build the package and fixtures.** Synthetic data only. Requires no
   authorisation from anyone.
2. **Draft the policy** with the package as its enforcing control.
3. **Table both** to whoever owns risk at the firm.
4. **Only after sign-off**, client data may flow, within the named scope.

Step 4 is a hard gate. The tool is not run over real client data before
sign-off under any framing, including testing or evaluation.

## 10. Open decisions

**Employment agreement IP position.** `accounting-review-pipeline` is public,
and this package handles a class of employer work product. The EA's intellectual
property clauses should be checked before the branch is pushed, not after.

## 11. Risks

| Risk | Mitigation |
| --- | --- |
| Contextual re-identification from surviving detail | Human check before sending; stated openly in the policy |
| Claim inflation in later documentation edits | `test_assurance_claims.py` retired-claims list |
| Halt rate high enough that the operator disables the check | Seed the map from the full client list first; measure before adding the hook |
| Copied patterns drift from the source module | Provenance note naming the origin file |
| Real data entering fixtures | Generated fixtures only, enforced in CONTRIBUTING and `test_packaging.py` |
| Policy never signed, tool used anyway | Step 4 gate stated in the policy and in the package README |
