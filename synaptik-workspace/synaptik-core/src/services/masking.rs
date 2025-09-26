use contracts::normalize::for_rules;

/// Normalization shim to keep call-sites concise.
pub fn norm_lower(s: &str) -> String { for_rules(s) }

/// Build a normalized character view of `s` along with original byte spans.
/// Each produced normalized char corresponds to an original (start,end) byte span.
/// Characters removed by normalization emit no span entries.
pub fn normalized_chars_with_spans(s: &str) -> (Vec<char>, Vec<(usize, usize)>) {
    let mut chars = Vec::new();
    let mut spans = Vec::new();
    for (orig_start, ch) in s.char_indices() {
        let orig_end = orig_start + ch.len_utf8();
        let norm_frag = norm_lower(&ch.to_string());
        if norm_frag.is_empty() { continue; }
        for nc in norm_frag.chars() {
            chars.push(nc);
            spans.push((orig_start, orig_end));
        }
    }
    (chars, spans)
}

/// Case-insensitive masking of literal patterns using normalization-aware span mapping.
/// Replaces matches with the literal token "[masked]".
pub fn apply_masks_ci(text: &str, patterns: &[String]) -> String {
    if patterns.is_empty() { return text.to_string(); }
    let mut out = text.to_string();
    const MASK: &str = "[masked]";

    for pat in patterns {
        if pat.is_empty() { continue; }
        let pat_chars: Vec<char> = norm_lower(pat).chars().collect();
        if pat_chars.is_empty() { continue; }

        // Recompute normalized view for current `out` so earlier replacements are visible.
        let (norm_chars, spans) = normalized_chars_with_spans(&out);
        if norm_chars.is_empty() { continue; }
        if pat_chars.len() > norm_chars.len() { continue; }

        // Collect original byte ranges for all matches in this pass.
        let plen = pat_chars.len();
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        let mut i = 0usize;
        while i + plen <= norm_chars.len() {
            let mut ok = true;
            for j in 0..plen {
                if norm_chars[i + j] != pat_chars[j] { ok = false; break; }
            }
            if ok {
                let (s, _) = spans[i];
                let (_, e) = spans[i + plen - 1];
                ranges.push((s, e));
                // Advance by 1 to allow overlapping matches (e.g., pattern "aa" in "aaa").
                i += 1;
            } else {
                i += 1;
            }
        }
        if ranges.is_empty() { continue; }

        // Merge overlapping/adjacent ranges then replace from the end to keep indices stable.
        ranges.sort_by_key(|r| r.0);
        let mut binding: Vec<(usize, usize)> = Vec::new();
        for (s, e) in ranges.into_iter() {
            if let Some(last) = binding.last_mut() {
                if s <= last.1 { last.1 = last.1.max(e); continue; }
            }
            binding.push((s, e));
        }
        for (s, e) in binding.into_iter().rev() {
            if s >= e || e > out.len() { continue; }
            out.replace_range(s..e, MASK);
        }
    }

    out
}

