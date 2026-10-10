"""Fictional identifiers must not consume a following numbered-list marker."""
import shutil
import subprocess
import time

import pytest
from evatt import cli, patterns
from evatt.entities import Entity
from evatt.errors import Halt
from evatt.redact import redact, residual
from evatt.verify import findings

AMBIGUOUS = (
    "TFN: 12345678\n1. Next item",
    "Medicare: 212345670\n1. Next item",
    "TFN: 1234567\n82. next",
    "TFN: 1234567\n82. 3 next",
    "ABN: 51 824 75\n3556. next",
    "ACN: 1234567\n80. next",
    "Medicare: 2123456\n701. next",
    "ABN: 0412345678\n1. next",
    "123456\n782. next",
    "51 824\n753556. next",
    "123456\n780. next",
    "2123\n456701. next",
    "TFN: 1234567\n8200. Next item",
    "ABN: 51 824 75\n355600. Next item",
    "ACN: 1234567\n8000. Next item",
    "Medicare: 2123456\n70100. Next item",
    "123456\n78200. Next item",
    "51 824\n7535560. Next item",
    "123456\n78000. Next item",
    "2123\n4567010. Next item",
)
BREAKS = ("\n", "\r\n", "\r", "\v", "\f", "\x1c", "\x1d", "\x1e", "\x85",
          "\u2028", "\u2029")


@pytest.mark.parametrize("source", AMBIGUOUS)
def test_ambiguous_identifier_spans_leave_the_list_marker(source):
    assert patterns.structured_spans(source) == []


@pytest.mark.parametrize("source", AMBIGUOUS)
def test_ambiguous_identifiers_reach_strict_numeric_triage(source):
    assert redact(source, (), strict=False) == (source, {})
    with pytest.raises(Halt) as caught:
        redact(source, ())
    assert any(u.kind == "number" for u in caught.value.unknowns)
    assert any(u.kind == "number" for u in residual(source))
    assert any(f.kind == "number" for f in findings(source, ()))


@pytest.mark.parametrize("ending", BREAKS)
@pytest.mark.parametrize("marker", ["82. next", "82) next", "82)\tnext", " 82. next"])
def test_short_labelled_fragments_halt_across_each_line_break(ending, marker):
    source = "TFN: 1234567" + ending + marker
    assert patterns.structured_spans(source) == []
    with pytest.raises(Halt) as caught:
        redact(source, ())
    assert any(u.kind == "number" for u in caught.value.unknowns)
    assert any(f.kind == "number" for f in findings(source, ()))


@pytest.mark.parametrize("source", AMBIGUOUS[:12])
@pytest.mark.parametrize("space", ["\u00a0", "\u202f"])
@pytest.mark.parametrize("position", ["indent", "after"])
def test_unicode_horizontal_marker_whitespace_halts(source, space, position):
    source = (source.replace("\n", "\n" + space) if position == "indent"
              else source.replace(". ", "." + space))
    assert patterns.structured_spans(source) == []
    with pytest.raises(Halt):
        redact(source, ())
    assert any(f.kind == "number" for f in findings(source, ()))


@pytest.mark.parametrize("source,expected,kind", [
    ("TFN: 123\n456 782", "TFN: TFN_01", "tfn"),
    ("Medicare: 2123\n45670 1", "Medicare: MEDICARE_01", "medicare"),
    ("ABN: 51 824\n753 556", "ABN: ABN_01", "abn"),
    ("ACN: 123\n456 780", "ACN: ACN_01", "acn"),
    ("123\n456 782", "TFN_01", "tfn"),
    ("51 824\n753 556", "ABN_01", "abn"),
    ("123\n456 780", "ACN_01", "acn"),
    ("2123\n45670 1", "MEDICARE_01", "medicare"),
])
@pytest.mark.parametrize("ending", ["\n", "\r\n"])
def test_genuine_hard_wrapped_identifiers_are_still_replaced(source, expected, kind, ending):
    source = source.replace("\n", ending)
    assert len(patterns.structured_spans(source)) == 1
    assert redact(source, ()) == (expected, {kind: 1})
    assert findings(expected, ()) == ()


@pytest.mark.parametrize("label", ["Medicare: ", ""])
def test_the_accepted_next_line_medicare_reference_limit(label):
    source = label + "2123456701\n1"
    output = label + "MEDICARE_01\n1"
    assert redact(source, ()) == (output, {"medicare": 1})
    assert findings(output, ()) == ()
    assert redact(label + "2123456701/1", ()) == (label + "MEDICARE_01", {"medicare": 1})


@pytest.mark.parametrize("source", ["TFN: 1234567\n83. next", "Medicare: 2123456\n711. next"])
def test_a_failed_labelled_checksum_does_not_avoid_the_ambiguity_halt(source):
    with pytest.raises(Halt):
        redact(source, ())
    assert any(f.kind == "number" for f in findings(source, ()))


@pytest.mark.parametrize("value", ["1234567", "82", "TFN", "TFN: 1234567",
                                 "TFN: 1234567\n82"])
def test_a_partial_entity_cannot_erase_short_ambiguity_evidence(value):
    source = "TFN: 1234567\n82. next"
    mapping = (Entity(value, "ENTITY_01", "entity", "2026-10-11"),)
    assert redact(source, mapping, strict=False) == (source, {})
    with pytest.raises(Halt):
        redact(source, mapping)


