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
    )
    for claim in retired:
        assert claim not in public_docs, claim


def test_docs_carry_no_em_dash() -> None:
    for document in ALL_DOCS:
        assert "\u2014" not in document
