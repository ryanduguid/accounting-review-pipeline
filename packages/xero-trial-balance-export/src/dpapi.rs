// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use crate::{Failure, Result};

#[cfg(windows)]
fn transform(data: &[u8], protect: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::{
        Foundation::{GetLastError, LocalFree},
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    let length = data
        .len()
        .try_into()
        .map_err(|_| Failure::message("error: token cache payload is too large"))?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: length,
        pbData: data.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: input points to live bytes; output is owned by DPAPI and released
    // with LocalFree. No entropy or UI is requested; the scope is current-user.
    let ok = unsafe {
        if protect {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        // SAFETY: reads this thread's error immediately after the DPAPI call failed.
        return Err(refusal(protect, unsafe { GetLastError() }));
    }
    // SAFETY: a successful DPAPI call returns cbData readable bytes, or an empty output.
    Ok(unsafe { take(&output, |data| LocalFree(data.cast())) })
}

/// Copy DPAPI's output, then free a non-null allocation exactly once.
///
/// # Safety
/// `output` is null or holds `cbData` readable bytes that `free` releases.
#[cfg(windows)]
unsafe fn take(
    output: &windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB,
    free: impl FnOnce(*mut u8) -> *mut std::ffi::c_void,
) -> Vec<u8> {
    if output.pbData.is_null() {
        return Vec::new();
    }
    // SAFETY: the caller guarantees cbData readable bytes; they are copied before the free.
    let bytes =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    free(output.pbData);
    bytes
}

/// Why DPAPI refused: protection reports the Windows error, unprotection the remedy.
#[cfg(windows)]
fn refusal(protect: bool, code: u32) -> Failure {
    if protect {
        Failure::message(format!(
            "error: Windows DPAPI could not protect token.json (Windows error {code}); no token was saved."
        ))
    } else {
        Failure::message(
            "error: token.json's DPAPI payload is corrupt or cannot be opened by this Windows user; the file was left unchanged. Run: xero-tb-auth",
        )
    }
}

#[cfg(not(windows))]
fn transform(_: &[u8], _: bool) -> Result<Vec<u8>> {
    Err(Failure::message(
        "error: Windows DPAPI is unavailable on this platform; token.json was not changed.",
    ))
}

pub fn protect(data: &[u8]) -> Result<Vec<u8>> {
    transform(data, true)
}
pub fn unprotect(data: &[u8]) -> Result<Vec<u8>> {
    transform(data, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn windows_token_protection_is_explicitly_unavailable() {
        let bytes = b"synthetic token-cache bytes";
        for failure in [protect(bytes).unwrap_err(), unprotect(bytes).unwrap_err()] {
            assert_eq!(failure.code, 1);
            assert!(failure.stdout.is_empty());
            assert_eq!(
                failure.stderr,
                "error: Windows DPAPI is unavailable on this platform; token.json was not changed.\n"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_protection_refusal_names_the_windows_error_and_a_null_output_is_empty() {
        let failure = refusal(true, 5);
        assert!(
            failure
                .stderr
                .contains("(Windows error 5); no token was saved.")
        );
        let null = windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: a null output has nothing to read or free.
        let bytes = unsafe { take(&null, |_| panic!("a null output was freed")) };
        assert!(bytes.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn real_windows_dpapi_round_trip_and_corruption() {
        let original = b"synthetic-win32-round-trip-7302 \0\xff binary tail";
        let mut encrypted = protect(original).unwrap();
        assert_ne!(encrypted, original);
        assert_eq!(unprotect(&encrypted).unwrap(), original);
        let last = encrypted.len() - 1;
        encrypted[last] ^= 1;
        for corrupt in [&encrypted[..], &b"not-a-dpapi-envelope"[..]] {
            let failure = unprotect(corrupt).unwrap_err();
            assert!(failure.stderr.contains("Run: xero-tb-auth"));
            assert!(!failure.stderr.contains("auth.py"));
        }
    }
}
