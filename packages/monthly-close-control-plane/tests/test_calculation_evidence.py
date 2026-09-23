"""The optional calculation-evidence control.

Every test here runs with sockets blocked. The control reads files; it makes
no calculation and contacts no service, and the block below is what proves it
rather than what asserts it in a docstring.
"""

from __future__ import annotations

import hashlib
import json
import socket
from datetime import date
from decimal import Decimal
from pathlib import Path

import pytest
from closecontrol import calculation_evidence as module
from closecontrol.engine import review_close
from closecontrol.errors import SchemaError
from closecontrol.report import write_review_pack

EXAMPLES = Path(__file__).resolve().parents[1] / "examples"


@pytest.fixture(autouse=True)
def no_network(monkeypatch):
    """No test in this file may open a socket."""
    def refuse(*args, **kwargs):  # noqa: ANN002, ANN003
        raise AssertionError("the evidence control opened a socket")

    monkeypatch.setattr(socket, "create_connection", refuse)
    monkeypatch.setattr(socket.socket, "connect", refuse)


def canonical(payload) -> bytes:
    return json.dumps(payload, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False, allow_nan=False).encode("utf-8")


def build_record(**overrides):
    """A fabricated, well-formed evidence record with a correct digest."""
    calculation = {
        "schema": "lodgeit-calculation-evidence/1",
        "label": "coal-lsl-levy",
        "synthetic_input": True,
        "provider": {"name": "lodgeit-labs", "contract_snapshot": "snap-1",
                     "contract_sha256": "a" * 64},
        "call": {
            "calculator": "urn:sbrm:calc:coal-lsl-levy",
            "period": "urn:sbrm:period:coal-lsl-levy:2026-07",
            "status": "COMPUTED",
            "http_status": 200,
        },
        "input": {"reporting_month": "2026-07"},
        "input_json": '{"reporting_month":"2026-07"}',
        "upstream": {
            "response": {"levy": "192.38"},
            "manifest": {"calculator": "urn:sbrm:calc:coal-lsl-levy",
                         "rate_table_uris": [{"uri": "urn:sbrm:rate:coal-lsl-levy:2026-07:levy-rate",
                                              "sha256": "b" * 64}]},
            "advisory": {"figure_type": "payroll_levy_estimate",
                         "notes": ["Review aid for a qualified person; not a lodgement figure."]},
            "refusal_class": None,
        },
        "normalised": {"values": {"levy": "192.38", "eligible_wages": "7125.00"},
                       "note": "Parsed from the response's own decimal strings."},
        "validation": {"findings": [], "accepted": True},
        "engine": {"name": "coal-lsl-levy", "version": "0.1.0"},
        "local_result": None,
        "notes": [],
        "boundary": ["Calculated on supplied facts. Not advice."],
    }
    for path, value in overrides.items():
        target = calculation
        parts = path.split(".")
        for part in parts[:-1]:
            target = target[part]
        if value is module.__name__:  # never used; keeps the signature simple
            continue
        target[parts[-1]] = value
    return {
        "schema": "lodgeit-calculation-evidence/1",
        "calculation_sha256": hashlib.sha256(canonical(calculation)).hexdigest(),
        "calculation": calculation,
        "observation": {"captured_at": "2026-09-18T00:00:00+00:00", "elapsed_ms": 42,
                        "base_url": "https://example.invalid"},
    }


def write(tmp_path: Path, record, name="evidence.json") -> Path:
    path = tmp_path / name
    path.write_text(json.dumps(record, indent=2), encoding="utf-8")
    return path


def run(tmp_path, **kwargs):
    return review_close(
        current_path=EXAMPLES / "current_trial_balance.csv",
        prior_path=EXAMPLES / "prior_trial_balance.csv",
        **kwargs,
    )


# -- loading ---------------------------------------------------------------


def test_a_well_formed_file_loads_with_its_figures(tmp_path):
    evidence = module.load(write(tmp_path, build_record()))
    assert evidence.label == "coal-lsl-levy"
    assert evidence.status == "COMPUTED"
    assert evidence.values["levy"] == Decimal("192.38")
    assert evidence.rate_tables == ("urn:sbrm:rate:coal-lsl-levy:2026-07:levy-rate",)
    assert evidence.usable is True
    assert evidence.findings == ()


def test_a_tampered_file_is_detected_by_its_own_digest(tmp_path):
    record = build_record()
    record["calculation"]["normalised"]["values"]["levy"] = "1.00"
    evidence = module.load(write(tmp_path, record))
    assert evidence.usable is False
    assert any("does not match" in finding for finding in evidence.findings)


