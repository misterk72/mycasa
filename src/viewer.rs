use std::{
    sync::mpsc::{self, Receiver},
    thread,
};

use egui::{Color32, ColorImage, RichText, TextureHandle, TextureOptions, Vec2};

use crate::catalog::Photo;

#[derive(Clone, Copy)]
pub enum NavigationDirection {
    Previous,
    Next,
}

#[derive(Default)]
pub struct ViewerState {
    current: Option<Photo>,
    zoom: f32,
    loaded_photo_id: Option<i64>,
    texture: Option<TextureHandle>,
    receiver: Option<Receiver<ViewerResult>>,
    loading: bool,
}

struct ViewerResult {
    id: i64,
    image: Option<ColorImage>,
}

impl ViewerState {
    pub fn current_photo_id(&self) -> Option<i64> {
        self.current.as_ref().map(|photo| photo.id)
    }

    pub fn open(&mut self, photo: Photo) {
        self.current = Some(photo);
        self.zoom = 1.0;
        self.loaded_photo_id = None;
        self.texture = None;
        self.receiver = None;
        self.loading = false;
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let Some(photo) = self.current.clone() else {
            return;
        };

        self.poll_loaded(ctx);
        self.ensure_loading(ctx, &photo);

        let mut open = true;
        egui::Window::new("Viewer")
            .open(&mut open)
            .resizable(true)
            .vscroll(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("-").clicked() {
                        self.zoom = (self.zoom - 0.1).max(0.2);
                    }
                    ui.label(format!("{:.0}%", self.zoom * 100.0));
                    if ui.button("+").clicked() {
                        self.zoom = (self.zoom + 0.1).min(4.0);
                    }
                });

                ui.separator();
                self.show_image(ui);
                ui.separator();
                ui.label(photo.path.display().to_string());
                if let (Some(width), Some(height)) = (photo.width, photo.height) {
                    ui.label(format!("{width} x {height}"));
                }
                if let Some(captured_at) = &photo.captured_at {
                    ui.label(captured_at);
                }
                if photo.picasa_starred {
                    ui.label("Picasa: favori");
                }
                if photo.picasa_face_count > 0 {
                    ui.label(format!("Visages Picasa: {}", photo.picasa_face_count));
                }
                if let Some(caption) = &photo.picasa_caption {
                    ui.label(format!("Legende Picasa: {caption}"));
                }
                if let Some(keywords) = &photo.picasa_keywords {
                    ui.label(format!("Mots-cles Picasa: {keywords}"));
                }
            });

        if !open {
            self.current = None;
        }
    }

    fn poll_loaded(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.receiver else {
            return;
        };

        match receiver.try_recv() {
            Ok(result) => {
                self.loading = false;
                self.receiver = None;
                self.loaded_photo_id = Some(result.id);
                self.texture = result.image.map(|image| {
                    let texture_name = format!("viewer-photo-{}", result.id);
                    ctx.load_texture(texture_name, image, TextureOptions::LINEAR)
                });
                ctx.request_repaint();
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                self.loading = false;
                self.receiver = None;
            }
        }
    }

    fn ensure_loading(&mut self, ctx: &egui::Context, photo: &Photo) {
        if self.loaded_photo_id == Some(photo.id) || self.loading {
            return;
        }

        let (sender, receiver) = mpsc::channel();
        let photo = photo.clone();
        let repaint_context = ctx.clone();
        self.receiver = Some(receiver);
        self.loading = true;

        thread::spawn(move || {
            let image = load_viewer_image(&photo);
            let _ = sender.send(ViewerResult {
                id: photo.id,
                image,
            });
            repaint_context.request_repaint();
        });
    }

    fn show_image(&self, ui: &mut egui::Ui) {
        let available = ui.available_size_before_wrap();
        let viewer_size = Vec2::new(available.x.max(360.0), available.y.clamp(260.0, 680.0));
        let (rect, _) = ui.allocate_exact_size(viewer_size, egui::Sense::drag());

        ui.painter()
            .rect_filled(rect, 6.0, Color32::from_rgb(17, 19, 23));

        if let Some(texture) = &self.texture {
            let image_size = texture.size_vec2();
            let scale = (rect.width() / image_size.x).min(rect.height() / image_size.y) * self.zoom;
            let fitted_size = image_size * scale;
            let image_rect = egui::Rect::from_center_size(rect.center(), fitted_size);
            ui.painter().image(
                texture.id(),
                image_rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            let label = if self.loading {
                "Chargement de l'image"
            } else {
                "Image indisponible"
            };
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                RichText::new(label).color(Color32::LIGHT_GRAY).text(),
                egui::TextStyle::Body.resolve(ui.style()),
                Color32::LIGHT_GRAY,
            );
        }
    }
}

pub fn adjacent_photo_id(
    photos: &[Photo],
    current_id: i64,
    direction: NavigationDirection,
) -> Option<i64> {
    let current_index = photos.iter().position(|photo| photo.id == current_id)?;
    let adjacent_index = match direction {
        NavigationDirection::Previous => current_index.checked_sub(1)?,
        NavigationDirection::Next => current_index + 1,
    };

    photos.get(adjacent_index).map(|photo| photo.id)
}

fn load_viewer_image(photo: &Photo) -> Option<ColorImage> {
    let image = image::open(&photo.path)
        .ok()?
        .thumbnail(2048, 2048)
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    Some(ColorImage::from_rgba_unmultiplied(size, &pixels))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn finds_adjacent_photo_ids() {
        let photos = vec![photo_for_test(10), photo_for_test(20), photo_for_test(30)];

        assert_eq!(
            adjacent_photo_id(&photos, 20, NavigationDirection::Previous),
            Some(10)
        );
        assert_eq!(
            adjacent_photo_id(&photos, 20, NavigationDirection::Next),
            Some(30)
        );
    }

    #[test]
    fn adjacent_photo_id_stops_at_edges() {
        let photos = vec![photo_for_test(10), photo_for_test(20)];

        assert_eq!(
            adjacent_photo_id(&photos, 10, NavigationDirection::Previous),
            None
        );
        assert_eq!(
            adjacent_photo_id(&photos, 20, NavigationDirection::Next),
            None
        );
        assert_eq!(
            adjacent_photo_id(&photos, 99, NavigationDirection::Next),
            None
        );
    }

    fn photo_for_test(id: i64) -> Photo {
        Photo {
            id,
            path: PathBuf::from(format!("/tmp/photo-{id}.jpg")),
            file_size: None,
            modified_at: None,
            width: None,
            height: None,
            captured_at: None,
            picasa_caption: None,
            picasa_keywords: None,
            picasa_starred: false,
            picasa_face_count: 0,
        }
    }
}
