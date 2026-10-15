The native harness replays fabricated bytes through Monthly Close Controls'
trial-balance, mapping, mapping-policy, balance-policy and subledger loaders.
Expected validation errors are returned; a panic fails the run. Seeds are at
most 4 KiB. From the repository root:

```sh
cargo run --locked --release --manifest-path packages/monthly-close-control-plane/Cargo.toml --features tools --bin close-control-fuzz -- fuzz/corpus 60
```

Use `0` seconds for corpus replay only. A non-zero duration adds deterministic
single-byte changes, deletions and appends until its deadline. The workflow
adds an external 90-second process timeout and a five-minute job limit.
`tests/test_fuzz_harness.py` checks native corpus replay in the root suite.
The scheduled workflow retains read-only repository access and is not a merge
gate. Its mutation pass does not measure code coverage; the former Python
Atheris coverage does not establish Rust coverage. Successful runs are bounded
evidence, not proof that every input is handled.
