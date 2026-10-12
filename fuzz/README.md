The Atheris target writes fabricated bytes to a file and reads it with Monthly
Close Controls' trial-balance, mapping, mapping-policy, balance-policy and
subledger loaders. Each malformed file must be refused with
`ControlInputError`, the error the command line reports. Anything else that
escapes fails the run. The seed corpus holds the component's example inputs,
truncated to 4 KiB, and an empty file.

On Linux with Python 3.14, from the repository root, run:

```sh
python -m pip install --require-hashes --only-binary :all: -r fuzz/requirements-fuzz.txt
python -m fuzz.close_control_loader fuzz/corpus -max_total_time=60 -max_len=4096 -timeout=2 -rss_limit_mb=1024
```

The component has no runtime dependencies, so the harness imports it from
`packages/monthly-close-control-plane`. `.github/workflows/fuzzing.yml` runs
the target after main pushes, weekly and on demand, with read-only repository
access and a five-minute timeout; it is not a merge gate.
`tests/test_fuzz_harness.py` replays the corpus in the root suite. Atheris
does not support Windows, so coverage-guided fuzzing runs on Linux. A
successful run is bounded evidence, not proof that every input is handled.

After changing the pinned version, regenerate the hashes with:

```sh
uv pip compile fuzz/requirements-fuzz.in --python-version 3.14 --python-platform x86_64-unknown-linux-gnu --generate-hashes --output-file fuzz/requirements-fuzz.txt
```
