"""Grammar-generated argparse comparison, with no readable map or writable output.

python3.14 tests/differential/arguments.py --python-root <oracle> --binary <evatt> --seed 1 --cases 1000
Uses a fabricated empty scratch directory so successful parsing cannot mutate
the checkout. Compare colour, narrow wrapping, option abbreviations, malformed
options, missing commands, private diagnostics and non-UTF-8 argv.
"""

import argparse
import json
import os
from pathlib import Path
import random
import subprocess
import sys
import tempfile

COMMANDS = ["redact", "restore", "verify", "disclosure-record", "disclosure-check"]
OPTIONS = [
    "--in",
    "--map",
    "--out",
    "--destination",
    "--decision-ref",
    "--record",
    "--help",
    "--version",
]
VALUES = [
    "x",
    "",
    "-1",
    "-1.0",
    "-٣",
    "--",
    "--missing",
    "Sample Sensitive Sentinel",
    "é",
    "\udcff",
]


def generated(rng):
    argv = (
        [] if rng.random() < 0.25 else [rng.choice(COMMANDS + ["redct", "disclosure-recrod", "-"])]
    )
    for _ in range(rng.randrange(8)):
        option = rng.choice(OPTIONS + ["--missing", "-h", "--no-strict", "---in"])
        if option.startswith("--") and rng.random() < 0.4:
            option = option[: rng.randint(3, len(option))]
        if rng.random() < 0.3:
            argv.append(option + "=" + rng.choice(VALUES))
        else:
            argv.append(option)
            if rng.random() < 0.7:
                argv.append(rng.choice(VALUES))
    return argv


def run(program, argv, root, env):
    result = subprocess.run(
        program + [os.fsencode(v) for v in argv], cwd=root, env=env, capture_output=True
    )
    # Windows text-mode stdout ends the version line with CRLF.
    version = result.stdout in (b"0.2.0\n", b"0.2.0\r\n")
    stdout = result.stdout.replace(b"0.2.0", b"1.0.0") if version else result.stdout
    return result.returncode, stdout, result.stderr


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--python-root", required=True)
    parser.add_argument("--python", default=sys.executable)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--cases", type=int, default=1000)
    args = parser.parse_args()
    rng = random.Random(args.seed)
    errors = []
    with tempfile.TemporaryDirectory(prefix="evatt-arguments-", dir=os.environ.get("EVATT_DIFFERENTIAL_TMP")) as root:
        for i in range(args.cases):
            argv = generated(rng)
            env = {
                **os.environ,
                "PYTHONPATH": args.python_root,
                "PYTHON_COLORS": rng.choice(["0", "1"]),
            }
            env["COLUMNS"] = str(rng.choice([5, 10, 20, 40, 80, 120]))
            env["LINES"] = "24"
            py = run([args.python, "-m", "evatt"], argv, root, env)
            rust = run([args.binary], argv, root, env)
            if py != rust:
                errors.append(
                    [
                        argv,
                        env["COLUMNS"],
                        env["PYTHON_COLORS"],
                        [x.hex() if isinstance(x, bytes) else x for x in py],
                        [x.hex() if isinstance(x, bytes) else x for x in rust],
                    ]
                )
                if len(errors) <= 8:
                    print(
                        "difference",
                        i,
                        repr(argv),
                        "width",
                        env["COLUMNS"],
                        "colour",
                        env["PYTHON_COLORS"],
                        py,
                        rust,
                        flush=True,
                    )
            assert not list(Path(root).iterdir()), "parser fuzz unexpectedly wrote a file"
    print(f"seed {args.seed}: {args.cases} argument cases; {len(errors)} differences")
    if errors:
        Path(tempfile.gettempdir(), f"evatt-argument-differences-{args.seed}.json").write_text(
            json.dumps(errors, indent=2)
        )
    return int(bool(errors))


if __name__ == "__main__":
    sys.exit(main())
