use egui::Color32;

use crate::catalog::Photo;

pub enum ThumbnailState {
    Pending,
    Ready(Color32),
}

pub struct ThumbnailCache;

impl ThumbnailCache {
    pub fn new() -> Self {
        Self
    }

    pub fn state_for(&self, photo: &Photo) -> ThumbnailState {
        if !photo.path.exists() {
            return ThumbnailState::Pending;
        }

        let mut hash = photo.id as u64;
        for byte in photo.path.to_string_lossy().bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(byte as u64);
        }

        let red = 72 + (hash & 0x3f) as u8;
        let green = 82 + ((hash >> 8) & 0x3f) as u8;
        let blue = 92 + ((hash >> 16) & 0x3f) as u8;
        ThumbnailState::Ready(Color32::from_rgb(red, green, blue))
    }
}
