from __future__ import annotations

import json
import os
import shutil
import signal
import sys
from pathlib import Path

import pytest
from reviewready import publication, viewer
from reviewready.cli import _managed_publication
from reviewready.errors import GateInputError
from reviewready.report import PACK_FILE_NAMES, write_review_pack
from tests.test_gate import _not_ready_pack, _ready_pack
from tests.test_publication import assert_clean, payloads


@pytest.mark.parametrize('delivery', ['handler', 'signal'])
@pytest.mark.parametrize('cut', [
    'mkdir', 'lock', 'initial-state', 'active-state', 'prepared-state',
    'stage-0', 'stage-1', 'stage-2', 'commit-state', 'cleanup-state',
])
def test_managed_cancellation_at_control_and_payload_boundaries(
    monkeypatch, tmp_path, cut, delivery
):
    if delivery == 'signal' and sys.platform == 'win32':
        pytest.skip('Windows SIGTERM is process termination, not managed POSIX delivery')
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    previous = payloads(output)
    if cut in ('mkdir', 'lock', 'initial-state'):
        shutil.rmtree(output / '.reviewready')
    fired = False

    def cancel():
        nonlocal fired
        if fired:
            return
        fired = True
        if delivery == 'signal':
            os.kill(os.getpid(), signal.SIGTERM)
        else:
            handler = signal.getsignal(signal.SIGTERM)
            assert callable(handler)
            handler(signal.SIGTERM, None)

    mkdir, open_file, replace = Path.mkdir, os.open, os.replace

    def create(path, *args, **kwargs):
        result = mkdir(path, *args, **kwargs)
        if cut == 'mkdir' and path.name == '.reviewready':
            cancel()
        return result

    def lock(path, *args, **kwargs):
        descriptor = open_file(path, *args, **kwargs)
        if cut == 'lock' and Path(path).name == 'writer.lock':
            cancel()
        return descriptor

    def swap(source, destination):
        source = Path(source)
        if source.name == 'state.next':
            state = json.loads(source.read_bytes())
            active = state['status'] == 'in_progress'
            prepared = any(value is not None for value in state['previous'])
            initial = state['transaction'] is None and not Path(destination).exists()
            cleanup = state['transaction'] is None and Path(destination).exists()
            match = (
                (cut == 'initial-state' and initial)
                or (cut == 'active-state' and active and not prepared)
                or (cut == 'prepared-state' and active and prepared)
                or (cut == 'commit-state' and not active and state['transaction'] is not None)
                or (cut == 'cleanup-state' and cleanup)
            )
            if match:
                cancel()
        replace(source, destination)
        if source.name == cut and cut.startswith('stage-'):
            cancel()

    with monkeypatch.context() as inner:
        inner.setattr(Path, 'mkdir', create)
        inner.setattr(publication.os, 'open', lock)
        inner.setattr(publication.os, 'replace', swap)
        with _managed_publication():
            with pytest.raises(OSError, match='SIGTERM|pack published'):
                write_review_pack(_not_ready_pack(), output)
    assert fired
    committed = cut in ('commit-state', 'cleanup-state')
    assert viewer.verify_pack(output)[0]['overall_status'] == ('NOT_READY' if committed else 'READY')
    if not committed:
        assert payloads(output) == previous
    assert not (output / '.reviewready/state.next').exists()
    write_review_pack(_not_ready_pack(), output)
    assert_clean(output)


@pytest.mark.parametrize('name', PACK_FILE_NAMES)
@pytest.mark.parametrize('kind', ['symlink', 'hardlink', 'directory', 'fifo'])
def test_viewer_refuses_linked_and_special_payload_before_reading(
    monkeypatch, tmp_path, name, kind
):
    output, target = tmp_path / 'pack', tmp_path / 'target'
    write_review_pack(_ready_pack(), output)
    write_review_pack(_ready_pack(), target)
    path = output / name
    path.unlink()
    if kind == 'symlink':
        try:
            path.symlink_to(target / name)
        except OSError:
            pytest.skip('Symlink creation unavailable on this host')
    elif kind == 'hardlink':
        os.link(target / name, path)
    elif kind == 'directory':
        path.mkdir()
    elif sys.platform != 'win32':
        os.mkfifo(path)
    else:
        pytest.skip('POSIX FIFO')
    before = path.lstat()
    read = Path.read_bytes

    def forbidden(candidate):
        if candidate == path:
            pytest.fail('Rejected payload was opened')
        return read(candidate)

    monkeypatch.setattr(Path, 'read_bytes', forbidden)
    with pytest.raises(GateInputError, match='regular, single-link'):
        viewer.verify_pack(output)
    assert path.lstat() == before


