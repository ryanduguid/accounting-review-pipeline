"""Fabricated paths for static-link refusal and ordinary Windows aliases."""
import ctypes
import os
import stat
import struct
import uuid
from pathlib import Path
from types import SimpleNamespace
from unittest import TestCase
from unittest.mock import Mock

import pytest
from evatt import cli
from evatt.errors import EvattError

if __package__:
    from . import test_disclosure_cli as disclosure_tests
else:
    import test_disclosure_cli as disclosure_tests

SENTINEL = disclosure_tests.SENTINEL
arguments = disclosure_tests.arguments
assert_private = disclosure_tests.assert_private
files = disclosure_tests.files
symlink_or_skip = disclosure_tests.symlink_or_skip
_checks = TestCase()


@pytest.mark.parametrize("suffix", ["linked", "linked/..", "linked/new/deeper"])
@pytest.mark.parametrize("command", ["disclosure-record", "disclosure-check"])
def test_missing_prefix_cannot_conceal_a_static_link(files, suffix, command):
    source, mapping, record = files
    target = source.parent / "real"
    target.mkdir()
    linked = source.parent / "linked"
    symlink_or_skip(target, linked, directory=True)
    if command == "disclosure-check":
        _checks.assertEqual(cli.main(arguments(files)), 0)
    missing = source.parent / "missing"
    disguised = missing / ".." / suffix / record.name
    _checks.assertEqual(cli.main(arguments((source, mapping, disguised), command)), 1)
    _checks.assertFalse(missing.exists())
    _checks.assertEqual(list(target.iterdir()), [])
    _checks.assertEqual(record.exists(), command == "disclosure-check")


@pytest.mark.parametrize("relative", ["missing/../record", "new/deep/record", "real/../record"])
def test_ordinary_relative_and_missing_paths_work(files, monkeypatch, relative):
    source, mapping, record = files
    (source.parent / "real").mkdir()
    monkeypatch.chdir(source.parent)
    spelled = Path(relative).with_name(record.name)
    selected = (source, mapping, spelled)
    _checks.assertEqual(cli.main(arguments(selected)), 0)
    _checks.assertEqual(cli.main(arguments(selected, "disclosure-check")), 0)


@pytest.mark.parametrize("error", [PermissionError, NotADirectoryError, OSError, RuntimeError])
def test_record_metadata_errors_are_private(files, monkeypatch, capsys, error):
    source, mapping, record = files
    _checks.assertEqual(cli.main(arguments(files)), 0)
    assert_private(capsys)
    original = Path.lstat

    def fail(path, *args, **kwargs):
        if path == record:
            raise error(SENTINEL)
        return original(path, *args, **kwargs)

    monkeypatch.setattr(Path, "lstat", fail)
    _checks.assertEqual(cli.main(arguments(files, "disclosure-check")), 1)
    output = assert_private(capsys)
    _checks.assertEqual(output.out, "")
    _checks.assertNotIn(str(source), output.err)
    _checks.assertNotIn(str(mapping), output.err)
    _checks.assertNotIn(str(record), output.err)


def test_symbolic_link_mode_is_refused_without_windows_attributes(files, monkeypatch):
    _source, _mapping, record = files
    _checks.assertEqual(cli.main(arguments(files)), 0)
    original = Path.lstat

    def metadata(path, *args, **kwargs):
        if path == record:
            return SimpleNamespace(st_mode=stat.S_IFLNK)
        return original(path, *args, **kwargs)

    monkeypatch.setattr(Path, "lstat", metadata)
    _checks.assertEqual(cli.main(arguments(files, "disclosure-check")), 1)


@pytest.mark.skipif(os.name != "nt", reason="native Windows entry attributes")
@pytest.mark.parametrize("attributes", [0x400, 0x410, 0x420])
def test_native_reparse_attributes_are_refused_before_lstat(
    files, monkeypatch, capsys, attributes
):
    _source, _mapping, record = files

    class GetAttributes:
        def __call__(self, _path):
            return attributes

    monkeypatch.setattr(ctypes, "WinDLL", lambda *args, **kwargs: SimpleNamespace(
        GetFileAttributesW=GetAttributes()))
    original = Path.lstat

    def forbid(path, *args, **kwargs):
        if path == record:
            pytest.fail("reparse metadata was followed")
        return original(path, *args, **kwargs)

    monkeypatch.setattr(Path, "lstat", forbid)
    _checks.assertEqual(cli.main(arguments(files)), 1)
    _checks.assertFalse(record.exists())
    assert_private(capsys)


