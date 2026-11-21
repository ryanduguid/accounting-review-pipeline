// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use crate::Result;
use std::collections::BTreeSet;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug)]
pub struct Slot {
    pub name: &'static str,
    pub filename: &'static str,
    pub required: bool,
    pub kind: &'static str,
}

const fn slot(
    name: &'static str,
    filename: &'static str,
    required: bool,
    kind: &'static str,
) -> Slot {
    Slot {
        name,
        filename,
        required,
        kind,
    }
}
const OPEN: Slot = slot("open_items", "open_items.csv", true, "open_items");
const SELF: Slot = slot("self_review", "self_review.json", true, "self_review");
const PRIOR: Slot = slot(
    "prior_findings",
    "prior_findings.csv",
    false,
    "prior_findings",
);
const TB: Slot = slot("trial_balance", "trial_balance.csv", true, "trial_balance");
const OLD_TB: Slot = slot(
    "prior_trial_balance",
    "prior_trial_balance.csv",
    true,
    "trial_balance",
);
pub const PROFILE_NAMES: [&str; 3] = ["bas", "month_end", "year_end"];
const BAS: &[Slot] = &[
    TB,
    slot(
        "activity_statement",
        "activity_statement.csv",
        true,
        "activity_statement",
    ),
    slot(
        "gst_control_gl",
        "gst_control_gl.csv",
        true,
        "gst_control_gl",
    ),
    slot("bank_rec", "bank_rec.csv", false, "bank_rec"),
    OPEN,
    SELF,
    PRIOR,
];
const MONTH: &[Slot] = &[
    TB,
    OLD_TB,
    slot("bank_rec", "bank_rec.csv", true, "bank_rec"),
    OPEN,
    SELF,
    PRIOR,
];
const YEAR: &[Slot] = &[
    TB,
    OLD_TB,
    slot(
        "tie_out_matrix",
        "tie_out_matrix.csv",
        true,
        "tie_out_matrix",
    ),
    slot("fs_mapping", "fs_mapping.csv", false, "fs_mapping"),
    OPEN,
    SELF,
    PRIOR,
];

pub(crate) static CONTROL_NAMES: LazyLock<BTreeSet<&'static str>> = LazyLock::new(|| {
    [BAS, MONTH, YEAR]
        .into_iter()
        .flatten()
        .map(|slot| slot.name)
        .collect()
});

/// The names `--require-control` accepts: every slot of every profile.
pub fn control_names() -> BTreeSet<&'static str> {
    CONTROL_NAMES.clone()
}

pub fn slots_for(profile: &str) -> Result<&'static [Slot]> {
    match profile {
        "bas" => Ok(BAS),
        "month_end" => Ok(MONTH),
        "year_end" => Ok(YEAR),
        _ => Err(format!(
            "unknown engagement profile {}; expected one of: bas, month_end, year_end.",
            crate::pytext::repr(profile)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::control_names;

    #[test]
    fn caller_can_edit_its_control_names_without_changing_the_registry() {
        let mut names = control_names();
        assert!(names.remove("trial_balance"));
        assert!(names.insert("not_a_control"));
        assert_eq!(
            control_names().into_iter().collect::<Vec<_>>(),
            [
                "activity_statement",
                "bank_rec",
                "fs_mapping",
                "gst_control_gl",
                "open_items",
                "prior_findings",
                "prior_trial_balance",
                "self_review",
                "tie_out_matrix",
                "trial_balance",
            ]
        );
    }
}
