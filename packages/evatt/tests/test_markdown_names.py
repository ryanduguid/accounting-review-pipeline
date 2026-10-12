"""Mapped names in markdown and spaced identifiers (roadmap findings RARP-1, RARP-2).

Each positive case asserts the exact redacted text and counts, a clean verify
on the output, a finding from verify on the original, and restore to the map's
canonical spelling. A clean verify alone is not the test: verify shares the
detector, so it agrees with whatever redact missed (verify.py explains why).
"""
import json
import subprocess
import sys
import time
import unicodedata

import pytest
from evatt import entities, patterns
from evatt.entities import Entity
from evatt.errors import EvattError, Halt
from evatt.redact import redact
from evatt.restore import restore
from evatt.verify import findings

PERSON = Entity("Jane Roe", "PERSON_01", "person", "2026-09-27")
ACCENTED = Entity("Zo\N{LATIN SMALL LETTER E WITH DIAERESIS} Nu\N{LATIN SMALL LETTER N WITH TILDE}ez",
                  "PERSON_02", "person", "2026-09-27")
MAP = (PERSON, ACCENTED)
JOINERS = [chr(0x200B), chr(0x200C), chr(0x200D), chr(0x2060)]


def assert_person_round_trip(text: str, expected: str, entity: Entity = PERSON) -> None:
    redacted, counts = redact(text, MAP, strict=False)
    assert redacted == expected
    assert counts == {"person": 1}
    assert findings(redacted, MAP) == ()
    assert ("person", entity.value) in [(f.kind, f.value) for f in findings(text, MAP)]
    assert entity.value in restore(redacted, MAP)


@pytest.mark.parametrize(("text", "expected"), [
    ("_Jane Roe_ signed", "*PERSON_01* signed"),
    ("__Jane Roe__ signed", "**PERSON_01** signed"),
    ("*Jane Roe* signed", "*PERSON_01* signed"),
    ("_jane roe_ signed", "*PERSON_01* signed"),
    ("| Jane<br>Roe | signed |", "| PERSON_01 | signed |"),
    ("| Jane<BR/>Roe | signed |", "| PERSON_01 | signed |"),
    ("| Jane<br />Roe | signed |", "| PERSON_01 | signed |"),
    ("> Jane\n> Roe signed", "> PERSON_01 signed"),
    ("> > Jane\n> > Roe signed", "> > PERSON_01 signed"),
    ("> Jane\r\n> Roe signed", "> PERSON_01 signed"),
    ("Jane_Roe signed", "PERSON_01 signed"),
])
def test_a_mapped_name_in_markdown_is_replaced_and_restores(text: str, expected: str) -> None:
    assert_person_round_trip(text, expected)


@pytest.mark.parametrize(("text", "expected", "count"), [
    # Lower case, so the residual sweep cannot rescue a miss (Oracle review of 27 Sep).
    ("paid**jane roe** today", "paid**PERSON_01** today", 1),
    ("**jane roe****jane roe**", "**PERSON_01****PERSON_01**", 2),
    ("paid**_jane roe_**", "paid***PERSON_01***", 1),
])
def test_a_mapped_name_straight_after_a_star_is_replaced(text: str, expected: str, count: int) -> None:
    redacted, counts = redact(text, MAP, strict=False)
    assert (redacted, counts) == (expected, {"person": count})
    assert findings(redacted, MAP) == ()
    assert restore(redacted, MAP).count(PERSON.value) == count


@pytest.mark.parametrize("joiner", JOINERS)
def test_a_mapped_name_split_by_an_invisible_joiner_is_replaced(joiner: str) -> None:
    assert_person_round_trip(f"Jane{joiner}Roe signed", "PERSON_01 signed")


@pytest.mark.parametrize("form", ["NFC", "NFD"])
def test_an_accented_mapped_name_in_emphasis_is_replaced_in_either_composition(form: str) -> None:
    text = unicodedata.normalize(form, f"_{ACCENTED.value}_ signed")
    assert_person_round_trip(text, "*PERSON_02* signed", ACCENTED)


@pytest.mark.parametrize("text", ["XJane Roe", "Jane Roebuck", "x_Jane_Roe_y", "Jane>Roe", "JaneRoe"])
def test_a_name_inside_a_longer_token_or_across_other_punctuation_is_not_replaced(text: str) -> None:
    redacted, counts = redact(text, MAP, strict=False)
    assert "person" not in counts
    assert redacted == text


