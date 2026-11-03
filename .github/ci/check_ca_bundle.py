"""Compare the Xero exporter's embedded CA bundle with the current certifi release.

usage: python -I .github/ci/check_ca_bundle.py [PACKAGE_DIR]

Run from the repository root. PACKAGE_DIR (default
packages/xero-trial-balance-export) holds NOTICE, which records the certifi
version and SHA-256 of data/cacert.pem. The check confirms that record, takes
the current certifi wheel from PyPI, checks it against the SHA-256 PyPI lists
and compares its certifi/cacert.pem with data/cacert.pem byte for byte. Any
difference fails with the refresh step.

Network, PyPI and NOTICE errors exit with one line, "ExceptionType: message",
rather than a traceback.
"""

import hashlib
import io
import json
import re
import sys
import urllib.request
import zipfile
from pathlib import Path

PYPI = "https://pypi.org/pypi/certifi/json"
REFRESH = "Follow 'CA bundle refresh' in packages/xero-trial-balance-export/CONTRIBUTING.md."
RECORD = re.compile(r"CA bundle from certifi (\S+)\s.*?Its SHA-256 is\s+([0-9a-f]{64})\.", re.S)


def recorded(notice: str) -> tuple[str, str]:
    """Return the certifi version and bundle SHA-256 that NOTICE records."""
    match = RECORD.search(notice)
    if match is None:
        raise ValueError("NOTICE no longer records the certifi version and SHA-256; "
                         "update this check")
    return match[1], match[2]


def fetch(url: str) -> bytes:
    if not url.startswith("https://"):
        raise ValueError(f"refusing a URL that is not HTTPS: {url}")
    with urllib.request.urlopen(url, timeout=60) as response:  # nosec B310 - HTTPS only
        return response.read()


def current_release() -> tuple[str, bytes]:
    """Return the current certifi version and the cacert.pem its wheel ships."""
    release = json.loads(fetch(PYPI))
    version = release["info"]["version"]
    wheels = [file for file in release["urls"] if file["packagetype"] == "bdist_wheel"]
    if len(wheels) != 1:
        raise ValueError(f"expected one wheel for certifi {version} on PyPI, found {len(wheels)}")
    wheel = fetch(wheels[0]["url"])
    if hashlib.sha256(wheel).hexdigest() != wheels[0]["digests"]["sha256"]:
        raise ValueError(f"the certifi {version} wheel does not match the SHA-256 PyPI lists")
    with zipfile.ZipFile(io.BytesIO(wheel)) as archive:
        return version, archive.read("certifi/cacert.pem")


def check(notice: str, bundle: bytes, version: str, upstream: bytes) -> str | None:
    """Return why the embedded bundle needs a refresh, or None when it is current."""
    recorded_version, digest = recorded(notice)
    if hashlib.sha256(bundle).hexdigest() != digest:
        return (f"data/cacert.pem does not match the SHA-256 NOTICE records for certifi "
                f"{recorded_version}. {REFRESH}")
    if bundle != upstream:
        return (f"data/cacert.pem is certifi {recorded_version}, but certifi {version} ships "
                f"a different bundle. {REFRESH}")
    return None


def main(argv: list[str]) -> None:
    if len(argv) > 1:
        raise ValueError("expected at most one PACKAGE_DIR")
    package = Path(argv[0] if argv else "packages/xero-trial-balance-export")
    notice = (package / "NOTICE").read_text(encoding="utf-8")
    bundle = (package / "data" / "cacert.pem").read_bytes()
    version, upstream = current_release()
    problem = check(notice, bundle, version, upstream)
    if problem:
        sys.exit(problem)
    print(f"data/cacert.pem matches the current certifi release, {version}.")


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except (OSError, ValueError, KeyError, TypeError, zipfile.BadZipFile) as error:
        sys.exit(f"{type(error).__name__}: {error}")
