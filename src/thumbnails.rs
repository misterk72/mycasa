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

const MAX_ACTIVE_THUMBNAIL_LOADS: usize = 6;
const THUMBNAIL_EDGE: u32 = 256;

pub enum ThumbnailState<'a> {
    Pending,
    Ready(&'a TextureHandle),
}

enum ThumbnailEntry {
    Loading,
    Ready(TextureHandle),
    Failed,
}

pub struct ThumbnailCache {
    entries: HashMap<i64, ThumbnailEntry>,
    pending: VecDeque<Photo>,
    active_loads: usize,
    cache_dir: Option<PathBuf>,
    sender: Sender<ThumbnailResult>,
    receiver: Receiver<ThumbnailResult>,
}

struct ThumbnailResult {
    id: i64,
    image: Option<ColorImage>,
}

impl ThumbnailCache {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            entries: HashMap::new(),
            pending: VecDeque::new(),
            active_loads: 0,
            cache_dir: thumbnail_cache_dir(),
            sender,
            receiver,
        }
    }

    pub fn state_for(&mut self, ctx: &egui::Context, photo: &Photo) -> ThumbnailState<'_> {
        self.poll_completed(ctx);

        if !photo.path.exists() {
            return ThumbnailState::Pending;
        }

        if !self.entries.contains_key(&photo.id) {
            self.entries.insert(photo.id, ThumbnailEntry::Loading);
            self.pending.push_back(photo.clone());
            self.start_pending_loads(ctx);
        }

        match self.entries.get(&photo.id) {
            Some(ThumbnailEntry::Ready(texture)) => ThumbnailState::Ready(texture),
            Some(ThumbnailEntry::Loading | ThumbnailEntry::Failed) | None => {
                ThumbnailState::Pending
            }
        }
    }

    fn poll_completed(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.receiver.try_recv() {
            self.active_loads = self.active_loads.saturating_sub(1);
            let entry = result.image.map_or(ThumbnailEntry::Failed, |image| {
                let texture_name = format!("photo-thumbnail-{}", result.id);
                ThumbnailEntry::Ready(ctx.load_texture(texture_name, image, TextureOptions::LINEAR))
            });
            self.entries.insert(result.id, entry);
            ctx.request_repaint();
        }

        self.start_pending_loads(ctx);
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
            let image = load_thumbnail_image(&photo, cache_dir);
            let _ = sender.send(ThumbnailResult {
                id: photo.id,
                image,
            });
            ctx.request_repaint();
        });
    }
}

fn load_thumbnail_image(photo: &Photo, cache_dir: Option<PathBuf>) -> Option<ColorImage> {
    let cache_path = cache_dir
        .as_ref()
        .map(|cache_dir| cache_dir.join(cache_file_name(photo)));

    if let Some(cache_path) = &cache_path {
        if cache_path.exists() {
            if let Some(image) = decode_color_image(cache_path) {
                return Some(image);
            }
        }
    }

    let image = image::open(&photo.path)
        .ok()?
        .thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE);

    if let Some(cache_path) = &cache_path {
        if let Some(parent) = cache_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = image.save_with_format(cache_path, ImageFormat::Png);
    }

    Some(dynamic_to_color_image(image))
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
        photo.id,
        photo.modified_at.unwrap_or_default(),
        photo.file_size.unwrap_or_default()
    )
}

fn thumbnail_cache_dir() -> Option<PathBuf> {
    let project_dirs = ProjectDirs::from("org", "MyCasa", "MyCasa")?;
    let cache_dir = project_dirs.cache_dir().join("thumbnails");
    std::fs::create_dir_all(&cache_dir).ok()?;
    Some(cache_dir)
}
