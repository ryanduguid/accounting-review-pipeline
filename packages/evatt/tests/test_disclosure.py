"""Fabricated disclosure examples. No sender or external authority is installed."""
import hashlib
import json
from dataclasses import replace
from pathlib import Path

import pytest
from evatt import disclosure, entities
from evatt.errors import EvattError
from evatt.version import __version__

SAMPLES = Path(entities.__file__).resolve().parent / "samples"
DESTINATION = "model:sample-tenant:sample-project"
DECISION = "sample-decision:42"
PAYLOAD = b"CLIENT_01\n"


@pytest.fixture
def entries():
    return entities.load(SAMPLES / "entities.sample.json")


def record(payload, entries, destination=DESTINATION, decision_ref=DECISION):
    return disclosure.create_record(payload, entries, destination=destination,
                                    decision_ref=decision_ref)


def check(payload, entries, encoded, destination=DESTINATION, decision_ref=DECISION):
    return disclosure.check_record(payload, entries, encoded, expected_destination=destination,
                                   expected_decision_ref=decision_ref)


@pytest.mark.parametrize("payload", [b"", PAYLOAD, b"TFN_01\n", b"CLIENT_01\r\n", b"CLIENT_01\r",
                                      "CLIENT_01\ncafé\n".encode("utf-8")])
def test_returns_the_original_snapshot_without_reencoding(entries, payload):
    encoded = record(payload, entries)
    assert check(payload, entries, encoded) is payload
    document = json.loads(encoded)
    assert document["output_sha256"] == hashlib.sha256(payload).hexdigest()
    assert document["tool_version"] == __version__
    assert document["schema"] == disclosure.SCHEMA
    assert document["destination"] == DESTINATION
    assert document["decision_ref"] == DECISION
    assert len(encoded) <= disclosure.MAX_RECORD_BYTES
    for entry in entries:
        assert entry.value.encode("utf-8") not in encoded


@pytest.mark.parametrize("changed", [b"CLIENT_01\r\n", b"CLIENT_01\r", b"CLIENT_01",
                                      b"CLIENT_01 \n", b"CLIENT_01\n\n"])
def test_any_byte_change_refuses_the_unchanged_record(entries, changed):
    with pytest.raises(disclosure.DisclosureRefused):
        check(changed, entries, record(PAYLOAD, entries))


def test_composed_and_decomposed_text_have_different_evidence(entries):
    original = "CLIENT_01\ncafé\n".encode("utf-8")
    changed = "CLIENT_01\ncafe\u0301\n".encode("utf-8")
    with pytest.raises(disclosure.DisclosureRefused):
        check(changed, entries, record(original, entries))


@pytest.mark.parametrize("change", [
    {"value": "Sample Other Entity"}, {"placeholder": "CLIENT_42"},
    {"kind": "entity", "placeholder": "ENTITY_42"}, {"added": "2026-10-02"},
])
def test_every_loaded_map_field_is_bound(entries, change):
    original = record(b"", entries)
    changed = (replace(entries[0], **change), *entries[1:])
    assert json.loads(record(b"", changed))["entity_map_sha256"] != json.loads(original)[
        "entity_map_sha256"]
    with pytest.raises(disclosure.DisclosureRefused):
        check(b"", changed, original)


def test_map_entry_order_is_bound(entries):
    with pytest.raises(disclosure.DisclosureRefused):
        check(b"", tuple(reversed(entries)), record(b"", entries))


def test_map_file_formatting_does_not_change_the_loaded_digest(tmp_path, entries):
    document = json.loads((SAMPLES / "entities.sample.json").read_bytes())
    path = tmp_path / "fabricated.json"
    path.write_text(json.dumps(document, sort_keys=True, separators=(",", ":")), encoding="utf-8")
    assert record(b"", entities.load(path)) == record(b"", entries)


def test_empty_map_canonicalisation_vector():
    # The literal encoding pins the domain, field names and JSON separators.
    canonical = b'evatt.entity-map/v1\0{"entries":[],"schema_version":1}'
    document = json.loads(record(b"", ()))
    assert document["entity_map_sha256"] == hashlib.sha256(canonical).hexdigest()


@pytest.mark.parametrize("destination, decision_ref", [
    ("model:sample-other-tenant:sample-project", DECISION),
    (DESTINATION, "sample-decision:43"),
])
def test_expected_context_must_match(entries, destination, decision_ref):
    with pytest.raises(disclosure.DisclosureRefused):
        check(PAYLOAD, entries, record(PAYLOAD, entries), destination, decision_ref)


