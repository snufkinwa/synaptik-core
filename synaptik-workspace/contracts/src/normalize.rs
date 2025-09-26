//! Text normalization helpers used for rule matching, stop phrases, and masking.
//!
//! Policy:
//! - Drop control characters.
//! - Drop common zero-width characters (ZWS/ZWNJ/ZWJ/WJ/BOM).
//! - Unicode-aware lowercasing (char.to_lowercase()).
//!
//! Keep this logic single-sourced to avoid drift between evaluators and runtime gates.

/// Normalize text for rule matching and case-insensitive search.
pub fn for_rules(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch.is_control() { continue; }
        for lc in ch.to_lowercase() {
            match lc {
                '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' => {},
                _ => out.push(lc),
            }
        }
    }
    out
}

