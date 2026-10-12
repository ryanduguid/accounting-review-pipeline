"""Edge paths of the token cache that one platform alone never reaches.

The cache code branches on the platform: DPAPI envelopes on Windows, an
owner-only plaintext file elsewhere, msvcrt byte locks on Windows and flock
elsewhere. CI measures coverage on both, so each side is driven here on every
platform by faking only the platform boundary (``_is_windows``, the DPAPI
wrappers, ``ctypes.WinDLL`` and the lock modules). The real DPAPI round trip
stays in test_dpapi_win32.py and the Windows-only cases in test_xero_client.py.
"""

import base64
import ctypes
import errno
import json
import os
import stat
import sys
import tempfile
import types
import unittest
from contextlib import ExitStack, contextmanager
from datetime import datetime, timezone
from unittest import mock

import xero_client

_FAKE_PREFIX = b"FAKE-DPAPI:"


def _fake_protect(data: bytes) -> bytes:
    """Reversible stand-in for CryptProtectData: never returns the plaintext."""
    return _FAKE_PREFIX + bytes(byte ^ 0x5A for byte in data)


def _fake_unprotect(data: bytes) -> bytes:
    if not data.startswith(_FAKE_PREFIX):
        raise AssertionError("fake DPAPI asked to open bytes it never protected")
    return bytes(byte ^ 0x5A for byte in data[len(_FAKE_PREFIX):])


@contextmanager
def _fake_windows():
    """Run the Windows cache path with a fake DPAPI on any platform."""
    with ExitStack() as stack:
        stack.enter_context(mock.patch.object(xero_client, "_is_windows", return_value=True))
        stack.enter_context(
            mock.patch.object(xero_client, "_dpapi_protect", side_effect=_fake_protect)
        )
        stack.enter_context(
            mock.patch.object(xero_client, "_dpapi_unprotect", side_effect=_fake_unprotect)
        )
        yield


def _envelope(payload: str) -> bytes:
    return json.dumps(
        {
            "format": xero_client.TOKEN_CACHE_FORMAT,
            "version": xero_client.TOKEN_CACHE_VERSION,
            "protection": xero_client.TOKEN_CACHE_PROTECTION,
            "payload": payload,
        }
    ).encode("utf-8")


class _FakeCryptFunction:
    """Stands in for crypt32's CryptProtectData or CryptUnprotectData."""

    def __init__(self, transform, *, succeed=True, fill=True):
        self.transform = transform
        self.succeed = succeed
        self.fill = fill
        self.calls = []
        self.buffers = []

    def __call__(self, data_in, description, entropy, reserved, prompt, flags, data_out):
        blob_in = data_in._obj
        plaintext = ctypes.string_at(blob_in.pbData, blob_in.cbData)
        self.calls.append((plaintext, description, entropy, reserved, prompt, flags))
        if not self.succeed:
            return 0
        if self.fill:
            result = self.transform(plaintext)
            buffer = (ctypes.c_ubyte * len(result)).from_buffer_copy(result)
            self.buffers.append(buffer)
            blob_out = data_out._obj
            blob_out.cbData = len(result)
            blob_out.pbData = ctypes.cast(buffer, ctypes.POINTER(ctypes.c_ubyte))
        return 1


class _FakeLocalFree:
    def __init__(self):
        self.freed = []

    def __call__(self, pointer):
        self.freed.append(pointer.value)


