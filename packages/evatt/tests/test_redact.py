import pytest

from evatt import redact as redact_module
from evatt.entities import Entity
from evatt.errors import Halt

CLIENT = Entity("Sample Holdings Pty Ltd", "CLIENT_01", "client", "2026-09-09")
PERSON = Entity("Jane Roe", "PERSON_01", "person", "2026-09-09")
SHORT = Entity("Sample", "CLIENT_02", "client", "2026-09-09")
MAP = (CLIENT, PERSON)


def redact(text, entities=MAP):
    return redact_module.redact(text, entities, strict=False)


def test_a_valid_tfn_is_replaced_with_a_typed_placeholder() -> None:
    text, counts = redact("TFN: 123 456 782 on file")
    assert "123 456 782" not in text
    assert "TFN_01" in text
    assert counts["tfn"] == 1


def test_an_invalid_tfn_is_left_alone() -> None:
    text, counts = redact("reference 123 456 783 on file")
    assert "123 456 783" in text
    assert counts.get("tfn", 0) == 0


def test_the_same_identifier_twice_gets_the_same_placeholder() -> None:
    text, _counts = redact("TFN 123 456 782 and again 123 456 782")
    assert text.count("TFN_01") == 2
    assert "TFN_02" not in text


def test_two_identifiers_get_different_placeholders() -> None:
    text, counts = redact("ABN 51 824 753 556 and ABN 53 004 085 616")
    assert "ABN_01" in text and "ABN_02" in text
    assert counts["abn"] == 2


def test_a_mapped_entity_is_replaced() -> None:
    text, counts = redact("Sample Holdings Pty Ltd lodged late")
    assert text == "CLIENT_01 lodged late"
    assert counts["client"] == 1


def test_a_longer_entity_wins_over_a_shorter_one_it_contains() -> None:
    text, _counts = redact("Sample Holdings Pty Ltd reported", (CLIENT, SHORT))
    assert text == "CLIENT_01 reported"


def test_entity_replacement_respects_word_boundaries() -> None:
    text, _counts = redact("Samples were taken", (SHORT,))
    assert text == "Samples were taken"


def test_manifest_records_counts_and_never_values() -> None:
    _text, counts = redact("Jane Roe, TFN 123 456 782")
    assert counts == {"person": 1, "tfn": 1}
    assert all(isinstance(v, int) for v in counts.values())


def test_placeholders_are_not_rescanned_as_identifiers() -> None:
    text, _counts = redact("ABN 51 824 753 556")
    second, counts = redact(text)
    assert second == text
    assert counts == {}


def test_an_unmapped_name_is_reported_as_an_unknown() -> None:
    unknowns = redact_module.residual("John Smith called today")
    assert [(u.kind, u.value) for u in unknowns] == [("name", "John Smith")]
    assert unknowns[0].line == 1
    assert unknowns[0].context == "John Smith called today"


def test_a_mapped_name_is_not_an_unknown() -> None:
    text, _counts = redact("Jane Roe called today")
    assert redact_module.residual(text) == ()


def test_placeholders_are_never_reported_as_unknowns() -> None:
    assert redact_module.residual("CLIENT_01 paid PERSON_01 on time") == ()


def test_statutory_vocabulary_is_not_an_unknown() -> None:
    assert redact_module.residual("The Commissioner of Taxation issued a Determination") == ()


def test_an_address_is_reported() -> None:
    unknowns = redact_module.residual("Site at 12 Hunter Street was sold")
    assert [u.kind for u in unknowns] == ["address"]


def test_a_date_of_birth_is_reported() -> None:
    unknowns = redact_module.residual("Born 14 March 1982")
    assert [u.kind for u in unknowns] == ["date"]


def test_line_numbers_are_one_based_and_accurate() -> None:
    unknowns = redact_module.residual("clean line\nJohn Smith called")
    assert unknowns[0].line == 2


def test_strict_mode_halts_and_reports_every_unknown() -> None:
    with pytest.raises(Halt) as caught:
        redact_module.redact("John Smith and Mary Jones met", MAP)
    values = {u.value for u in caught.value.unknowns}
    assert values == {"John Smith", "Mary Jones"}


def test_non_strict_mode_does_not_halt() -> None:
    text, _counts = redact("John Smith called")
    assert "John Smith" in text


def test_a_name_split_by_a_line_wrap_is_reported() -> None:
    """NAME's ``\\s+`` matches a newline, so a per-line scan could never see this."""
    unknowns = redact_module.residual("the return prepared for John\nSmith was lodged late")
    assert [(u.kind, u.value) for u in unknowns] == [("name", "John\nSmith")]
    assert unknowns[0].line == 1
    assert unknowns[0].context == "the return prepared for John"


def test_a_wrapped_name_reports_the_line_it_starts_on() -> None:
    unknowns = redact_module.residual("a\nb\nprepared for John\nSmith was late")
    assert [(u.line, u.value) for u in unknowns] == [(3, "John\nSmith")]


def test_the_other_breaks_splitlines_honours_cannot_hide_a_name_either() -> None:
    for break_character in ("\u2028", "\x0b", "\x0c", "\x85"):
        unknowns = redact_module.residual("prepared for John" + break_character + "Smith today")
        assert [u.kind for u in unknowns] == ["name"], repr(break_character)


def test_an_address_does_not_shift_the_line_a_later_name_reports_on() -> None:
    """The address mask is the same length as what it covers, so offsets still line up."""
    unknowns = redact_module.residual("12 Hunter Street was sold\nJohn Smith called")
    assert [(u.kind, u.line) for u in unknowns] == [("address", 1), ("name", 2)]
    assert unknowns[1].context == "John Smith called"


def test_overlapping_entity_values_leave_no_fragment_of_a_person() -> None:
    """Sorting by length handles containment; only a single scan handles overlap."""
    overlap = (PERSON, Entity("Roe Holdings", "CLIENT_05", "client", "2026-09-09"))
    text, counts = redact("Jane Roe Holdings lodged the return", overlap)
    assert text == "PERSON_01 Holdings lodged the return"
    assert counts == {"person": 1}
    assert redact_module.residual(text) == ()
    # The order the map happens to be in must not decide the outcome.
    assert redact("Jane Roe Holdings lodged the return", tuple(reversed(overlap)))[0] == text


def test_an_empty_entity_value_cannot_rewrite_the_document() -> None:
    """re.escape("") gives a pattern that matches at every non-word boundary."""
    for value in ("", "   "):
        blank = (Entity(value, "CLIENT_03", "client", "2026-09-09"),)
        text, counts = redact("Total: $1,000.", blank)
        assert text == "Total: $1,000."
        assert counts == {}


def test_a_placeholder_already_in_the_input_halts() -> None:
    """Otherwise restore writes a real client name where it never appeared."""
    with pytest.raises(Halt) as caught:
        redact_module.redact("CLIENT_01 was the code used.", MAP)
    reported = [(u.kind, u.value, u.line) for u in caught.value.unknowns]
    assert reported == [("placeholder", "CLIENT_01", 1)]
