import hashlib
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest
from evatt import cli, disclosure
from evatt.errors import EvattError

if __package__:
    from .test_cli import workspace
else:
    from test_cli import workspace

DESTINATION = "model:sample-tenant:sample-project"
DECISION = "sample-decision:42"
SENTINEL = "Sample Sensitive Sentinel"


@pytest.fixture
def files(tmp_path, monkeypatch):
    if shutil.which("git") is None:
        pytest.skip("git is not installed")
    for name in ("GIT_CONFIG_GLOBAL", "GIT_CONFIG_SYSTEM"):
        monkeypatch.setenv(name, str(tmp_path / "absent-gitconfig"))
    monkeypatch.setenv("GIT_CEILING_DIRECTORIES", str(tmp_path.parent))
    root = workspace(tmp_path, "entities.json\n*.triage.md\n*.disclosure.json\n")
    source = root / "out.md"
    source.write_bytes(b"CLIENT_01\n")
    return source, root / "entities.json", root / "out.disclosure.json"


def arguments(files, command="disclosure-record", destination=DESTINATION, decision=DECISION):
    source, mapping, record = files
    return [command, "--in", str(source), "--map", str(mapping),
            "--out" if command == "disclosure-record" else "--record", str(record),
            "--destination", destination, "--decision-ref", decision]


def assert_private(capsys):
    output = capsys.readouterr()
    assert SENTINEL not in output.out + output.err
    assert "Traceback" not in output.out + output.err
    return output


def test_record_and_check_are_local_and_preserve_the_count_manifest(files, capsys):
    source, mapping, record = files
    manifest = source.with_name(source.name + ".manifest.json")
    manifest.write_bytes(b'{"schema_version":1,"counts":{}}\n')
    originals = {path: path.read_bytes() for path in (source, mapping, manifest)}
    assert cli.main(arguments(files)) == 0
    assert cli.main(arguments(files, "disclosure-check")) == 0
    assert {path: path.read_bytes() for path in originals} == originals
    assert set(json.loads(record.read_bytes())) == {
        "schema", "output_sha256", "entity_map_sha256", "tool_version", "destination", "decision_ref"}
    output = assert_private(capsys)
    assert output.err == ""
    assert output.out.count("External authorisation has not been checked.") == 2
    assert DESTINATION not in output.out
    assert DECISION not in output.out
    assert str(record) not in output.out
    if os.name != "nt":
        assert record.stat().st_mode & 0o777 == 0o600


@pytest.mark.parametrize("change", ["bytes", "map", "destination", "decision"])
def test_changed_context_is_a_refusal(files, change, capsys):
    source, mapping, _record = files
    assert cli.main(arguments(files)) == 0
    destination, decision = DESTINATION, DECISION
    if change == "bytes":
        source.write_bytes(b"CLIENT_01\r\n")
    elif change == "map":
        document = json.loads(mapping.read_bytes())
        document["entries"][0]["added"] = "2026-10-02"
        mapping.write_text(json.dumps(document), encoding="utf-8")
    elif change == "destination":
        destination = "model:sample-other"
    else:
        decision = "sample-decision:43"
    assert cli.main(arguments(files, "disclosure-check", destination, decision)) == 2
    assert_private(capsys)


def test_findings_never_create_a_record_or_print_values(files, capsys):
    source, mapping, record = files
    value = json.loads(mapping.read_bytes())["entries"][0]["value"]
    source.write_text(value, encoding="utf-8")
    assert cli.main(arguments(files)) == 2
    assert not record.exists()
    output = assert_private(capsys)
    assert value not in output.out + output.err


def test_check_with_matching_hashes_still_refuses_findings_privately(files, capsys):
    source, mapping, record = files
    assert cli.main(arguments(files)) == 0
    value = json.loads(mapping.read_bytes())["entries"][0]["value"]
    leaked = value.encode("utf-8")
    source.write_bytes(leaked)
    document = json.loads(record.read_bytes())
    document["output_sha256"] = hashlib.sha256(leaked).hexdigest()
    record.write_text(json.dumps(document), encoding="utf-8")
    assert cli.main(arguments(files, "disclosure-check")) == 2
    output = assert_private(capsys)
    assert value not in output.out + output.err


