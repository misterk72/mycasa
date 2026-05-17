use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::mpsc::{self, Receiver, Sender},
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
const VIEWER_IMAGE_MARGIN: f32 = 16.0;
const VIEWER_MATTE_PADDING: f32 = 8.0;
const VIEWER_TOOL_BUTTON_WIDTH: f32 = 96.0;
const VIEWER_TOOL_BUTTON_HEIGHT: f32 = 23.0;
const VIEWER_NAV_BUTTON_HEIGHT: f32 = 22.0;
const VIEWER_PRELOAD_CACHE_CAPACITY: usize = 9;
const VIEWER_MAX_PENDING_FULL_LOADS: usize = 4;
#[cfg(test)]
const VIEWER_PANEL_GAP: f32 = 8.0;

#[derive(Clone, Copy)]
pub enum NavigationDirection {
    Previous,
    Next,
}

pub struct ViewerState {
    current: Option<Photo>,
    zoom: f32,
    loaded_photo_id: Option<i64>,
    preview_texture: Option<TextureHandle>,
    full_texture: Option<TextureHandle>,
    sender: Sender<ViewerMessage>,
    receiver: Receiver<ViewerMessage>,
    preloaded_images: HashMap<i64, ColorImage>,
    preload_order: VecDeque<i64>,
    pending_full_loads: HashSet<i64>,
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

impl Default for ViewerState {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            current: None,
            zoom: 1.0,
            loaded_photo_id: None,
            preview_texture: None,
            full_texture: None,
            sender,
            receiver,
            preloaded_images: HashMap::new(),
            preload_order: VecDeque::new(),
            pending_full_loads: HashSet::new(),
            loading: false,
        }
    }
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
        self.loaded_photo_id = None;
        self.preview_texture = None;
        self.full_texture = None;
        self.loading = false;
    }

    pub fn open(&mut self, photo: Photo) {
        self.current = Some(photo);
        self.zoom = 1.0;
        self.loaded_photo_id = None;
        self.preview_texture = None;
        self.full_texture = None;
        self.loading = false;
    }

    pub fn preload_photos(&mut self, ctx: &egui::Context, photos: &[Photo]) {
        let current_id = self.current_photo_id();
        for photo in photos {
            if self.pending_full_loads.len() >= VIEWER_MAX_PENDING_FULL_LOADS {
                break;
            }
            if current_id == Some(photo.id)
                || self.preloaded_images.contains_key(&photo.id)
                || self.pending_full_loads.contains(&photo.id)
            {
                continue;
            }

            self.spawn_full_load(ctx, photo.clone(), false);
        }
    }

    pub fn show_docked(&mut self, ctx: &egui::Context) {
        let Some(photo) = self.current.clone() else {
            return;
        };

        self.poll_loaded(ctx);
        self.ensure_loading(ctx, &photo);

        egui::TopBottomPanel::top("viewer_filmstrip")
            .exact_height(VIEWER_FILMSTRIP_HEIGHT)
            .frame(egui::Frame::default().fill(VIEWER_BG))
            .show(ctx, |ui| {
                apply_viewer_visuals(ui);
                self.show_filmstrip(ui, ui.max_rect());
            });

        egui::SidePanel::left("viewer_tools")
            .resizable(false)
            .exact_width(VIEWER_TOOL_PANEL_WIDTH)
            .frame(egui::Frame::default().fill(VIEWER_PANEL_BG))
            .show(ctx, |ui| {
                apply_viewer_visuals(ui);
                self.show_tool_panel(ui);
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(VIEWER_BG))
            .show(ctx, |ui| {
                apply_viewer_visuals(ui);
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(ui.max_rect())
                        .layout(Layout::top_down(Align::Center)),
                    |ui| {
                        let stage_size = viewer_panel_stage_size(ui.max_rect());
                        self.show_image(ui, stage_size);
                        ui.separator();
                        self.show_metadata(ui, &photo);
                    },
                );
            });
    }

    fn show_filmstrip(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        ui.allocate_rect(rect, egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, VIEWER_BG);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(4.0)), |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add_sized(
                        Vec2::new(100.0, VIEWER_NAV_BUTTON_HEIGHT),
                        viewer_button("← Phototheque"),
                    )
                    .clicked()
                {
                    self.close();
                }
                if ui
                    .add_sized(
                        Vec2::new(92.0, VIEWER_NAV_BUTTON_HEIGHT),
                        viewer_button("▶ Diaporama"),
                    )
                    .clicked()
                {
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
                                .add_sized(viewer_tool_button_size(), viewer_button(*tool))
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
                    if ui
                        .add_sized(
                            Vec2::new(48.0, VIEWER_NAV_BUTTON_HEIGHT),
                            viewer_button("- Zoom"),
                        )
                        .clicked()
                    {
                        self.zoom = adjusted_zoom(self.zoom, -VIEWER_ZOOM_STEP);
                    }
                    ui.label(format!("{:.0}%", self.zoom * 100.0));
                    if ui
                        .add_sized(
                            Vec2::new(54.0, VIEWER_NAV_BUTTON_HEIGHT),
                            viewer_button("+ Zoom"),
                        )
                        .clicked()
                    {
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
        loop {
            match self.receiver.try_recv() {
                Ok(message)
                    if !message_matches_current_photo(self.current_photo_id(), &message) =>
                {
                    let ViewerMessage::Full { id, image } = message else {
                        continue;
                    };
                    self.pending_full_loads.remove(&id);
                    if let Some(image) = image {
                        self.remember_preloaded_image(id, image);
                    }
                }
                Ok(ViewerMessage::Preview { id, image }) => {
                    let texture_name = format!("viewer-preview-{}", id);
                    self.preview_texture =
                        Some(ctx.load_texture(texture_name, image, TextureOptions::LINEAR));
                    ctx.request_repaint();
                }
                Ok(ViewerMessage::Full { id, image }) => {
                    self.pending_full_loads.remove(&id);
                    self.loading = false;
                    self.loaded_photo_id = Some(id);
                    self.full_texture = image.map(|image| {
                        self.remember_preloaded_image(id, image.clone());
                        let texture_name = format!("viewer-photo-{}", id);
                        ctx.load_texture(texture_name, image, TextureOptions::LINEAR)
                    });
                    ctx.request_repaint();
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.loading = false;
                    break;
                }
            }
        }
    }

    fn ensure_loading(&mut self, ctx: &egui::Context, photo: &Photo) {
        if self.loaded_photo_id == Some(photo.id) {
            return;
        }

        if let Some(image) = self.cached_viewer_image(photo.id) {
            let texture_name = format!("viewer-photo-{}", photo.id);
            self.full_texture = Some(ctx.load_texture(texture_name, image, TextureOptions::LINEAR));
            self.loaded_photo_id = Some(photo.id);
            self.loading = false;
            self.pending_full_loads.remove(&photo.id);
            ctx.request_repaint();
            return;
        }

        if self.loading || self.pending_full_loads.contains(&photo.id) {
            self.load_preview_from_cache(ctx, photo);
            self.loading = true;
            return;
        }

        self.spawn_full_load(ctx, photo.clone(), true);
        self.loading = true;
    }

    fn spawn_full_load(&mut self, ctx: &egui::Context, photo: Photo, include_preview: bool) {
        if !self.pending_full_loads.insert(photo.id) {
            return;
        }

        let sender = self.sender.clone();
        let photo = photo.clone();
        let repaint_context = ctx.clone();

        thread::spawn(move || {
            if include_preview && let Some(preview) = load_cached_thumbnail_image(&photo) {
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

    fn remember_preloaded_image(&mut self, id: i64, image: ColorImage) {
        if !self.preloaded_images.contains_key(&id) {
            self.preload_order.push_back(id);
        }
        self.preloaded_images.insert(id, image);
        self.evict_old_preloaded_images();
    }

    fn cached_viewer_image(&mut self, id: i64) -> Option<ColorImage> {
        let image = self.preloaded_images.get(&id).cloned();
        if image.is_some() {
            self.mark_preloaded_image_recent(id);
        }
        image
    }

    fn mark_preloaded_image_recent(&mut self, id: i64) {
        self.preload_order.retain(|cached_id| *cached_id != id);
        self.preload_order.push_back(id);
    }

    fn load_preview_from_cache(&mut self, ctx: &egui::Context, photo: &Photo) {
        if self.visible_texture().is_some() {
            return;
        }

        if let Some(preview) = load_cached_thumbnail_image(photo) {
            let texture_name = format!("viewer-preview-{}", photo.id);
            self.preview_texture =
                Some(ctx.load_texture(texture_name, preview, TextureOptions::LINEAR));
            ctx.request_repaint();
        }
    }

    fn evict_old_preloaded_images(&mut self) {
        while self.preloaded_images.len() > VIEWER_PRELOAD_CACHE_CAPACITY {
            let Some(oldest_id) = self.preload_order.pop_front() else {
                break;
            };
            self.preloaded_images.remove(&oldest_id);
        }
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
            let image_rect = viewer_image_rect(rect, image_size, self.zoom);
            let matte_rect = image_rect.expand(VIEWER_MATTE_PADDING);
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

pub fn viewer_image_rect(canvas_rect: egui::Rect, image_size: Vec2, zoom: f32) -> egui::Rect {
    if image_size.x <= 0.0 || image_size.y <= 0.0 {
        return egui::Rect::from_center_size(canvas_rect.center(), Vec2::ZERO);
    }

    let reserved_margin = (VIEWER_IMAGE_MARGIN + VIEWER_MATTE_PADDING) * 2.0;
    let fit_size = Vec2::new(
        (canvas_rect.width() - reserved_margin).max(1.0),
        (canvas_rect.height() - reserved_margin).max(1.0),
    );
    let fit_scale = (fit_size.x / image_size.x).min(fit_size.y / image_size.y);
    let fitted_size = image_size * fit_scale * zoom;

    egui::Rect::from_center_size(canvas_rect.center(), fitted_size)
}

pub fn viewer_panel_stage_size(panel_rect: egui::Rect) -> Vec2 {
    panel_rect.size()
}

pub fn viewer_tool_button_size() -> Vec2 {
    Vec2::new(VIEWER_TOOL_BUTTON_WIDTH, VIEWER_TOOL_BUTTON_HEIGHT)
}

fn viewer_button(label: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(label).color(Color32::from_rgb(43, 48, 52)))
        .fill(VIEWER_BUTTON_BG)
        .stroke(Stroke::new(1.0, VIEWER_BUTTON_STROKE))
        .corner_radius(2.0)
}

#[cfg(test)]
pub fn viewer_stage_size(total: Vec2) -> Vec2 {
    Vec2::new(
        (total.x - VIEWER_TOOL_PANEL_WIDTH - VIEWER_PANEL_GAP).max(0.0),
        total.y,
    )
}

#[cfg(test)]
pub fn viewer_body_rect(top_left: egui::Pos2, bottom_right: egui::Pos2) -> egui::Rect {
    egui::Rect::from_min_max(
        top_left,
        egui::pos2(
            bottom_right.x.max(top_left.x),
            bottom_right.y.max(top_left.y),
        ),
    )
}

#[cfg(test)]
pub fn viewer_filmstrip_rect(root_rect: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_size(
        root_rect.min,
        Vec2::new(root_rect.width(), VIEWER_FILMSTRIP_HEIGHT),
    )
}

#[cfg(test)]
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
    fn viewer_tool_button_size_matches_two_column_panel() {
        assert_eq!(
            viewer_tool_button_size(),
            Vec2::new(VIEWER_TOOL_BUTTON_WIDTH, VIEWER_TOOL_BUTTON_HEIGHT)
        );
        assert!(viewer_tool_button_size().x * 2.0 < VIEWER_TOOL_PANEL_WIDTH);
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
    fn viewer_image_rect_keeps_margin_for_landscape_images() {
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1680.0, 916.0));
        let image = viewer_image_rect(canvas, Vec2::new(4032.0, 2268.0), 1.0);
        let matte = image.expand(VIEWER_MATTE_PADDING);

        assert!(matte.left() >= canvas.left() + VIEWER_IMAGE_MARGIN - 0.1);
        assert!(matte.right() <= canvas.right() - VIEWER_IMAGE_MARGIN + 0.1);
        assert!(matte.top() > canvas.top());
        assert!(matte.bottom() < canvas.bottom());
    }

    #[test]
    fn viewer_image_rect_keeps_margin_for_portrait_images() {
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1680.0, 916.0));
        let image = viewer_image_rect(canvas, Vec2::new(2268.0, 4032.0), 1.0);
        let matte = image.expand(VIEWER_MATTE_PADDING);

        assert!(matte.top() >= canvas.top() + VIEWER_IMAGE_MARGIN - 0.1);
        assert!(matte.bottom() <= canvas.bottom() - VIEWER_IMAGE_MARGIN + 0.1);
        assert!((image.center().x - canvas.center().x).abs() < 0.1);
        assert!((image.center().y - canvas.center().y).abs() < 0.1);
    }

    #[test]
    fn viewer_image_rect_zoom_can_expand_from_fit_size() {
        let canvas = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1000.0, 800.0));
        let fit = viewer_image_rect(canvas, Vec2::new(1000.0, 500.0), 1.0);
        let zoomed = viewer_image_rect(canvas, Vec2::new(1000.0, 500.0), 2.0);

        assert!(zoomed.width() > fit.width());
        assert_eq!(zoomed.center(), fit.center());
    }

    #[test]
    fn viewer_panel_stage_size_uses_max_rect_size() {
        let panel = egui::Rect::from_min_size(egui::pos2(210.0, 34.0), Vec2::new(1490.0, 900.0));

        assert_eq!(viewer_panel_stage_size(panel), Vec2::new(1490.0, 900.0));
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
        let preview =
            ctx.load_texture("preview", color_image_for_test(255), TextureOptions::LINEAR);
        let full = ctx.load_texture("full", color_image_for_test(255), TextureOptions::LINEAR);
        let mut viewer = ViewerState::default();
        viewer.preview_texture = Some(preview);
        assert!(viewer.visible_texture().is_some());
        viewer.full_texture = Some(full);

        assert_eq!(viewer.visible_texture().unwrap().name(), "full");
    }

    #[test]
    fn viewer_preload_cache_evicts_oldest_images() {
        let mut viewer = ViewerState::default();

        for id in 1..=(VIEWER_PRELOAD_CACHE_CAPACITY as i64 + 1) {
            viewer.remember_preloaded_image(id, color_image_for_test(id as u8));
        }

        assert!(!viewer.preloaded_images.contains_key(&1));
        assert!(viewer.preloaded_images.contains_key(&2));
        assert!(
            viewer
                .preloaded_images
                .contains_key(&(VIEWER_PRELOAD_CACHE_CAPACITY as i64 + 1))
        );
        assert_eq!(viewer.preloaded_images.len(), VIEWER_PRELOAD_CACHE_CAPACITY);
    }

    #[test]
    fn viewer_uses_preloaded_image_without_consuming_cache() {
        let ctx = egui::Context::default();
        let photo = photo_for_test(7);
        let mut viewer = ViewerState::default();
        viewer.remember_preloaded_image(photo.id, color_image_for_test(7));

        viewer.open(photo.clone());
        viewer.ensure_loading(&ctx, &photo);

        assert_eq!(viewer.loaded_photo_id, Some(photo.id));
        assert!(!viewer.loading);
        assert!(viewer.full_texture.is_some());
        assert!(viewer.preloaded_images.contains_key(&photo.id));
        assert!(!viewer.pending_full_loads.contains(&photo.id));
    }

    #[test]
    fn viewer_keeps_current_full_image_in_memory_cache() {
        let ctx = egui::Context::default();
        let photo = photo_for_test(11);
        let mut viewer = ViewerState::default();
        viewer.open(photo.clone());

        viewer
            .sender
            .send(ViewerMessage::Full {
                id: photo.id,
                image: Some(color_image_for_test(11)),
            })
            .unwrap();
        viewer.poll_loaded(&ctx);

        assert_eq!(viewer.loaded_photo_id, Some(photo.id));
        assert!(viewer.full_texture.is_some());
        assert!(viewer.preloaded_images.contains_key(&photo.id));
    }

    #[test]
    fn viewer_preload_skips_current_cached_and_pending_photos() {
        let ctx = egui::Context::default();
        let mut viewer = ViewerState::default();
        let current = photo_for_test(1);
        viewer.open(current.clone());
        viewer.remember_preloaded_image(2, color_image_for_test(2));
        viewer.pending_full_loads.insert(3);

        viewer.preload_photos(
            &ctx,
            &[
                current,
                photo_for_test(2),
                photo_for_test(3),
                photo_for_test(4),
            ],
        );

        assert!(!viewer.pending_full_loads.contains(&1));
        assert!(viewer.pending_full_loads.contains(&3));
        assert!(viewer.pending_full_loads.contains(&4));
    }

    #[test]
    fn viewer_preload_limits_pending_full_loads() {
        let ctx = egui::Context::default();
        let mut viewer = ViewerState::default();
        let photos: Vec<Photo> = (1..=10).map(photo_for_test).collect();

        viewer.preload_photos(&ctx, &photos);

        assert_eq!(
            viewer.pending_full_loads.len(),
            VIEWER_MAX_PENDING_FULL_LOADS
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

    fn color_image_for_test(red: u8) -> ColorImage {
        ColorImage::from_rgba_unmultiplied([1, 1], &[red, 0, 0, 255])
    }
}