class WindowsDpapiWrapperTest(unittest.TestCase):
    """_windows_dpapi against a fake crypt32/kernel32, on every platform."""

    def _run(self, function_name, data, crypt_function, last_error=0):
        local_free = _FakeLocalFree()
        libraries = {
            "crypt32": types.SimpleNamespace(**{function_name: crypt_function}),
            "kernel32": types.SimpleNamespace(LocalFree=local_free),
        }
        opened = []

        def win_dll(name, use_last_error=False):
            opened.append((name, use_last_error))
            return libraries[name]

        with mock.patch.object(xero_client, "_is_windows", return_value=True), \
                mock.patch.object(xero_client.ctypes, "WinDLL", win_dll, create=True), \
                mock.patch.object(
                    xero_client.ctypes, "get_last_error", return_value=last_error, create=True
                ):
            wrapper = (
                xero_client._dpapi_protect
                if function_name == "CryptProtectData"
                else xero_client._dpapi_unprotect
            )
            result = wrapper(data)
        return result, local_free, opened

    def test_protect_passes_the_bytes_without_ui_and_frees_the_output(self):
        crypt = _FakeCryptFunction(_fake_protect)
        result, local_free, opened = self._run("CryptProtectData", b"plain \x00 bytes", crypt)

        self.assertEqual(result, _fake_protect(b"plain \x00 bytes"))
        self.assertEqual(
            crypt.calls,
            [(b"plain \x00 bytes", None, None, None, None, xero_client.CRYPTPROTECT_UI_FORBIDDEN)],
        )
        self.assertEqual(opened, [("crypt32", True), ("kernel32", True)])
        self.assertEqual(local_free.freed, [ctypes.addressof(crypt.buffers[0])])

    def test_unprotect_returns_the_original_bytes(self):
        crypt = _FakeCryptFunction(_fake_unprotect)
        result, local_free, _ = self._run(
            "CryptUnprotectData", _fake_protect(b"round trip"), crypt
        )
        self.assertEqual(result, b"round trip")
        self.assertEqual(len(local_free.freed), 1)

    def test_a_null_output_buffer_is_not_freed(self):
        crypt = _FakeCryptFunction(_fake_protect, fill=False)
        result, local_free, _ = self._run("CryptProtectData", b"x", crypt)
        self.assertEqual(result, b"")
        self.assertEqual(local_free.freed, [])

    def test_a_failed_unprotect_names_the_corrupt_cache(self):
        crypt = _FakeCryptFunction(_fake_unprotect, succeed=False)
        with self.assertRaises(SystemExit) as raised:
            self._run("CryptUnprotectData", b"garbage", crypt, last_error=13)
        message = str(raised.exception)
        self.assertIn("corrupt or cannot be opened by this Windows user", message)
        self.assertNotIn("13", message)

    def test_a_failed_protect_reports_the_windows_error_and_saves_nothing(self):
        crypt = _FakeCryptFunction(_fake_protect, succeed=False)
        with self.assertRaises(SystemExit) as raised:
            self._run("CryptProtectData", b"secret", crypt, last_error=87)
        self.assertEqual(
            str(raised.exception),
            "error: Windows DPAPI could not protect token.json (Windows error 87); "
            "no token was saved.",
        )

    def test_dpapi_is_refused_off_windows_without_touching_ctypes(self):
        with mock.patch.object(xero_client, "_is_windows", return_value=False), \
                mock.patch.object(
                    xero_client.ctypes, "create_string_buffer",
                    side_effect=AssertionError("reached ctypes"),
                ), \
                self.assertRaises(SystemExit) as raised:
            xero_client._dpapi_protect(b"x")
        self.assertIn("Windows DPAPI is unavailable on this platform", str(raised.exception))


class TokenCacheCodecTest(unittest.TestCase):
    def test_duplicate_keys_and_non_utf8_bytes_are_refused_as_corrupt(self):
        for raw in (b'{"access_token": "A", "access_token": "B"}', b"\xff\xfe{}"):
            with self.subTest(raw=raw), self.assertRaises(SystemExit) as raised:
                xero_client._decode_token_cache(raw)
            self.assertIn("token.json is unreadable or corrupt", str(raised.exception))

    def test_the_posix_fallback_writes_indented_plaintext(self):
        tokens = {"access_token": "A", "refresh_token": "R"}
        with mock.patch.object(xero_client, "_is_windows", return_value=False):
            encoded = xero_client._encode_token_cache(tokens)
        self.assertEqual(encoded, (json.dumps(tokens, indent=2) + "\n").encode("utf-8"))
        self.assertEqual(xero_client._decode_token_cache(encoded), (tokens, True))

    def test_the_windows_envelope_holds_only_ciphertext(self):
        tokens = {"access_token": "SYNTHETIC-A-3381", "refresh_token": "SYNTHETIC-R-1197"}
        with _fake_windows():
            encoded = xero_client._encode_token_cache(tokens)
            decoded = xero_client._decode_token_cache(encoded)
        envelope = json.loads(encoded)
        self.assertEqual(set(envelope), xero_client.TOKEN_CACHE_ENVELOPE_KEYS)
        self.assertNotIn(b"SYNTHETIC", encoded)
        self.assertEqual(decoded, (tokens, False))

    def test_bad_windows_payloads_fail_closed_with_their_own_message(self):
        cases = (
            ("not base64!", "has a corrupt DPAPI payload"),
            ("", "has an empty DPAPI payload"),
            (
                base64.b64encode(_fake_protect(b"[1, 2]")).decode("ascii"),
                "decrypted payload is not a JSON object",
            ),
            (
                base64.b64encode(_fake_protect(b"\xff")).decode("ascii"),
                "token.json's decrypted payload is unreadable or corrupt",
            ),
        )
        for payload, expected in cases:
            with self.subTest(payload=payload), _fake_windows(), \
                    self.assertRaises(SystemExit) as raised:
                xero_client._decode_token_cache(_envelope(payload))
            self.assertIn(expected, str(raised.exception))


