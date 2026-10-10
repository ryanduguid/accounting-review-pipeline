from __future__ import annotations

import ctypes
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import pytest
from reviewready import publication, viewer
from reviewready.cli import main
from reviewready.errors import GateInputError
from reviewready.report import PACK_FILE_NAMES, write_review_pack
from tests.test_gate import _not_ready_pack, _ready_pack

# Child interpreters import this package's tests module, so they start here
# whichever directory pytest ran from.
PACKAGE = Path(__file__).resolve().parents[1]


class Cancelled(BaseException):
    pass


def payloads(output):
    return {name: (output / name).read_bytes() for name in PACK_FILE_NAMES if (output / name).is_file()}


def assert_clean(output):
    assert sorted(path.name for path in output.iterdir()) == sorted((*PACK_FILE_NAMES, '.reviewready'))
    control = output / '.reviewready'
    assert sorted(path.name for path in control.iterdir()) == ['state.json', 'writer.lock']
    assert publication._read_state(control)['transaction'] is None


@pytest.mark.parametrize('index', [0, 1, 2])
@pytest.mark.parametrize('missing', [False, True])
def test_cancel_after_install_before_bookkeeping(monkeypatch, tmp_path, index, missing):
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    if missing:
        (output / PACK_FILE_NAMES[0]).unlink()
    previous = payloads(output)
    revision = publication.snapshot_revision(output)
    replace = os.replace

    def cancel(source, destination):
        replace(source, destination)
        if Path(source).name == f'stage-{index}':
            raise Cancelled()

    monkeypatch.setattr(publication.os, 'replace', cancel)
    with pytest.raises(Cancelled):
        write_review_pack(_not_ready_pack(), output)
    assert payloads(output) == previous
    assert publication.snapshot_revision(output) != revision
    assert sorted(path.name for path in (output / '.reviewready').iterdir()) == ['state.json', 'writer.lock']


def test_failed_rollback_retains_backups_and_refuses_retry(monkeypatch, tmp_path):
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    replace = os.replace

    def fail(source, destination):
        if Path(source).name.startswith('restore-'):
            raise OSError('restore denied')
        replace(source, destination)
        if Path(source).name == 'stage-0':
            raise Cancelled()

    monkeypatch.setattr(publication.os, 'replace', fail)
    with pytest.raises(OSError, match='recovery incomplete'):
        write_review_pack(_not_ready_pack(), output)
    state = publication._read_state(output / '.reviewready')
    transaction = output / '.reviewready' / ('tx-' + state['transaction'])
    assert (transaction / 'backup-0').is_file()
    with pytest.raises(GateInputError, match='unresolved'):
        viewer.verify_pack(output)
    with pytest.raises(OSError, match='unresolved'):
        write_review_pack(_ready_pack(), output)
    assert (transaction / 'backup-0').is_file()


@pytest.mark.parametrize('rollback', [False, True])
def test_viewer_detects_a_whole_intervening_transaction(monkeypatch, tmp_path, rollback):
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    load = viewer._load_artefact_bytes
    replace = os.replace

    def cancel(source, destination):
        replace(source, destination)
        if Path(source).name == 'stage-0':
            raise Cancelled()

    def interleave(directory):
        captured = load(directory)
        if rollback:
            with monkeypatch.context() as inner:
                inner.setattr(publication.os, 'replace', cancel)
                with pytest.raises(Cancelled):
                    write_review_pack(_not_ready_pack(), output)
        else:
            write_review_pack(_ready_pack(), output)
        return captured

    monkeypatch.setattr(viewer, '_load_artefact_bytes', interleave)
    with pytest.raises(GateInputError, match='changed during acquisition'):
        viewer.verify_pack(output)


def test_legacy_viewer_is_read_only(tmp_path):
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    legacy = tmp_path / 'legacy'
    legacy.mkdir()
    for name in PACK_FILE_NAMES:
        shutil.copyfile(output / name, legacy / name)
    previous = payloads(legacy)
    viewer.verify_pack(legacy)
    assert payloads(legacy) == previous
    assert sorted(path.name for path in legacy.iterdir()) == sorted(PACK_FILE_NAMES)


