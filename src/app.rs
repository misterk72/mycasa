use std::{
    collections::HashSet,
    path::PathBuf,
    time::{Duration, Instant},
};

use egui::{Align, Color32, Layout, RichText, ScrollArea, Sense, Stroke, Vec2};

use crate::catalog::{Catalog, CatalogError, Photo};
use crate::debounce::Debouncer;
use crate::folders::add_folder_once;
use crate::grid::{columns_for_width, item_range_for_row, row_count};
use crate::indexer::{IndexJob, IndexedPhoto, Indexer};
use crate::photo_limit::{INITIAL_PHOTO_LIMIT, MAX_PHOTO_LIMIT, next_photo_limit};
use crate::scan_state::{begin_scan, finish_scan};
use crate::thumbnails::{ThumbnailCache, ThumbnailState};
use crate::ui_text::middle_truncate;
use crate::viewer::{NavigationDirection, ViewerState, adjacent_photo_id, viewer_root_size};

const THUMBNAIL_SIZE: f32 = 132.0;
const TILE_PADDING: f32 = 8.0;
const TILE_WIDTH: f32 = THUMBNAIL_SIZE + TILE_PADDING * 2.0;
const TILE_HEIGHT: f32 = THUMBNAIL_SIZE + 28.0;
const MAX_INDEX_EVENTS_PER_FRAME: usize = 80;
const REFRESH_AFTER_IMPORTED_PHOTOS: usize = 50;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
const PICASA_BLUE: Color32 = Color32::from_rgb(59, 139, 190);
const PANEL_BG: Color32 = Color32::from_rgb(231, 235, 239);
const LIGHTBOX_BG: Color32 = Color32::from_rgb(250, 250, 248);
const TILE_SHADOW: Color32 = Color32::from_rgb(205, 205, 202);
const CHROME_BG: Color32 = Color32::from_rgb(236, 237, 238);
const CHROME_BORDER: Color32 = Color32::from_rgb(176, 182, 188);

pub struct MyCasaApp {
    catalog: Result<Catalog, CatalogError>,
    indexer: Indexer,
    thumbnails: ThumbnailCache,
    viewer: ViewerState,
    photos: Vec<Photo>,
    photo_limit: usize,
    folders: Vec<PathBuf>,
    albums: Vec<String>,
    selected_photo: Option<i64>,
    status: String,
    search: String,
    search_debouncer: Debouncer,
    imported_since_refresh: usize,
    pending_catalog_writes: Vec<IndexedPhoto>,
    active_scan_folders: HashSet<PathBuf>,
    frame_count: u64,
    fps_window_started_at: Instant,
    displayed_fps: f32,
}

impl MyCasaApp {
    pub fn new(_creation_context: &eframe::CreationContext<'_>) -> Self {
        let catalog = Catalog::open_default();
        let mut status = String::from("Catalogue pret");
        let mut photos = Vec::new();
        let mut folders = Vec::new();

        if let Ok(catalog) = &catalog {
            match catalog.load_recent_photos(INITIAL_PHOTO_LIMIT) {
                Ok(loaded) => photos = loaded,
                Err(error) => {
                    status = format!("Catalogue ouvert, lecture photos impossible: {error}")
                }
            }
            if let Ok(loaded_folders) = catalog.load_folders(50) {
                folders = loaded_folders;
            }
        } else if let Err(error) = &catalog {
            status = format!("Catalogue indisponible: {error}");
        }

        Self {
            catalog,
            indexer: Indexer::new(),
            thumbnails: ThumbnailCache::new(),
            viewer: ViewerState::default(),
            photos,
            photo_limit: INITIAL_PHOTO_LIMIT,
            folders,
            albums: vec![
                "Toutes les photos".to_owned(),
                "Favoris".to_owned(),
                "Import recent".to_owned(),
            ],
            selected_photo: None,
            status,
            search: String::new(),
            search_debouncer: Debouncer::new(SEARCH_DEBOUNCE),
            imported_since_refresh: 0,
            pending_catalog_writes: Vec::new(),
            active_scan_folders: HashSet::new(),
            frame_count: 0,
            fps_window_started_at: Instant::now(),
            displayed_fps: 0.0,
        }
    }

