import subprocess
import sys
from pathlib import Path

import pytest

from evatt import entities as entities_module
from evatt import patterns
from evatt import redact as redact_module
from evatt import verify as verify_module
from evatt.entities import Entity
from evatt.errors import Halt
from evatt.restore import restore

CLIENT = Entity("Sample Holdings Pty Ltd", "CLIENT_01", "client", "2026-09-09")
PERSON = Entity("Jane Roe", "PERSON_01", "person", "2026-09-09")
SHORT = Entity("Sample", "CLIENT_02", "client", "2026-09-09")
MAP = (CLIENT, PERSON)
ENTITY_ONLY = "Jane Roe of Sample Holdings Pty Ltd lodged on time"
STRUCTURED = "Jane Roe, TFN 123 456 782, ABN 51 824 753 556, BSB 062-000"


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


def test_an_accented_street_name_is_reported_as_an_address_not_a_person() -> None:
    """ADDRESS was ASCII-only while NAME was not, so the kind came out wrong.

    "12 Gr\u00fcner Street" missed ADDRESS, so it was never masked before the
    name sweep and came back as the person "Gr\u00fcner Street". The halt fired
    either way, so nothing leaked, but triage was asked the wrong question about
    the wrong kind of thing.
    """
    for text, value in (
        ("Site at 12 Gr\u00fcner Street was sold", "12 Gr\u00fcner Street"),
        ("Site at 4 \u00d6lund Road was sold", "4 \u00d6lund Road"),
        (
            "Site at 9 Jos\u00e9 Ram\u00edrez Avenue was sold",
            "9 Jos\u00e9 Ram\u00edrez Avenue",
        ),
    ):
        unknowns = redact_module.residual(text)
        assert [(u.kind, u.value) for u in unknowns] == [("address", value)], text


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


def test_a_hand_built_placeholder_value_cannot_destroy_a_real_placeholder() -> None:
    """``load`` refuses this value, but ``redact`` takes any Sequence[Entity]."""
    forged = (Entity("TFN_01", "CLIENT_07", "client", "2026-09-09"),)
    text, counts = redact("TFN: 123 456 782 on file", forged)
    assert "123 456 782" not in text
    assert text == "TFN: TFN_01 on file"
    assert "CLIENT_07" not in text
    assert counts == {"tfn": 1}


def test_a_placeholder_already_in_the_input_halts() -> None:
    """Otherwise restore writes a real client name where it never appeared."""
    with pytest.raises(Halt) as caught:
        redact_module.redact("CLIENT_01 was the code used.", MAP)
    reported = [(u.kind, u.value, u.line) for u in caught.value.unknowns]
    assert reported == [("placeholder", "CLIENT_01", 1)]


def test_round_trip_on_named_entities_is_exact() -> None:
    text, _counts = redact_module.redact(ENTITY_ONLY, MAP)
    assert restore(text, MAP) == ENTITY_ONLY


def test_structured_identifiers_never_come_back() -> None:
    text, _counts = redact(STRUCTURED)
    restored = restore(text, MAP)
    for original in ("123 456 782", "51 824 753 556", "062-000"):
        assert original not in restored


def test_restore_does_not_confuse_a_placeholder_with_its_prefix() -> None:
    many = tuple(
        Entity(f"Client Number {n}", f"CLIENT_{n:02d}", "client", "2026-09-09")
        for n in (1, 10, 100)
    )
    text = "CLIENT_01 CLIENT_10 CLIENT_100"
    assert restore(text, many) == "Client Number 1 Client Number 10 Client Number 100"


def test_the_prefix_guard_is_checked_on_values_that_cannot_heal_themselves() -> None:
    """"Client Number 10" plus the leftover "0" spells the right answer by luck.

    Dropping the ``(?!\\d)`` therefore leaves the test above passing. These
    values have no such arithmetic, so the shorter placeholder eating the
    longer one is visible.
    """
    many = (
        Entity("Alpha", "CLIENT_10", "client", "2026-09-09"),
        Entity("Beta", "CLIENT_100", "client", "2026-09-09"),
    )
    assert restore("CLIENT_10 and CLIENT_100", many) == "Alpha and Beta"


