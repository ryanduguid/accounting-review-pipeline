//! Hold the README's Controls table to the controls the engine raises.
use monthly_close_control_plane::models::Control;

const README: &str = include_str!("../README.md");

/// The backticked first cell of each row in the table under `## Controls`.
fn documented_controls() -> Vec<&'static str> {
    let section: Vec<&str> = README
        .lines()
        .skip_while(|line| *line != "## Controls")
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .collect();
    assert!(
        !section.is_empty(),
        "README.md has no '## Controls' section"
    );
    section
        .into_iter()
        .filter_map(|line| line.strip_prefix("| `"))
        .map(|row| row.split_once('`').expect("unclosed control ID").0)
        .collect()
}

#[test]
fn readme_controls_table_matches_the_engine() {
    let documented = documented_controls();
    let emitted: Vec<&str> = Control::ALL.iter().map(|c| c.as_str()).collect();
    let missing: Vec<_> = emitted
        .iter()
        .filter(|id| !documented.contains(id))
        .collect();
    let extra: Vec<_> = documented
        .iter()
        .filter(|id| !emitted.contains(id))
        .collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "README Controls table differs from models::Control: missing {missing:?}, extra {extra:?}"
    );
    assert_eq!(
        documented, emitted,
        "README Controls table must list each control once, in models::Control order"
    );
}
