# Copyright 2026 Ryan Duguid
# SPDX-License-Identifier: AGPL-3.0-or-later
# Additional terms under AGPL-3.0 section 7(b): see NOTICE.

"""Execute only the frozen Python guard for the Rust comparison harness."""

import runpy
import sys

check_bytes = runpy.run_path(sys.argv[1])["check_bytes"]
for line in sys.stdin:
    print("\t".join(check_bytes(bytes.fromhex(line.strip()))), flush=True)
