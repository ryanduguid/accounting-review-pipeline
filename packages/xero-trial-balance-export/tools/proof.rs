// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use std::path::Path;

use xero_trial_balance_export::{Failure, Result, offline, stdio};
#[expect(
    clippy::too_many_lines,
    reason = "the proof tool's arguments and steps together"
)]
fn run(args: &[String], root: &Path) -> Result<()> {
    if args == ["--help"] || args == ["-h"] {
        stdio::out("usage: xero-tb-proof [--check]\n");
        return Ok(());
    }
    if !args.is_empty() && args != ["--check"] {
        return Err(Failure {
            code: 2,
            stdout: String::new(),
            stderr: "usage: xero-tb-proof [--check]\n".into(),
        });
    }
    let text = std::fs::read_to_string(root.join("samples/sample-output.csv"))
        .map_err(|_| Failure::message("the fabricated sample cannot be read"))?;
    let summary = offline::summary(&text)?;
    let assets = [
        ("assets/quick-proof.svg", offline::render_svg(&summary)),
        (
            "assets/quick-proof.md",
            offline::render_transcript(&summary),
        ),
    ];
    let check = args == ["--check"];
    let mut stale = false;
    for (name, expected) in assets {
        let path = root.join(name);
        if check {
            if !std::fs::read_to_string(path)
                .is_ok_and(|text| text.replace("\r\n", "\n") == expected)
            {
                stdio::out(&format!("stale: {name}\n"));
                stale = true;
            }
        } else {
            std::fs::write(path, expected)
                .map_err(|_| Failure::message("the fabricated proof asset cannot be written"))?;
            stdio::out(&format!("wrote {name}\n"));
        }
    }
    if stale {
        return Err(Failure {
            code: 1,
            stdout: String::new(),
            stderr: String::new(),
        });
    }
    if check {
        stdio::out("quick proof is current\n");
    }
    Ok(())
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if let Err(error) = run(&args, &offline::component_root()) {
        stdio::out(&error.stdout);
        stdio::err(&error.stderr);
        std::process::exit(error.code);
    }
}

#[cfg(test)]
#[path = "../tests/support/scratch.rs"]
mod scratch;

#[cfg(test)]
mod tests {
    use super::{Failure, run, scratch::Scratch};
    use std::{fs, path::Path};

    const SAMPLE_PATH: &str = "samples/sample-output.csv";
    const SVG_PATH: &str = "assets/quick-proof.svg";
    const TRANSCRIPT_PATH: &str = "assets/quick-proof.md";
    const SVG: &str = include_str!("../assets/quick-proof.svg");
    const TRANSCRIPT: &str = include_str!("../assets/quick-proof.md");

    fn sample_fixture() -> Scratch {
        let root = Scratch::new();
        fs::create_dir(root.0.join("samples")).unwrap();
        fs::create_dir(root.0.join("assets")).unwrap();
        fs::write(
            root.0.join(SAMPLE_PATH),
            include_bytes!("../samples/sample-output.csv"),
        )
        .unwrap();
        root
    }

    fn check(root: &Path) -> super::Result<()> {
        run(&["--check".into()], root)
    }

    #[test]
    fn missing_or_invalid_sample_leaves_assets_unwritten() {
        let root = Scratch::new();
        fs::create_dir(root.0.join("samples")).unwrap();
        fs::create_dir(root.0.join("assets")).unwrap();

        assert_eq!(
            run(&[], &root.0),
            Err(Failure::message("the fabricated sample cannot be read"))
        );
        fs::write(root.0.join(SAMPLE_PATH), "Wrong column\nvalue\n").unwrap();
        assert_eq!(
            run(&[], &root.0),
            Err(Failure::message(
                "the fabricated sample has unexpected columns"
            ))
        );
        assert_eq!(fs::read_dir(root.0.join("assets")).unwrap().count(), 0);
    }

    #[test]
    fn writes_both_canonical_assets_and_checks_crlf_without_rewriting() {
        let root = sample_fixture();
        run(&[], &root.0).unwrap();
        let assets = [(SVG_PATH, SVG), (TRANSCRIPT_PATH, TRANSCRIPT)];
        for (name, expected) in assets {
            let path = root.0.join(name);
            assert_eq!(fs::read_to_string(&path).unwrap(), expected);
            fs::write(path, expected.replace('\n', "\r\n")).unwrap();
        }
        check(&root.0).unwrap();
        for (name, expected) in assets {
            assert_eq!(
                fs::read_to_string(root.0.join(name)).unwrap(),
                expected.replace('\n', "\r\n")
            );
        }
    }

    #[test]
    fn check_refuses_missing_or_stale_assets_without_repairing_them() {
        let root = sample_fixture();
        let svg = root.0.join(SVG_PATH);
        let transcript = root.0.join(TRANSCRIPT_PATH);
        let stale = Failure {
            code: 1,
            stdout: String::new(),
            stderr: String::new(),
        };
        fs::write(&svg, SVG).unwrap();
        assert_eq!(check(&root.0), Err(stale.clone()));
        assert_eq!(fs::read_to_string(&svg).unwrap(), SVG);
        assert!(!transcript.exists());

        fs::write(&svg, "stale proof").unwrap();
        fs::write(&transcript, TRANSCRIPT).unwrap();
        assert_eq!(check(&root.0), Err(stale));
        assert_eq!(fs::read_to_string(&svg).unwrap(), "stale proof");
        assert_eq!(fs::read_to_string(&transcript).unwrap(), TRANSCRIPT);
    }

    #[test]
    fn reports_a_write_failure_after_writing_the_first_asset() {
        let root = sample_fixture();
        let transcript = root.0.join(TRANSCRIPT_PATH);
        fs::create_dir(&transcript).unwrap();
        assert_eq!(
            run(&[], &root.0),
            Err(Failure::message(
                "the fabricated proof asset cannot be written"
            ))
        );
        assert_eq!(fs::read_to_string(root.0.join(SVG_PATH)).unwrap(), SVG);
        assert!(transcript.is_dir());
    }
}
