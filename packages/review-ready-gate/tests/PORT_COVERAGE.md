# Native port coverage

The Rust tests and differential tool replace the Python runtime tests from
commit `8061079d0b4206d064c54751aa047af8588fa265`. Inputs are fabricated.

| Original behaviour | Native coverage |
| --- | --- |
| Loader grammar, dates, identifiers and JSON | `tools/differential.rs`: money, dates, CSV and self-review corpora; `src/contract_tests.rs`: acknowledgement validation |
| Profile controls, precision, status and review separation | `src/engine.rs` tests; profile/tolerance and acknowledgement differential cases |
| Pack rendering and checkout protection | Three-file byte comparisons; checkout marker, environment, alias and refusal-before-read process tests |
| Publication failures, recovery and concurrency | `src/publication.rs` fault matrix and cleanup/startup tests; real contention, process death and Unix SIGTERM tests |
| Viewer schemas, witnesses and file admission | Saved-pack mutation corpus; finding/member/encoding tests; real symlink, hardlink, directory and Unix FIFO refusals |
| Display commands | Real view/compare byte and mtime snapshots, malformed/empty pack refusals, closed-output-pipe tests |
| Document intake, review and comparison | Fifteen fabricated extraction fixtures; review/binding tests; occurrence, context, original-evidence and coverage corpus |
| Evaluation and downstream contract | `src/evaluation_tests.rs`; root trial-balance and shared-behaviour conformance; native Power BI producer tests |
| Package/release metadata | Root metadata checks and explicit blocked native publisher; release-binary smokes |

Python import/class/ambient Decimal-context assertions and Python AST/source
identity checks are retired with the binary-only API. Historical fixture
provenance remains pinned.

This family mapping and the differential case counts do not establish complete
method-by-method equivalence to all 286 original test functions or 504
parametrised platform cases. The bounded independent pass identified additional
acknowledgement, reader, display, document-occurrence and recovery cases, which
now have native coverage.

Run `cargo test --locked --all-features`. Build all probe binaries before
running the differential: `cargo build --locked --features probe --bins`.
The differential requires CPython 3.15 with Unicode 17.0.0 and the frozen source.
