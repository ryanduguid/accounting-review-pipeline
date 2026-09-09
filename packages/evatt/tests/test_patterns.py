"""Check-digit vectors are documented ATO/ASIC test identifiers, not real ones.

They live in test code only. Nothing under evatt/samples/ carries them.
"""
from evatt import patterns

VALID_TFN = "123456782"
VALID_ABN = "51824753556"
VALID_ACN = "000000019"
VALID_MEDICARE = "2123456701"


def test_valid_check_digits_are_accepted() -> None:
    assert patterns.valid_tfn(VALID_TFN)
    assert patterns.valid_abn(VALID_ABN)
    assert patterns.valid_acn(VALID_ACN)
    assert patterns.valid_medicare(VALID_MEDICARE)


def test_one_digit_changed_is_rejected() -> None:
    assert not patterns.valid_tfn("123456783")
    assert not patterns.valid_abn("51824753557")
    assert not patterns.valid_acn("000000018")
    # Index 8 is Medicare's check digit. Index 9 is the issue number and takes
    # no part in the sum, so changing the last digit must NOT invalidate it.
    assert not patterns.valid_medicare("2123456711")


def test_the_medicare_issue_number_is_not_part_of_the_check() -> None:
    assert patterns.valid_medicare("2123456701")
    assert patterns.valid_medicare("2123456702")


def test_wrong_length_is_rejected() -> None:
    assert not patterns.valid_tfn("12345678")
    assert not patterns.valid_abn("5182475355")
    assert not patterns.valid_acn("00000001")
    assert not patterns.valid_medicare("212345670")


def test_medicare_rejects_a_leading_digit_outside_two_to_six() -> None:
    assert not patterns.valid_medicare("7123456701")


def test_structured_spans_find_a_labelled_tfn() -> None:
    spans = patterns.structured_spans(f"TFN: {VALID_TFN}")
    assert [(kind, text) for _s, _e, kind, text in spans] == [("tfn", VALID_TFN)]


def test_structured_spans_find_a_spaced_abn() -> None:
    spans = patterns.structured_spans("ABN 51 824 753 556 applies")
    assert [kind for _s, _e, kind, _t in spans] == ["abn"]


def test_structured_spans_find_email_and_phone() -> None:
    text = "Reach a.person@example.com or 0412 345 678 today"
    kinds = sorted(kind for _s, _e, kind, _t in patterns.structured_spans(text))
    assert kinds == ["email", "phone"]


def test_a_nine_digit_run_valid_as_both_is_reported_as_a_tfn() -> None:
    """Safe direction: on an exact tie the more sensitive kind wins.

    000000019 satisfies both the TFN mod-11 sum and the ASIC ACN check digit,
    so it is the vector that actually exercises the tie-break.
    """
    assert patterns.valid_tfn(VALID_ACN)
    assert patterns.valid_acn(VALID_ACN)
    spans = patterns.structured_spans("000 000 019")
    assert [kind for _s, _e, kind, _t in spans] == ["tfn"]


def test_the_sample_acn_is_not_also_a_valid_tfn() -> None:
    """Pins the fixture choice in evatt/samples/identifiers.md.

    If this ever fails, that sample can no longer produce an "acn" count and
    Task 7's kind-coverage test will fail with it.
    """
    assert patterns.valid_acn("123456780")
    assert not patterns.valid_tfn("123456780")


def test_negatives_from_the_source_module_do_not_trigger() -> None:
    """Cases the origin module already paid a review cycle to learn."""
    for text in (
        "see section 12345678 of the Act",
        "the balance was 1,234,567 at year end",
        "in 2024 the rate changed",
        "Division 7A applies",
    ):
        assert patterns.structured_spans(text) == [], text


def test_bsb_requires_the_hyphen() -> None:
    assert [k for _s, _e, k, _t in patterns.structured_spans("BSB 062-000")] == ["bsb"]
    assert patterns.structured_spans("total 062000 units") == []


def test_person_names_survive_and_statutory_vocabulary_does_not() -> None:
    assert "Jane Roe" in patterns.person_names("Jane Roe attended")
    assert patterns.person_names("The Commissioner of Taxation decided") == set()


def test_person_names_handle_all_caps_and_curly_apostrophes() -> None:
    assert "SMITH, John" in patterns.person_names("SMITH, John was present")
    assert "Mary O\u2019Brien" in patterns.person_names("Mary O\u2019Brien signed")


def test_address_and_date_of_birth_shapes() -> None:
    assert patterns.ADDRESS.search("12 Hunter Street was sold")
    assert patterns.DOB.search("born 14 March 1982 in Newcastle")


def test_placeholder_pattern_matches_assigned_placeholders() -> None:
    assert patterns.PLACEHOLDER.fullmatch("CLIENT_01")
    assert patterns.PLACEHOLDER.fullmatch("TFN_12")
    assert not patterns.PLACEHOLDER.fullmatch("CLIENT")
