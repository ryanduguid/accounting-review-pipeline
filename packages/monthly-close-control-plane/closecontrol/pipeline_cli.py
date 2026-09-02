"""
OpenAccountants Australian Monthly Close & ATO Benchmark Review CLI.
"""

from __future__ import annotations

import argparse
import sys


def main() -> int:
    parser = argparse.ArgumentParser(
        prog="openaccountants-au",
        description="Quarantined. Use close-control and ato-benchmark-compare.",
    )
    parser.parse_args()
    print(
        "openaccountants-au is quarantined. It printed canned ATO range "
        "verdicts and is not a benchmark engine. Use close-control for the "
        "close pack and ato-benchmark-compare for range tests.",
        file=sys.stderr,
    )
    return 2


if __name__ == "__main__":
    sys.exit(main())
