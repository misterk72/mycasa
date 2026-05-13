use std::ops::Range;

pub fn columns_for_width(available_width: f32, tile_width: f32) -> usize {
    (available_width.max(tile_width) / tile_width)
        .floor()
        .max(1.0) as usize
}

pub fn row_count(item_count: usize, columns: usize) -> usize {
    if item_count == 0 {
        return 0;
    }

    item_count.div_ceil(columns.max(1))
}

pub fn item_range_for_row(row_index: usize, columns: usize, item_count: usize) -> Range<usize> {
    let columns = columns.max(1);
    let start = (row_index * columns).min(item_count);
    let end = (start + columns).min(item_count);
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_at_least_one_column() {
        assert_eq!(columns_for_width(20.0, 160.0), 1);
    }

    #[test]
    fn computes_row_count_for_partial_last_row() {
        assert_eq!(row_count(0, 4), 0);
        assert_eq!(row_count(1, 4), 1);
        assert_eq!(row_count(8, 4), 2);
        assert_eq!(row_count(9, 4), 3);
    }

    #[test]
    fn computes_item_range_for_row() {
        assert_eq!(item_range_for_row(0, 4, 10), 0..4);
        assert_eq!(item_range_for_row(1, 4, 10), 4..8);
        assert_eq!(item_range_for_row(2, 4, 10), 8..10);
        assert_eq!(item_range_for_row(3, 4, 10), 10..10);
        assert_eq!(item_range_for_row(0, 0, 2), 0..1);
    }
}
