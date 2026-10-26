//! Keep the documented read-only OAuth boundary tied to the inspected native source.
use sha2::{Digest, Sha256};
use xero_trial_balance_export::{auth, http};

fn digest(raw: &[u8]) -> Option<String> {
    if raw.starts_with(&[0xef, 0xbb, 0xbf]) {
        return None;
    }
    let canonical = std::str::from_utf8(raw).ok()?.replace("\r\n", "\n");
    if canonical.contains('\r') {
        return None;
    }
    Some(
        Sha256::digest(canonical.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect(),
    )
}

#[test]
fn oauth_source_and_claims_require_review_before_their_pins_change() {
    for (name, source, expected) in [
        (
            "src/auth.rs",
            include_bytes!("../src/auth.rs").as_slice(),
            "6D8D788A3BD1A24B4D2B05E6BCC3E611A9E92BEAEC6F880A62BD6738872348F4",
        ),
        (
            "README.md",
            include_bytes!("../README.md").as_slice(),
            "1A8E4E07EC069C9E0427E0B0F76DFCCCA9BC9E1059267A1EAC43F5FB3952ECC0",
        ),
    ] {
        assert_eq!(
            digest(source).as_deref(),
            Some(expected),
            "{name} changed; review the OAuth boundary and documented claims before re-pinning"
        );
        let mut changed = source.to_vec();
        changed.extend_from_slice(b"\nchanged\n");
        assert_ne!(digest(&changed).as_deref(), Some(expected));
    }
    assert!(digest(b"\xef\xbb\xbftext").is_none());
    assert!(digest(b"bare\rreturn").is_none());
    assert!(digest(b"\xff").is_none());
    assert_eq!(digest(b"canonical\r\n"), digest(b"canonical\n"));
}

const SCOPE_PARAGRAPH: &str = "Read-only (`accounting.reports.trialbalance.read`); this tool cannot write to any ledger. Web and PKCE apps created on or after 2 March 2026 use granular scopes, while existing apps using the broad `accounting.reports.read` scope must migrate by 13 September 2027. Xero's [OAuth scope list](https://developer.xero.com/documentation/guides/oauth2/scopes/), [Granular Scopes FAQ](https://developer.xero.com/faq/granular-scopes) and [developer changelog](https://developer.xero.com/changelog) were checked on 20 August 2026 (`2026-08-20`); recheck them for apps created or used after that date. `token.json` and `.env` are gitignored. They are credentials, so treat them like passwords.";
const REFRESH_PARAGRAPH: &str = "Xero refresh tokens **rotate on use**: every refresh returns a replacement refresh token. If the refresh response does not arrive, Xero permits retrying the previous token for up to a 30-minute grace period; outside that window, the user must re-authorise. The [Xero OAuth FAQ](https://developer.xero.com/faq/oauth2) was checked on 20 August 2026 (`2026-08-20`); recheck it for apps created or used after that date.";

#[test]
fn native_scopes_endpoints_and_documented_boundary_match_the_reviewed_contract() {
    assert_eq!(
        auth::SCOPES,
        "offline_access accounting.reports.trialbalance.read"
    );
    assert_eq!(http::TOKEN_URL, "https://identity.xero.com/connect/token");
    assert_eq!(
        http::REPORT_URL,
        "https://api.xero.com/api.xro/2.0/Reports/TrialBalance"
    );
    let readme = include_str!("../README.md").replace("\r\n", "\n");
    assert!(readme.contains(SCOPE_PARAGRAPH));
    assert!(readme.contains(REFRESH_PARAGRAPH));
    assert!(
        !readme
            .replace(
                "accounting.reports.trialbalance.read",
                "accounting.transactions"
            )
            .contains(SCOPE_PARAGRAPH)
    );
    assert!(
        !readme
            .replace("30-minute grace period", "unlimited grace period")
            .contains(REFRESH_PARAGRAPH)
    );
}

#[test]
fn repository_wiring_has_no_python_runtime_or_release_caller() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for file in [
        "auth.py",
        "xero_client.py",
        "export_tb.py",
        "token_store.py",
        "pyproject.toml",
    ] {
        assert!(!root.join(file).exists(), "obsolete runtime: {file}");
    }
    assert!(
        !root
            .join("../../.github/workflows/release-xero-trial-balance-export.yml")
            .exists()
    );
    let cargo = include_str!("../Cargo.toml");
    assert!(cargo.contains("publish = false"));
    assert!(cargo.contains("version = \"1.0.0\""));
}
