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