@pytest.mark.parametrize("wrapped", ["_John Smith_", "__John Smith__", "*__John Smith__*", "**John Smith**"])
def test_an_unmapped_name_in_emphasis_reaches_the_residual_sweep(wrapped: str) -> None:
    names = [unknown.value for unknown in redact_residual(f"the reviewer was {wrapped} today")]
    assert names == ["John Smith"]


@pytest.mark.parametrize("text", ["see x_John Smith today", "see John Smith_x today", "see John Smith_01 today"])
def test_a_name_joined_to_a_word_by_one_underscore_is_a_longer_token(text: str) -> None:
    assert redact_residual(text) == ()


# Shapes this change does not join, recorded as unsupported in DECISIONS.md
# ruling 42 and the README. Each passes pass two untouched. Most also pass the
# residual sweep, so the operator's read of the whole output is the only
# control; a capitalised fragment left behind can still halt a strict run. A
# test that starts failing here means a shape became supported: move it and
# update both.
@pytest.mark.parametrize("text", [
    "Jane" + chr(92) + "\nRoe signed",
    "Jane&nbsp;Roe signed",
    "Jane <em>Roe</em> signed",
    "`Jane` `Roe` signed",
    "Ja" + chr(0x200B) + "ne Roe signed",
    "| Jane | Roe | signed |",
    "J. Roe signed",
])
def test_unsupported_name_shapes_pass_untouched(text: str) -> None:
    redacted, counts = redact(text, MAP, strict=False)
    assert (redacted, counts) == (text, {})


def redact_residual(text: str):
    from evatt.redact import residual
    return residual(text)


def test_the_tools_own_placeholders_never_trigger_the_residual_sweep() -> None:
    assert redact_residual("Dear PERSON_01 and CLIENT_02 Holdings") == ()


@pytest.mark.parametrize(("text", "expected", "kind"), [
    ("her tfn:  123  456  782", "her tfn:  TFN_01", "tfn"),
    ("abn  51  824  753  556", "abn  ABN_01", "abn"),
    ("her tax file no. 123 456 789", "her tax file no. TFN_01", "tfn"),
    ("Tax file no: 12 345 678", "Tax file no: TFN_01", "tfn"),
    ("TFN:\t123\t\t456\t782", "TFN:\tTFN_01", "tfn"),
    ("TFN: 123 456  782 on file", "TFN: TFN_01 on file", "tfn"),
    ("ACN  123  456  780", "ACN  ACN_01", "acn"),
    ("Medicare  2123  45671  1", "Medicare  MEDICARE_01", "medicare"),
    ("tax file number  123  456  782", "tax file number  TFN_01", "tfn"),
    ("TFN: 123 456\n782 on file", "TFN: TFN_01 on file", "tfn"),
    ("TFN:" + chr(0xA0) + "123" + chr(0xA0) + "456" + chr(0xA0) + "782", "TFN:" + chr(0xA0) + "TFN_01", "tfn"),
    ("MY_TFN: 123 456 783", "MY_TFN: TFN_01", "tfn"),
    # A bare TFN's 2 gaps are independent; the check digit is what admits it.
    ("123  456 782", "TFN_01", "tfn"),
    # A qualifier takes an emphasis edge and may close its emphasis before the colon.
    ("__tax file no__: 123456783", "__tax file no__: TFN_01", "tfn"),
    ("__Medicare card__: 2123456711", "__Medicare card__: MEDICARE_01", "medicare"),
    ("**Medicare card**: 2123456711", "**Medicare card**: MEDICARE_01", "medicare"),
    ("**ABN no.**: 51 824 753 557", "**ABN no.**: ABN_01", "abn"),
])
def test_spaced_and_labelled_identifiers_are_replaced(text: str, expected: str, kind: str) -> None:
    redacted, counts = redact(text, (), strict=False)
    assert redacted == expected
    assert counts == {kind: 1}
    assert findings(redacted, ()) == ()
    assert kind in [f.kind for f in findings(text, ())]


@pytest.mark.parametrize("text", ["TFN: 123 456 782  2026", "row 7 123 456 782  45,000.00"])
def test_two_space_numeric_continuations_reach_triage_without_partial_replacement(text):
    assert redact(text, (), strict=False) == (text, {})
    assert [(u.kind, u.value) for u in redact_residual(text)] == [
        ("number", text.split(": ", 1)[-1] if text.startswith("TFN:") else text[4:]),
    ]
    assert any(f.kind == "number" for f in findings(text, ()))


