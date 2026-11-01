//! Fabricated bytes to every review-ready pack loader as one source snapshot.
//! A refusal is a result; a panic is a crash.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    review_ready_gate::fuzzing::exercise(data);
});