def test_second_process_cannot_publish_or_clean_live_writer(tmp_path):
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    previous = payloads(output)
    with publication._admission(output / '.reviewready'):
        result = subprocess.run([
            sys.executable, '-c',
            'from pathlib import Path;from reviewready.report import write_review_pack;'
            'from tests.test_gate import _not_ready_pack;import sys;'
            'write_review_pack(_not_ready_pack(),Path(sys.argv[1]))', str(output),
        ], capture_output=True, text=True, timeout=10, cwd=PACKAGE)
    assert result.returncode != 0
    assert 'holds admission' in result.stderr
    assert payloads(output) == previous
    assert_clean(output)


@pytest.mark.parametrize('boundary', ['stage-0', 'stage-1', 'stage-2', 'commit'])
def test_process_death_refuses_or_recognises_committed_generation(tmp_path, boundary):
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    code = '''
import os,sys
from pathlib import Path
from reviewready import publication
from reviewready.report import write_review_pack
from tests.test_gate import _not_ready_pack
boundary=sys.argv[2]
replace=os.replace
write=publication._write_state
def cut_replace(source,destination):
    replace(source,destination)
    if Path(source).name==boundary: os._exit(73)
def cut_state(control,state):
    write(control,state)
    if boundary=='commit' and state['status']=='settled' and state['transaction'] is not None:
        os._exit(73)
publication.os.replace=cut_replace
publication._write_state=cut_state
write_review_pack(_not_ready_pack(),Path(sys.argv[1]))
'''
    result = subprocess.run([sys.executable, '-c', code, str(output), boundary], timeout=10, cwd=PACKAGE)
    assert result.returncode == 73
    if boundary == 'commit':
        assert viewer.verify_pack(output)[0]['overall_status'] == 'NOT_READY'
        write_review_pack(_not_ready_pack(), output)
        assert_clean(output)
    else:
        before = {path.relative_to(output): path.read_bytes() for path in output.rglob('*') if path.is_file()}
        with pytest.raises(GateInputError, match='unresolved'):
            viewer.verify_pack(output)
        with pytest.raises(OSError, match='unresolved'):
            write_review_pack(_ready_pack(), output)
        assert before == {path.relative_to(output): path.read_bytes() for path in output.rglob('*') if path.is_file()}


@pytest.mark.parametrize('damage', ['json', 'duplicate', 'escape', 'unknown'])
def test_damaged_control_preserves_payload_and_foreign_entries(tmp_path, damage):
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    previous = payloads(output)
    control = output / '.reviewready'
    state_path = control / 'state.json'
    if damage == 'json':
        state_path.write_bytes(b'{')
    elif damage == 'duplicate':
        state_path.write_bytes(b'{"format":1,"format":1}')
    elif damage == 'escape':
        state = publication._read_state(control)
        state['transaction'] = '../../foreign'
        state_path.write_text(json.dumps(state), encoding='utf-8')
    else:
        (control / 'foreign.partial').write_bytes(b'keep')
    with pytest.raises(OSError):
        write_review_pack(_not_ready_pack(), output)
    assert payloads(output) == previous
    if damage == 'unknown':
        assert (control / 'foreign.partial').read_bytes() == b'keep'


def test_unrelated_partials_survive_success_and_retry(tmp_path):
    output = tmp_path / 'pack'
    output.mkdir()
    foreign = output / 'readiness-pack.json.aaaaaaaaaaaa.partial'
    foreign.write_bytes(b'earlier evidence')
    write_review_pack(_ready_pack(), output)
    write_review_pack(_not_ready_pack(), output)
    assert foreign.read_bytes() == b'earlier evidence'


def test_post_commit_cleanup_failure_is_retryable(monkeypatch, tmp_path):
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    unlink = Path.unlink

    def denied(path, *args, **kwargs):
        if path.name == 'backup-0':
            raise OSError('cleanup denied')
        return unlink(path, *args, **kwargs)

    with monkeypatch.context() as inner:
        inner.setattr(Path, 'unlink', denied)
        with pytest.raises(OSError, match='pack published; cleanup incomplete, inspect retained control before retry'):
            write_review_pack(_not_ready_pack(), output)
    assert viewer.verify_pack(output)[0]['overall_status'] == 'NOT_READY'
    write_review_pack(_not_ready_pack(), output)
    assert_clean(output)


@pytest.mark.skipif(sys.platform != 'win32', reason='Windows sharing handle')
def test_real_windows_handle_refuses_replace_without_losing_previous_pack(tmp_path):
    from ctypes import wintypes

    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    previous = payloads(output)
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    create = kernel.CreateFileW
    create.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p,
                       wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    create.restype = wintypes.HANDLE
    close = kernel.CloseHandle
    close.argtypes = [wintypes.HANDLE]
    close.restype = wintypes.BOOL
    handle = create(str(output / PACK_FILE_NAMES[0]), 0x80000000, 3, None, 3, 0x80, None)
    assert handle != ctypes.c_void_p(-1).value
    try:
        with pytest.raises(OSError):
            write_review_pack(_not_ready_pack(), output)
        assert payloads(output) == previous
        viewer.verify_pack(output)
    finally:
        assert close(handle)
    assert_clean(output)