class OwnerOnlyModeTest(unittest.TestCase):
    def test_the_plaintext_fallback_is_restricted_to_mode_0600(self):
        with mock.patch.object(xero_client, "_is_windows", return_value=False), \
                mock.patch.object(xero_client.os, "chmod") as chmod:
            xero_client._enforce_owner_only("cache-path")
        chmod.assert_called_once_with("cache-path", stat.S_IRUSR | stat.S_IWUSR)

    def test_a_mode_that_cannot_be_set_stops_the_cache_being_used(self):
        for error in (OSError(1, "Operation not permitted"), NotImplementedError("no chmod")):
            with self.subTest(error=type(error).__name__), \
                    mock.patch.object(xero_client, "_is_windows", return_value=False), \
                    mock.patch.object(xero_client.os, "chmod", side_effect=error), \
                    self.assertRaises(SystemExit) as raised:
                xero_client._enforce_owner_only("cache-path")
            message = str(raised.exception)
            self.assertIn("could not restrict plaintext token cache cache-path", message)
            self.assertIn("the cache was not used", message)

    def test_windows_leaves_the_mode_alone(self):
        with mock.patch.object(xero_client, "_is_windows", return_value=True), \
                mock.patch.object(xero_client.os, "chmod") as chmod:
            xero_client._enforce_owner_only("cache-path")
        chmod.assert_not_called()


class TokenLockPlatformTest(unittest.TestCase):
    """Both lock implementations, each behind a fake of its OS module."""

    def setUp(self):
        self.lock_file = tempfile.TemporaryFile()
        self.addCleanup(self.lock_file.close)

    def _posix(self, flock):
        fcntl = types.SimpleNamespace(LOCK_EX=2, LOCK_NB=4, LOCK_UN=8, flock=flock)
        stack = ExitStack()
        stack.enter_context(mock.patch.object(xero_client.sys, "platform", "linux"))
        stack.enter_context(mock.patch.dict(sys.modules, {"fcntl": fcntl}))
        return stack

    def _windows(self, locking):
        msvcrt = types.SimpleNamespace(LK_NBLCK=2, LK_UNLCK=0, locking=locking)
        stack = ExitStack()
        stack.enter_context(mock.patch.object(xero_client.sys, "platform", "win32"))
        stack.enter_context(mock.patch.dict(sys.modules, {"msvcrt": msvcrt}))
        return stack

    def test_flock_takes_and_releases_an_exclusive_non_blocking_lock(self):
        flock = mock.Mock()
        with self._posix(flock):
            self.assertTrue(xero_client._try_token_lock(self.lock_file))
            xero_client._release_token_lock(self.lock_file)
        fd = self.lock_file.fileno()
        self.assertEqual(flock.call_args_list, [mock.call(fd, 2 | 4), mock.call(fd, 8)])

    def test_msvcrt_locks_and_unlocks_byte_zero(self):
        locking = mock.Mock()
        self.lock_file.write(b"abc")
        with self._windows(locking):
            self.assertTrue(xero_client._try_token_lock(self.lock_file))
            xero_client._release_token_lock(self.lock_file)
        fd = self.lock_file.fileno()
        self.assertEqual(locking.call_args_list, [mock.call(fd, 2, 1), mock.call(fd, 0, 1)])
        self.assertEqual(self.lock_file.tell(), 0)

    def test_a_held_lock_reads_as_busy_and_any_other_error_is_raised(self):
        for platform in ("posix", "windows"):
            for code in (errno.EACCES, errno.EAGAIN, errno.EDEADLK):
                with self.subTest(platform=platform, errno=code):
                    fake = mock.Mock(side_effect=OSError(code, "held"))
                    with getattr(self, "_" + platform)(fake):
                        self.assertFalse(xero_client._try_token_lock(self.lock_file))
            with self.subTest(platform=platform, errno=errno.EBADF):
                fake = mock.Mock(side_effect=OSError(errno.EBADF, "bad descriptor"))
                with getattr(self, "_" + platform)(fake), self.assertRaises(OSError) as raised:
                    xero_client._try_token_lock(self.lock_file)
                self.assertEqual(raised.exception.errno, errno.EBADF)