@pytest.mark.parametrize("destination, decision_ref", [
    ("Model:sample", DECISION), (" model:sample", DECISION), ("sample\nrecipient", DECISION),
    ("sample/recipient", DECISION), ("x" * 129, DECISION), (True, DECISION),
    (DESTINATION, "Sample Sensitive Sentinel"), (DESTINATION, "sample@example.invalid"),
    (DESTINATION, "x" * 129), (DESTINATION, None),
])
def test_noncanonical_context_is_rejected_without_echoing_it(entries, destination, decision_ref):
    with pytest.raises(EvattError) as error:
        record(PAYLOAD, entries, destination, decision_ref)
    assert "Sensitive Sentinel" not in str(error.value)
    assert "example.invalid" not in str(error.value)


@pytest.mark.parametrize("payload", [b"\xff", b"\xef\xbb\xbfCLIENT_01\n", bytearray(PAYLOAD)])
def test_only_immutable_utf8_without_a_bom_is_supported(entries, payload):
    with pytest.raises(EvattError):
        record(payload, entries)
    with pytest.raises(EvattError):
        check(payload, entries, record(PAYLOAD, entries))


def test_invalid_library_map_uses_existing_validation_without_echoing_values(entries):
    invalid = (replace(entries[0], value="TFN_01"),)
    with pytest.raises(EvattError, match="invalid entity map") as error:
        record(b"", invalid)
    assert "TFN_01" not in str(error.value)


@pytest.mark.parametrize("duplicate", ["JANE  ROE", "JANE_ROE", "Jane<br>Roe", "Jane\u200dRoe"])
def test_library_map_uses_current_name_equivalence_privately(entries, duplicate):
    invalid = (entries[2], replace(entries[3], value=duplicate))
    for operation in (lambda: record(b"", invalid),
                      lambda: check(b"", invalid, record(b"", entries))):
        with pytest.raises(EvattError, match="invalid entity map") as error:
            operation()
        assert entries[2].value not in str(error.value)
        assert duplicate not in str(error.value)


@pytest.mark.parametrize("payload", [b"CLIENT_99\n", b"**Jane**<br>Roe\r\n", b"Jane_Roe\n",
                                      "Jane\u200dRoe\n".encode("utf-8")])
def test_both_operations_inherit_current_verifier_rules(entries, payload):
    with pytest.raises(disclosure.DisclosureRefused):
        record(payload, entries)
    forged = json.loads(record(PAYLOAD, entries))
    forged["output_sha256"] = hashlib.sha256(payload).hexdigest()
    with pytest.raises(disclosure.DisclosureRefused):
        check(payload, entries, json.dumps(forged).encode("utf-8"))


def test_both_operations_scan_even_when_a_forged_record_has_matching_hashes(entries):
    leaked = entries[0].value.encode("utf-8")
    with pytest.raises(disclosure.DisclosureRefused) as error:
        record(leaked, entries)
    assert entries[0].value not in str(error.value)
    forged = json.loads(record(PAYLOAD, entries))
    forged["output_sha256"] = hashlib.sha256(leaked).hexdigest()
    with pytest.raises(disclosure.DisclosureRefused) as error:
        check(leaked, entries, json.dumps(forged).encode("utf-8"))
    assert entries[0].value not in str(error.value)


def test_both_successful_operations_rescan(entries, monkeypatch):
    scans = []
    original = disclosure.findings

    def counted(text, mapping):
        scans.append(text)
        return original(text, mapping)

    monkeypatch.setattr(disclosure, "findings", counted)
    check(PAYLOAD, entries, record(PAYLOAD, entries))
    assert scans == [PAYLOAD.decode("utf-8"), PAYLOAD.decode("utf-8")]


@pytest.mark.parametrize("field, value", [
    ("schema", "evatt.disclosure-evidence/v2"), ("schema", True),
    ("output_sha256", "A" * 64), ("entity_map_sha256", "0" * 63),
    ("tool_version", []), ("tool_version", "bad\nversion"), ("destination", "Sample Tenant"),
    ("decision_ref", 42), ("decision_ref", "x" * 129), ("Sample Sensitive Sentinel", "secret"),
])
def test_malformed_fields_are_rejected_without_echoing_them(entries, field, value):
    document = json.loads(record(PAYLOAD, entries))
    document[field] = value
    with pytest.raises(EvattError) as error:
        check(PAYLOAD, entries, json.dumps(document).encode("utf-8"))
    assert "Sensitive Sentinel" not in str(error.value)


def test_missing_field_and_tool_version_policy(entries):
    document = json.loads(record(PAYLOAD, entries))
    del document["decision_ref"]
    with pytest.raises(EvattError):
        check(PAYLOAD, entries, json.dumps(document).encode("utf-8"))
    document = json.loads(record(PAYLOAD, entries))
    document["tool_version"] = "0.0.0"
    with pytest.raises(disclosure.DisclosureRefused):
        check(PAYLOAD, entries, json.dumps(document).encode("utf-8"))