def test_cli_refuses_control_input_before_opening_it(tmp_path, capsys):
    output = tmp_path / 'pack'
    assert main(['gate', '--profile', 'bas', '--pack', str(output / '.reviewready'),
                 '--output', str(output)]) == 1
    assert 'overlaps publication control' in capsys.readouterr().err
    assert not output.exists()


def test_commit_acknowledgement_interruption_does_not_roll_back(monkeypatch, tmp_path):
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    write = publication._write_state

    def interrupt(control, state):
        write(control, state)
        if state['status'] == 'settled' and state['transaction'] is not None:
            raise Cancelled()

    with monkeypatch.context() as inner:
        inner.setattr(publication, '_write_state', interrupt)
        with pytest.raises(OSError, match='pack published; cleanup incomplete'):
            write_review_pack(_not_ready_pack(), output)
    assert viewer.verify_pack(output)[0]['overall_status'] == 'NOT_READY'
    write_review_pack(_not_ready_pack(), output)
    assert_clean(output)


def test_cancellation_after_active_marker_restores_settled_state(monkeypatch, tmp_path):
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    previous = payloads(output)
    revision = publication.snapshot_revision(output)
    write = publication._write_state

    def interrupt(control, state):
        write(control, state)
        if state['status'] == 'in_progress':
            raise Cancelled()

    monkeypatch.setattr(publication, '_write_state', interrupt)
    with pytest.raises(Cancelled):
        write_review_pack(_not_ready_pack(), output)
    assert payloads(output) == previous
    assert publication.snapshot_revision(output) != revision
    assert_clean(output)


def test_state_replace_failure_retains_evidence_and_refuses_retry(monkeypatch, tmp_path):
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    previous = payloads(output)
    replace = os.replace

    def denied(source, destination):
        if Path(source).name == 'state.next':
            raise OSError('state replacement denied')
        replace(source, destination)

    with monkeypatch.context() as inner:
        inner.setattr(publication.os, 'replace', denied)
        with pytest.raises(OSError):
            write_review_pack(_not_ready_pack(), output)
    assert payloads(output) == previous
    assert (output / '.reviewready/state.next').is_file()
    with pytest.raises(OSError, match='unknown publication control'):
        write_review_pack(_not_ready_pack(), output)


def test_unknown_transaction_entry_is_preserved_on_retry(monkeypatch, tmp_path):
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    cleanup = publication._cleanup

    def foreign(control, state):
        if state['transaction'] is not None:
            (control / ('tx-' + state['transaction']) / 'foreign.txt').write_bytes(b'keep')
        cleanup(control, state)

    with monkeypatch.context() as inner:
        inner.setattr(publication, '_cleanup', foreign)
        with pytest.raises(OSError, match='pack published; cleanup incomplete, inspect retained control before retry'):
            write_review_pack(_not_ready_pack(), output)
    state = publication._read_state(output / '.reviewready')
    foreign_path = output / '.reviewready' / ('tx-' + state['transaction']) / 'foreign.txt'
    with pytest.raises(OSError, match='unknown transaction'):
        write_review_pack(_ready_pack(), output)
    assert foreign_path.read_bytes() == b'keep'
    assert viewer.verify_pack(output)[0]['overall_status'] == 'NOT_READY'


def test_cli_signal_adapter_restores_handler_and_embedded_policy():
    import signal

    from reviewready.cli import _managed_publication

    previous = signal.getsignal(signal.SIGTERM)
    try:
        signal.signal(signal.SIGTERM, signal.SIG_DFL)
        with pytest.raises(OSError, match='SIGTERM'):
            with _managed_publication():
                handler = signal.getsignal(signal.SIGTERM)
                assert callable(handler)
                handler(signal.SIGTERM, None)
                publication._check_cancellation()
        assert signal.getsignal(signal.SIGTERM) == signal.SIG_DFL
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        with _managed_publication():
            assert signal.getsignal(signal.SIGTERM) == signal.SIG_IGN
    finally:
        signal.signal(signal.SIGTERM, previous)