def test_an_unknown_schema_is_refused_rather_than_guessed_at(tmp_path):
    record = build_record()
    record["schema"] = "some-other-evidence/9"
    with pytest.raises(SchemaError, match="not one this pipeline reads"):
        module.load(write(tmp_path, record))


def test_a_json_number_for_money_is_refused(tmp_path):
    record = build_record()
    record["calculation"]["normalised"]["values"]["levy"] = 192.38
    record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
    with pytest.raises(SchemaError, match="not a decimal string"):
        module.load(write(tmp_path, record))


def test_a_producer_finding_travels_with_the_file(tmp_path):
    record = build_record()
    record["calculation"]["validation"]["findings"] = ["no advisory block"]
    record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
    evidence = module.load(write(tmp_path, record))
    assert evidence.usable is False
    assert any("producer recorded a validation finding" in item for item in evidence.findings)


def test_a_control_character_in_provider_text_is_refused(tmp_path):
    record = build_record()
    record["calculation"]["upstream"]["advisory"]["notes"] = ["right‮to‭left"]
    record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
    with pytest.raises(SchemaError, match="control or formatting character"):
        module.load(write(tmp_path, record))


def test_a_missing_manifest_or_advisory_on_a_computed_figure_is_a_finding(tmp_path):
    for field in ("manifest", "advisory"):
        record = build_record()
        record["calculation"]["upstream"][field] = None
        record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
        evidence = module.load(write(tmp_path, record, f"{field}.json"))
        assert evidence.usable is False, field
        assert any(field in finding for finding in evidence.findings), field


def test_an_unreadable_file_is_reported_not_dropped(tmp_path):
    bad = tmp_path / "broken.json"
    bad.write_text("{not json", encoding="utf-8")
    loaded, unreadable = module.load_all([bad])
    assert loaded == []
    assert len(unreadable) == 1


def test_two_files_with_one_label_cannot_both_stand(tmp_path):
    first = write(tmp_path, build_record(), "one.json")
    second = write(tmp_path, build_record(), "two.json")
    loaded, unreadable = module.load_all([first, second])
    assert len(loaded) == 1
    assert any("more than one evidence file" in item for item in unreadable)


@pytest.mark.parametrize(
    "period,report_date,expected",
    [
        ("urn:sbrm:period:coal-lsl-levy:2026-06", date(2026, 6, 30), True),
        ("urn:sbrm:period:coal-lsl-levy:2026-05", date(2026, 6, 30), False),
        ("urn:sbrm:period:div7a:fy2026", date(2026, 6, 30), True),
        ("urn:sbrm:period:div7a:fy2026", date(2026, 7, 31), False),
        ("urn:sbrm:period:fbt:fy2026", date(2026, 3, 31), True),
        ("urn:sbrm:period:fbt:fy2026", date(2026, 6, 30), False),
        ("urn:sbrm:period:depreciation:unscoped", date(2026, 6, 30), None),
        ("", date(2026, 6, 30), None),
    ],
)
def test_period_coverage_is_read_or_refused_never_assumed(tmp_path, period, report_date, expected):
    record = build_record()
    record["calculation"]["call"]["period"] = period
    record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
    evidence = module.load(write(tmp_path, record))
    assert module.covers_period(evidence, report_date) is expected


# -- the control inside a close -------------------------------------------


def test_a_pack_without_the_feature_is_unchanged(tmp_path):
    """The optional control must not touch a pack that did not ask for it."""
    from closecontrol.report import _as_json

    before = _as_json(run(tmp_path))
    assert "calculation_evidence" not in before
    assert run(tmp_path).status == "REVIEW"


def test_valid_evidence_adds_provenance_without_approving_anything(tmp_path):
    path = write(tmp_path, build_record())
    pack = run(tmp_path, calculation_evidence_paths=[path],
               required_calculations=("coal-lsl-levy",))
    from closecontrol.report import _as_json

    payload = _as_json(pack)
    assert payload["calculation_evidence"]["required"] == ["coal-lsl-levy"]
    supplied = payload["calculation_evidence"]["supplied"][0]
    assert supplied["usable"] is True
    assert supplied["values"]["levy"] == "192.38"
    assert supplied["file_sha256"] in payload["source_sha256"].values()
    assert "does not approve" in payload["calculation_evidence"]["effect"]
    # No new control fired, so the pack's status comes from the same controls
    # it always did.
    assert not [item for item in pack.exceptions if item.control == "calculation_evidence"]


