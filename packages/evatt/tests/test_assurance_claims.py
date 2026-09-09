"""Pins what the documentation may and may not claim.

Modelled on packages/elizabeth-anne-alexander/tests/test_assurance_claims.py.
The retired list is the important half: pseudonymisation is a risk reduction,
not a compliance conclusion, and the wording must not drift into the latter.
"""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
README = (ROOT / "README.md").read_text(encoding="utf-8")
DATA_FLOW = (ROOT / "DATA-FLOW.md").read_text(encoding="utf-8")
DISCLAIMER = (ROOT / "DISCLAIMER.md").read_text(encoding="utf-8")
CONTRIBUTING = (ROOT / "CONTRIBUTING.md").read_text(encoding="utf-8")
SECURITY = (ROOT / "SECURITY.md").read_text(encoding="utf-8")

ALL_DOCS = (README, DATA_FLOW, DISCLAIMER, CONTRIBUTING, SECURITY)


def normalise(text: str) -> str:
    return " ".join(text.casefold().split())


def test_readme_states_the_boundary_before_any_product_claim() -> None:
    opening = normalise(README[: README.index("```")])
    assert "pseudonymisation with local key retention" in opening
    assert "the key never leaves the local machine" in opening


def test_public_docs_state_the_privacy_act_position() -> None:
    for document in (README, DATA_FLOW, DISCLAIMER):
        assert "this does not take the data outside the privacy act" in normalise(document)


def test_public_docs_state_the_residual_risk() -> None:
    for document in (README, DISCLAIMER):
        assert "residual risk is contextual re-identification" in normalise(document)


def test_readme_keeps_the_two_disclosures_that_flatter_nobody() -> None:
    """The honest detail is the first thing a tidying edit drops.

    Both are behaviours an operator only finds out about by being bitten:
    the BSB pattern fires on any hyphenated three-three digit pair, so
    Australian GL account codes produce BSB placeholders, and ``verify``
    prints what it found, so a full tax file number can land in a shell
    recording. Neither is a defect the package can fix, which is exactly why
    the README has to keep saying so.
    """
    readme = normalise(README)
    assert "410-100" in readme
    assert "prints the values it found" in readme


def test_the_readme_states_the_sign_off_gate() -> None:
    """Spec section 11 requires the gate in the policy AND in the package README.

    Nothing in any package document said client data must not pass through the
    tool before the policy is signed off, and "we were only testing" is exactly
    the sentence the gate exists to answer.
    """
    readme = normalise(README)
    assert "no client data passes through evatt until the policy governing it" in readme
    assert "that includes testing" in readme


def test_the_docs_record_the_ato_client_reference_as_a_named_limit() -> None:
    """Spec section 3 asked for a detector. The recorded decision is not to invent one.

    There is no single published fixed format for an ATO client reference, so a
    pattern would be guesswork producing either noise or false confidence. The
    gap is documented rather than filled, and it has to stay documented: an
    undetected identifier nobody has been told about is the same failure as a
    detector that misses one.
    """
    for document in (README, DATA_FLOW):
        assert "ato client reference" in normalise(document)


def test_verify_is_not_described_as_an_independent_detector() -> None:
    """It re-runs the same detection, which catches an application bug and not a detection one.

    A mapped name in lower case was the counter-example: redaction replaced
    nothing, the residual sweep could not report it, and verify, running the
    same detection, exited 0 on the leaked file.
    """
    assert "re-runs the same detection" in normalise(README)


def test_the_gitignore_claim_is_scoped_to_this_repository() -> None:
    """The rules ship in neither artefact, so "the rule this package ships" was false.

    Anyone installing evatt from PyPI gets no .gitignore at all, and the map
    guard then refuses every command until they write their own.
    """
    for document in (README, DATA_FLOW):
        assert "this repository" in normalise(document)
    assert "must write your own" in normalise(README)


def test_overclaims_never_appear() -> None:
    """The durable half of this file.

    Each entry is a substring, chosen to catch the family rather than one
    spelling: "anonymis" takes anonymise, anonymised and anonymisation,
    "guarantee" takes the plural, and "not personal information" takes any
    sentence claiming the output has left the Act. Nothing in the documented
    position needs any of them, so a failure here is a claim that grew, not a
    sentence that was worded awkwardly.
    """
    public_docs = normalise("\n".join(ALL_DOCS))
    retired = (
        "anonymis",
        "anonymiz",
        "anonymous",
        "anonymity",
        "not personal information",
        "privacy act compliant",
        "compliant with the privacy act",
        "cannot be re-identified",
        "safe to send",
        "fully compliant",
        "guarantee",
        # verify re-runs the detection it is checking, so it can catch a
        # redaction application bug and never a detection bug. Both of these
        # wordings claimed the second, and finding 1 was the counter-example.
        "independently of the redaction",
        "second chance to be caught",
    )
    for claim in retired:
        assert claim not in public_docs, claim


def test_docs_carry_no_em_dash() -> None:
    for document in ALL_DOCS:
        assert "\u2014" not in document