    fn update_frame_metrics(&mut self) {
        self.frame_count += 1;
        let elapsed = self.fps_window_started_at.elapsed();
        if elapsed >= Duration::from_secs(1) {
            self.displayed_fps = self.frame_count as f32 / elapsed.as_secs_f32();
            self.frame_count = 0;
            self.fps_window_started_at = Instant::now();
        }
    }

    fn start_scan_folder(&mut self, folder: PathBuf) {
        if !begin_scan(&mut self.active_scan_folders, &folder) {
            self.status = format!("Scan deja en cours: {}", folder.display());
            return;
        }

        self.indexer.scan_folder(folder);
    }

    fn pick_folder_and_scan(&mut self) {
        match rfd::FileDialog::new()
            .set_title("Ajouter un dossier photo")
            .pick_folder()
        {
            Some(path) => {
                add_folder_once(&mut self.folders, path.clone());
                self.start_scan_folder(path);
            }
            None => {
                self.status = "Selection de dossier annulee".to_owned();
            }
        }
    }

    fn scan_current_dir(&mut self) {
        match std::env::current_dir() {
            Ok(path) => {
                add_folder_once(&mut self.folders, path.clone());
                self.start_scan_folder(path);
            }
            Err(error) => self.status = format!("Dossier courant introuvable: {error}"),
        }
    }

    fn load_more_photos(&mut self) {
        let next_limit = next_photo_limit(self.photo_limit);
        if next_limit == self.photo_limit {
            self.status = format!("Limite d'affichage atteinte: {} photos", MAX_PHOTO_LIMIT);
            return;
        }

        self.photo_limit = next_limit;
        self.refresh_photos();
    }

    fn refresh_photos(&mut self) {
        if let Ok(catalog) = &self.catalog {
            match catalog.search_photos(&self.search, self.photo_limit) {
                Ok(photos) => {
                    self.photos = photos;
                    self.status = format!("{} photo(s) dans le catalogue", self.photos.len());
                }
                Err(error) => self.status = format!("Erreur catalogue: {error}"),
            }
        }
    }

    fn flush_catalog_writes(&mut self) {
        if self.pending_catalog_writes.is_empty() {
            return;
        }

        let pending = std::mem::take(&mut self.pending_catalog_writes);
        let last_path = pending.last().map(|photo| photo.path.clone());
        let result = match &mut self.catalog {
            Ok(catalog) => catalog.upsert_photos(&pending),
            Err(error) => {
                self.status = format!("Catalogue indisponible: {error}");
                return;
            }
        };

        match result {
            Ok(written) => {
                self.imported_since_refresh += written;
                if let Some(path) = last_path {
                    self.status =
                        format!("{written} photo(s) ecrite(s), derniere: {}", path.display());
                }
                if self.imported_since_refresh >= REFRESH_AFTER_IMPORTED_PHOTOS {
                    self.imported_since_refresh = 0;
                    self.refresh_photos();
                }
            }
            Err(error) => {
                self.status = format!("Ecriture catalogue impossible: {error}");
            }
        }
    }

