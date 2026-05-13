use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use directories::ProjectDirs;
use egui::{ColorImage, TextureHandle, TextureOptions};
use image::ImageFormat;

use crate::catalog::Photo;

const MAX_ACTIVE_THUMBNAIL_LOADS: usize = 3;
const MAX_READY_THUMBNAILS: usize = 800;
const THUMBNAIL_EDGE: u32 = 256;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ThumbnailMetrics {
    pub cache_hits: usize,
    pub generated: usize,
    pub failed: usize,
    pub evicted: usize,
    pub pending: usize,
    pub active_loads: usize,
    pub ready: usize,
}

pub enum ThumbnailState<'a> {
    Pending,
    Ready(&'a TextureHandle),
    Unavailable,
}

enum ThumbnailEntry {
    Loading,
    Ready(TextureHandle),
    Failed,
}

pub struct ThumbnailCache {
    entries: HashMap<i64, ThumbnailEntry>,
    ready_order: VecDeque<i64>,
    pending: VecDeque<Photo>,
    active_loads: usize,
    cache_dir: Option<PathBuf>,
    metrics: ThumbnailMetrics,
    sender: Sender<ThumbnailResult>,
    receiver: Receiver<ThumbnailResult>,
}

struct ThumbnailResult {
    id: i64,
    image: Option<ColorImage>,
    source: ThumbnailSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ThumbnailSource {
    Cache,
    Generated,
    Failed,
}

impl ThumbnailCache {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            entries: HashMap::new(),
            ready_order: VecDeque::new(),
            pending: VecDeque::new(),
            active_loads: 0,
            cache_dir: thumbnail_cache_dir(),
            metrics: ThumbnailMetrics::default(),
            sender,
            receiver,
        }
    }

    pub fn metrics(&self) -> ThumbnailMetrics {
        ThumbnailMetrics {
            pending: self.pending.len(),
            active_loads: self.active_loads,
            ready: self
                .entries
                .values()
                .filter(|entry| matches!(entry, ThumbnailEntry::Ready(_)))
                .count(),
            ..self.metrics
        }
    }

    pub fn state_for(&mut self, ctx: &egui::Context, photo: &Photo) -> ThumbnailState<'_> {
        self.poll_completed(ctx);

        if !self.entries.contains_key(&photo.id) {
            self.entries.insert(photo.id, ThumbnailEntry::Loading);
            self.queue_visible_request(photo.clone());
            self.start_pending_loads(ctx);
        }

        match self.entries.get(&photo.id) {
            Some(ThumbnailEntry::Ready(texture)) => ThumbnailState::Ready(texture),
            Some(ThumbnailEntry::Failed) => ThumbnailState::Unavailable,
            Some(ThumbnailEntry::Loading) | None => ThumbnailState::Pending,
        }
    }

    fn poll_completed(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.receiver.try_recv() {
            self.active_loads = self.active_loads.saturating_sub(1);
            match result.source {
                ThumbnailSource::Cache => self.metrics.cache_hits += 1,
                ThumbnailSource::Generated => self.metrics.generated += 1,
                ThumbnailSource::Failed => self.metrics.failed += 1,
            }
            let entry = result.image.map_or(ThumbnailEntry::Failed, |image| {
                let texture_name = format!("photo-thumbnail-{}", result.id);
                ThumbnailEntry::Ready(ctx.load_texture(texture_name, image, TextureOptions::LINEAR))
            });
            let is_ready = matches!(entry, ThumbnailEntry::Ready(_));
            self.entries.insert(result.id, entry);
            if is_ready {
                self.ready_order.push_back(result.id);
                self.evict_old_ready_entries();
            }
            ctx.request_repaint();
        }

        self.start_pending_loads(ctx);
    }

    fn queue_visible_request(&mut self, photo: Photo) {
        self.pending.push_front(photo);
    }

    fn evict_old_ready_entries(&mut self) {
        while self.ready_order.len() > MAX_READY_THUMBNAILS {
            let Some(photo_id) = self.ready_order.pop_front() else {
                break;
            };

            if matches!(self.entries.get(&photo_id), Some(ThumbnailEntry::Ready(_))) {
                self.entries.remove(&photo_id);
                self.metrics.evicted += 1;
            }
        }
    }

    fn start_pending_loads(&mut self, ctx: &egui::Context) {
        while self.active_loads < MAX_ACTIVE_THUMBNAIL_LOADS {
            let Some(photo) = self.pending.pop_front() else {
                break;
            };
            self.active_loads += 1;
            self.spawn_load(photo, ctx.clone());
        }
    }

    fn spawn_load(&self, photo: Photo, ctx: egui::Context) {
        let sender = self.sender.clone();
        let cache_dir = self.cache_dir.clone();
        thread::spawn(move || {
            let (image, source) = load_thumbnail_image(&photo, cache_dir);
            let _ = sender.send(ThumbnailResult {
                id: photo.id,
                image,
                source,
            });
            ctx.request_repaint();
        });
    }
}

