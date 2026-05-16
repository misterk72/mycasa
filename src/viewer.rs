use std::{
    sync::mpsc::{self, Receiver},
    thread,
};

use egui::{
    Align, Color32, ColorImage, Layout, RichText, Stroke, TextureHandle, TextureOptions, Vec2,
};

use crate::catalog::Photo;
use crate::thumbnails::load_cached_thumbnail_image;

#[cfg(test)]
const VIEWER_MIN_SIZE: Vec2 = Vec2::new(980.0, 680.0);
#[cfg(test)]
const VIEWER_MAX_SIZE: Vec2 = Vec2::new(1280.0, 880.0);
const VIEWER_TOOL_PANEL_WIDTH: f32 = 210.0;
const VIEWER_FILMSTRIP_HEIGHT: f32 = 34.0;
const VIEWER_BG: Color32 = Color32::from_rgb(224, 226, 229);
const VIEWER_PANEL_BG: Color32 = Color32::from_rgb(238, 240, 244);
const VIEWER_CANVAS_BG: Color32 = Color32::from_rgb(154, 154, 154);
const VIEWER_BUTTON_BG: Color32 = Color32::from_rgb(244, 246, 249);
const VIEWER_BUTTON_HOVER_BG: Color32 = Color32::from_rgb(230, 238, 249);
const VIEWER_BUTTON_STROKE: Color32 = Color32::from_rgb(168, 176, 184);
const VIEWER_BLUE: Color32 = Color32::from_rgb(86, 132, 199);
const VIEWER_MIN_ZOOM: f32 = 0.2;
const VIEWER_MAX_ZOOM: f32 = 4.0;
const VIEWER_ZOOM_STEP: f32 = 0.1;
const VIEWER_IMAGE_MAX_EDGE: u32 = 1600;
const VIEWER_METADATA_HEIGHT: f32 = 44.0;
const VIEWER_MIN_CANVAS_HEIGHT: f32 = 240.0;
const VIEWER_MIN_CANVAS_WIDTH: f32 = 680.0;
const VIEWER_PANEL_GAP: f32 = 8.0;

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
    preview_texture: Option<TextureHandle>,
    full_texture: Option<TextureHandle>,
    receiver: Option<Receiver<ViewerMessage>>,
    loading: bool,
}

enum ViewerMessage {
    Preview { id: i64, image: ColorImage },
    Full { id: i64, image: Option<ColorImage> },
}

impl ViewerMessage {
    fn id(&self) -> i64 {
        match self {
            ViewerMessage::Preview { id, .. } | ViewerMessage::Full { id, .. } => *id,
        }
    }
}

fn message_matches_current_photo(current_photo_id: Option<i64>, message: &ViewerMessage) -> bool {
    current_photo_id == Some(message.id())
}

impl ViewerState {
    pub fn is_open(&self) -> bool {
        self.current.is_some()
    }

    pub fn current_photo_id(&self) -> Option<i64> {
        self.current.as_ref().map(|photo| photo.id)
    }

    pub fn close(&mut self) {
        self.current = None;
        self.receiver = None;
        self.loading = false;
    }

    pub fn open(&mut self, photo: Photo) {
        self.current = Some(photo);
        self.zoom = 1.0;
        self.loaded_photo_id = None;
        self.preview_texture = None;
        self.full_texture = None;
        self.receiver = None;
        self.loading = false;
    }

    pub fn show_embedded_in_rect(
        &mut self,
        ctx: &egui::Context,
        ui: &mut egui::Ui,
        root_rect: egui::Rect,
    ) {
        let Some(photo) = self.current.clone() else {
            return;
        };

        apply_viewer_visuals(ui);
        self.poll_loaded(ctx);
        self.ensure_loading(ctx, &photo);
        self.show_contents(ui, &photo, root_rect);
    }

