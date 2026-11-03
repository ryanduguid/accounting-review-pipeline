//! The release workflow is the shared archive policy, not a local copy.

mod common;

use common::{Source, regex};

const POLICY_REF: &str = "ryanduguid/release-policy/.github/workflows/release-archive.yml@";

/// The words `shlex.split` returns for a shell line: ASCII space, tab, CR and LF separate
/// words (other Unicode spaces stay inside a word, as in the shell), single quotes are
/// literal, and inside double quotes a backslash escapes only `"` and `\`.
fn shell_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut characters = line.chars();
    let unclosed = || panic!("no closing quotation in {line:?}");
    while let Some(character) = characters.next() {
        if matches!(character, ' ' | '\t' | '\r' | '\n') {
            words.extend(word.take());
            continue;
        }
        let current = word.get_or_insert_with(String::new);
        match character {
            '\'' => loop {
                match characters.next() {
                    Some('\'') => break,
                    Some(quoted) => current.push(quoted),
                    None => unclosed(),
                }
            },
            '"' => loop {
                match characters.next() {
                    Some('"') => break,
                    Some('\\') => match characters.next() {
                        Some(escaped @ ('"' | '\\')) => current.push(escaped),
                        Some(other) => current.extend(['\\', other]),
                        None => unclosed(),
                    },
                    Some(quoted) => current.push(quoted),
                    None => unclosed(),
                }
            },
            '\\' => current.push(
                characters
                    .next()
                    .unwrap_or_else(|| panic!("no escaped character in {line:?}")),
            ),
            other => current.push(other),
        }
    }
    words.extend(word);
    words
}

#[test]
fn release_workflow_uses_the_shared_archive_policy() {
    let workflow = Source::repository(".github/workflows/release-accounting-excel-toolkit.yml");
    workflow.has(POLICY_REF);
    // The policy must be pinned to an immutable 40-hex commit, never a branch or a tag.
    // Which commit it names moves with every policy bump and is reviewed in that bump's
    // own diff, so it is not frozen here.
    let pin = workflow.text.split_once(POLICY_REF).unwrap().1;
    let pin = pin.split_whitespace().next().unwrap_or_default();
    assert_eq!(pin.len(), 40, "{pin}");
    assert!(
        pin.bytes().all(|byte| b"0123456789abcdef".contains(&byte)),
        "{pin}"
    );
    workflow.has_all(&[
        "artifact-stem: accounting-excel-toolkit",
        "source-directory: adapters/accounting-excel-toolkit",
        "tag-prefix: accounting-excel-toolkit",
        "\"accounting-excel-toolkit/v*\"",
    ]);
    workflow.lacks("\n          git archive ");
}

#[test]
fn release_guide_verifies_spdx_for_both_archives() {
    let guide = Source::adapter("RELEASING.md");
    let found = regex(r"(?s)for archive in ([^\n]+); do\n(.*?)\ndone")
        .captures(&guide.text)
        .expect("The guide must verify each archive's SPDX attestation");
    assert_eq!(
        shell_words(&found[1]),
        [
            "accounting-excel-toolkit-${tag##*/v}.zip",
            "accounting-excel-toolkit-${tag##*/v}.tar.gz",
        ]
    );
    Source::new("the guide's archive loop", &found[2]).has_all(&[
        "gh attestation verify \"$archive\" -R \"$repo\"",
        "--predicate-type https://spdx.dev/Document/v2.3",
        "--source-digest \"$release_commit\"",
        "--source-ref \"refs/tags/$tag\"",
        "--signer-workflow ryanduguid/release-policy/.github/workflows/publish-archives.yml",
        "--signer-digest \"$policy_sha\"",
    ]);
}

#[test]
fn shell_words_follow_shlex_quoting() {
    assert_eq!(
        shell_words(r#" "a b" 'c d'  e\ f "g\"h" "i\$j" "#),
        ["a b", "c d", "e f", "g\"h", "i\\$j"]
    );
    assert_eq!(shell_words(r#""" x"#), ["", "x"]);
    // shlex.split('a\xa0b\tc d e') == ['a\xa0b', 'c d', 'e']
    assert_eq!(
        shell_words("a\u{a0}b\tc\u{2003}d e"),
        ["a\u{a0}b", "c\u{2003}d", "e"]
    );
}
