pub const INITIAL_PHOTO_LIMIT: usize = 500;
pub const PHOTO_LIMIT_STEP: usize = 500;
pub const AUTO_EXTEND_SCROLL_THRESHOLD: f32 = 900.0;
pub const PHOTO_WINDOW_MAX_COUNT: usize = 3_000;
pub const PHOTO_WINDOW_RETAIN_COUNT: usize = 2_000;

pub fn next_photo_limit(current: usize) -> usize {
    current.saturating_add(PHOTO_LIMIT_STEP)
}

pub fn should_extend_photo_window(
    scroll_offset: f32,
    max_scroll_offset: f32,
    loaded_photos: usize,
    all_photos_loaded: bool,
) -> bool {
    if all_photos_loaded || loaded_photos == 0 || max_scroll_offset <= 0.0 {
        return false;
    }

    max_scroll_offset - scroll_offset <= AUTO_EXTEND_SCROLL_THRESHOLD
}

pub fn photo_window_trim_count(loaded_photos: usize) -> usize {
    if loaded_photos <= PHOTO_WINDOW_MAX_COUNT {
        return 0;
    }

    loaded_photos.saturating_sub(PHOTO_WINDOW_RETAIN_COUNT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_photo_limit_increases_by_step() {
        assert_eq!(next_photo_limit(INITIAL_PHOTO_LIMIT), 1_000);
    }

    #[test]
    fn next_photo_limit_has_no_user_visible_cap() {
        assert_eq!(next_photo_limit(5_000), 5_500);
    }

    #[test]
    fn should_extend_photo_window_near_scroll_bottom() {
        assert!(should_extend_photo_window(4_200.0, 5_000.0, 500, false));
    }

    #[test]
    fn should_not_extend_photo_window_far_from_bottom_or_when_done() {
        assert!(!should_extend_photo_window(2_000.0, 5_000.0, 500, false));
        assert!(!should_extend_photo_window(4_500.0, 5_000.0, 500, true));
        assert!(!should_extend_photo_window(0.0, 0.0, 500, false));
        assert!(!should_extend_photo_window(0.0, 5_000.0, 0, false));
    }

    #[test]
    fn photo_window_trim_count_keeps_window_under_memory_budget() {
        assert_eq!(photo_window_trim_count(PHOTO_WINDOW_MAX_COUNT), 0);
        assert_eq!(photo_window_trim_count(PHOTO_WINDOW_MAX_COUNT + 1), 1_001);
        assert_eq!(photo_window_trim_count(3_500), 1_500);
    }
}
