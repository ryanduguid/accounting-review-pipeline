// Copyright 2026 Ryan Duguid
// SPDX-License-Identifier: LicenseRef-Duguid-Public-Attribution-1.0

use monthly_close_control_plane::pytext;

#[test]
fn lowercase_preserves_python_unicode_context_and_expansion() {
    for (input, expected) in [
        ("", ""),
        ("AbC", "abc"),
        ("Σ", "σ"),
        ("ΟΣ", "ος"),
        ("ΟΣΑ", "οσα"),
        ("A\u{301}Σ\u{301}", "a\u{301}ς\u{301}"),
        ("AΣ\u{301}B", "aσ\u{301}b"),
        ("A Σ", "a σ"),
        ("AΣ B", "aς b"),
        ("İ", "i\u{307}"),
    ] {
        assert_eq!(pytext::lower(input), expected, "{input:?}");
    }
}