@pytest.mark.parametrize("name", ["out.md", "entities.json", "out.md.manifest.json",
                                 "out.md.triage.md", "existing.disclosure.json"])
def test_creation_never_overwrites_protected_or_existing_files(files, name, capsys):
    source, mapping, _record = files
    destination = source.parent / name
    if not destination.exists():
        destination.write_bytes(b"fabricated pre-existing evidence")
    original = destination.read_bytes()
    assert cli.main(arguments((source, mapping, destination))) == 1
    assert destination.read_bytes() == original
    assert_private(capsys)


@pytest.mark.parametrize("suffix", [".manifest.json", ".triage.md"])
def test_creation_refuses_even_an_absent_reserved_sidecar(files, suffix):
    source, mapping, _record = files
    reserved = source.with_name(source.name + suffix)
    assert not reserved.exists()
    assert cli.main(arguments((source, mapping, reserved))) == 1
    assert not reserved.exists()


def symlink_or_skip(target, link, *, directory=False):
    try:
        link.symlink_to(target, target_is_directory=directory)
    except OSError:
        pytest.skip("this platform cannot create test symlinks")


@pytest.mark.parametrize("target", ["source", "map", "absent"])
def test_creation_refuses_symlinks_including_dangling_links(files, target):
    source, mapping, record = files
    selected = {"source": source, "map": mapping, "absent": source.parent / "absent.md"}[target]
    symlink_or_skip(selected, record)
    originals = (source.read_bytes(), mapping.read_bytes())
    assert cli.main(arguments(files)) == 1
    assert (source.read_bytes(), mapping.read_bytes()) == originals
    assert record.is_symlink()


def test_creation_refuses_a_hardlink_alias(files):
    source, mapping, record = files
    record.hardlink_to(mapping)
    original = mapping.read_bytes()
    assert cli.main(arguments(files)) == 1
    assert mapping.read_bytes() == original
    assert record.read_bytes() == original


def test_existing_name_created_during_the_open_is_preserved(files, monkeypatch):
    _source, _mapping, record = files
    original_open = os.open

    def race(path, flags, mode=0o777, **kwargs):
        if Path(path) == record:
            record.write_bytes(b"fabricated concurrent file")
        return original_open(path, flags, mode, **kwargs)

    monkeypatch.setattr(os, "open", race)
    assert cli.main(arguments(files)) == 1
    assert record.read_bytes() == b"fabricated concurrent file"


@pytest.mark.parametrize("state", ["unignored", "tracked", "outside"])
def test_record_hygiene_is_checked_on_creation(files, state):
    source, mapping, record = files
    if state == "unignored":
        (source.parent / ".gitignore").write_text("entities.json\n", encoding="utf-8")
    elif state == "tracked":
        record.write_bytes(b"fabricated tracked record")
        subprocess.run(["git", "add", "-f", str(record)], cwd=source.parent, check=True,
                       capture_output=True)
    else:
        record = source.parent.parent / "outside.disclosure.json"
    assert cli.main(arguments((source, mapping, record))) == 1
    if state == "tracked":
        assert record.read_bytes() == b"fabricated tracked record"
    else:
        assert not record.exists()


@pytest.mark.parametrize("state", ["unignored", "tracked"])
def test_record_hygiene_is_checked_again_when_checking(files, state):
    source, _mapping, record = files
    assert cli.main(arguments(files)) == 0
    if state == "unignored":
        (source.parent / ".gitignore").write_text("entities.json\n", encoding="utf-8")
    else:
        subprocess.run(["git", "add", "-f", str(record)], cwd=source.parent, check=True,
                       capture_output=True)
    assert cli.main(arguments(files, "disclosure-check")) == 1