fn load_thumbnail_image(
    photo: &Photo,
    cache_dir: Option<PathBuf>,
) -> (Option<ColorImage>, ThumbnailSource) {
    let cache_path = cache_dir
        .as_ref()
        .map(|cache_dir| cache_dir.join(cache_file_name(photo)));

    if let Some(cache_path) = &cache_path {
        if cache_path.exists() {
            if let Some(image) = decode_color_image(cache_path) {
                return (Some(image), ThumbnailSource::Cache);
            }
        }
    }

    let Some(image) = image::open(&photo.path)
        .ok()
        .map(|image| image.thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE))
    else {
        return (None, ThumbnailSource::Failed);
    };

    if let Some(cache_path) = &cache_path {
        if let Some(parent) = cache_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = image.save_with_format(cache_path, ImageFormat::Png);
    }

    (
        Some(dynamic_to_color_image(image)),
        ThumbnailSource::Generated,
    )
}

fn decode_color_image(path: &PathBuf) -> Option<ColorImage> {
    image::open(path).ok().map(dynamic_to_color_image)
}

fn dynamic_to_color_image(image: image::DynamicImage) -> ColorImage {
    let image = image.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    ColorImage::from_rgba_unmultiplied(size, &pixels)
}

fn cache_file_name(photo: &Photo) -> String {
    format!(
        "{}-{}-{}.png",
        stable_cache_key(&photo.path.to_string_lossy()),
        photo.modified_at.unwrap_or_default(),
        photo.file_size.unwrap_or_default()
    )
}