@pytest.mark.skipif(os.name != "nt", reason="native Windows entry errors")
@pytest.mark.parametrize("error", [0, 5, 32, 1920])
def test_unknown_native_attribute_failures_are_private(files, monkeypatch, capsys, error):
    class GetAttributes:
        def __call__(self, _path):
            return 0xFFFFFFFF

    monkeypatch.setattr(ctypes, "WinDLL", lambda *args, **kwargs: SimpleNamespace(
        GetFileAttributesW=GetAttributes()))
    monkeypatch.setattr(ctypes, "get_last_error", lambda: error)
    _checks.assertEqual(cli.main(arguments(files)), 1)
    assert_private(capsys)


def short_path(path):
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    function = kernel.GetShortPathNameW
    function.argtypes = [ctypes.c_wchar_p, ctypes.c_wchar_p, ctypes.c_uint32]
    function.restype = ctypes.c_uint32
    buffer = ctypes.create_unicode_buffer(32768)
    size = function(str(path), buffer, len(buffer))
    if not size or size >= len(buffer):
        raise ctypes.WinError(ctypes.get_last_error())
    result = Path(buffer.value)
    if os.path.normcase(str(result)) == os.path.normcase(str(path)):
        pytest.skip("this volume does not provide a distinct ordinary short name")
    _checks.assertEqual(result.resolve(), path.resolve())
    _checks.assertFalse(result.lstat().st_file_attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT)
    return result


def short_leaf(path):
    """Set a distinct alias on this owned fixture without changing volume policy."""
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    create = kernel.CreateFileW
    create.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32,
                      ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p]
    create.restype = ctypes.c_void_p
    set_name = kernel.SetFileShortNameW
    set_name.argtypes = [ctypes.c_void_p, ctypes.c_wchar_p]
    set_name.restype = ctypes.c_int
    close = kernel.CloseHandle
    close.argtypes = [ctypes.c_void_p]
    close.restype = ctypes.c_int
    handle = create(str(path), 0x10000, 7, None, 3, 0x02000000, None)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        if not set_name(handle, "RECORD~1.JSN"):
            raise ctypes.WinError(ctypes.get_last_error())
    finally:
        if not close(handle):
            raise ctypes.WinError(ctypes.get_last_error())
    result = path.parent / short_path(path).name
    _checks.assertNotEqual(result.stem.casefold(), path.stem.casefold())
    _checks.assertNotEqual(result.suffix.casefold(), path.suffix.casefold())
    _checks.assertEqual(result.resolve(), path.resolve())
    return result


@pytest.mark.skipif(os.name != "nt", reason="native Windows short names")
@pytest.mark.parametrize("short_creation", [True, False])
def test_native_short_directory_and_leaf_names_preserve_record_controls(files, short_creation):
    source, mapping, _record = files
    folder = source.parent / "ordinary long records directory"
    folder.mkdir()
    alias = short_path(folder)
    record = folder / "ordinary long disclosure record.disclosure.json"
    spelled = alias / record.name
    # The original resolve-versus-abspath predicate rejects this ordinary alias.
    _checks.assertNotEqual(spelled.resolve(), Path(os.path.abspath(spelled)))
    creation = spelled if short_creation else record
    checking = record if short_creation else spelled
    _checks.assertEqual(cli.main(arguments((source, mapping, creation))), 0)
    _checks.assertEqual(cli.main(arguments((source, mapping, checking), "disclosure-check")), 0)
    leaf_alias = short_leaf(record)
    _checks.assertEqual(record.lstat().st_nlink, 1)
    selected = (source, mapping, leaf_alias)
    _checks.assertEqual(cli.main(arguments(selected, "disclosure-check")), 0)
    original = record.read_bytes()
    _checks.assertEqual(cli.main(arguments(selected)), 1)
    _checks.assertEqual(record.read_bytes(), original)
    hardlink = folder / "hardlink.disclosure.json"
    hardlink.hardlink_to(record)
    _checks.assertEqual(cli.main(arguments(selected, "disclosure-check")), 1)
    hardlink.unlink()
    # Reuse absolute executable resolution and the isolated Git environment.
    _checks.assertEqual(cli.entities_module._git(["add", "-f"], record), 0)
    _checks.assertEqual(cli.main(arguments(selected, "disclosure-check")), 1)
    _checks.assertEqual(cli.entities_module._git(["reset"], record), 0)
    (source.parent / ".gitignore").write_text("entities.json\n", encoding="utf-8")
    _checks.assertEqual(cli.main(arguments(selected, "disclosure-check")), 1)


@pytest.mark.skipif(os.name != "nt", reason="native Windows short names")
@pytest.mark.parametrize("name", ["out.md", "entities.json", "out.md.manifest.json", "out.md.triage.md"])
def test_short_parent_spelling_cannot_overwrite_protected_files(files, name):
    source, mapping, _record = files
    alias = short_path(source.parent)
    protected = source.parent / name
    original = protected.read_bytes() if protected.exists() else None
    _checks.assertEqual(cli.main(arguments((source, mapping, alias / name))), 1)
    _checks.assertEqual(protected.read_bytes() if protected.exists() else None, original)


