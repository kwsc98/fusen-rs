pub fn mask_str(value: &str) -> String {
    let len = value.chars().count();
    let hidden = len / 2;
    let visible_prefix = hidden / 2;
    let mut chars = value.chars();
    let mut result = chars.by_ref().take(visible_prefix).collect::<String>();
    result.push_str(&"*".repeat(hidden));
    chars.by_ref().take(hidden).for_each(drop);
    result.extend(chars);
    result
}

pub fn limit_str(value: &str, limit: usize) -> String {
    if value.chars().count() > limit {
        format!("{}..", value.chars().take(limit).collect::<String>())
    } else {
        value.to_owned()
    }
}