@pytest.mark.parametrize("kind", ["symlink", "hardlink", "directory"])
def test_check_refuses_links_and_nonregular_records(files, kind):
    source, mapping, record = files
    assert cli.main(arguments(files)) == 0
    alias = source.parent / "alias.disclosure.json"
    if kind == "symlink":
        symlink_or_skip(record, alias)
    elif kind == "hardlink":
        alias.hardlink_to(record)
    else:
        alias.mkdir()
    assert cli.main(arguments((source, mapping, alias), "disclosure-check")) == 1


def test_check_refuses_a_fifo_without_opening_it(files):
    if not hasattr(os, "mkfifo"):
        pytest.skip("FIFOs are not available on this platform")
    _source, _mapping, record = files
    os.mkfifo(record)
    assert cli.main(arguments(files, "disclosure-check")) == 1


def test_linked_record_parent_is_refused(files):
    source, mapping, _record = files
    target = source.parent / "records"
    target.mkdir()
    linked = source.parent / "linked"
    symlink_or_skip(target, linked, directory=True)
    output = linked / "out.disclosure.json"
    assert cli.main(arguments((source, mapping, output))) == 1
    assert not (target / output.name).exists()


def test_record_symlink_loop_is_a_private_error(files, capsys):
    _source, _mapping, record = files
    symlink_or_skip(record.name, record)
    assert cli.main(arguments(files)) == 1
    assert cli.main(arguments(files, "disclosure-check")) == 1
    assert_private(capsys)


@pytest.mark.parametrize("command", ["disclosure-record", "disclosure-check"])
def test_linked_parent_cannot_be_hidden_by_dotdot(files, command):
    source, mapping, record = files
    ordinary = source.parent / "real"
    ordinary.mkdir()
    linked = source.parent / "linked"
    symlink_or_skip(ordinary, linked, directory=True)
    if command == "disclosure-check":
        assert cli.main(arguments(files)) == 0
    disguised = linked / ".." / record.name
    assert cli.main(arguments((source, mapping, disguised), command)) == 1
    assert record.exists() == (command == "disclosure-check")


def test_ordinary_dotdot_record_paths_still_work(files):
    source, mapping, record = files
    ordinary = source.parent / "real"
    ordinary.mkdir()
    spelled = ordinary / ".." / record.name
    assert cli.main(arguments((source, mapping, spelled))) == 0
    assert cli.main(arguments((source, mapping, spelled), "disclosure-check")) == 0


@pytest.mark.parametrize("command", ["disclosure-record", "disclosure-check"])
def test_oversized_map_integer_is_a_controlled_private_error(files, command, capsys):
    _source, mapping, record = files
    if not hasattr(sys, "set_int_max_str_digits"):
        pytest.skip("integer conversion limit is unavailable")
    previous = sys.get_int_max_str_digits()
    try:
        sys.set_int_max_str_digits(4300)
        mapping.write_bytes(b'{"schema_version":' + b"9" * 4301 + b',"entries":[]}')
        assert cli.main(arguments(files, command)) == 1
    finally:
        sys.set_int_max_str_digits(previous)
    assert not record.exists()
    output = assert_private(capsys)
    assert output.out == ""
    assert "cannot use entity map" in output.err


@pytest.mark.parametrize("size", [disclosure.MAX_RECORD_BYTES, disclosure.MAX_RECORD_BYTES + 1])
def test_cli_reads_enough_to_reject_oversized_valid_json(files, size):
    _source, _mapping, record = files
    assert cli.main(arguments(files)) == 0
    encoded = record.read_bytes()
    record.write_bytes(encoded + b" " * (size - len(encoded)))
    assert cli.main(arguments(files, "disclosure-check")) == (
        0 if size == disclosure.MAX_RECORD_BYTES else 1)


def test_record_can_be_created_in_a_new_ignored_subdirectory(files, monkeypatch):
    source, mapping, record = files
    output = source.parent / "nested" / record.name
    monkeypatch.chdir(source.parent)
    assert cli.main(arguments((source, mapping, output))) == 0
    monkeypatch.chdir(output.parent)
    assert cli.main(arguments((source, mapping, output), "disclosure-check")) == 0


