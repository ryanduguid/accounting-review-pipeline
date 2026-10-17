"""Execute only the frozen Python guard for the Rust comparison harness."""

import runpy
import sys

check_bytes = runpy.run_path(sys.argv[1])["check_bytes"]
for line in sys.stdin:
    print("\t".join(check_bytes(bytes.fromhex(line.strip()))), flush=True)
