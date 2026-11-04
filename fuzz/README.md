Two harnesses replay fabricated bytes through Monthly Close Controls'
trial-balance, mapping, mapping-policy, balance-policy, subledger,
GST-mapping and GST-settlements loaders.
Both call the same `monthly_close_control_plane::fuzzing::exercise` function
from the component's `tools` feature. Expected validation errors are returned;
a panic fails the run. Seeds are at most 4 KiB. Workpaper Review Gate and
Xero Trial Balance Export each have their own cargo-fuzz target, described
after these 2.

## Native mutation harness

From the repository root:

```sh
cargo run --locked --release --manifest-path packages/monthly-close-control-plane/Cargo.toml --features tools --bin close-control-fuzz -- fuzz/corpus 60
```

Use `0` seconds for corpus replay only. A non-zero duration adds deterministic
single-byte changes, deletions and appends until its deadline. The workflow
adds an external 90-second process timeout and a 5-minute job limit.
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

## Review Ready target

`packages/review-ready-gate/fuzz/` is a cargo-fuzz crate in the same pattern,
calling the component's `fuzzing::exercise` function from its `tools` feature.
Its `pack_loaders` target gives the same bytes as one source snapshot to every
pack loader: trial balance, open items, activity statement, GST control, bank
reconciliation, tie-out matrix, prior findings and the self-review under each
engagement profile. Its seeds are in `fuzz/seeds/`, and the component's
`cargo test` replays them. From `packages/review-ready-gate`:

```sh
mkdir -p /path/to/scratch
cargo +nightly-2026-10-15 fuzz run --target x86_64-unknown-linux-gnu pack_loaders /path/to/scratch fuzz/seeds -- -max_total_time=60 -max_len=4096 -timeout=2 -rss_limit_mb=1024
```

## Xero Trial Balance target

`packages/xero-trial-balance-export/fuzz/` is a cargo-fuzz crate in the same
pattern, calling the component's `fuzzing::exercise` function from its `tools`
feature. Its `report_json` target reads the bytes as a Trial Balance response
body, then validates the report, builds the output rows and checks that they
balance. Its seeds are in `fuzz/seeds/`, and the component's `cargo test`
replays them. From `packages/xero-trial-balance-export`:

```sh
mkdir -p /path/to/scratch
cargo +nightly-2026-10-15 fuzz run --target x86_64-unknown-linux-gnu report_json /path/to/scratch fuzz/seeds -- -max_total_time=60 -max_len=4096 -timeout=2 -rss_limit_mb=1024
```

The workflow's libFuzzer job runs both targets with `close_loaders` weekly and
on demand.

Both workflow jobs retain read-only repository access and neither is a merge
gate. Successful runs are bounded evidence, not proof that every input is
handled.