    fn show_contents(&mut self, ui: &mut egui::Ui, photo: &Photo, root_rect: egui::Rect) {
        ui.allocate_rect(root_rect, egui::Sense::hover());
        ui.painter().rect_filled(root_rect, 0.0, VIEWER_BG);
        let filmstrip_rect = viewer_filmstrip_rect(root_rect);
        self.show_filmstrip(ui, filmstrip_rect);

        let body_rect = viewer_body_rect(
            egui::pos2(root_rect.left(), filmstrip_rect.bottom() + 1.0),
            root_rect.right_bottom(),
        );
        ui.allocate_rect(body_rect, egui::Sense::hover());
        let body_size = body_rect.size();
        let panel_rect = egui::Rect::from_min_size(
            body_rect.min,
            Vec2::new(VIEWER_TOOL_PANEL_WIDTH, body_rect.height()),
        );
        let stage_size = viewer_stage_size(body_size);
        let stage_rect = egui::Rect::from_min_size(
            egui::pos2(panel_rect.right() + VIEWER_PANEL_GAP, body_rect.top()),
            stage_size,
        );

        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(panel_rect)
                .layout(Layout::top_down(Align::Min)),
            |ui| self.show_tool_panel(ui),
        );
        ui.painter().line_segment(
            [
                egui::pos2(panel_rect.right() + 1.0, body_rect.top()),
                egui::pos2(panel_rect.right() + 1.0, body_rect.bottom()),
            ],
            Stroke::new(1.0, Color32::from_rgb(154, 160, 166)),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(stage_rect)
                .layout(Layout::top_down(Align::Center)),
            |ui| {
                self.show_image(ui, stage_rect.size());
                ui.separator();
                self.show_metadata(ui, photo);
            },
        );
    }

    fn show_filmstrip(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        ui.allocate_rect(rect, egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, VIEWER_BG);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(4.0)), |ui| {
            ui.horizontal(|ui| {
                if ui.button("← Phototheque").clicked() {
                    self.close();
                }
                if ui.button("▶ Diaporama").clicked() {
                    self.zoom = 1.0;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("A+  A-  A/A").color(Color32::from_gray(70)));
                    ui.add_space(12.0);
                    ui.label(RichText::new("◀  ▣ ▣ ▣ ▣  ▶").color(Color32::from_gray(65)));
                });
            });
        });
    }

    fn show_tool_panel(&mut self, ui: &mut egui::Ui) {
        ui.set_width(VIEWER_TOOL_PANEL_WIDTH);
        let panel_rect = ui.max_rect();
        ui.painter().rect_filled(panel_rect, 0.0, VIEWER_PANEL_BG);
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(panel_rect.shrink2(Vec2::new(8.0, 6.0))),
            |ui| {
                ui.horizontal(|ui| {
                    let _ =
                        ui.selectable_label(true, RichText::new("Ret. simples").color(VIEWER_BLUE));
                    let _ = ui.selectable_label(false, "Reglages");
                    let _ = ui.selectable_label(false, "Effets");
                });
                ui.separator();
                ui.label(RichText::new("Retouches courantes").strong());
                let tools = [
                    "Recadrer",
                    "Redresser",
                    "Yeux rouges",
                    "J'ai de la chance",
                    "Contraste auto",
                    "Couleur auto",
                    "Retoucher",
                    "Texte",
                ];
                egui::Grid::new("viewer-basic-tools")
                    .num_columns(2)
                    .spacing(Vec2::new(5.0, 5.0))
                    .show(ui, |ui| {
                        for (index, tool) in tools.iter().enumerate() {
                            if ui
                                .add_sized(Vec2::new(96.0, 23.0), egui::Button::new(*tool))
                                .clicked()
                            {
                                self.zoom = 1.0;
                            }
                            if index % 2 == 1 {
                                ui.end_row();
                            }
                        }
                    });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("- Zoom").clicked() {
                        self.zoom = adjusted_zoom(self.zoom, -VIEWER_ZOOM_STEP);
                    }
                    ui.label(format!("{:.0}%", self.zoom * 100.0));
                    if ui.button("+ Zoom").clicked() {
                        self.zoom = adjusted_zoom(self.zoom, VIEWER_ZOOM_STEP);
                    }
                });
                ui.add_space(10.0);
                ui.label(RichText::new("Histogramme et infos").color(Color32::from_gray(95)));
            },
        );
    }

    fn show_metadata(&self, ui: &mut egui::Ui, photo: &Photo) {
        let available = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(Vec2::new(available, 36.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_rgb(232, 235, 238));
        ui.painter().line_segment(
            [rect.left_top(), rect.right_top()],
            egui::Stroke::new(1.0, Color32::from_rgb(84, 126, 157)),
        );
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(6.0, 5.0))),
            |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(photo.path.display().to_string())
                            .color(Color32::from_gray(85)),
                    );
                    if let (Some(width), Some(height)) = (photo.width, photo.height) {
                        ui.label(
                            RichText::new(format!("{width} x {height}"))
                                .color(Color32::from_gray(85)),
                        );
                    }
                    if let Some(captured_at) = &photo.captured_at {
                        ui.label(RichText::new(captured_at).color(Color32::from_gray(85)));
                    }
                    if photo.picasa_starred {
                        ui.label(RichText::new("Picasa: favori").color(Color32::from_gray(85)));
                    }
                    if photo.picasa_face_count > 0 {
                        ui.label(
                            RichText::new(format!("Visages Picasa: {}", photo.picasa_face_count))
                                .color(Color32::from_gray(85)),
                        );
                    }
                });
            },
        );
        if let Some(caption) = &photo.picasa_caption {
            ui.label(format!("Legende Picasa: {caption}"));
        }
        if let Some(keywords) = &photo.picasa_keywords {
            ui.label(format!("Mots-cles Picasa: {keywords}"));
        }
    }

    fn poll_loaded(&mut self, ctx: &egui::Context) {
        let Some(receiver) = &self.receiver else {
            return;
        };

        loop {
            match receiver.try_recv() {
                Ok(message)
                    if !message_matches_current_photo(self.current_photo_id(), &message) =>
                {
                    continue;
                }
                Ok(ViewerMessage::Preview { id, image }) => {
                    let texture_name = format!("viewer-preview-{}", id);
                    self.preview_texture =
                        Some(ctx.load_texture(texture_name, image, TextureOptions::LINEAR));
                    ctx.request_repaint();
                }
                Ok(ViewerMessage::Full { id, image }) => {
                    self.loading = false;
                    self.receiver = None;
                    self.loaded_photo_id = Some(id);
                    self.full_texture = image.map(|image| {
                        let texture_name = format!("viewer-photo-{}", id);
                        ctx.load_texture(texture_name, image, TextureOptions::LINEAR)
                    });
                    ctx.request_repaint();
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.loading = false;
                    self.receiver = None;
                    break;
                }
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
            if let Some(preview) = load_cached_thumbnail_image(&photo) {
                let _ = sender.send(ViewerMessage::Preview {
                    id: photo.id,
                    image: preview,
                });
                repaint_context.request_repaint();
            }

            let image = load_viewer_image(&photo);
            let _ = sender.send(ViewerMessage::Full {
                id: photo.id,
                image,
            });
            repaint_context.request_repaint();
        });
    }

    fn visible_texture(&self) -> Option<&TextureHandle> {
        self.full_texture.as_ref().or(self.preview_texture.as_ref())
    }

    fn show_image(&self, ui: &mut egui::Ui, stage_size: Vec2) {
        let viewer_size = viewer_canvas_size(stage_size);
        let (rect, _) = ui.allocate_exact_size(viewer_size, egui::Sense::drag());

        ui.painter().rect_filled(rect, 0.0, VIEWER_CANVAS_BG);

        if let Some(texture) = self.visible_texture() {
            let image_size = texture.size_vec2();
            let scale = (rect.width() / image_size.x).min(rect.height() / image_size.y) * self.zoom;
            let fitted_size = image_size * scale;
            let image_rect = egui::Rect::from_center_size(rect.center(), fitted_size);
            let matte_rect = image_rect.expand(8.0);
            ui.painter()
                .rect_filled(matte_rect, 0.0, Color32::from_rgb(224, 224, 220));
            ui.painter().rect_stroke(
                matte_rect,
                0.0,
                egui::Stroke::new(1.0, Color32::from_rgb(118, 118, 118)),
                egui::StrokeKind::Inside,
            );
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

pub fn adjusted_zoom(current: f32, delta: f32) -> f32 {
    (current + delta).clamp(VIEWER_MIN_ZOOM, VIEWER_MAX_ZOOM)
}

pub fn viewer_canvas_size(available: Vec2) -> Vec2 {
    let height = (available.y - VIEWER_METADATA_HEIGHT)
        .max(VIEWER_MIN_CANVAS_HEIGHT)
        .min(available.y.max(VIEWER_MIN_CANVAS_HEIGHT));

    Vec2::new(available.x.max(VIEWER_MIN_CANVAS_WIDTH), height)
}

pub fn viewer_stage_size(total: Vec2) -> Vec2 {
    Vec2::new(
        (total.x - VIEWER_TOOL_PANEL_WIDTH - VIEWER_PANEL_GAP).max(0.0),
        total.y,
    )
}

pub fn viewer_body_rect(top_left: egui::Pos2, bottom_right: egui::Pos2) -> egui::Rect {
    egui::Rect::from_min_max(
        top_left,
        egui::pos2(
            bottom_right.x.max(top_left.x),
            bottom_right.y.max(top_left.y),
        ),
    )
}

pub fn viewer_filmstrip_rect(root_rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(
        root_rect.min,
        Vec2::new(root_rect.width(), VIEWER_FILMSTRIP_HEIGHT),
    )
}

pub fn viewer_root_size(content_rect: egui::Rect) -> Vec2 {
    content_rect.size()
}

fn apply_viewer_visuals(ui: &mut egui::Ui) {
    let visuals = ui.visuals_mut();
    visuals.override_text_color = Some(Color32::from_rgb(54, 58, 62));
    visuals.widgets.inactive.bg_fill = VIEWER_BUTTON_BG;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(54, 58, 62));
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, VIEWER_BUTTON_STROKE);
    visuals.widgets.hovered.bg_fill = VIEWER_BUTTON_HOVER_BG;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::from_rgb(40, 44, 48));
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, VIEWER_BLUE);
    visuals.widgets.active.bg_fill = Color32::from_rgb(211, 225, 244);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::from_rgb(30, 42, 54));
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, VIEWER_BLUE);
}

