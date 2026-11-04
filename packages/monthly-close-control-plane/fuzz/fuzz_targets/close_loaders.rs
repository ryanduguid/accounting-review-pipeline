//! Fabricated bytes to the trial-balance, mapping, mapping-policy,
//! balance-policy, subledger and GST-mapping loaders. A refusal is a result; a
//! panic is a crash.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    monthly_close_control_plane::fuzzing::exercise(data);
});
