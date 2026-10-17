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
        return Err(if protect {
            Failure::message(format!(
                "error: Windows DPAPI could not protect token.json (Windows error {}); no token was saved.",
                unsafe { GetLastError() }
            ))
        } else {
            Failure::message(
                "error: token.json's DPAPI payload is corrupt or cannot be opened by this Windows user; the file was left unchanged. Run: xero-tb-auth",
            )
        });
    }
    // SAFETY: a successful DPAPI call returns cbData readable bytes, or an empty
    // output. Copy them before freeing the allocation exactly once.
    let bytes = if output.pbData.is_null() {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() }
    };
    if !output.pbData.is_null() {
        unsafe {
            LocalFree(output.pbData.cast());
        }
    }
    Ok(bytes)
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;
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