def test_a_required_calculation_cannot_be_silently_omitted(tmp_path):
    pack = run(tmp_path, required_calculations=("coal-lsl-levy",))
    evidence_exceptions = [item for item in pack.exceptions if item.control == "calculation_evidence"]
    assert len(evidence_exceptions) == 1
    assert evidence_exceptions[0].status == "REVIEW"
    assert "configured as required" in evidence_exceptions[0].reason
    assert pack.status in ("REVIEW", "BLOCKED")


def test_tampering_blocks_the_pack(tmp_path):
    record = build_record()
    record["calculation"]["normalised"]["values"]["levy"] = "0.01"
    path = write(tmp_path, record)
    pack = run(tmp_path, calculation_evidence_paths=[path])
    blocked = [item for item in pack.exceptions
               if item.control == "calculation_evidence" and item.status == "BLOCKED"]
    assert blocked
    assert pack.status == "BLOCKED"


def test_a_period_mismatch_is_raised_not_ignored(tmp_path):
    record = build_record()
    record["calculation"]["call"]["period"] = "urn:sbrm:period:coal-lsl-levy:2026-01"
    record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
    pack = run(tmp_path, calculation_evidence_paths=[write(tmp_path, record)])
    assert any("does not include the current report date" in item.reason
               for item in pack.exceptions if item.control == "calculation_evidence")


def test_an_upstream_refusal_never_becomes_a_nil_amount(tmp_path):
    record = build_record()
    record["calculation"]["call"]["status"] = "UPSTREAM_REFUSED"
    record["calculation"]["normalised"]["values"] = {}
    record["calculation"]["upstream"]["refusal_class"] = "insufficient_facts"
    record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
    pack = run(tmp_path, calculation_evidence_paths=[write(tmp_path, record)],
               required_calculations=("coal-lsl-levy",))
    reasons = [item.reason for item in pack.exceptions if item.control == "calculation_evidence"]
    assert any("no figure was produced" in reason for reason in reasons)
    assert not any("0.00" in reason for reason in reasons)


def test_an_unreconciled_depreciation_evidence_file_is_flagged(tmp_path):
    record = build_record()
    record["calculation"]["label"] = "accounting-depreciation"
    record["calculation"]["call"]["calculator"] = "urn:sbrm:calculator:depreciation:range"
    record["calculation"]["call"]["period"] = "urn:sbrm:period:depreciation:unscoped"
    record["calculation"]["call"]["status"] = "CONTRACT_FAILURE"
    record["calculation"]["validation"] = {
        "findings": ["movement does not close: opening 0.00 plus additions 0.00 less "
                     "depreciation 12000.00 is -12000.00, against a closing balance of 108000.00"],
        "accepted": False,
    }
    record["calculation"]["normalised"]["values"] = {}
    record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
    pack = run(tmp_path, calculation_evidence_paths=[write(tmp_path, record)],
               required_calculations=("accounting-depreciation",))
    reasons = " ".join(item.reason for item in pack.exceptions
                       if item.control == "calculation_evidence")
    assert "does not close" in reasons
    assert pack.status == "BLOCKED"


def test_a_computed_figure_with_no_rate_table_is_traceable_or_flagged(tmp_path):
    record = build_record()
    record["calculation"]["upstream"]["manifest"]["rate_table_uris"] = []
    record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
    pack = run(tmp_path, calculation_evidence_paths=[write(tmp_path, record)])
    assert any("names no rate table" in item.reason for item in pack.exceptions
               if item.control == "calculation_evidence")


def test_a_figure_with_no_rate_table_is_not_published_as_relied_upon(tmp_path):
    """The same shape as the wrong period: a REVIEW exception beside usable true."""
    from closecontrol.report import _as_json

    record = build_record()
    record["calculation"]["upstream"]["manifest"]["rate_table_uris"] = []
    record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
    pack = run(tmp_path, calculation_evidence_paths=[write(tmp_path, record)])
    assert _as_json(pack)["calculation_evidence"]["supplied"][0]["usable"] is False
    assert pack.relied_on == frozenset()


def test_an_acknowledgement_does_not_clear_an_evidence_exception(tmp_path):
    note = tmp_path / "note.json"
    note.write_text(json.dumps({
        "reviewer_initials": "RD",
        "reviewed_on": "2026-07-31",
        "comment": "Reviewed.",
    }), encoding="utf-8")
    pack = run(tmp_path, required_calculations=("coal-lsl-levy",), acknowledgement_path=note)
    assert pack.acknowledgement is not None
    assert pack.status in ("REVIEW", "BLOCKED")
    assert any(item.control == "calculation_evidence" for item in pack.exceptions)