@pytest.mark.parametrize("suffix", [".", ")", ".\nnext", ")\r\nnext"])
def test_empty_list_items_still_halt(suffix):
    with pytest.raises(Halt):
        redact("TFN: 1234567\n82" + suffix, ())


@pytest.mark.parametrize("source", [
    "123456\n783. next", "000000\n018. next", "51 824\n753557. next",
    "2123\n456711. next",
])
def test_invalid_bare_checksums_do_not_gain_short_ambiguity_findings(source):
    assert patterns.structured_spans(source) == []
    assert redact(source, ()) == (source, {})
    assert findings(source, ()) == ()


def test_disjoint_identifiers_are_replaced_beside_an_ambiguity():
    source = "TFN: 1234567\n82. next\nTFN: 123456782"
    assert [(kind, value) for _, _, kind, value in patterns.structured_spans(source)] == [
        ("tfn", "123456782"),
    ]
    assert redact(source, (), strict=False) == (
        "TFN: 1234567\n82. next\nTFN: TFN_01", {"tfn": 1},
    )
    with pytest.raises(Halt):
        redact(source, ())


def test_ambiguity_does_not_prevent_other_identifiers_being_sanitised_in_triage():
    source = "jane@example.com; TFN: 1234567\n82. next"
    with pytest.raises(Halt) as caught:
        redact(source, ())
    assert any(u.kind == "number" for u in caught.value.unknowns)
    assert all("jane@example.com" not in u.context for u in caught.value.unknowns)


@pytest.mark.parametrize("scan", [patterns.structured_spans, residual])
@pytest.mark.parametrize("unit", [
    "TFN: 1234567\n82. next\n",
    "2123\n456701) next\n",
    "ABN: 51 824\n753556. next\n",
    "TFN: 1234567\n82. 3 next\n",
    "TFN: 1234567\n8200. next\n",
    "TFN: 1234567\n" + "8" * 1_000 + ". next\n",
    "TFN: 1234567\n8200." + " " * 1_000 + "next\n",
    "TFN: " + " " * 100 + "1234567\n82. next\n",
])
def test_ambiguous_identifier_handling_scales_linearly(scan, unit):
    def elapsed(size):
        source = unit * size
        best = float("inf")
        for _ in range(3):
            started = time.perf_counter()
            result = scan(source)
            assert bool(result) == (scan is residual)
            best = min(best, time.perf_counter() - started)
        return best

    small = elapsed(500)
    large = elapsed(2_000)
    assert large < small * 8 + 0.010, (small, large)


@pytest.fixture
def empty_workspace(tmp_path, monkeypatch):
    if shutil.which("git") is None:
        pytest.skip("git is not installed")
    monkeypatch.setenv("GIT_CONFIG_GLOBAL", str(tmp_path / "absent-gitconfig"))
    monkeypatch.setenv("GIT_CONFIG_SYSTEM", str(tmp_path / "absent-gitconfig"))
    monkeypatch.setenv("GIT_CEILING_DIRECTORIES", str(tmp_path.parent))
    subprocess.run(["git", "init", "-q"], cwd=tmp_path, check=True, capture_output=True)
    (tmp_path / ".gitignore").write_text("entities.json\n*.triage.md\n", encoding="utf-8")
    (tmp_path / "entities.json").write_text('{"schema_version": 1, "entries": []}',
                                            encoding="utf-8")
    return tmp_path


@pytest.mark.parametrize("source", AMBIGUOUS)
@pytest.mark.parametrize("stale", [False, True])
def test_cli_ambiguity_halt_leaves_no_output_or_manifest(empty_workspace, source, stale, capsys):
    root = empty_workspace
    original, output = root / "in.md", root / "out.md"
    manifest = root / "out.md.manifest.json"
    original.write_text(source, encoding="utf-8")
    if stale:
        output.write_text("fabricated stale output", encoding="utf-8")
        manifest.write_text('{"schema_version": 1, "counts": {}}', encoding="utf-8")
    assert cli.main(["redact", "--in", str(original), "--map", str(root / "entities.json"),
                     "--out", str(output)]) == 2
    assert not output.exists()
    assert not manifest.exists()
    assert original.read_text(encoding="utf-8") == source
    assert "(number, line 1)" in (root / "out.md.triage.md").read_text(encoding="utf-8")
    capsys.readouterr()
    assert cli.main(["verify", "--in", str(original),
                     "--map", str(root / "entities.json")]) == 2
    console = capsys.readouterr()
    assert "number:" in console.out
    assert "finding(s)" in console.out
    assert all(value not in console.out + console.err for value in source.split("\n"))


def test_cli_indexes_ambiguous_locations_once(empty_workspace, monkeypatch, capsys):
    root = empty_workspace
    # A replaced wrap before the findings changes redacted line offsets.
    source = "TFN: 123\n456 782\n" + "TFN: 1234567\n82. next\n" * 400
    original = root / "in.md"
    original.write_text(source, encoding="utf-8")

    original_pattern = cli.value_pattern

    def no_repeated_search(value):
        assert value != "1234567\n82", "an ambiguous finding must use the location index"
        return original_pattern(value)

    monkeypatch.setattr(cli, "value_pattern", no_repeated_search)
    assert cli.main(["verify", "--in", str(original),
                     "--map", str(root / "entities.json")]) == 2
    output = capsys.readouterr().out.splitlines()
    assert [line for line in output if line.startswith("number:")] == [
        f"number: line {line}" for line in range(3, 803, 2)
    ]
