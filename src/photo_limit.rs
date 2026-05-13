pub const INITIAL_PHOTO_LIMIT: usize = 500;
pub const PHOTO_LIMIT_STEP: usize = 500;
pub const MAX_PHOTO_LIMIT: usize = 5_000;

pub fn next_photo_limit(current: usize) -> usize {
    current
        .saturating_add(PHOTO_LIMIT_STEP)
        .min(MAX_PHOTO_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_photo_limit_increases_by_step() {
        assert_eq!(next_photo_limit(INITIAL_PHOTO_LIMIT), 1_000);
    }

    #[test]
    fn next_photo_limit_stops_at_maximum() {
        assert_eq!(next_photo_limit(4_900), MAX_PHOTO_LIMIT);
        assert_eq!(next_photo_limit(MAX_PHOTO_LIMIT), MAX_PHOTO_LIMIT);
    }
}