class TokenSessionPathsTest(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp_dir.cleanup)
        self.token_file = os.path.join(os.path.realpath(self.temp_dir.name), "token.json")
        patcher = mock.patch.object(xero_client, "TOKEN_FILE", self.token_file)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.session = xero_client.TokenSession(self.token_file)
        self.tokens = {
            "access_token": "SYNTHETIC-ACCESS-5170",
            "refresh_token": "SYNTHETIC-REFRESH-6624",
            "expires_in": 1800,
            "obtained_at": 1_000.0,
        }

    def _raw(self, path=None):
        with open(path or self.token_file, "rb") as source:
            return source.read()

    def _temps(self):
        return [name for name in os.listdir(self.temp_dir.name) if name.endswith(".tmp")]

    def _write_plaintext(self):
        raw = json.dumps(self.tokens).encode("utf-8")
        with open(self.token_file, "wb") as destination:
            destination.write(raw)
        return raw

    def test_a_path_outside_every_allowed_root_is_refused(self):
        with mock.patch.object(xero_client, "allowed_token_roots", return_value=()), \
                self.assertRaises(SystemExit) as raised:
            xero_client.TokenSession(self.token_file)
        self.assertIn("token cache path must stay under the home directory", str(raised.exception))

    def test_a_lock_that_cannot_be_opened_stops_before_the_cache_is_read(self):
        with mock.patch.object(
            xero_client.os, "makedirs", side_effect=PermissionError(13, "Access is denied")
        ), self.assertRaises(SystemExit) as raised:
            self.session.load()
        message = str(raised.exception)
        self.assertIn("could not open the token cache lock", message)
        self.assertIn(self.token_file + ".lock", message)
        self.assertIn("token.json was not read or changed", message)

    def test_a_busy_lock_is_polled_until_it_frees(self):
        with mock.patch.object(xero_client, "_try_token_lock", side_effect=[False, False, True]), \
                mock.patch.object(xero_client, "_release_token_lock") as release, \
                mock.patch.object(xero_client.time, "sleep") as sleep:
            with self.session._locked():
                pass
        self.assertEqual(
            sleep.call_args_list,
            [mock.call(xero_client.TOKEN_LOCK_POLL), mock.call(xero_client.TOKEN_LOCK_POLL)],
        )
        release.assert_called_once()

    def test_a_missing_cache_points_at_auth(self):
        with self.assertRaises(SystemExit) as raised:
            self.session.load()
        self.assertEqual(str(raised.exception), "No token.json - run: python auth.py")

    def test_an_unreadable_cache_is_left_unchanged(self):
        before = self._write_plaintext()
        with _fake_windows(), \
                mock.patch.object(
                    xero_client, "open", side_effect=PermissionError(13, "Access is denied"),
                    create=True,
                ), \
                self.assertRaises(SystemExit) as raised:
            self.session._load_unlocked()
        message = str(raised.exception)
        self.assertIn("token.json could not be read", message)
        self.assertIn("Access is denied", message)
        self.assertEqual(self._raw(), before)

    def test_the_posix_read_tightens_the_mode_before_reading(self):
        self._write_plaintext()
        with mock.patch.object(xero_client, "_is_windows", return_value=False), \
                mock.patch.object(xero_client.os, "chmod") as chmod:
            loaded = self.session.load()
        self.assertEqual(loaded, self.tokens)
        chmod.assert_called_once_with(self.token_file, stat.S_IRUSR | stat.S_IWUSR)

    def test_a_fresh_cached_token_is_returned_without_a_refresh(self):
        self._write_plaintext()
        refresh = mock.Mock(side_effect=AssertionError("refreshed a fresh token"))
        with mock.patch.object(xero_client, "_is_windows", return_value=False), \
                mock.patch.object(xero_client.os, "chmod"), \
                mock.patch.object(xero_client.time, "time", return_value=1_100.0):
            self.assertEqual(self.session.access_token(refresh), self.tokens["access_token"])
        refresh.assert_not_called()

    def test_the_encrypted_cache_round_trips_through_the_session(self):
        with _fake_windows(), mock.patch.object(xero_client.time, "time", return_value=1_000.0):
            self.session.save({k: v for k, v in self.tokens.items() if k != "obtained_at"})
            raw = self._raw()
            loaded = self.session.load()
        self.assertNotIn(self.tokens["refresh_token"].encode(), raw)
        self.assertEqual(json.loads(raw)["protection"], xero_client.TOKEN_CACHE_PROTECTION)
        self.assertEqual(loaded, self.tokens)

    def test_a_legacy_plaintext_cache_migrates_before_any_request(self):
        legacy_raw = self._write_plaintext()
        with _fake_windows(), \
                mock.patch.object(xero_client.time, "time", return_value=1_100.0), \
                mock.patch.object(xero_client.requests, "post") as post:
            access_token = xero_client.get_access_token("client", "secret")
            migrated = self._raw()
            decoded = xero_client._decode_token_cache(migrated)
        self.assertEqual(access_token, self.tokens["access_token"])
        post.assert_not_called()
        self.assertNotEqual(migrated, legacy_raw)
        self.assertNotIn(self.tokens["refresh_token"].encode(), migrated)
        # Migration keeps obtained_at, so an old access token never looks new.
        self.assertEqual(decoded, (self.tokens, False))

    def test_a_legacy_migration_that_cannot_flush_keeps_the_legacy_cache(self):
        legacy_raw = self._write_plaintext()
        with _fake_windows(), \
                mock.patch.object(xero_client.os, "fsync", side_effect=OSError(5, "I/O error")), \
                mock.patch.object(xero_client.requests, "post") as post, \
                self.assertRaises(SystemExit) as raised:
            xero_client.get_access_token("client", "secret")
        message = str(raised.exception)
        post.assert_not_called()
        self.assertIn("wrote an encrypted replacement for the legacy token cache", message)
        self.assertIn("could not flush it to disk (", message)
        self.assertIn("no Xero request was made", message)
        self.assertEqual(self._raw(), legacy_raw)
        self.assertEqual(len(self._temps()), 1)
        self.assertIn(self._temps()[0], message)

    def test_a_legacy_migration_that_cannot_replace_keeps_the_legacy_cache(self):
        legacy_raw = self._write_plaintext()
        with _fake_windows(), \
                mock.patch.object(
                    xero_client.os, "replace", side_effect=PermissionError(32, "held open")
                ), \
                mock.patch.object(xero_client.time, "sleep"), \
                self.assertRaises(SystemExit) as raised:
            self.session.load()
        message = str(raised.exception)
        self.assertIn(
            f"after {xero_client.REPLACE_ATTEMPTS} attempts", message
        )
        self.assertIn("The legacy cache was left unchanged and no Xero request was made", message)
        self.assertEqual(self._raw(), legacy_raw)
        self.assertEqual(len(self._temps()), 1)

    def test_a_temp_file_that_cannot_be_restricted_is_closed_and_removed(self):
        opened = []
        real_mkstemp = tempfile.mkstemp

        def mkstemp(*args, **kwargs):
            fd, path = real_mkstemp(*args, **kwargs)
            opened.append(fd)
            return fd, path

        with mock.patch.object(xero_client, "_is_windows", return_value=False), \
                mock.patch.object(xero_client.tempfile, "mkstemp", side_effect=mkstemp), \
                mock.patch.object(xero_client.os, "chmod", side_effect=OSError(1, "denied")), \
                self.assertRaises(SystemExit) as raised:
            self.session._persist_unlocked(self.tokens, legacy_migration=False)
        self.assertIn("could not restrict plaintext token cache", str(raised.exception))
        self.assertEqual(self._temps(), [])
        self.assertFalse(os.path.exists(self.token_file))
        with self.assertRaises(OSError):
            os.fstat(opened[0])

    def test_a_restriction_failure_tolerates_a_temp_file_already_gone(self):
        with mock.patch.object(xero_client, "_is_windows", return_value=False), \
                mock.patch.object(xero_client.os, "chmod", side_effect=OSError(1, "denied")), \
                mock.patch.object(xero_client.os.path, "exists", return_value=False), \
                mock.patch.object(xero_client.os, "unlink") as unlink, \
                self.assertRaises(SystemExit) as raised:
            self.session._persist_unlocked(self.tokens, legacy_migration=False)
        self.assertIn("could not restrict plaintext token cache", str(raised.exception))
        unlink.assert_not_called()

    def test_a_short_write_is_an_error_and_leaves_no_temp(self):
        real_fdopen = os.fdopen

        class ShortWriter:
            def __init__(self, fd, mode):
                self._file = real_fdopen(fd, mode)

            def __enter__(self):
                return self

            def __exit__(self, *args):
                self._file.close()

            def write(self, value):
                return self._file.write(value[:-1])

        with mock.patch.object(xero_client.os, "fdopen", side_effect=ShortWriter), \
                self.assertRaises(OSError) as raised:
            self.session._persist_unlocked(self.tokens, legacy_migration=False)
        self.assertEqual(str(raised.exception), "short write while saving token cache")
        self.assertEqual(self._temps(), [])
        self.assertFalse(os.path.exists(self.token_file))