    fn poll_background_work(&mut self) {
        let mut processed = 0;
        while processed < MAX_INDEX_EVENTS_PER_FRAME {
            let Some(event) = self.indexer.try_recv() else {
                break;
            };
            processed += 1;
            match event {
                IndexJob::Started(path) => {
                    self.imported_since_refresh = 0;
                    self.status = format!("Indexation demarree: {}", path.display());
                }
                IndexJob::FoundPhoto(photo) => {
                    self.pending_catalog_writes.push(photo);
                    self.status = format!(
                        "Indexation en cours: {} photo(s) en attente d'ecriture",
                        self.pending_catalog_writes.len()
                    );
                }
                IndexJob::Finished {
                    folder,
                    photos_found,
                } => {
                    finish_scan(&mut self.active_scan_folders, &folder);
                    self.flush_catalog_writes();
                    self.status = format!(
                        "Indexation terminee: {} photo(s) trouvee(s) dans {}",
                        photos_found,
                        folder.display()
                    );
                    add_folder_once(&mut self.folders, folder);
                    self.refresh_photos();
                }
                IndexJob::Failed(message) => {
                    self.status = format!("Indexation impossible: {message}");
                }
            }
        }

        self.flush_catalog_writes();

        if processed == MAX_INDEX_EVENTS_PER_FRAME {
            self.status =
                format!("Indexation en cours: traitement par lots de {MAX_INDEX_EVENTS_PER_FRAME}");
        }
    }

