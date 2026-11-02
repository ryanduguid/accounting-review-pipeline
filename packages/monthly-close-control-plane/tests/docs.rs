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
    // Every data row counts, so a row whose first cell is not a backticked ID
    // fails here instead of escaping the comparison.
    section
        .into_iter()
        .filter(|line| line.starts_with('|'))
        .skip(2)
        .map(|row| {
            let cell = row.split('|').nth(1).unwrap_or_default().trim();
            cell.strip_prefix('`')
                .and_then(|rest| rest.strip_suffix('`'))
                .filter(|id| !id.is_empty() && !id.contains('`'))
                .unwrap_or_else(|| {
                    panic!("Controls row's first cell is not a backticked control ID: {row}")
                })
        })
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
