use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use directories::ProjectDirs;
use egui::{
    Align, Color32, ColorImage, Layout, RichText, Stroke, TextureHandle, TextureOptions, Vec2,
};
use image::{ImageFormat, RgbaImage};

use crate::catalog::Photo;
use crate::thumbnails::{ThumbnailCache, ThumbnailState, load_cached_thumbnail_image};

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
const VIEWER_FILMSTRIP_RADIUS: usize = 4;
const VIEWER_FILMSTRIP_THUMB_SIZE: f32 = 28.0;
const VIEWER_FILMSTRIP_THUMB_GAP: f32 = 4.0;
const VIEWER_PRELOAD_CACHE_CAPACITY: usize = 9;
const VIEWER_MAX_PENDING_FULL_LOADS: usize = 4;
const VIEWER_DISK_CACHE_DIR: &str = "viewer";
#[cfg(test)]
const VIEWER_PANEL_GAP: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationDirection {
    Previous,
    Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewerNavigationRequest {
    Direction(NavigationDirection),
    Photo(i64),
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
    navigation_request: Option<ViewerNavigationRequest>,
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
            navigation_request: None,
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

    pub fn take_navigation_request(&mut self) -> Option<ViewerNavigationRequest> {
        self.navigation_request.take()
    }

    pub fn can_preload_around_current(&self) -> bool {
        let Some(current_id) = self.current_photo_id() else {
            return false;
        };

        self.loaded_photo_id == Some(current_id)
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

    pub fn poll_background_results(&mut self) {
        while let Ok(message) = self.receiver.try_recv() {
            let ViewerMessage::Full { id, image } = message else {
                continue;
            };
            self.pending_full_loads.remove(&id);
            if let Some(image) = image {
                self.remember_preloaded_image(id, image);
            }
        }
    }

    pub fn show_docked(
        &mut self,
        ctx: &egui::Context,
        photos: &[Photo],
        thumbnails: &mut ThumbnailCache,
    ) {
        let Some(photo) = self.current.clone() else {
            return;
        };

        self.poll_loaded(ctx);
        self.ensure_loading(ctx, &photo);
        let filmstrip_photos = viewer_filmstrip_photos(photos, photo.id, VIEWER_FILMSTRIP_RADIUS);

        egui::TopBottomPanel::top("viewer_filmstrip")
            .exact_height(VIEWER_FILMSTRIP_HEIGHT)
            .frame(egui::Frame::default().fill(VIEWER_BG))
            .show(ctx, |ui| {
                apply_viewer_visuals(ui);
                self.show_filmstrip(ui, ui.max_rect(), thumbnails, &filmstrip_photos, photo.id);
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
                let panel_rect = ui.max_rect();
                self.show_image(ui, viewer_canvas_rect(panel_rect));
                self.show_metadata(ui, viewer_metadata_rect(panel_rect), &photo);
            });
    }

    fn show_filmstrip(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        thumbnails: &mut ThumbnailCache,
        photos: &[Photo],
        current_id: i64,
    ) {
        ui.allocate_rect(rect, egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, VIEWER_BG);
        let left_rect = egui::Rect::from_min_size(
            rect.min + Vec2::new(4.0, 4.0),
            Vec2::new(205.0, rect.height() - 8.0),
        );
        let right_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - 160.0, rect.top() + 4.0),
            Vec2::new(156.0, rect.height() - 8.0),
        );
        let preview_width = filmstrip_preview_width(photos.len());
        let center_rect = egui::Rect::from_center_size(
            rect.center(),
            Vec2::new(
                preview_width.min(rect.width() - 420.0).max(150.0),
                rect.height() - 6.0,
            ),
        );

        ui.scope_builder(egui::UiBuilder::new().max_rect(left_rect), |ui| {
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
            });
        });
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(center_rect)
                .layout(Layout::left_to_right(Align::Center)),
            |ui| {
                if ui
                    .add_sized(
                        Vec2::new(22.0, VIEWER_NAV_BUTTON_HEIGHT),
                        viewer_button("◀"),
                    )
                    .clicked()
                {
                    self.request_navigation(NavigationDirection::Previous);
                }
                for photo in photos {
                    self.show_filmstrip_thumbnail(ui, thumbnails, photo, photo.id == current_id);
                }
                if ui
                    .add_sized(
                        Vec2::new(22.0, VIEWER_NAV_BUTTON_HEIGHT),
                        viewer_button("▶"),
                    )
                    .clicked()
                {
                    self.request_navigation(NavigationDirection::Next);
                }
            },
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(right_rect)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                ui.label(RichText::new("A+  A-  A/A").color(Color32::from_gray(70)));
            },
        );
    }

    fn show_filmstrip_thumbnail(
        &mut self,
        ui: &mut egui::Ui,
        thumbnails: &mut ThumbnailCache,
        photo: &Photo,
        selected: bool,
    ) {
        let (rect, response) = ui.allocate_exact_size(
            Vec2::splat(VIEWER_FILMSTRIP_THUMB_SIZE),
            egui::Sense::click(),
        );
        let fill = if selected {
            Color32::from_rgb(222, 236, 252)
        } else if response.hovered() {
            Color32::from_rgb(235, 239, 244)
        } else {
            Color32::from_rgb(207, 211, 215)
        };
        ui.painter().rect_filled(rect, 2.0, fill);

        match thumbnails.state_for(ui.ctx(), photo) {
            ThumbnailState::Ready(texture) => {
                let image_rect = viewer_fit_rect(rect.shrink(2.0), texture.size_vec2());
                ui.painter().image(
                    texture.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            ThumbnailState::Pending => {
                ui.painter()
                    .circle_filled(rect.center(), 3.0, Color32::from_rgb(145, 150, 155));
            }
            ThumbnailState::Unavailable => {
                ui.painter().line_segment(
                    [
                        rect.left_top() + Vec2::splat(6.0),
                        rect.right_bottom() - Vec2::splat(6.0),
                    ],
                    Stroke::new(1.0, Color32::from_rgb(145, 70, 70)),
                );
                ui.painter().line_segment(
                    [
                        egui::pos2(rect.right() - 6.0, rect.top() + 6.0),
                        egui::pos2(rect.left() + 6.0, rect.bottom() - 6.0),
                    ],
                    Stroke::new(1.0, Color32::from_rgb(145, 70, 70)),
                );
            }
        }

        let stroke = if selected {
            Stroke::new(2.0, VIEWER_BLUE)
        } else {
            Stroke::new(1.0, Color32::from_rgb(150, 155, 160))
        };
        ui.painter()
            .rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);

        if response.clicked() {
            self.navigation_request = Some(ViewerNavigationRequest::Photo(photo.id));
        }
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

    fn show_metadata(&self, ui: &mut egui::Ui, rect: egui::Rect, photo: &Photo) {
        ui.allocate_rect(rect, egui::Sense::hover());
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
                    if let Some(caption) = &photo.picasa_caption {
                        ui.label(
                            RichText::new(format!("Legende: {caption}"))
                                .color(Color32::from_gray(85)),
                        );
                    }
                    if let Some(keywords) = &photo.picasa_keywords {
                        ui.label(
                            RichText::new(format!("Mots-cles: {keywords}"))
                                .color(Color32::from_gray(85)),
                        );
                    }
                });
            },
        );
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

    fn show_image(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        ui.allocate_rect(rect, egui::Sense::drag());
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

        self.show_canvas_navigation_arrows(ui, rect);
    }

    fn show_canvas_navigation_arrows(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        let button_size = Vec2::new(34.0, 54.0);
        let previous_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + 28.0, rect.center().y),
            button_size,
        );
        let next_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 28.0, rect.center().y),
            button_size,
        );

        if ui.put(previous_rect, viewer_overlay_button("‹")).clicked() {
            self.request_navigation(NavigationDirection::Previous);
        }
        if ui.put(next_rect, viewer_overlay_button("›")).clicked() {
            self.request_navigation(NavigationDirection::Next);
        }
    }

    fn request_navigation(&mut self, direction: NavigationDirection) {
        self.navigation_request = Some(ViewerNavigationRequest::Direction(direction));
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

pub fn viewer_canvas_rect(panel_rect: egui::Rect) -> egui::Rect {
    let canvas_size = viewer_canvas_size(panel_rect.size());
    egui::Rect::from_min_size(panel_rect.min, canvas_size)
}

pub fn viewer_metadata_rect(panel_rect: egui::Rect) -> egui::Rect {
    let canvas_rect = viewer_canvas_rect(panel_rect);
    egui::Rect::from_min_size(
        egui::pos2(canvas_rect.left(), canvas_rect.bottom()),
        Vec2::new(canvas_rect.width(), VIEWER_METADATA_HEIGHT),
    )
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

pub fn viewer_fit_rect(container: egui::Rect, content_size: Vec2) -> egui::Rect {
    if content_size.x <= 0.0 || content_size.y <= 0.0 {
        return egui::Rect::from_center_size(container.center(), Vec2::ZERO);
    }

    let scale = (container.width() / content_size.x).min(container.height() / content_size.y);
    egui::Rect::from_center_size(container.center(), content_size * scale)
}

#[cfg(test)]
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

fn viewer_overlay_button(label: &str) -> egui::Button<'_> {
    egui::Button::new(
        RichText::new(label)
            .size(30.0)
            .color(Color32::from_gray(45)),
    )
    .fill(Color32::from_rgba_premultiplied(244, 246, 249, 190))
    .stroke(Stroke::new(
        1.0,
        Color32::from_rgba_premultiplied(100, 105, 110, 190),
    ))
    .corner_radius(3.0)
}

pub fn viewer_filmstrip_photos(photos: &[Photo], current_id: i64, radius: usize) -> Vec<Photo> {
    let Some(current_index) = photos.iter().position(|photo| photo.id == current_id) else {
        return Vec::new();
    };

    let start = current_index.saturating_sub(radius);
    let end = (current_index + radius + 1).min(photos.len());
    photos[start..end].to_vec()
}

fn filmstrip_preview_width(photo_count: usize) -> f32 {
    let thumbnails_width = photo_count as f32 * VIEWER_FILMSTRIP_THUMB_SIZE
        + photo_count.saturating_sub(1) as f32 * VIEWER_FILMSTRIP_THUMB_GAP;
    thumbnails_width + 2.0 * 22.0 + 2.0 * VIEWER_FILMSTRIP_THUMB_GAP
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
    let cache_path = viewer_cache_path(photo);
    if let Some(cache_path) = &cache_path {
        if let Some(image) = load_viewer_cache_image(cache_path) {
            return Some(image);
        }
    }

    let image = image::open(&photo.path)
        .ok()?
        .thumbnail(VIEWER_IMAGE_MAX_EDGE, VIEWER_IMAGE_MAX_EDGE);
    if let Some(cache_path) = &cache_path {
        if let Some(parent) = cache_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = image.save_with_format(cache_path, ImageFormat::Png);
    }

    Some(dynamic_to_color_image(image))
}

fn load_viewer_cache_image(path: &PathBuf) -> Option<ColorImage> {
    if !path.exists() {
        return None;
    }

    image::open(path).ok().map(dynamic_to_color_image)
}

fn dynamic_to_color_image(image: image::DynamicImage) -> ColorImage {
    let image = image.to_rgba8();
    rgba_to_color_image(image)
}

fn rgba_to_color_image(image: RgbaImage) -> ColorImage {
    let size = [image.width() as usize, image.height() as usize];
    let pixels = image.into_raw();
    ColorImage::from_rgba_unmultiplied(size, &pixels)
}

fn viewer_cache_path(photo: &Photo) -> Option<PathBuf> {
    Some(viewer_cache_dir()?.join(viewer_cache_file_name(photo)))
}

fn viewer_cache_dir() -> Option<PathBuf> {
    let project_dirs = ProjectDirs::from("org", "MyCasa", "MyCasa")?;
    let cache_dir = project_dirs.cache_dir().join(VIEWER_DISK_CACHE_DIR);
    std::fs::create_dir_all(&cache_dir).ok()?;
    Some(cache_dir)
}

fn viewer_cache_file_name(photo: &Photo) -> String {
    format!(
        "{}-{}-{}-{}.png",
        stable_viewer_cache_key(&photo.path.to_string_lossy()),
        photo.modified_at.unwrap_or_default(),
        photo.file_size.unwrap_or_default(),
        VIEWER_IMAGE_MAX_EDGE
    )
}

fn stable_viewer_cache_key(value: &str) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
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
    fn viewer_navigation_request_is_consumed_once() {
        let mut viewer = ViewerState::default();
        viewer.navigation_request = Some(ViewerNavigationRequest::Direction(
            NavigationDirection::Next,
        ));

        assert_eq!(
            viewer.take_navigation_request(),
            Some(ViewerNavigationRequest::Direction(
                NavigationDirection::Next
            ))
        );
        assert_eq!(viewer.take_navigation_request(), None);
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
    fn viewer_canvas_and_metadata_rects_do_not_overlap() {
        let panel = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1680.0, 960.0));
        let canvas = viewer_canvas_rect(panel);
        let metadata = viewer_metadata_rect(panel);

        assert_eq!(canvas.bottom(), metadata.top());
        assert_eq!(metadata.height(), VIEWER_METADATA_HEIGHT);
        assert!(metadata.bottom() <= panel.bottom());
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
    fn viewer_fit_rect_preserves_aspect_ratio_inside_container() {
        let container = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(30.0, 20.0));
        let fitted = viewer_fit_rect(container, Vec2::new(100.0, 50.0));

        assert_eq!(fitted.size(), Vec2::new(30.0, 15.0));
        assert_eq!(fitted.center(), container.center());
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
    fn viewer_filmstrip_photos_centers_current_photo() {
        let photos: Vec<Photo> = (1..=10).map(photo_for_test).collect();

        let filmstrip = viewer_filmstrip_photos(&photos, 5, 2);
        let ids: Vec<i64> = filmstrip.iter().map(|photo| photo.id).collect();

        assert_eq!(ids, vec![3, 4, 5, 6, 7]);
    }

    #[test]
    fn viewer_filmstrip_photos_clamps_at_edges() {
        let photos: Vec<Photo> = (1..=4).map(photo_for_test).collect();

        let filmstrip = viewer_filmstrip_photos(&photos, 1, 3);
        let ids: Vec<i64> = filmstrip.iter().map(|photo| photo.id).collect();

        assert_eq!(ids, vec![1, 2, 3, 4]);
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
    fn viewer_disk_cache_file_name_changes_when_source_changes() {
        let mut photo = photo_for_test(70);
        photo.modified_at = Some(100);
        photo.file_size = Some(200);
        let initial = viewer_cache_file_name(&photo);

        photo.modified_at = Some(101);
        let changed_mtime = viewer_cache_file_name(&photo);
        photo.modified_at = Some(100);
        photo.file_size = Some(201);
        let changed_size = viewer_cache_file_name(&photo);

        assert_ne!(initial, changed_mtime);
        assert_ne!(initial, changed_size);
        assert!(initial.ends_with(&format!("-{VIEWER_IMAGE_MAX_EDGE}.png")));
    }

    #[test]
    fn load_viewer_image_uses_disk_cache_without_original() {
        let source_path = std::env::temp_dir().join(format!(
            "mycasa-viewer-source-{}-{}.png",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let mut photo = photo_for_test(71);
        photo.path = source_path.clone();
        photo.file_size = Some(123);
        photo.modified_at = Some(456);

        let cache_path = viewer_cache_path(&photo).unwrap();
        let _ = std::fs::remove_file(&cache_path);
        let original = RgbaImage::from_pixel(80, 40, image::Rgba([10, 20, 30, 255]));
        original
            .save_with_format(&source_path, ImageFormat::Png)
            .unwrap();

        let first = load_viewer_image(&photo).unwrap();
        assert!(cache_path.exists());

        std::fs::remove_file(&source_path).unwrap();
        let second = load_viewer_image(&photo).unwrap();

        assert_eq!(first.size, second.size);

        let _ = std::fs::remove_file(cache_path);
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
    fn viewer_caches_completed_full_image_while_closed() {
        let photo = photo_for_test(14);
        let mut viewer = ViewerState::default();
        viewer.open(photo.clone());
        viewer.pending_full_loads.insert(photo.id);
        viewer.close();

        viewer
            .sender
            .send(ViewerMessage::Full {
                id: photo.id,
                image: Some(color_image_for_test(14)),
            })
            .unwrap();
        viewer.poll_background_results();

        assert!(viewer.preloaded_images.contains_key(&photo.id));
        assert!(!viewer.pending_full_loads.contains(&photo.id));
        assert_eq!(viewer.current_photo_id(), None);
    }

    #[test]
    fn viewer_allows_preload_only_after_current_photo_is_loaded() {
        let mut viewer = ViewerState::default();
        let photo = photo_for_test(12);

        assert!(!viewer.can_preload_around_current());

        viewer.open(photo.clone());
        assert!(!viewer.can_preload_around_current());

        viewer.loaded_photo_id = Some(photo.id);
        assert!(viewer.can_preload_around_current());

        viewer.open(photo_for_test(13));
        assert!(!viewer.can_preload_around_current());
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