@pytest.mark.skipif(os.name != "nt", reason="native Windows junctions")
@pytest.mark.parametrize("command", ["disclosure-record", "disclosure-check"])
def test_native_junction_is_refused(files, command):
    import _winapi

    source, mapping, record = files
    folder = source.parent / "ordinary long records directory"
    folder.mkdir()
    target_record = folder / record.name
    _checks.assertEqual(cli.main(arguments((source, mapping, target_record))), 0)
    junction = source.parent / "junction"
    _winapi.CreateJunction(str(folder), str(junction))
    try:
        _checks.assertTrue(junction.lstat().st_file_attributes & stat.FILE_ATTRIBUTE_REPARSE_POINT)
        _checks.assertEqual(cli.main(arguments((source, mapping, junction / record.name), command)), 1)
    finally:
        junction.rmdir()


@pytest.mark.skipif(os.name != "nt", reason="native Windows non-surrogate reparse point")
def test_native_non_surrogate_reparse_entry_is_refused(files):
    source, _mapping, _record = files
    point = source.parent / "non-surrogate"
    point.mkdir()
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    create = kernel.CreateFileW
    create.argtypes = [ctypes.c_wchar_p, ctypes.c_uint32, ctypes.c_uint32,
                      ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_void_p]
    create.restype = ctypes.c_void_p
    control = kernel.DeviceIoControl
    control.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_uint32,
                       ctypes.c_void_p, ctypes.c_uint32, ctypes.POINTER(ctypes.c_uint32),
                       ctypes.c_void_p]
    control.restype = ctypes.c_int
    close = kernel.CloseHandle
    close.argtypes = [ctypes.c_void_p]
    close.restype = ctypes.c_int
    attributes = kernel.GetFileAttributesW
    attributes.argtypes = [ctypes.c_wchar_p]
    attributes.restype = ctypes.c_uint32
    # Prepare buffers before acquiring a handle that needs cleanup.
    # A third-party tag with neither Microsoft nor name-surrogate bits.
    data = ctypes.create_string_buffer(struct.pack("<IHH16s", 0x42, 0, 0, uuid.uuid4().bytes_le))
    returned = ctypes.c_uint32()
    installed = False
    handle = create(str(point), 0x40000000, 7, None, 3, 0x02200000, None)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        if not control(handle, 0x900A4, data, 24, None, 0, ctypes.byref(returned), None):
            raise ctypes.WinError(ctypes.get_last_error())
        installed = True
        observed = attributes(str(point))
        if observed == 0xFFFFFFFF:
            raise ctypes.WinError(ctypes.get_last_error())
        _checks.assertTrue(observed & stat.FILE_ATTRIBUTE_REPARSE_POINT)
        with pytest.raises(EvattError, match="must not use links"):
            cli._record_path(point)
        with pytest.raises((EvattError, OSError)):
            cli._record_path(point / "out.disclosure.json")
    finally:
        try:
            if installed:
                if not control(handle, 0x900AC, data, 24, None, 0, ctypes.byref(returned), None):
                    raise ctypes.WinError(ctypes.get_last_error())
        finally:
            if not close(handle):
                raise ctypes.WinError(ctypes.get_last_error())
        point.rmdir()


@pytest.mark.skipif(os.name != "nt", reason="native Windows fixture failure paths")
@pytest.mark.parametrize("stage", ["uuid", "buffer", "set", "delete", "attributes"])
def test_native_fixture_failures_preserve_handle_cleanup(files, monkeypatch, stage):
    kernel = SimpleNamespace(
        CreateFileW=Mock(return_value=42),
        DeviceIoControl=Mock(side_effect=[0] if stage == "set" else [1, 0]
                            if stage == "delete" else [1, 1]),
        CloseHandle=Mock(return_value=1),
        GetFileAttributesW=Mock(return_value=0xFFFFFFFF if stage == "attributes" else 0x400),
    )
    monkeypatch.setattr(ctypes, "WinDLL", Mock(return_value=kernel))
    monkeypatch.setattr(ctypes, "get_last_error", lambda: 5)
    if stage == "uuid":
        monkeypatch.setattr(uuid, "uuid4", Mock(side_effect=RuntimeError("fixture preparation")))
    elif stage == "buffer":
        monkeypatch.setattr(ctypes, "create_string_buffer",
                            Mock(side_effect=RuntimeError("fixture preparation")))
    expected = RuntimeError if stage in ("uuid", "buffer") else OSError
    with pytest.raises(expected):
        test_native_non_surrogate_reparse_entry_is_refused(files)
    if stage in ("uuid", "buffer"):
        kernel.CreateFileW.assert_not_called()
        kernel.CloseHandle.assert_not_called()
    else:
        kernel.CloseHandle.assert_called_once_with(42)
        if stage == "attributes":
            kernel.GetFileAttributesW.assert_called_once()