@pytest.mark.parametrize("failure", ["guard", "map", "source", "record", "metadata"])
def test_errors_do_not_echo_sensitive_values_or_paths(files, failure, monkeypatch, capsys):
    source, mapping, record = files
    if failure == "guard":
        def failed_guard(*args):
            raise EvattError(SENTINEL)
        monkeypatch.setattr(cli.entities_module, "require_gitignored", failed_guard)
    elif failure == "map":
        document = json.loads(mapping.read_bytes())
        document["entries"][0]["kind"] = SENTINEL
        mapping.write_text(json.dumps(document), encoding="utf-8")
    elif failure == "source":
        source = source.with_name(SENTINEL + ".md")
    elif failure == "record":
        assert cli.main(arguments(files)) == 0
        record.write_text('{"' + SENTINEL + '":NaN}', encoding="utf-8")
    command = "disclosure-check" if failure == "record" else "disclosure-record"
    destination = SENTINEL if failure == "metadata" else DESTINATION
    assert cli.main(arguments((source, mapping, record), command, destination)) == 1
    assert_private(capsys)


@pytest.mark.parametrize("extra", ["missing", "unknown", "global"])
def test_argument_errors_do_not_echo_supplied_values(files, extra, capsys):
    args = arguments(files, destination=SENTINEL)
    if extra == "missing":
        args.remove("--destination")
    elif extra == "unknown":
        args.extend(["--unknown", SENTINEL])
    else:
        args = ["--unknown", SENTINEL, *args]
    assert cli.main(args) == 1
    assert_private(capsys)


@pytest.mark.parametrize("args", [
    ["--destination", SENTINEL],
    ["disclosure-reocrd", "--destination", SENTINEL],
    [f"--destination={SENTINEL}"],
    [f"--decision-ref={SENTINEL}"],
    [f"--record={SENTINEL}"],
    ["--dest", SENTINEL],
    [f"--decision={SENTINEL}"],
    [f"--rec={SENTINEL}"],
])
def test_disclosure_argument_typos_are_private(args, capsys):
    assert cli.main(args) == 1
    output = assert_private(capsys)
    assert "invalid disclosure arguments" in output.err


def test_disclosure_commands_bypass_legacy_collision_diagnostics(files, monkeypatch, capsys):
    def unexpected(_args):
        pytest.fail("disclosure reached legacy collision diagnostics")

    monkeypatch.setattr(cli, "_collision", unexpected)
    assert cli.main(arguments(files)) == 0
    assert cli.main(arguments(files, "disclosure-check")) == 0
    assert_private(capsys)


@pytest.mark.parametrize("failure", ["write", "short", "close"])
def test_write_failures_never_report_success_or_overwrite_another_file(
    files, failure, monkeypatch, capsys
):
    _source, mapping, record = files
    original_map = mapping.read_bytes()
    original_fdopen = os.fdopen

    class BrokenWriter:
        def __init__(self, descriptor, mode):
            self.handle = original_fdopen(descriptor, mode)

        def __enter__(self):
            return self

        def __exit__(self, *args):
            self.handle.close()
            if failure == "close":
                raise OSError(SENTINEL)

        def write(self, data):
            if failure in ("write", "short"):
                self.handle.write(data[:10])
                if failure == "write":
                    raise OSError(SENTINEL)
                return 10
            return self.handle.write(data)

    monkeypatch.setattr(os, "fdopen", BrokenWriter)
    assert cli.main(arguments(files)) == 1
    assert mapping.read_bytes() == original_map
    assert_private(capsys)
    monkeypatch.setattr(os, "fdopen", original_fdopen)
    if failure != "close":
        assert cli.main(arguments(files, "disclosure-check")) == 1
    assert record.exists()


def test_creation_hashes_a_snapshot_and_check_detects_a_later_edit(files, monkeypatch):
    source, _mapping, _record = files
    original = disclosure.create_record

    def changed_after_capture(payload, entries, **kwargs):
        encoded = original(payload, entries, **kwargs)
        source.write_bytes(b"CLIENT_01\r\n")
        return encoded

    monkeypatch.setattr(disclosure, "create_record", changed_after_capture)
    assert cli.main(arguments(files)) == 0
    assert cli.main(arguments(files, "disclosure-check")) == 2
