// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: AGPL-3.0-or-later
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

//! argparse's usage line for a parser of optional arguments only, wrapped as
//! `HelpFormatter._format_usage` wraps it at the default 80 columns.
//!
//! The setup and utility workflow examples each keep an identical copy of
//! this file, as the standalone crates do for their other support modules.

/// argparse's text width at its default 80 columns.
const TEXT_WIDTH: usize = 78;

/// `_format_usage`'s `get_lines`: wrap parts under `indent`, the first line
/// starting `first_len` characters in.
fn usage_lines(parts: &[&str], indent: &str, first_len: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line: Vec<&str> = Vec::new();
    let mut line_len = first_len - 1;
    for part in parts {
        let part_len = part.chars().count();
        if line_len + 1 + part_len > TEXT_WIDTH && !line.is_empty() {
            lines.push(line.join(" "));
            line.clear();
            line_len = indent.len() - 1;
        }
        line.push(part);
        line_len += part_len + 1;
    }
    if !line.is_empty() {
        lines.push(line.join(" "));
    }
    lines
}

/// `usage: {prog} {options}`, on one line when it fits, else wrapped under
/// the program name, or under `usage: ` when the name is long.
pub fn usage_line(prog: &str, options: &[&str]) -> String {
    let prefix = "usage: ";
    let prog_len = prog.chars().count();
    let single = format!("{prog} {}", options.join(" "));
    if prefix.len() + single.chars().count() <= TEXT_WIDTH {
        return format!("{prefix}{single}");
    }
    if (prefix.len() + prog_len) as f64 <= 0.75 * TEXT_WIDTH as f64 {
        let indent = " ".repeat(prefix.len() + prog_len + 1);
        let mut parts = vec![prog];
        parts.extend(options);
        let lines = usage_lines(&parts, &indent, prefix.len());
        return format!("{prefix}{}", lines.join(&format!("\n{indent}")));
    }
    let indent = " ".repeat(prefix.len());
    let lines = usage_lines(options, &indent, indent.len());
    let body = lines.join(&format!("\n{indent}"));
    format!("{prefix}{prog}\n{indent}{body}")
}