    fn ui_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.visuals_mut().widgets.noninteractive.bg_fill = PANEL_BG;
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Phototheque")
                    .strong()
                    .color(Color32::from_gray(70)),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let _ = ui.small_button("+");
            });
        });
        ui.separator();

        egui::CollapsingHeader::new(format!("Albums ({})", self.albums.len()))
            .default_open(true)
            .show(ui, |ui| {
                for album in &self.albums {
                    let _ = ui.selectable_label(album == "Toutes les photos", album);
                }
            });

        egui::CollapsingHeader::new(format!("Dossiers ({})", self.folders.len()))
            .default_open(true)
            .show(ui, |ui| {
                if self.folders.is_empty() {
                    ui.label("Aucun dossier indexe");
                } else {
                    for folder in self.folders.clone() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("▸").size(12.0).color(Color32::from_gray(110)));
                            let folder_name = folder
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or_else(|| folder.to_str().unwrap_or("Dossier"));
                            if ui.selectable_label(false, folder_name).clicked() {
                                self.start_scan_folder(folder.clone());
                            }
                        });
                    }
                }
            });

        ui.add_space(8.0);
        if ui.button("Gestionnaire de dossiers").clicked() {
            self.pick_folder_and_scan();
        }
        if ui.button("Charger dossiers Picasa").clicked() {
            let picasa_folders = crate::picasa_db::load_watched_folders();
            let picasa_contacts = crate::picasa_db::load_contacts();
            let mut added = 0;
            for folder in picasa_folders {
                if add_folder_once(&mut self.folders, folder) {
                    added += 1;
                }
            }
            self.status = format!(
                "{added} dossier(s) surveille(s) Picasa charges, {} contact(s) detecte(s)",
                picasa_contacts.len()
            );
        }
    }

    fn ui_top_chrome(&mut self, ui: &mut egui::Ui) {
        ui.painter().rect_filled(ui.max_rect(), 0.0, CHROME_BG);
        ui.painter().line_segment(
            [ui.max_rect().left_bottom(), ui.max_rect().right_bottom()],
            Stroke::new(1.0, CHROME_BORDER),
        );
        ui.horizontal(|ui| {
            for label in [
                "Fichier",
                "Edition",
                "Affichage",
                "Dossier",
                "Photo",
                "Creation",
                "Outils",
                "Aide",
            ] {
                ui.label(
                    RichText::new(label)
                        .size(13.0)
                        .color(Color32::from_gray(35)),
                );
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.hyperlink_to("Connexion aux albums Web", "https://picasa.google.com/");
            });
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Importer").clicked() {
                self.pick_folder_and_scan();
            }
            if ui.button("Diaporama").clicked() {
                self.status = "Diaporama: a implementer".to_owned();
            }
            if ui.button("Chronologie").clicked() {
                self.status = "Chronologie: a implementer".to_owned();
            }
            if ui.button("CD cadeau").clicked() {
                self.status = "CD cadeau: a implementer".to_owned();
            }
            if ui.button("Scanner dossier courant").clicked() {
                self.scan_current_dir();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let response = ui.add_sized(
                    [260.0, 22.0],
                    egui::TextEdit::singleline(&mut self.search).hint_text("Rechercher"),
                );
                if response.changed() {
                    self.search_debouncer.mark_changed(Instant::now());
                }
                if ui.button("Rechercher").clicked() {
                    self.refresh_photos();
                }
            });
        });
    }

    fn ui_collection_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("▱")
                    .size(28.0)
                    .color(Color32::from_rgb(190, 139, 65)),
            );
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Phototheque")
                        .size(22.0)
                        .color(Color32::from_rgb(174, 112, 45)),
                );
                ui.label(
                    RichText::new(format!("{} photo(s) affichee(s)", self.photos.len()))
                        .color(Color32::from_gray(95)),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Charger plus").clicked() {
                    self.load_more_photos();
                }
                ui.label(format!("limite {}", self.photo_limit));
            });
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Ajouter une description").color(Color32::from_gray(170)));
        ui.add_space(6.0);
    }

    fn ui_top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Petites vignettes").color(Color32::from_gray(100)));
            ui.label(
                RichText::new("Vignettes normales")
                    .strong()
                    .color(PICASA_BLUE),
            );
            ui.label(RichText::new("Centre de retouche").color(Color32::from_gray(100)));
            if ui.button("Actualiser").clicked() {
                self.refresh_photos();
            }
        });
    }

    fn ui_grid(&mut self, ui: &mut egui::Ui) {
        if self.photos.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Aucune photo indexee pour le moment");
            });
            return;
        }

        let columns = columns_for_width(ui.available_width(), TILE_WIDTH);
        let total_rows = row_count(self.photos.len(), columns);

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(
                ui,
                TILE_HEIGHT + TILE_PADDING,
                total_rows,
                |ui, row_range| {
                    for row_index in row_range {
                        ui.horizontal(|ui| {
                            for photo_index in
                                item_range_for_row(row_index, columns, self.photos.len())
                            {
                                let photo = self.photos[photo_index].clone();
                                self.photo_tile(ui, &photo);
                            }
                        });
                    }
                },
            );
    }

    fn handle_viewer_keyboard(&mut self, ctx: &egui::Context) {
        let Some(current_id) = self.viewer.current_photo_id() else {
            return;
        };

        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.viewer.close();
            return;
        }

        let direction = ctx.input(|input| {
            if input.key_pressed(egui::Key::ArrowLeft) {
                Some(NavigationDirection::Previous)
            } else if input.key_pressed(egui::Key::ArrowRight) {
                Some(NavigationDirection::Next)
            } else {
                None
            }
        });

        let Some(direction) = direction else {
            return;
        };
        let Some(next_id) = adjacent_photo_id(&self.photos, current_id, direction) else {
            return;
        };
        let Some(photo) = self
            .photos
            .iter()
            .find(|photo| photo.id == next_id)
            .cloned()
        else {
            return;
        };

        self.selected_photo = Some(photo.id);
        self.viewer.open(photo);
    }

    fn preload_viewer_neighbors(&mut self, ctx: &egui::Context) {
        let Some(current_id) = self.viewer.current_photo_id() else {
            return;
        };

        let neighbor_ids = [
            adjacent_photo_id(&self.photos, current_id, NavigationDirection::Previous),
            adjacent_photo_id(&self.photos, current_id, NavigationDirection::Next),
        ];
        let neighbors: Vec<Photo> = neighbor_ids
            .into_iter()
            .flatten()
            .filter_map(|id| self.photos.iter().find(|photo| photo.id == id).cloned())
            .collect();

        for photo in &neighbors {
            let _ = self.thumbnails.state_for(ctx, photo);
        }
    }

    fn photo_tile(&mut self, ui: &mut egui::Ui, photo: &Photo) {
        let selected = self.selected_photo == Some(photo.id);
        let desired_size = Vec2::new(THUMBNAIL_SIZE + TILE_PADDING, TILE_HEIGHT);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());

        if response.clicked() {
            self.selected_photo = Some(photo.id);
            self.viewer.open(photo.clone());
        }

        let fill = if selected {
            Color32::from_rgb(232, 242, 255)
        } else if response.hovered() {
            Color32::from_rgb(248, 252, 255)
        } else {
            Color32::WHITE
        };

        let shadow_rect = rect.translate(Vec2::new(1.5, 1.5));
        ui.painter().rect_filled(shadow_rect, 1.0, TILE_SHADOW);
        ui.painter().rect_filled(rect, 1.0, fill);
        ui.painter().rect_stroke(
            rect,
            1.0,
            Stroke::new(
                if selected { 2.0 } else { 1.0 },
                if selected {
                    PICASA_BLUE
                } else {
                    Color32::from_rgb(204, 204, 204)
                },
            ),
            egui::StrokeKind::Inside,
        );

        let thumb_rect = egui::Rect::from_min_size(
            rect.min + Vec2::new(TILE_PADDING, TILE_PADDING),
            Vec2::splat(THUMBNAIL_SIZE - TILE_PADDING),
        );

        match self.thumbnails.state_for(ui.ctx(), photo) {
            ThumbnailState::Pending => {
                ui.painter()
                    .rect_filled(thumb_rect, 4.0, Color32::from_rgb(235, 235, 232));
                ui.painter().text(
                    thumb_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "miniature",
                    egui::TextStyle::Small.resolve(ui.style()),
                    Color32::from_gray(135),
                );
            }
            ThumbnailState::Ready(texture) => {
                let image_size = texture.size_vec2();
                let scale = (thumb_rect.width() / image_size.x)
                    .min(thumb_rect.height() / image_size.y)
                    .min(1.0);
                let fitted_size = image_size * scale;
                let image_rect = egui::Rect::from_center_size(thumb_rect.center(), fitted_size);
                ui.painter()
                    .rect_filled(thumb_rect, 4.0, Color32::from_rgb(246, 246, 244));
                ui.painter().image(
                    texture.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
            ThumbnailState::Unavailable => {
                ui.painter()
                    .rect_filled(thumb_rect, 4.0, Color32::from_rgb(242, 232, 232));
                ui.painter().text(
                    thumb_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "indisponible",
                    egui::TextStyle::Small.resolve(ui.style()),
                    Color32::from_rgb(210, 140, 140),
                );
            }
        }

        let name = photo
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("photo");
        let label = crate::ui_text::middle_truncate(name, 22);
        ui.painter().text(
            rect.center_bottom() - Vec2::new(0.0, 11.0),
            egui::Align2::CENTER_CENTER,
            label,
            egui::TextStyle::Small.resolve(ui.style()),
            Color32::from_gray(55),
        );
    }

    fn ui_people_panel(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(220.0);
        ui.visuals_mut().widgets.noninteractive.bg_fill = PANEL_BG;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Personnes")
                    .strong()
                    .color(Color32::from_gray(70)),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let _ = ui.small_button("⚙");
            });
        });
        ui.separator();
        ui.label(RichText::new("Les personnes qui apparaissent dans les photos selectionnees seront repertoriees ici.").color(Color32::from_gray(105)));
        ui.add_space(8.0);
        if let Some(photo_id) = self.selected_photo {
            if let Some(photo) = self.photos.iter().find(|photo| photo.id == photo_id) {
                ui.label(RichText::new("Photo selectionnee").strong());
                let available_chars = (ui.available_width() / 7.0).max(24.0) as usize;
                ui.label(middle_truncate(
                    &photo.path.display().to_string(),
                    available_chars,
                ))
                .on_hover_text(photo.path.display().to_string());
                if photo.picasa_face_count > 0 {
                    ui.label(format!("{} visage(s) Picasa", photo.picasa_face_count));
                } else {
                    ui.label("Aucun visage detecte dans le catalogue");
                }
            }
        } else {
            ui.label("Aucune photo selectionnee");
        }
    }

    fn ui_bottom_tray(&mut self, ui: &mut egui::Ui) {
        let metrics = self.thumbnails.metrics();
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, Color32::from_rgb(238, 240, 242));
        ui.vertical(|ui| {
            let strip_rect =
                egui::Rect::from_min_size(ui.min_rect().min, Vec2::new(ui.available_width(), 5.0));
            ui.painter().rect_filled(strip_rect, 0.0, PICASA_BLUE);
            ui.add_space(7.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&self.status).color(Color32::from_gray(55)));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(format!(
                        "{} photos | cache:{} gen:{} pending:{} active:{} fps:{:.0}",
                        self.photos.len(),
                        metrics.cache_hits,
                        metrics.generated,
                        metrics.pending,
                        metrics.active_loads,
                        self.displayed_fps
                    ));
                });
            });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Dossier selectionne")
                        .color(PICASA_BLUE)
                        .strong(),
                );
                ui.add_space(24.0);
                for label in [
                    "Album Web",
                    "E-mail",
                    "Imprimer",
                    "Commander",
                    "BlogThis!",
                    "Montage",
                    "Exporter",
                ] {
                    if ui.button(label).clicked() {
                        self.status = format!("{label}: a implementer");
                    }
                }
                if let Ok(catalog) = &self.catalog {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let path = catalog.path().display().to_string();
                        ui.label(middle_truncate(&path, 36)).on_hover_text(path);
                    });
                }
            });
        });
    }
}