@pytest.mark.parametrize("text", ["tax file 2026-2027", "$123 456 782", "tax file  123  456  783 lodged"])
def test_unlabelled_or_money_shaped_digits_stay_untouched(text: str) -> None:
    redacted, counts = redact(text, (), strict=False)
    assert redacted == text
    assert counts == {}


@pytest.mark.parametrize("text", [
    # Past the two-character interior bound (ruling 42 lists it as unsupported).
    "TFN: 123   456   782",
    # A digit one space after the run: the one-character trailing pin refuses
    # to take a prefix of what may be a longer number (unsupported, ruling 42).
    "TFN: 123456783 2026",
    # Overlength runs are not identifiers, so no prefix is replaced.
    "TFN: 123 456 7823",
    "ABN 51 824 753 5561",
])
def test_labelled_digits_outside_the_bounds_stay_untouched(text: str) -> None:
    assert redact(text, (), strict=False) == (text, {})


@pytest.mark.parametrize("value", ["TFN 01", "TFN", "01", "Medicare", "Email", "tfn_01"])
def test_a_map_value_cannot_rewrite_a_pass_one_placeholder(value: str) -> None:
    # A hand-built map skips load's checks, so pass two itself must leave the
    # placeholders pass one wrote alone. A value may still replace a label word.
    rogue = Entity(value, "CLIENT_07", "client", "2026-09-27")
    redacted, counts = redact("TFN: 123 456 782, Medicare 2123 45671 1, jo@example.com", (rogue,), strict=False)
    assert [m.group(0) for m in patterns.PLACEHOLDER.finditer(redacted) if m.group(0) != "CLIENT_07"] == [
        "TFN_01", "MEDICARE_01", "EMAIL_01"]
    assert (counts["tfn"], counts["medicare"], counts["email"]) == (1, 1, 1)


@pytest.mark.parametrize("value", ["TFN 01", "TFN 01 Jane"])
def test_a_map_value_cannot_rewrite_an_underscore_wrapped_placeholder(value: str) -> None:
    # Pass one keeps the underscores around "_123456783_", and PLACEHOLDER's
    # left boundary did not see the TFN_01 inside them, so pass two rewrote it.
    rogue = Entity(value, "CLIENT_07", "client", "2026-09-27")
    assert redact("TFN: _123456783_ Jane", (rogue,), strict=False) == ("TFN: _TFN_01_ Jane", {"tfn": 1})


def test_an_underscore_wrapped_placeholder_in_the_input_is_carried() -> None:
    with pytest.raises(Halt):
        redact("_PERSON_09_ Smith", MAP)
    assert ("placeholder", "PERSON_09") in [(f.kind, f.value) for f in findings("_PERSON_09_ Smith", MAP)]


def test_a_placeholder_inside_a_longer_token_is_not_carried() -> None:
    text = "see MY_CLIENT_01, MY__CLIENT_01 and XCLIENT_01"
    assert redact(text, ()) == (text, {})
    assert findings(text, ()) == ()


def test_a_value_enclosing_a_placeholder_shaped_run_is_still_replaced() -> None:
    # Protection covers placeholders pass one wrote and tokens the input
    # carries, not a placeholder-shaped run inside a mapped value's own word.
    client = Entity("XCLIENT 01", "CLIENT_07", "client", "2026-09-27")
    assert redact("paid XCLIENT_01 today", (client,), strict=False) == ("paid CLIENT_07 today", {"client": 1})


@pytest.mark.parametrize(("value", "text"), [
    ("Jane>Roe", "met Jane>Roe today"),
    ("A > B", "met A > B today"),
    # A value written across a quoted line splits the way the joiner reads it.
    ("Jane" + chr(10) + "> Roe", "met Jane" + chr(10) + "> Roe today"),
    ("Jane" + chr(10) + "> Roe", "met Jane Roe today"),
])
def test_a_value_holding_a_greater_than_sign_matches_its_own_spelling(value: str, text: str) -> None:
    entity = Entity(value, "CLIENT_07", "client", "2026-09-27")
    assert redact(text, (entity,), strict=False) == ("met CLIENT_07 today", {"client": 1})