#[cfg(test)]
pub fn viewer_window_size(width: Option<u32>, height: Option<u32>) -> Vec2 {
    let Some(width) = width.filter(|value| *value > 0) else {
        return Vec2::new(1180.0, 800.0);
    };
    let Some(height) = height.filter(|value| *value > 0) else {
        return Vec2::new(1180.0, 800.0);
    };

    let aspect = width as f32 / height as f32;
    let mut size = if aspect >= 1.0 {
        Vec2::new(VIEWER_MAX_SIZE.x, VIEWER_MAX_SIZE.x / aspect + 120.0)
    } else {
        Vec2::new((VIEWER_MAX_SIZE.y - 120.0) * aspect, VIEWER_MAX_SIZE.y)
    };

    size.x = size.x.clamp(VIEWER_MIN_SIZE.x, VIEWER_MAX_SIZE.x);
    size.y = size.y.clamp(VIEWER_MIN_SIZE.y, VIEWER_MAX_SIZE.y);
    size
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
        .thumbnail(VIEWER_IMAGE_MAX_EDGE, VIEWER_IMAGE_MAX_EDGE)
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
    fn viewer_tool_panel_width_matches_reference_layout() {
        assert_eq!(VIEWER_TOOL_PANEL_WIDTH, 210.0);
    }

    #[test]
    fn close_clears_current_photo() {
        let mut viewer = ViewerState::default();
        viewer.open(photo_for_test(42));

        viewer.close();

        assert_eq!(viewer.current_photo_id(), None);
        assert!(!viewer.is_open());
    }

    #[test]
    fn adjusted_zoom_is_bounded() {
        assert_eq!(adjusted_zoom(1.0, 0.1), 1.1);
        assert_eq!(adjusted_zoom(0.2, -0.1), VIEWER_MIN_ZOOM);
        assert_eq!(adjusted_zoom(4.0, 0.1), VIEWER_MAX_ZOOM);
    }

    #[test]
    fn viewer_canvas_keeps_room_for_metadata_bar() {
        let large = viewer_canvas_size(Vec2::new(1200.0, 860.0));
        let small = viewer_canvas_size(Vec2::new(640.0, 400.0));

        assert_eq!(large, Vec2::new(1200.0, 816.0));
        assert_eq!(small, Vec2::new(680.0, 356.0));
    }

    #[test]
    fn viewer_canvas_uses_full_stage_width() {
        let canvas = viewer_canvas_size(Vec2::new(1680.0, 960.0));

        assert_eq!(canvas, Vec2::new(1680.0, 916.0));
    }

    #[test]
    fn viewer_stage_uses_remaining_width_after_tool_panel() {
        let stage = viewer_stage_size(Vec2::new(1800.0, 900.0));
        let narrow = viewer_stage_size(Vec2::new(180.0, 900.0));

        assert_eq!(
            stage,
            Vec2::new(1800.0 - VIEWER_TOOL_PANEL_WIDTH - VIEWER_PANEL_GAP, 900.0)
        );
        assert_eq!(narrow, Vec2::new(0.0, 900.0));
    }

    #[test]
    fn viewer_body_rect_uses_full_panel_bounds() {
        let body = viewer_body_rect(egui::pos2(0.0, 70.0), egui::pos2(1920.0, 1040.0));

        assert_eq!(body.size(), Vec2::new(1920.0, 970.0));
    }

    #[test]
    fn viewer_filmstrip_spans_root_width() {
        let root = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(1920.0, 1040.0));
        let filmstrip = viewer_filmstrip_rect(root);

        assert_eq!(filmstrip.width(), 1920.0);
        assert_eq!(filmstrip.height(), VIEWER_FILMSTRIP_HEIGHT);
    }

    #[test]
    fn viewer_root_size_matches_content_rect() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(1920.0, 1040.0));

        assert_eq!(viewer_root_size(rect), Vec2::new(1920.0, 1040.0));
    }

    #[test]
    fn viewer_window_size_follows_image_orientation() {
        let landscape = viewer_window_size(Some(4000), Some(3000));
        let portrait = viewer_window_size(Some(2000), Some(3000));
        let unknown = viewer_window_size(None, None);

        assert!(landscape.x > landscape.y);
        assert!(portrait.y >= VIEWER_MIN_SIZE.y);
        assert!(portrait.x >= VIEWER_MIN_SIZE.x);
        assert_eq!(unknown, Vec2::new(1180.0, 800.0));
        assert!(landscape.x <= VIEWER_MAX_SIZE.x);
        assert!(portrait.y <= VIEWER_MAX_SIZE.y);
    }

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
    fn ignores_messages_for_previous_photo() {
        let message = ViewerMessage::Full { id: 2, image: None };

        assert!(message_matches_current_photo(Some(2), &message));
        assert!(!message_matches_current_photo(Some(1), &message));
        assert!(!message_matches_current_photo(None, &message));
    }

    #[test]
    fn viewer_prefers_full_texture_over_preview() {
        let ctx = egui::Context::default();
        let preview = ctx.load_texture(
            "preview",
            ColorImage::from_rgba_unmultiplied([1, 1], &[255, 255, 255, 255]),
            TextureOptions::LINEAR,
        );
        let full = ctx.load_texture(
            "full",
            ColorImage::from_rgba_unmultiplied([1, 1], &[255, 255, 255, 255]),
            TextureOptions::LINEAR,
        );
        let mut viewer = ViewerState::default();
        viewer.preview_texture = Some(preview);
        assert!(viewer.visible_texture().is_some());
        viewer.full_texture = Some(full);

        assert_eq!(viewer.visible_texture().unwrap().name(), "full");
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