impl eframe::App for MyCasaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_frame_metrics();
        self.poll_background_work();
        if self.search_debouncer.should_run(Instant::now()) {
            self.refresh_photos();
        }

        if self.viewer.is_open() {
            egui::CentralPanel::default()
                .frame(egui::Frame::default().fill(Color32::from_rgb(224, 226, 229)))
                .show(ctx, |_| {});
            let content_rect = ctx.content_rect();
            egui::Area::new("viewer_fullscreen_area".into())
                .fixed_pos(content_rect.min)
                .default_size(viewer_root_size(content_rect))
                .movable(false)
                .constrain_to(content_rect)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    let root_size = viewer_root_size(content_rect);
                    ui.set_min_size(root_size);
                    ui.set_max_size(root_size);
                    self.viewer.show_embedded_in_rect(ctx, ui, content_rect);
                });
        } else {
            egui::TopBottomPanel::top("picasa_top_chrome")
                .exact_height(60.0)
                .show(ctx, |ui| self.ui_top_chrome(ui));

            egui::SidePanel::left("sidebar")
                .resizable(true)
                .default_width(220.0)
                .frame(egui::Frame::default().fill(PANEL_BG))
                .show(ctx, |ui| self.ui_sidebar(ui));

            egui::SidePanel::right("people_panel")
                .resizable(true)
                .default_width(220.0)
                .width_range(210.0..=320.0)
                .frame(egui::Frame::default().fill(PANEL_BG))
                .show(ctx, |ui| self.ui_people_panel(ui));

            egui::TopBottomPanel::bottom("picasa_bottom_tray")
                .exact_height(96.0)
                .show(ctx, |ui| self.ui_bottom_tray(ui));

            egui::CentralPanel::default()
                .frame(egui::Frame::default().fill(LIGHTBOX_BG))
                .show(ctx, |ui| {
                    self.ui_collection_header(ui);
                    self.ui_top_bar(ui);
                    ui.separator();
                    self.ui_grid(ui);
                });
        }

        self.handle_viewer_keyboard(ctx);
        self.preload_viewer_neighbors(ctx);
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}
