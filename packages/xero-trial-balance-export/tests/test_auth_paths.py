"""auth.py refusals and callback-handler edges the main suites leave out."""

import io
import os
import socket
import threading
import types
import unittest
from contextlib import redirect_stdout
from unittest import mock

import auth
import xero_client


def _free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


class _TokenResponse:
    status_code = 200
    text = ""

    def __init__(self, payload):
        self._payload = payload

    def raise_for_status(self):
        return None

    def json(self):
        return self._payload


class CallbackConfigPortTest(unittest.TestCase):
    def test_an_out_of_range_port_is_a_configuration_error(self):
        with self.assertRaises(ValueError) as raised:
            auth.callback_server_config("http://localhost:99999/callback")
        self.assertEqual(str(raised.exception), "XERO_REDIRECT_URI has an invalid port")
        self.assertIsInstance(raised.exception.__cause__, ValueError)


class ExclusiveBindTest(unittest.TestCase):
    """SO_EXCLUSIVEADDRUSE is set on Windows only, and only where it exists."""

    def _bind(self, os_name, exclusive):
        server = object.__new__(auth._CallbackServer)
        server.socket = mock.Mock()
        fake_socket = types.SimpleNamespace(SOL_SOCKET=0xFFFF)
        if exclusive is not None:
            fake_socket.SO_EXCLUSIVEADDRUSE = exclusive
        with mock.patch.object(auth, "os", types.SimpleNamespace(name=os_name)), \
                mock.patch.object(auth, "socket", fake_socket), \
                mock.patch.object(auth.HTTPServer, "server_bind") as base_bind:
            server.server_bind()
        base_bind.assert_called_once_with()
        return server.socket.setsockopt

    def test_windows_claims_the_port_exclusively_before_binding(self):
        setsockopt = self._bind("nt", -5)
        setsockopt.assert_called_once_with(0xFFFF, -5, 1)

    def test_other_platforms_and_missing_options_bind_plainly(self):
        for os_name, exclusive in (("posix", -5), ("nt", None)):
            with self.subTest(os_name=os_name, exclusive=exclusive):
                self._bind(os_name, exclusive).assert_not_called()


class HandlerTeardownTest(unittest.TestCase):
    def _handler(self):
        return object.__new__(auth._CallbackHandler)

    def test_an_expiry_on_a_closed_connection_is_ignored(self):
        handler = self._handler()
        handler.connection = mock.Mock()
        handler.connection.shutdown.side_effect = OSError(9, "Bad file descriptor")
        self.assertIsNone(handler._expire())
        handler.connection.shutdown.assert_called_once_with(socket.SHUT_RDWR)

    def test_finish_without_a_timer_swallows_a_closing_error(self):
        handler = self._handler()
        handler.wfile = mock.Mock(closed=False)
        handler.wfile.close.side_effect = OSError(104, "Connection reset by peer")
        handler.rfile = mock.Mock()
        self.assertIsNone(handler.finish())
        handler.wfile.close.assert_called_once_with()


class CallbackPageTest(unittest.TestCase):
    """A stray request gets a 404; a refused consent gets its own page."""

    def test_noise_is_ignored_and_a_refusal_is_recorded(self):
        port = _free_port()
        server = auth._CallbackServer(("127.0.0.1", port), auth._CallbackHandler)
        self.addCleanup(server.server_close)
        server.callback_path = "/callback"
        server.auth_code = None
        server.auth_error = None
        server.returned_state = None
        server.timeout = 0.2
        replies = []

        def browse():
            for target in ("/favicon.ico", "/callback?error=access_denied&state=s-1"):
                with socket.create_connection(("127.0.0.1", port), timeout=5) as sock:
                    sock.sendall(
                        f"GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n"
                        "Connection: close\r\n\r\n".encode("ascii")
                    )
                    chunks = []
                    while chunk := sock.recv(4096):
                        chunks.append(chunk)
                    replies.append(b"".join(chunks))

        caller = threading.Thread(target=browse, daemon=True)
        caller.start()
        auth.wait_for_callback(server, timeout=10)
        caller.join(timeout=5)

        self.assertEqual(len(replies), 2)
        self.assertTrue(replies[0].startswith(b"HTTP/1.0 404"), replies[0][:40])
        self.assertTrue(replies[1].startswith(b"HTTP/1.0 200"), replies[1][:40])
        self.assertIn(b"Authorisation was not completed", replies[1])
        self.assertIsNone(server.auth_code)
        self.assertEqual(server.auth_error, "access_denied")
        self.assertEqual(server.returned_state, "s-1")