@pytest.mark.parametrize("bad", [
    b"", b"[]", b"true", b"\xff", b"{} {}", b"{" * 1024, b"[" * 1024,
    b" " * (disclosure.MAX_RECORD_BYTES + 1), bytearray(b"{}"),
    b'{"schema":NaN}', b'{"schema":Infinity}', b'{"schema":-Infinity}',
])
def test_invalid_record_encodings_and_sizes(entries, bad):
    with pytest.raises(EvattError):
        check(PAYLOAD, entries, bad)


@pytest.mark.parametrize("size", [disclosure.MAX_RECORD_BYTES, disclosure.MAX_RECORD_BYTES + 1])
def test_valid_json_at_and_over_the_size_limit(entries, size):
    encoded = record(PAYLOAD, entries)
    padded = encoded + b" " * (size - len(encoded))
    assert json.loads(padded) == json.loads(encoded)
    if size == disclosure.MAX_RECORD_BYTES:
        assert check(PAYLOAD, entries, padded) is PAYLOAD
    else:
        with pytest.raises(EvattError, match="at most 4096"):
            check(PAYLOAD, entries, padded)


@pytest.mark.parametrize("key", [b"destination", b"destin\\u0061tion"])
def test_duplicate_fields_are_rejected_after_escape_decoding(entries, key):
    encoded = record(PAYLOAD, entries).rstrip()[:-1]
    encoded += b',"' + key + b'":"model:sample"}'
    with pytest.raises(EvattError, match="invalid disclosure record JSON"):
        check(PAYLOAD, entries, encoded)


def test_checked_snapshot_survives_a_later_source_change(tmp_path, entries):
    source = tmp_path / "fabricated.md"
    source.write_bytes(PAYLOAD)
    snapshot = source.read_bytes()
    checked = check(snapshot, entries, record(snapshot, entries))
    source.write_bytes(entries[0].value.encode("utf-8"))
    inert_sink = []
    inert_sink.append(checked)
    assert inert_sink == [PAYLOAD]
    assert source.read_bytes() != inert_sink[0]


@pytest.mark.parametrize("authority_state", ["allow", "absent", "denied", "revoked",
                                            "other-output", "other-destination", "other-ref"])
def test_outer_sender_needs_a_separate_exact_decision(entries, authority_state):
    # A test-only authority and inert sink show the caller's separate obligation.
    expected = (hashlib.sha256(PAYLOAD).hexdigest(), DESTINATION, DECISION)
    granted = expected
    if authority_state == "other-output":
        granted = ("0" * 64, DESTINATION, DECISION)
    elif authority_state == "other-destination":
        granted = (expected[0], "model:sample-other", DECISION)
    elif authority_state == "other-ref":
        granted = (expected[0], DESTINATION, "sample-decision:43")
    authority = {} if authority_state == "absent" else {
        granted: authority_state not in ("denied", "revoked")}
    checked = check(PAYLOAD, entries, record(PAYLOAD, entries))
    inert_sink = []
    if authority.get((hashlib.sha256(checked).hexdigest(), DESTINATION, DECISION)) is True:
        inert_sink.append(checked)
    assert inert_sink == ([PAYLOAD] if authority_state == "allow" else [])


def test_consistently_edited_unsigned_record_can_match_without_permission(entries):
    document = json.loads(record(PAYLOAD, entries))
    changed_destination = "model:sample-other"
    document["destination"] = changed_destination
    checked = check(PAYLOAD, entries, json.dumps(document).encode("utf-8"), changed_destination)
    authority = {(DESTINATION, DECISION): True}
    inert_sink = []
    if authority.get((changed_destination, DECISION)) is True:
        inert_sink.append(checked)
    assert checked == PAYLOAD
    assert inert_sink == []


def test_failed_check_never_reaches_the_inert_sink(entries):
    inert_sink = []
    with pytest.raises(disclosure.DisclosureRefused):
        checked = check(b"CLIENT_01\r\n", entries, record(PAYLOAD, entries))
        inert_sink.append(checked)
    assert inert_sink == []


def test_disclosure_documentation_keeps_authority_separate():
    root = Path(__file__).resolve().parents[1]
    readme = (root / "README.md").read_text(encoding="utf-8")
    assert "Matching evidence is not permission" in readme
    assert "record is unsigned and can be edited" in readme
    assert "independently authenticate the external human" in readme
    assert "same immutable payload bytes" in readme
    assert "existing replacement-count manifest stays unchanged" in readme