def test_restore_leaves_a_placeholder_with_no_left_boundary_alone() -> None:
    """A ledger column, a code fence or a heading of this shape passes both guards.

    ``redact`` and ``verify`` read placeholders through PLACEHOLDER, which
    carries a left boundary, so neither reports "PRIOR_CLIENT_01". While
    ``restore`` reimplemented the match without one, it wrote the real client
    name into a position it had never occupied, in front of the partner.
    """
    for text in ("PRIOR_CLIENT_01 column", "XCLIENT_01 filed", "9CLIENT_01 row"):
        assert restore(text, MAP) == text, text


def test_restore_leaves_a_placeholder_followed_by_a_word_character_alone() -> None:
    """Symmetric rather than trailing-digit: unrestored is visible, wrong is not."""
    for text in ("CLIENT_01s lodged", "CLIENT_01_old lodged", "CLIENT_01A lodged"):
        assert restore(text, MAP) == text, text


def test_redact_verify_and_restore_agree_on_where_a_placeholder_can_start() -> None:
    """One shared boundary, so a guard cannot miss what ``restore`` will rewrite.

    The "XCLIENT_01" hole was exactly this disagreement: both guards use
    PLACEHOLDER and saw nothing, while ``restore`` built its own match without
    the left boundary and rewrote it anyway. ``verify`` is asked about an
    unassigned placeholder, because an assigned one standing alone is the
    residual case it cannot report.
    """
    for template, is_placeholder in (
        ("{} filed the return", True),
        ("the {}, filed", True),
        ("({}) filed", True),
        ("PRIOR_{} column", False),
        ("X{} filed", False),
        ("9{} row", False),
    ):
        assigned = template.format("CLIENT_01")
        halted = False
        try:
            redact_module.redact(assigned, MAP)
        except Halt as caught:
            halted = any(u.kind == "placeholder" for u in caught.unknowns)
        reported = [f.kind for f in verify_module.findings(template.format("CLIENT_09"), MAP)]
        assert halted is is_placeholder, template
        assert (reported == ["placeholder"]) is is_placeholder, template
        assert (restore(assigned, MAP) != assigned) is is_placeholder, template


def test_restore_is_never_more_permissive_than_the_two_guards() -> None:
    """PLACEHOLDER has no right boundary on purpose, and that asymmetry is safe.

    Both guards report "CLIENT_01s", because a placeholder-shaped token in an
    input is something an operator has to be told about. ``restore`` declines
    it, so the token stays visibly unrestored instead of silently becoming
    "Sample Holdings Pty Ltds".
    """
    with pytest.raises(Halt) as caught:
        redact_module.redact("CLIENT_01s lodged", MAP)
    assert [u.value for u in caught.value.unknowns] == ["CLIENT_01"]
    found = verify_module.findings("CLIENT_09s lodged", MAP)
    assert [(f.kind, f.value) for f in found] == [("placeholder", "CLIENT_09")]
    assert restore("CLIENT_01s lodged", MAP) == "CLIENT_01s lodged"


def test_restore_refuses_a_hand_built_entry_that_would_undo_pass_one() -> None:
    """``load`` rejects this map, but ``restore`` takes any Sequence[Entity].

    Structured identifiers are replaced one-way and nothing records their
    values, so an entity pointing a real TFN back at TFN_01 is the one thing
    that could undo the guarantee. The claimed kind is not consulted, because a
    hand-built entity can claim anything; the placeholder is what is checked.
    """
    text, _counts = redact_module.redact("TFN: 123 456 782 on file", MAP)
    assert text == "TFN: TFN_01 on file"
    forged = (Entity("123 456 782", "TFN_01", "client", "2026-09-09"),)
    assert restore(text, forged) == text
    assert "123 456 782" not in restore(text, forged)
    for prefix in ("TFN", "ABN", "ACN", "BSB", "MEDICARE", "EMAIL", "PHONE"):
        one = (Entity("a real value", f"{prefix}_01", "client", "2026-09-09"),)
        assert restore(f"{prefix}_01 on file", one) == f"{prefix}_01 on file", prefix


