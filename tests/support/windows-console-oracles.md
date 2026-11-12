# Windows console contracts

`windows_console.rs` runs the actual CLI in an owned, hidden Windows console.
It checks physical 80-column output, native width selection, colour presence on
stdout and stderr and the documented help and usage exit codes. Captured stream
tests separately check colour overrides. No colour or size override is passed to
the CLI under test.

The plain text fixtures come from independent Python implementations with
`COLUMNS=80`, `LINES=24` and `PYTHON_COLORS=0`:

- Grant uses the existing CPython 3.15 argparse fixtures in its CLI tests.
- EAA uses CPython 3.15.0, source commit
  `c6e932d2439623ec26f851379620f9fe057020ba` and the released differential runner.
  Both new fixtures also match across the 5 existing colour precedence cases.
- Evatt uses CPython 3.14.10 and the full 5-command Python implementation at
  `623d75391b5a707a8b80158c05a6dcc422365212`, immediately before the Rust port.
  Both fixtures match with `PYTHON_COLORS=0` and `1` after stripping ANSI styles.
  This is a newly pinned reference; equivalence to the earlier differential
  snapshot `efa253847e4ecdca5ae9b4abec045c0b16b6835c` is unverified.

The helper accounts for physical console wrapping independently of argparse's
logical output lines. It retains observed text on failure and bounds both the
CLI and the single-test console process. PowerShell only launches the console
test; Rust launches the CLI with inherited console handles so its streams keep
their native terminal behaviour.
