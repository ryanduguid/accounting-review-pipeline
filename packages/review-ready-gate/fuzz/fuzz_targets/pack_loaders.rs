// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Fabricated bytes to every review-ready pack loader as one source snapshot.
//! A refusal is a result; a panic is a crash.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    review_ready_gate::fuzzing::exercise(data);
});