def test_restore_ignores_a_placeholder_that_is_empty_or_misshapen() -> None:
    """An empty placeholder matches at every token edge and scatters the value."""
    for placeholder, text in (
        ("", "Total: $1,000."),
        ("   ", "a   b"),
        ("CLIENT_1", "CLIENT_1 filed"),
        ("client_01", "client_01 filed"),
        ("TOTAL_01", "TOTAL_01 filed"),
    ):
        forged = (Entity("Sample Holdings Pty Ltd", placeholder, "client", "2026-09-09"),)
        assert restore(text, forged) == text, repr(placeholder)


def test_restore_refuses_a_value_that_carries_another_placeholder() -> None:
    """The loop is sequential, so such a value is rewritten again downstream."""
    cascade = (Entity("PERSON_01 trading", "CLIENT_01", "client", "2026-09-09"), PERSON)
    assert restore("CLIENT_01 lodged", cascade) == "CLIENT_01 lodged"


def test_verify_finds_nothing_in_redacted_output() -> None:
    text, _counts = redact_module.redact(ENTITY_ONLY, MAP)
    assert verify_module.findings(text, MAP) == ()


def test_verify_finds_a_surviving_identifier() -> None:
    found = verify_module.findings("TFN 123 456 782", MAP)
    assert [f.kind for f in found] == ["tfn"]


def test_verify_finds_a_surviving_mapped_entity() -> None:
    found = verify_module.findings("Jane Roe was here", MAP)
    assert [f.kind for f in found] == ["person"]


def test_verify_finds_an_unmapped_name() -> None:
    found = verify_module.findings("John Smith was here", MAP)
    assert [f.kind for f in found] == ["name"]


def test_verify_finds_a_placeholder_the_map_cannot_reverse() -> None:
    """An entity placeholder with no map entry is a literal, or the wrong map."""
    found = verify_module.findings("CLIENT_09 filed the return", MAP)
    assert [(f.kind, f.value) for f in found] == [("placeholder", "CLIENT_09")]


def test_verify_finds_a_placeholder_carried_in_beside_its_own_value() -> None:
    """A half-redacted file: the input wrote CLIENT_01 itself, redaction did not."""
    found = verify_module.findings("CLIENT_01 is Sample Holdings Pty Ltd", MAP)
    assert [(f.kind, f.value) for f in found] == [
        ("client", "Sample Holdings Pty Ltd"),
        ("placeholder", "CLIENT_01"),
    ]


def test_verify_keeps_a_structured_placeholder_out_of_the_findings() -> None:
    """TFN_01 is what a clean redacted file looks like; the map never assigns it."""
    assert verify_module.findings("TFN_01 was on file", MAP) == ()


def test_redaction_is_deterministic_across_processes() -> None:
    """Placeholder ordinals must not depend on hash order or dict iteration."""
    script = (
        "from evatt.redact import redact\n"
        "from evatt.entities import Entity\n"
        "m = (Entity('Sample Holdings Pty Ltd','CLIENT_01','client','2026-09-09'),\n"
        "     Entity('Jane Roe','PERSON_01','person','2026-09-09'))\n"
        "t, c = redact('Jane Roe TFN 123 456 782 ABN 51 824 753 556 "
        "at Sample Holdings Pty Ltd', m)\n"
        "print(t)\n"
    )
    runs = {
        subprocess.run(
            [sys.executable, "-c", script], capture_output=True, text=True, check=True
        ).stdout
        for _ in range(3)
    }
    assert len(runs) == 1