fn stable_cache_key(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn thumbnail_cache_dir() -> Option<PathBuf> {
    let project_dirs = ProjectDirs::from("org", "MyCasa", "MyCasa")?;
    let cache_dir = project_dirs.cache_dir().join("thumbnails");
    std::fs::create_dir_all(&cache_dir).ok()?;
    Some(cache_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_file_name_changes_when_source_changes() {
        let mut photo = photo_for_test(1, Some(100), Some(200));
        let original = cache_file_name(&photo);

        photo.file_size = Some(101);
        assert_ne!(original, cache_file_name(&photo));

        photo.file_size = Some(100);
        photo.modified_at = Some(201);
        assert_ne!(original, cache_file_name(&photo));

        photo.modified_at = Some(200);
        photo.path = PathBuf::from("/tmp/other-photo.jpg");
        assert_ne!(original, cache_file_name(&photo));
    }

    #[test]
    fn cache_file_name_is_independent_from_catalog_id() {
        let first = Photo {
            path: PathBuf::from("/tmp/same-photo.jpg"),
            ..photo_for_test(1, Some(100), Some(200))
        };
        let second = Photo {
            path: PathBuf::from("/tmp/same-photo.jpg"),
            ..photo_for_test(99, Some(100), Some(200))
        };

        assert_eq!(cache_file_name(&first), cache_file_name(&second));
    }

    #[test]
    fn stable_cache_key_is_deterministic() {
        assert_eq!(
            stable_cache_key("/photos/IMG_0001.JPG"),
            stable_cache_key("/photos/IMG_0001.JPG")
        );
        assert_ne!(
            stable_cache_key("/photos/IMG_0001.JPG"),
            stable_cache_key("/photos/IMG_0002.JPG")
        );
    }

    #[test]
    fn metrics_reports_queue_and_ready_counts() {
        let mut cache = ThumbnailCache::new();
        cache
            .pending
            .push_back(photo_for_test(1, Some(100), Some(200)));
        cache.active_loads = 2;
        cache.metrics.cache_hits = 3;
        cache.metrics.evicted = 4;
        cache.entries.insert(1, ThumbnailEntry::Failed);

        let metrics = cache.metrics();

        assert_eq!(metrics.pending, 1);
        assert_eq!(metrics.active_loads, 2);
        assert_eq!(metrics.cache_hits, 3);
        assert_eq!(metrics.evicted, 4);
        assert_eq!(metrics.ready, 0);
    }

    #[test]
    fn visible_requests_are_prioritized() {
        let mut cache = ThumbnailCache::new();

        cache.queue_visible_request(photo_for_test(1, Some(100), Some(200)));
        cache.queue_visible_request(photo_for_test(2, Some(100), Some(200)));

        assert_eq!(cache.pending.pop_front().map(|photo| photo.id), Some(2));
        assert_eq!(cache.pending.pop_front().map(|photo| photo.id), Some(1));
    }

    #[test]
    fn evicts_old_ready_entries_when_memory_cache_is_full() {
        let mut cache = ThumbnailCache::new();
        for id in 0..=MAX_READY_THUMBNAILS {
            cache.entries.insert(id as i64, ThumbnailEntry::Failed);
            cache.ready_order.push_back(id as i64);
        }
        cache.entries.insert(0, ThumbnailEntry::Failed);

        cache.evict_old_ready_entries();

        assert_eq!(cache.metrics.evicted, 0);
        assert_eq!(cache.entries.len(), MAX_READY_THUMBNAILS + 1);

        cache.entries.clear();
        cache.ready_order.clear();
        for id in 0..=MAX_READY_THUMBNAILS {
            cache
                .entries
                .insert(id as i64, ThumbnailEntry::Ready(dummy_texture()));
            cache.ready_order.push_back(id as i64);
        }

        cache.evict_old_ready_entries();

        assert_eq!(cache.metrics.evicted, 1);
        assert!(!cache.entries.contains_key(&0));
        assert_eq!(
            cache
                .entries
                .values()
                .filter(|entry| matches!(entry, ThumbnailEntry::Ready(_)))
                .count(),
            MAX_READY_THUMBNAILS
        );
    }

    #[test]
    fn load_thumbnail_uses_disk_cache_without_original() {
        let root =
            std::env::temp_dir().join(format!("mycasa-thumb-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let original_path = root.join("source.png");
        image::RgbaImage::from_pixel(32, 24, image::Rgba([40, 80, 120, 255]))
            .save_with_format(&original_path, ImageFormat::Png)
            .unwrap();

        let photo = Photo {
            path: original_path.clone(),
            ..photo_for_test(7, Some(42), Some(123))
        };
        let cache_dir = root.join("cache");

        let (generated_image, generated_source) =
            load_thumbnail_image(&photo, Some(cache_dir.clone()));
        assert!(generated_image.is_some());
        assert_eq!(generated_source, ThumbnailSource::Generated);
        assert!(cache_dir.join(cache_file_name(&photo)).exists());

        std::fs::remove_file(&original_path).unwrap();

        let (cached_image, cached_source) = load_thumbnail_image(&photo, Some(cache_dir));
        assert!(cached_image.is_some());
        assert_eq!(cached_source, ThumbnailSource::Cache);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn load_thumbnail_fails_when_original_and_cache_are_missing() {
        let root =
            std::env::temp_dir().join(format!("mycasa-thumb-missing-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let photo = Photo {
            path: root.join("missing.png"),
            ..photo_for_test(8, Some(1), Some(2))
        };

        let (image, source) = load_thumbnail_image(&photo, Some(root.join("cache")));

        assert!(image.is_none());
        assert_eq!(source, ThumbnailSource::Failed);
    }

    fn dummy_texture() -> TextureHandle {
        let ctx = egui::Context::default();
        ctx.load_texture(
            "dummy-thumbnail",
            ColorImage::from_rgba_unmultiplied([1, 1], &[255, 255, 255, 255]),
            TextureOptions::LINEAR,
        )
    }

    fn photo_for_test(id: i64, file_size: Option<u64>, modified_at: Option<i64>) -> Photo {
        Photo {
            id,
            path: PathBuf::from(format!("/tmp/photo-{id}.jpg")),
            file_size,
            modified_at,
            width: Some(800),
            height: Some(600),
            captured_at: None,
            picasa_caption: None,
            picasa_keywords: None,
            picasa_starred: false,
            picasa_face_count: 0,
        }
    }
}