def test_post_commit_cleanup_state_error_requires_review(monkeypatch, tmp_path):
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    replace = os.replace

    def fail(source, destination):
        if Path(source).name == 'state.next':
            state = json.loads(Path(source).read_bytes())
            current = json.loads(Path(destination).read_bytes())
            if state['transaction'] is None and current['transaction'] is not None:
                raise OSError('cleanup state replacement denied')
        replace(source, destination)

    with monkeypatch.context() as inner:
        inner.setattr(publication.os, 'replace', fail)
        with pytest.raises(OSError, match='pack published; cleanup state uncertain, requires review'):
            write_review_pack(_not_ready_pack(), output)
    assert viewer.verify_pack(output)[0]['overall_status'] == 'NOT_READY'
    assert (output / '.reviewready/state.next').is_file()
    with pytest.raises(OSError, match='unknown publication control'):
        write_review_pack(_ready_pack(), output)


@pytest.mark.parametrize('delivery', ['handler', 'signal'])
def test_repeated_managed_sigterm_does_not_interrupt_restoration(monkeypatch, tmp_path, delivery):
    if delivery == 'signal' and sys.platform == 'win32':
        pytest.skip('Managed POSIX signal delivery')
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    previous = payloads(output)
    replace = os.replace
    signals = []

    def cancel():
        if delivery == 'signal':
            os.kill(os.getpid(), signal.SIGTERM)
        else:
            handler = signal.getsignal(signal.SIGTERM)
            assert callable(handler)
            handler(signal.SIGTERM, None)

    def swap(source, destination):
        replace(source, destination)
        name = Path(source).name
        if name in ('stage-0', 'restore-0'):
            signals.append(name)
            cancel()

    with monkeypatch.context() as inner:
        inner.setattr(publication.os, 'replace', swap)
        with _managed_publication():
            with pytest.raises(publication.PublicationCancelled):
                write_review_pack(_not_ready_pack(), output)
    assert signals == ['stage-0', 'restore-0']
    assert payloads(output) == previous
    viewer.verify_pack(output)
    assert_clean(output)


@pytest.mark.parametrize('rollback,missing', [(False, False), (True, False), (True, True)])
def test_cleanup_state_failure_before_creation_refuses_takeover(
    monkeypatch, tmp_path, rollback, missing
):
    output = tmp_path / 'pack'
    write_review_pack(_ready_pack(), output)
    if missing:
        (output / PACK_FILE_NAMES[0]).unlink()
    previous = payloads(output)
    control = output / '.reviewready'
    write_state, replace = publication._write_state, os.replace

    def deny_cleanup(path, state):
        if state['transaction'] is None and publication._read_state(path)['transaction'] is not None:
            raise OSError('cleanup state creation denied')
        write_state(path, state)

    def cancel(source, destination):
        replace(source, destination)
        if rollback and Path(source).name == 'stage-0':
            raise publication.PublicationCancelled('managed cancellation after first replacement')

    with monkeypatch.context() as inner:
        inner.setattr(publication, '_write_state', deny_cleanup)
        inner.setattr(publication.os, 'replace', cancel)
        message = 'recovery incomplete' if rollback else 'cleanup state uncertain, requires review'
        with pytest.raises(OSError, match=message):
            write_review_pack(_not_ready_pack(), output)
    if rollback:
        assert payloads(output) == previous
    else:
        assert viewer.verify_pack(output)[0]['overall_status'] == 'NOT_READY'
    state = publication._read_state(control)
    assert state['status'] == 'settled' and state['transaction'] is not None
    assert not (control / ('tx-' + state['transaction'])).exists()
    assert not (control / 'state.next').exists()
    contents = {path.name: path.read_bytes() for path in control.iterdir()}
    retained = payloads(output)
    revision = publication.snapshot_revision(output)
    with pytest.raises(OSError, match='referenced publication transaction is missing'):
        write_review_pack(_ready_pack(), output)
    assert payloads(output) == retained
    assert publication.snapshot_revision(output) == revision
    assert {path.name: path.read_bytes() for path in control.iterdir()} == contents