SAMPLES = Path(entities_module.__file__).resolve().parent / "samples"
SAMPLE_MAP = entities_module.load(SAMPLES / "entities.sample.json")


def sample(name: str) -> str:
    return (SAMPLES / name).read_text(encoding="utf-8")


def test_every_sample_exists() -> None:
    expected = {
        "clean.md", "entities-only.md", "identifiers.md",
        "unmapped-name.md", "negatives.md", "unicode-names.md",
    }
    assert {p.name for p in SAMPLES.glob("*.md")} == expected


def test_clean_sample_needs_no_redaction() -> None:
    text, counts = redact_module.redact(sample("clean.md"), SAMPLE_MAP)
    assert text == sample("clean.md")
    assert counts == {}


def test_entities_only_sample_round_trips() -> None:
    original = sample("entities-only.md")
    text, counts = redact_module.redact(original, SAMPLE_MAP)
    assert counts
    assert restore(text, SAMPLE_MAP) == original


def test_identifiers_sample_yields_every_structured_kind() -> None:
    _text, counts = redact_module.redact(sample("identifiers.md"), SAMPLE_MAP)
    assert {"tfn", "abn", "acn", "bsb", "medicare", "email", "phone"} <= set(counts)


def test_unmapped_name_sample_halts() -> None:
    with pytest.raises(Halt):
        redact_module.redact(sample("unmapped-name.md"), SAMPLE_MAP)


def test_negatives_sample_is_left_untouched() -> None:
    text, counts = redact_module.redact(sample("negatives.md"), SAMPLE_MAP)
    assert text == sample("negatives.md")
    assert counts == {}


def test_unicode_names_sample_is_recognised() -> None:
    text, counts = redact_module.redact(sample("unicode-names.md"), SAMPLE_MAP)
    assert counts
    assert restore(text, SAMPLE_MAP) == sample("unicode-names.md")


def test_verify_is_clean_on_every_redactable_sample() -> None:
    for name in ("clean.md", "entities-only.md", "identifiers.md",
                 "negatives.md", "unicode-names.md"):
        text, _counts = redact_module.redact(sample(name), SAMPLE_MAP)
        assert verify_module.findings(text, SAMPLE_MAP) == (), name


def test_no_sample_ships_a_real_identifier() -> None:
    """samples/ ships as package data, so no fixture may carry real data.

    51 824 753 556 is the Australian Taxation Office's own ABN, active since
    1999, and 0412 345 678 sits in an allocated mobile range. Both were in
    identifiers.md. Since Task 2 a labelled identifier is admitted on its label
    alone, so a checksum-invalid ABN still yields an "abn" count without
    shipping anybody's number, and ACMA reserves 0491 570 006 to 0491 570 016
    for fiction. The check-digit vectors stay in tests/test_patterns.py, which
    is where they belong.
    """
    text = sample("identifiers.md")
    assert "51 824 753 556" not in text
    assert "0412 345 678" not in text
    assert "ABN 12 345 678 901" in text
    assert not patterns.valid_abn("12345678901")
    assert "Phone 0491 570 006" in text


def test_a_carried_placeholder_whose_own_digits_are_redacted_still_halts() -> None:
    """The one carried token that does not survive pass one, still reported.

    "MEDICARE_2123456701" is placeholder-shaped and its own digits satisfy the
    Medicare check, so pass one replaces them and the token is gone from the
    output. Deciding which tokens count from the input is what keeps the halt;
    the context is still quoted from the redacted text, so the digits that were
    replaced do not come back through the triage file.
    """
    with pytest.raises(Halt) as caught:
        redact_module.redact("Row one.\nCard MEDICARE_2123456701 filed.\n", MAP)
    unknown = caught.value.unknowns[0]
    assert len(caught.value.unknowns) == 1
    assert (unknown.kind, unknown.value, unknown.line) == (
        "placeholder", "MEDICARE_2123456701", 2
    )
    assert "2123456701" not in unknown.context
