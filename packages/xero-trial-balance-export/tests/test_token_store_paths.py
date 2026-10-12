"""token_store's per-platform default and the legacy module-adjacent cache."""

import os
import tempfile
import unittest
from unittest import mock

import token_store


class StateHomeTest(unittest.TestCase):
    def _default(self, os_name):
        with mock.patch.object(token_store.os, "name", os_name):
            return token_store._state_home_token_file()

    def test_windows_uses_local_app_data(self):
        home = os.path.abspath(os.path.expanduser("~"))
        self.assertEqual(
            self._default("nt"),
            os.path.join(home, "AppData", "Local", "xero-trial-balance-export", "token.json"),
        )

    def test_other_platforms_use_the_xdg_state_directory(self):
        home = os.path.abspath(os.path.expanduser("~"))
        self.assertEqual(
            self._default("posix"),
            os.path.join(home, ".local", "state", "xero-trial-balance-export", "token.json"),
        )


class LegacyModuleCacheTest(unittest.TestCase):
    def test_an_existing_module_adjacent_cache_wins_over_the_default(self):
        with tempfile.TemporaryDirectory() as directory:
            legacy = os.path.join(directory, "token.json")
            with open(legacy, "w", encoding="utf-8") as fh:
                fh.write("{}")
            with mock.patch.object(token_store, "LEGACY_MODULE_TOKEN_FILE", legacy), \
                    mock.patch.dict(os.environ):
                os.environ.pop("XERO_TOKEN_FILE", None)
                resolved = token_store.resolve_token_file()
        self.assertEqual(resolved, os.path.realpath(legacy))
        self.assertNotEqual(resolved, token_store.DEFAULT_TOKEN_FILE)


if __name__ == "__main__":
    unittest.main()
