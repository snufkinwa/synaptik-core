/// Minimal three-way bind at line granularity.
/// - If both sides equal → keep
/// - If left == base → take right
/// - If right == base → take left
/// - Else emit conflict block with Git-style markers
pub fn three_way_bind_lines(base: &str, left: &str, right: &str) -> (String, bool) {
    if left == right {
        return (left.to_string(), false);
    }
    if left == base {
        return (right.to_string(), false);
    }
    if right == base {
        return (left.to_string(), false);
    }

    let mut out = String::new();
    out.push_str("<<<<<<< LEFT\n");
    out.push_str(left);
    if !left.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("=======\n");
    out.push_str(right);
    if !right.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(">>>>>>> RIGHT\n");
    (out, true)
}
