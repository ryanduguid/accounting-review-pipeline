// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: FSL-1.1-ALv2

//! Fabricated bytes to the trial-balance, mapping, mapping-policy,
//! balance-policy, subledger, GST-mapping and GST-settlements loaders. A
//! refusal is a result; a panic is a crash.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    monthly_close_control_plane::fuzzing::exercise(data);
});
