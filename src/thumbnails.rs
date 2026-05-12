use std::{
    collections::HashMap,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use egui::{ColorImage, TextureHandle, TextureOptions};

use crate::catalog::Photo;

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
            self.spawn_load(photo.clone(), ctx.clone());
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
            let entry = result.image.map_or(ThumbnailEntry::Failed, |image| {
                let texture_name = format!("photo-thumbnail-{}", result.id);
                ThumbnailEntry::Ready(ctx.load_texture(texture_name, image, TextureOptions::LINEAR))
            });
            self.entries.insert(result.id, entry);
            ctx.request_repaint();
        }
    }

    fn spawn_load(&self, photo: Photo, ctx: egui::Context) {
        let sender = self.sender.clone();
        thread::spawn(move || {
            let image = load_thumbnail_image(&photo);
            let _ = sender.send(ThumbnailResult {
                id: photo.id,
                image,
            });
            ctx.request_repaint();
        });
    }
}

fn load_thumbnail_image(photo: &Photo) -> Option<ColorImage> {
    let image = image::open(&photo.path)
        .ok()?
        .thumbnail(256, 256)
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    Some(ColorImage::from_rgba_unmultiplied(size, &pixels))
}
