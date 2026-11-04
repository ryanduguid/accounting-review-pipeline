// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

//! Fabricated bytes as a Xero Trial Balance response body: JSON decoding,
//! report validation, row building and the balance check. A refusal is a
//! result; a panic is a crash.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    xero_trial_balance_export::fuzzing::exercise(data);
});