@pytest.mark.parametrize("value", ["*", "<br>", "_ *"])
def test_a_value_the_matcher_refuses_is_skipped_by_redact_and_verify(value: str) -> None:
    with pytest.raises(ValueError):
        patterns.value_pattern(value)
    entity = Entity(value, "CLIENT_07", "client", "2026-09-27")
    assert redact("a ... b", (entity,), strict=False) == ("a ... b", {})
    assert findings("a ... b", (entity,)) == ()


@pytest.mark.parametrize("value", ["Jane <br Roe", "A <BR/ B", "ACME <Brown", "<br <br b"])
def test_a_value_holding_part_of_a_br_tag_matches_its_own_spelling(tmp_path, value: str) -> None:
    # Each join is taken atomically, so a token that begins like a tag cannot
    # lose characters to the joiner before it.
    (entity,) = entities.load(_map(tmp_path, [value]))
    text = f"met {value} today"
    redacted, counts = redact(text, (entity,), strict=False)
    assert (redacted, counts) == ("met PERSON_01 today", {"person": 1})
    assert restore(redacted, (entity,)) == text


@pytest.mark.parametrize("value", [
    "<br>Jane", "Jane<br/>", " Jane Roe", "Jane Roe ", "*Jane Roe*", "_Jane_",
    "Jane Roe" + chr(10), "Jane" + chr(10) + ">", chr(0x200B) + "Jane", "*", "<br>",
])
def test_a_map_refuses_a_value_with_whitespace_or_markup_at_either_end(tmp_path, value: str) -> None:
    # Pass two replaces the words alone, so "<br>Jane" became "<br>PERSON_01"
    # and restore wrote "<br><br>Jane".
    with pytest.raises(EvattError, match="starts or ends with whitespace or markup"):
        entities.load(_map(tmp_path, [value]))
    with pytest.raises(EvattError, match="starts or ends with whitespace or markup"):
        entities.assign([], value, "person", "2026-09-27")


def test_near_miss_placeholders_stay_unrestored() -> None:
    client = Entity("Sample Holdings Pty Ltd", "CLIENT_01", "client", "2026-09-27")
    text = "PRIOR_CLIENT_01 XCLIENT_01 CLIENT_01s"
    assert restore(text, (client,)) == text


def _map(tmp_path, values):
    document = {"schema_version": 1, "entries": [
        {"value": value, "placeholder": f"PERSON_{n:02d}", "kind": "person", "added": "2026-09-27"}
        for n, value in enumerate(values, start=1)
    ]}
    path = tmp_path / "entities.json"
    path.write_text(json.dumps(document), encoding="utf-8")
    return path


@pytest.mark.parametrize("values", [("Jane Roe", "Jane_Roe"), ("Jane_Roe", "Jane Roe"), ("Jane Roe", "Jane*Roe")])
def test_a_map_cannot_hold_two_spellings_the_markdown_matcher_joins(tmp_path, values) -> None:
    with pytest.raises(EvattError, match="once case, whitespace and composition are folded"):
        entities.load(_map(tmp_path, values))


@pytest.mark.parametrize("value", ["TFN", "Medicare", "Email", "TFN 01", "client 07"])
def test_a_map_refuses_a_value_made_only_of_placeholder_parts(tmp_path, value) -> None:
    with pytest.raises(EvattError, match="placeholder"):
        entities.load(_map(tmp_path, [value]))


@pytest.mark.parametrize(("value", "prefix", "unit"), [
    ("A * B", "A", "*"),
    # Long runs of every joiner kind, and a value holding part of a tag, which
    # was cubic before the joins were atomic.
    ("A B", "A ", "<br" + chr(10) + ">"),
    ("A <br B", "A ", "<br "),
    ("A B", "A", chr(10) + "> "),
    ("A B", "A", chr(0x200B) + "_*"),
])
def test_a_failing_value_search_stays_linear(value: str, prefix: str, unit: str) -> None:
    # A subprocess bounds a catastrophic regression to a failure, not a hung suite.
    probe = (f"from evatt import patterns; "
             f"assert patterns.value_pattern({value!r}).search({prefix!r} + {unit!r} * 20000 + 'X') is None")
    subprocess.run([sys.executable, "-c", probe], check=True, timeout=60)
    pattern = patterns.value_pattern(value)

    def duration(size: int) -> float:
        best = float("inf")
        for _ in range(3):
            text = prefix + unit * size + "X"
            start = time.perf_counter()
            assert pattern.search(text) is None
            best = min(best, time.perf_counter() - start)
        return best

    small = duration(5_000)
    assert small < 0.1
    large = duration(20_000)
    assert large < small * 8 + 0.01