class MainRefusalTest(unittest.TestCase):
    GENERATED = "state-this-run-generated"
    TOKENS = {"access_token": "A", "refresh_token": "R", "expires_in": 1800}

    def setUp(self):
        # main() re-resolves the module-level cache path; put it back afterwards.
        patcher = mock.patch.object(xero_client, "TOKEN_FILE", xero_client.TOKEN_FILE)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.env = {
            "XERO_CLIENT_ID": "id-not-a-secret",
            "XERO_CLIENT_SECRET": "secret-not-used",
            "XERO_REDIRECT_URI": f"http://localhost:{_free_port()}/callback",
        }

    def _main(self, *, env=None, unset=(), auth_error=None, browser_opened=False, server=None):
        def land_the_callback(server, timeout=auth.CALLBACK_TIMEOUT):
            server.auth_code = None if auth_error else "the-code"
            server.auth_error = auth_error
            server.returned_state = self.GENERATED

        out = io.StringIO()
        with mock.patch.object(auth, "load_dotenv", lambda *a, **k: None), \
                mock.patch.dict(os.environ, env or self.env, clear=False), \
                mock.patch.object(auth.secrets, "token_urlsafe", return_value=self.GENERATED), \
                mock.patch.object(auth.webbrowser, "open", return_value=browser_opened), \
                mock.patch.object(auth, "wait_for_callback", side_effect=land_the_callback), \
                mock.patch.object(
                    auth.requests, "post", return_value=_TokenResponse(dict(self.TOKENS))
                ) as post, \
                mock.patch.object(auth, "save_tokens") as save_tokens, \
                redirect_stdout(out):
            for name in unset:
                os.environ.pop(name, None)
            if server is not None:
                with mock.patch.object(auth, "_CallbackServer", **server):
                    raised = self._run()
            else:
                raised = self._run()
        return raised, out.getvalue(), post, save_tokens

    @staticmethod
    def _run():
        try:
            auth.main([])
        except SystemExit as exc:
            return exc
        return None

    def test_missing_credentials_stop_before_anything_is_opened(self):
        for missing in ("XERO_CLIENT_ID", "XERO_CLIENT_SECRET"):
            with self.subTest(missing=missing):
                raised, out, post, _ = self._main(unset=(missing,))
                self.assertEqual(
                    raised.code,
                    "Set XERO_CLIENT_ID and XERO_CLIENT_SECRET in .env (see .env.example).",
                )
                self.assertEqual(out, "")
                post.assert_not_called()

    def test_an_unusable_redirect_uri_is_named(self):
        env = dict(self.env, XERO_REDIRECT_URI="https://localhost:8400/callback")
        raised, out, post, _ = self._main(env=env)
        self.assertEqual(
            raised.code,
            "Invalid XERO_REDIRECT_URI: XERO_REDIRECT_URI must use http for this "
            "local callback server",
        )
        self.assertEqual(out, "")
        post.assert_not_called()

    def test_a_busy_port_is_reported_as_one(self):
        raised, _, post, _ = self._main(
            server={"side_effect": OSError(98, "Address already in use")}
        )
        message = raised.code
        self.assertTrue(message.startswith("error: cannot listen on localhost:"), message)
        self.assertIn("Address already in use", message)
        self.assertIn("Something else holds that port", message)
        post.assert_not_called()

    def test_an_opened_browser_still_prints_the_url(self):
        raised, out, post, save_tokens = self._main(browser_opened=True)
        self.assertIsNone(raised)
        self.assertIn("Opening browser for Xero consent...", out)
        self.assertIn("If the browser did not open, paste this URL into one:", out)
        self.assertIn(auth.AUTHORIZE_URL, out)
        self.assertNotIn("Could not open a browser", out)
        save_tokens.assert_called_once_with(self.TOKENS)

    def test_a_refused_consent_names_a_readable_error_code(self):
        raised, _, post, save_tokens = self._main(auth_error="access_denied")
        self.assertEqual(
            raised.code,
            "Xero returned 'access_denied' - consent was denied or cancelled. Run again.",
        )
        post.assert_not_called()
        save_tokens.assert_not_called()

    def test_an_unreadable_error_code_never_reaches_the_terminal(self):
        raised, _, post, _ = self._main(auth_error="\x1b[2Jpaste this into a shell")
        self.assertEqual(
            raised.code,
            "Xero returned an error code this script could not read - consent was "
            "denied or cancelled. Run again.",
        )
        post.assert_not_called()


if __name__ == "__main__":
    unittest.main()