def test_the_written_pack_carries_the_evidence_and_still_verifies(tmp_path):
    path = write(tmp_path, build_record())
    pack = run(tmp_path, calculation_evidence_paths=[path],
               required_calculations=("coal-lsl-levy",))
    destination = tmp_path / "out"
    outputs = write_review_pack(pack, destination)
    payload = json.loads(Path(outputs["json"]).read_text(encoding="utf-8"))
    assert payload["calculation_evidence"]["supplied"][0]["label"] == "coal-lsl-levy"
    # "still verifies" means the viewer accepts the pack, so ask it. The
    # earlier assertion checked that the word "close" appeared in the
    # summary, which any summary satisfies.
    from closecontrol.viewer import render_review_sheet

    sheet, _digests = render_review_sheet(destination)
    assert "- coal-lsl-levy: COMPUTED" in sheet
    summary = Path(outputs["summary"]).read_text(encoding="utf-8")
    assert "| coal-lsl-levy | COMPUTED |" in summary


# -- regressions from the 18 September 2026 review -------------------------


def test_a_pack_carrying_evidence_can_still_be_opened(tmp_path):
    """The viewer verifies the pack's members, and it did not know this one.

    Every pack the control produced was unopenable by `close-control view`.
    """
    from closecontrol.viewer import render_review_sheet

    path = write(tmp_path, build_record())
    pack = run(tmp_path, calculation_evidence_paths=[path],
               required_calculations=("coal-lsl-levy",))
    outputs = write_review_pack(pack, tmp_path / "out")
    sheet, _ = render_review_sheet(Path(outputs["json"]).parent)
    assert "coal-lsl-levy" in sheet or "calculation" in sheet.lower()


def test_a_computed_file_with_no_figure_does_not_satisfy_a_requirement(tmp_path):
    """A label alone used to pass the pack with exit 0 and usable true."""
    record = build_record()
    record["calculation"]["normalised"]["values"] = {}
    record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
    pack = run(tmp_path, calculation_evidence_paths=[write(tmp_path, record)],
               required_calculations=("coal-lsl-levy",))
    assert any("no normalised value" in item.reason for item in pack.exceptions
               if item.control == "calculation_evidence")
    assert pack.status in ("REVIEW", "BLOCKED")
    from closecontrol.report import _as_json

    assert _as_json(pack)["calculation_evidence"]["supplied"][0]["usable"] is False


def test_a_normalised_block_that_is_not_an_object_carries_no_figure(tmp_path):
    record = build_record()
    record["calculation"]["normalised"] = ["not", "an", "object"]
    record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
    pack = run(tmp_path, calculation_evidence_paths=[write(tmp_path, record)],
               required_calculations=("coal-lsl-levy",))
    assert any("no normalised value" in item.reason for item in pack.exceptions
               if item.control == "calculation_evidence")


def test_a_wrong_period_is_not_published_as_relied_upon(tmp_path):
    """The pack said usable true while raising a REVIEW exception about it."""
    from closecontrol.report import _as_json

    record = build_record()
    record["calculation"]["call"]["period"] = "urn:sbrm:period:coal-lsl-levy:2019-01"
    record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
    pack = run(tmp_path, calculation_evidence_paths=[write(tmp_path, record)])
    assert _as_json(pack)["calculation_evidence"]["supplied"][0]["usable"] is False
    assert any("does not include the current report date" in item.reason
               for item in pack.exceptions if item.control == "calculation_evidence")


def test_a_deeply_nested_file_is_unreadable_not_a_crash(tmp_path):
    path = tmp_path / "deep.json"
    path.write_text("[" * 40000 + "]" * 40000, encoding="utf-8")
    loaded, unreadable = module.load_all([path])
    assert loaded == []
    assert any("nested too deeply" in item for item in unreadable)


def test_a_huge_integer_is_unreadable_not_a_crash(tmp_path):
    path = tmp_path / "huge.json"
    path.write_text('{"schema": "x", "n": ' + "9" * 5000 + "}", encoding="utf-8")
    loaded, unreadable = module.load_all([path])
    assert loaded == []
    assert unreadable and "could not be read" in unreadable[0]


def test_a_year_the_calendar_has_no_room_for_is_unreadable(tmp_path):
    record = build_record()
    record["calculation"]["call"]["period"] = "urn:sbrm:period:div7a:fy0000"
    record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
    evidence = module.load(write(tmp_path, record))
    assert module.covers_period(evidence, date(2026, 7, 31)) is None
    pack = run(tmp_path, calculation_evidence_paths=[write(tmp_path, record, "fy0000.json")])
    assert any("cannot read as a period" in item.reason for item in pack.exceptions
               if item.control == "calculation_evidence")