class LoadDotenvParsingTest(unittest.TestCase):
    def test_comments_blank_lines_quotes_and_empty_keys(self):
        body = "\n".join(
            [
                "# a comment line",
                "",
                "NO_EQUALS_SIGN_HERE",
                'XTB_PATHS_DOUBLE="double quoted"',
                "XTB_PATHS_SINGLE='single quoted'",
                "XTB_PATHS_MIXED=\"mixed'",
                'XTB_PATHS_LONE="',
                "=orphan value",
                "  XTB_PATHS_PLAIN = plain value  ",
            ]
        )
        with tempfile.TemporaryDirectory() as temp_dir:
            env_path = os.path.join(temp_dir, ".env")
            with open(env_path, "w", encoding="utf-8") as fh:
                fh.write(body)
            with mock.patch.dict(os.environ, {}, clear=True):
                xero_client.load_dotenv(env_path)
                loaded = dict(os.environ)
        self.assertEqual(
            loaded,
            {
                "XTB_PATHS_DOUBLE": "double quoted",
                "XTB_PATHS_SINGLE": "single quoted",
                "XTB_PATHS_MIXED": "\"mixed'",
                "XTB_PATHS_LONE": '"',
                "XTB_PATHS_PLAIN": "plain value",
            },
        )


class SmallHelpersTest(unittest.TestCase):
    def test_an_http_date_without_a_zone_is_read_as_utc(self):
        now = datetime(2026, 6, 30, 8, 0, 0, tzinfo=timezone.utc)
        self.assertEqual(
            xero_client.retry_after_seconds("Tue, 30 Jun 2026 08:00:42 -0000", now=now), 42
        )

    def test_get_connections_returns_the_list_and_refuses_anything_else(self):
        tenants = [{"tenantId": "t-1", "tenantName": "Synthetic Org"}]
        with mock.patch.object(xero_client, "api_get", return_value=tenants) as api_get:
            self.assertIs(xero_client.get_connections(("id", "secret")), tenants)
        api_get.assert_called_once_with(xero_client.CONNECTIONS_URL, ("id", "secret"))

        with mock.patch.object(xero_client, "api_get", return_value={"tenants": tenants}), \
                self.assertRaises(SystemExit) as raised:
            xero_client.get_connections(("id", "secret"))
        self.assertEqual(str(raised.exception), "error: Xero connections response is not a list.")


if __name__ == "__main__":
    unittest.main()
