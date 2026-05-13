pub fn middle_truncate(text: &str, max_chars: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_chars {
        return text.to_owned();
    }

    if max_chars == 0 {
        return String::new();
    }

    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }

    let kept_chars = max_chars - 3;
    let prefix_len = kept_chars.div_ceil(2);
    let suffix_len = kept_chars / 2;
    let prefix: String = text.chars().take(prefix_len).collect();
    let suffix: String = text
        .chars()
        .rev()
        .take(suffix_len)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    format!("{prefix}...{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_short_text_unchanged() {
        assert_eq!(middle_truncate("photo.jpg", 12), "photo.jpg");
    }

    #[test]
    fn truncates_long_text_in_the_middle() {
        assert_eq!(
            middle_truncate("very-long-photo-name.jpg", 14),
            "very-l...e.jpg"
        );
    }

    #[test]
    fn respects_tiny_limits() {
        assert_eq!(middle_truncate("abcdef", 0), "");
        assert_eq!(middle_truncate("abcdef", 2), "..");
    }
}
