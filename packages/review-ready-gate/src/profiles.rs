// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

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

macro_rules! slot {
    ($name:literal, $extension:literal, $required:literal) => {
        slot!($name, $extension, $required, $name)
    };
    ($name:literal, $extension:literal, $required:literal, $kind:literal) => {
        Slot {
            name: $name,
            filename: concat!($name, $extension),
            required: $required,
            kind: $kind,
        }
    };
}
const OPEN: Slot = slot!("open_items", ".csv", true);
const SELF: Slot = slot!("self_review", ".json", true);
const PRIOR: Slot = slot!("prior_findings", ".csv", false);
const TB: Slot = slot!("trial_balance", ".csv", true);
const OLD_TB: Slot = slot!("prior_trial_balance", ".csv", true, "trial_balance");
const PAYGW: Slot = slot!("paygw_control_gl", ".csv", false);
pub const PROFILE_NAMES: [&str; 4] = ["bas", "ias", "month_end", "year_end"];
const BAS: &[Slot] = &[
    TB,
    slot!("activity_statement", ".csv", true),
    slot!("gst_control_gl", ".csv", true),
    slot!("bank_rec", ".csv", false),
    PAYGW,
    OPEN,
    SELF,
    PRIOR,
];
/// An instalment activity statement: no GST, so no 1A, 1B or GST control file.
const IAS: &[Slot] = &[
    TB,
    slot!("activity_statement", ".csv", true),
    PAYGW,
    OPEN,
    SELF,
    PRIOR,
];
const MONTH: &[Slot] = &[
    TB,
    OLD_TB,
    slot!("bank_rec", ".csv", true),
    OPEN,
    SELF,
    PRIOR,
];
const YEAR: &[Slot] = &[
    TB,
    OLD_TB,
    slot!("bank_rec", ".csv", false),
    slot!("tie_out_matrix", ".csv", true),
    slot!("fs_mapping", ".csv", false),
    OPEN,
    SELF,
    PRIOR,
];

pub(crate) static CONTROL_NAMES: LazyLock<BTreeSet<&'static str>> = LazyLock::new(|| {
    [BAS, IAS, MONTH, YEAR]
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
        "ias" => Ok(IAS),
        "month_end" => Ok(MONTH),
        "year_end" => Ok(YEAR),
        _ => Err(format!(
            "unknown engagement profile {}; expected one of: bas, ias, month_end, year_end.",
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
                "paygw_control_gl",
                "prior_findings",
                "prior_trial_balance",
                "self_review",
                "tie_out_matrix",
                "trial_balance",
            ]
        );
    }
}
