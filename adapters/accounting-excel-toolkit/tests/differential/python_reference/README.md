# Python benchmark reference

These two modules preserve the Python functions from pipeline commit
`e06647062efaf4410f7d4a414a0559ca51147267` for `benchmark_aged_review.py`.
Only their executable guards change, to refuse direct CLI use.

Benchmark schema 1 measures Python interpreter startup and traced Python
allocations. It continues to measure this Python reference. Its results do
not describe the native Rust commands or establish a Rust speed improvement.
Rust cannot supply the Python allocator measurements through `tracemalloc`.

Operational review, evidence preparation and replay use the native binaries.
Their compatibility oracle is a separate checkout of the original revision.
