//! Preserve the former assurance and fabricated-sample assertions in Rust.
use evatt::{
    entities, patterns,
    pyjson::{self, Json},
    pytext,
};
use std::path::Path;

const README: &str = include_str!("../README.md");
const DATA_FLOW: &str = include_str!("../DATA-FLOW.md");
const DISCLAIMER: &str = include_str!("../DISCLAIMER.md");
const CONTRIBUTING: &str = include_str!("../CONTRIBUTING.md");
const SECURITY: &str = include_str!("../SECURITY.md");

fn normalise(text: &str) -> String {
    pytext::casefold(text)
        .split(|c: char| pytext::is_space(c as u32))
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn public_docs_keep_the_assurance_boundary_and_no_retired_claims() {
    let opening = normalise(README.split("```").next().unwrap());
    // Scope and policy stay ahead of the first runnable example.
    assert!(opening.contains("pseudonymisation with local key retention"));
    assert!(opening.contains("the key never leaves the local machine"));
    let readme = normalise(README);
    for required in [
        "pseudonymisation with local key retention",
        "the key never leaves the local machine",
        "residual risk is contextual re-identification",
        "410-100",
        "prints finding kinds and locations, without detected values",
        "no client data passes through evatt until the policy governing it",
        "that includes testing",
        "re-runs the same detection",
        "must write your own",
        "matching evidence is not permission",
        "record is unsigned and can be edited",
        "independently authenticate the external human",
        "existing replacement-count manifest stays unchanged",
    ] {
        assert!(readme.contains(required), "{required}");
    }
    for document in [README, DATA_FLOW, DISCLAIMER] {
        assert!(normalise(document).contains("this does not take the data outside the privacy act"));
    }
    for document in [README, DATA_FLOW] {
        assert!(normalise(document).contains("ato client reference"));
    }
    for document in [README, DATA_FLOW, DISCLAIMER, CONTRIBUTING, SECURITY] {
        let text = normalise(document);
        for retired in [
            "anonymis",
            "anonymiz",
            "anonymous",
            "anonymity",
            "not personal information",
            "privacy act compliant",
            "compliant with the privacy act",
            "cannot be re-identified",
            "safe to send",
            "fully compliant",
            "guarantee",
            "independently of the redaction",
            "second chance to be caught",
        ] {
            assert!(!text.contains(retired), "{retired}");
        }
        assert!(!document.contains('\u{2014}'));
    }
}

#[test]
fn fabricated_samples_and_ignore_rules_retain_their_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let samples = root.join("evatt/samples");
    let mut markdown: Vec<_> = std::fs::read_dir(&samples)
        .unwrap()
        .map(|p| p.unwrap().path())
        .filter(|p| p.extension().is_some_and(|s| s == "md"))
        .collect();
    markdown.sort();
    let mut expected = vec![
        "clean.md",
        "entities-only.md",
        "identifiers.md",
        "unmapped-name.md",
        "negatives.md",
        "unicode-names.md",
        "research-question.md",
        "research-answer.md",
    ];
    expected.sort();
    assert_eq!(
        markdown
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect::<Vec<_>>(),
        expected
    );
    for path in &markdown {
        let text = std::fs::read_to_string(path).unwrap();
        for s in evatt::email::spans(&text) {
            let domain = s.value.split('@').nth(1).unwrap();
            assert!(["example.com", "example.net", "example.org"].contains(&domain) || domain.ends_with(".example"));
        }
    }
    sample_map(&samples);
    synthetic_identifiers(&samples);
    let ignored = std::fs::read_to_string(root.join(".gitignore")).unwrap();
    for rule in [
        "entities.json",
        "*.entities.json",
        "*.triage.md",
        "*.disclosure.json",
        "*.tmp",
    ] {
        assert!(ignored.lines().any(|line| line == rule));
    }
    assert!(!root.join("entities.json").exists());
}

fn sample_map(samples: &Path) {
    let mapping = pyjson::loads_plain(&std::fs::read_to_string(samples.join("entities.sample.json")).unwrap()).unwrap();
    let entries = entities::load(&mapping).unwrap();
    for e in entries {
        let value = e.value.to_utf8().unwrap();
        assert!(
            value.starts_with("Sample ") || ["Jane Roe", "Mary O’Brien", "MACDONALD, Alan"].contains(&value.as_str())
        );
    }
    assert!(matches!(mapping, Json::Object(_)));
}

fn synthetic_identifiers(samples: &Path) {
    let identifiers = std::fs::read_to_string(samples.join("identifiers.md")).unwrap();
    assert!(!identifiers.contains("51 824 753 556"));
    assert!(!identifiers.contains("0412 345 678"));
    assert!(identifiers.contains("ABN 12 345 678 901"));
    assert!(!patterns::valid("abn", "12345678901").unwrap());
    assert!(identifiers.contains("Phone 0491 570 006"));
}
