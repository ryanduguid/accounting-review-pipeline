// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! Fabricated bytes as a Xero Trial Balance response body: JSON decoding,
//! report validation, row building and the balance check. A refusal is a
//! result; a panic is a crash.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    xero_trial_balance_export::fuzzing::exercise(data);
});
