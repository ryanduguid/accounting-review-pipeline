Two harnesses replay fabricated bytes through Monthly Close Controls'
trial-balance, mapping, mapping-policy, balance-policy and subledger loaders.
Both call the same `monthly_close_control_plane::fuzzing::exercise` function
from the component's `tools` feature. Expected validation errors are returned;
a panic fails the run. Seeds are at most 4 KiB.

## Native mutation harness

From the repository root:

```sh
cargo run --locked --release --manifest-path packages/monthly-close-control-plane/Cargo.toml --features tools --bin close-control-fuzz -- fuzz/corpus 60
```

Use `0` seconds for corpus replay only. A non-zero duration adds deterministic
single-byte changes, deletions and appends until its deadline. The workflow
adds an external 90-second process timeout and a five-minute job limit.
`tests/test_fuzz_harness.py` checks native corpus replay in the root suite.
This mutation pass does not measure code coverage; the former Python Atheris
coverage does not establish Rust coverage.

## cargo-fuzz target

`packages/monthly-close-control-plane/fuzz/` is a separately locked cargo-fuzz
crate with one libFuzzer target, `close_loaders`. On Linux, with the pinned
nightly toolchain and `cargo-fuzz` 0.13.2 installed, run from
`packages/monthly-close-control-plane`:

```sh
mkdir -p /path/to/scratch
cargo +nightly-2026-10-15 fuzz run close_loaders /path/to/scratch ../../fuzz/corpus -- -max_total_time=60 -max_len=4096 -timeout=2 -rss_limit_mb=1024
```

libFuzzer writes new inputs to the first directory, so the scratch directory
keeps the checked-in seeds unchanged. Crash inputs go to the crate's ignored
`artifacts/` directory. The `cargo-fuzz` release binary is built for musl and
defaults to that target, so add `--target x86_64-unknown-linux-gnu` when using
it, as the workflow does. The workflow runs this target weekly and on demand, not after
each push, with a 20-minute job limit.

Both workflow jobs retain read-only repository access and neither is a merge
gate. Successful runs are bounded evidence, not proof that every input is
handled.