@pytest.mark.parametrize("hidden", ["⁦", "⁩", "؜", "‮", "​"])
def test_every_directional_character_class_is_refused(tmp_path, hidden):
    record = build_record()
    record["calculation"]["upstream"]["advisory"]["notes"] = [f"levy is {hidden}192.38 payable"]
    record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
    with pytest.raises(SchemaError, match="control or formatting character"):
        module.load(write(tmp_path, record))


def test_a_label_must_be_a_slug(tmp_path):
    for bad in ("coal⁦lsl", "Coal-LSL", "coal lsl", "coal_lsl", "-coal", "coal--lsl"):
        record = build_record()
        record["calculation"]["label"] = bad
        record["calculation_sha256"] = hashlib.sha256(canonical(record["calculation"])).hexdigest()
        with pytest.raises(SchemaError):
            module.load(write(tmp_path, record, "label.json"))


# -- malformed shapes a reader must report, never crash on -----------------


@pytest.mark.parametrize("path,value,expected", [
    ("upstream.manifest", {"rate_table_uris": 1}, "manifest.rate_table_uris is int"),
    ("upstream.advisory", {"notes": 1}, "advisory.notes is int"),
    ("validation", {"findings": 1, "accepted": True}, "validation.findings is int"),
])
def test_a_scalar_where_an_array_belongs_is_a_schema_error(tmp_path, path, value, expected):
    """A truthy scalar raised TypeError straight out of the iteration.

    Nothing above catches that, so a malformed file reached the caller as a
    traceback instead of as the unreadable-evidence state this control has.
    """
    record = build_record(**{path: value})
    with pytest.raises(SchemaError, match=expected):
        module.load(write(tmp_path, record))


def test_a_non_text_advisory_note_is_a_finding_not_a_silent_drop(tmp_path):
    # {"notes": [123]} satisfied the presence check above and then yielded no
    # advisory text at all, so the file read as though it carried a boundary
    # statement while carrying none.
    record = build_record(**{"upstream.advisory": {"notes": [123]}})
    evidence = module.load(write(tmp_path, record))
    assert evidence.advisory_notes == ()
    assert any("not text" in finding for finding in evidence.findings)
    assert evidence.usable is False


def test_a_malformed_manifest_entry_is_a_finding(tmp_path):
    record = build_record(**{"upstream.manifest": {"rate_table_uris": ["urn:not-an-object"]}})
    evidence = module.load(write(tmp_path, record))
    assert evidence.rate_tables == ()
    assert any("not an object" in finding for finding in evidence.findings)
    assert evidence.usable is False


def test_a_manifest_entry_without_a_uri_is_a_finding(tmp_path):
    record = build_record(**{"upstream.manifest": {"rate_table_uris": [{"sha256": "c" * 64}]}})
    evidence = module.load(write(tmp_path, record))
    assert any("carries no uri string" in finding for finding in evidence.findings)


def test_a_non_text_validation_finding_is_itself_a_finding(tmp_path):
    record = build_record(**{"validation": {"findings": [123], "accepted": False}})
    evidence = module.load(write(tmp_path, record))
    assert any("not text" in finding for finding in evidence.findings)
    assert evidence.usable is False


@pytest.mark.parametrize("character", ["\u00ad", "\u2060", "\ufeff", "\ufff9", "\u0090"])
def test_every_format_and_control_character_is_hidden(tmp_path, character):
    # The loader refuses every Cc, Cf and Cs character in a source file; a list of
    # ranges here missed these, and they reached close-summary.md unescaped.
    record = build_record(**{"call.status": f"COMPUTED{character}"})
    with pytest.raises(SchemaError, match="control or formatting character"):
        module.load(write(tmp_path, record))


def test_a_lone_surrogate_is_hidden():
    # json.loads turns an escaped lone surrogate into one; the test fixture cannot
    # write it, because it encodes the record as UTF-8 first.
    assert module._is_hidden("\ud800")


@pytest.mark.parametrize("character", ["\u0085", "\u2028", "\u2029"])
def test_a_line_separator_in_evidence_text_is_hidden(tmp_path, character):
    # str.splitlines() breaks at each of these, so a status carrying one was
    # written into a summary row as one line and read back as two, and the
    # writer's own pack failed to verify.
    record = build_record(**{"call.status": f"COMPUTED{character}"})
    with pytest.raises(SchemaError, match="control or formatting character"):
        module.load(write(tmp_path, record))
