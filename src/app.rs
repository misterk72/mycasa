use std::{
    collections::HashSet,
    path::PathBuf,
    time::{Duration, Instant},
};

use chrono::{Datelike, LocalResult, TimeZone, Utc};
use egui::{
    Align, Color32, Layout, RichText, ScrollArea, Sense, Stroke, Vec2,
    containers::scroll_area::ScrollSource,
};

use crate::catalog::{Catalog, CatalogError, Photo};
use crate::debounce::Debouncer;
use crate::folders::add_folder_once;
use crate::grid::{columns_for_width, item_range_for_row, row_count};
use crate::indexer::{IndexJob, IndexedPhoto, Indexer};
use crate::photo_limit::{INITIAL_PHOTO_LIMIT, MAX_PHOTO_LIMIT, next_photo_limit};
use crate::scan_state::{begin_scan, finish_scan};
use crate::thumbnails::{ThumbnailCache, ThumbnailState};
use crate::ui_text::middle_truncate;
use crate::viewer::{NavigationDirection, ViewerState, adjacent_photo_id};

const THUMBNAIL_SIZE: f32 = 132.0;
const TILE_PADDING: f32 = 8.0;
const TILE_WIDTH: f32 = THUMBNAIL_SIZE + TILE_PADDING * 2.0;
const TILE_HEIGHT: f32 = THUMBNAIL_SIZE + 28.0;
const CHRONO_SECTION_HEIGHT: f32 = 44.0;
const INERTIAL_SCROLL_WHEEL_MULTIPLIER: f32 = 1.15;
const INERTIAL_SCROLL_VELOCITY_MULTIPLIER: f32 = 0.55;
const INERTIAL_SCROLL_FRICTION_PER_SECOND: f32 = 0.055;
const INERTIAL_SCROLL_STOP_SPEED: f32 = 8.0;
const MAX_INDEX_EVENTS_PER_FRAME: usize = 80;
const REFRESH_AFTER_IMPORTED_PHOTOS: usize = 50;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(250);
const VIEWER_PRELOAD_RADIUS: usize = 3;
const PICASA_BLUE: Color32 = Color32::from_rgb(59, 139, 190);
const PANEL_BG: Color32 = Color32::from_rgb(231, 235, 239);
const LIGHTBOX_BG: Color32 = Color32::from_rgb(250, 250, 248);
const TILE_SHADOW: Color32 = Color32::from_rgb(205, 205, 202);
const CHROME_BG: Color32 = Color32::from_rgb(236, 237, 238);
const CHROME_BORDER: Color32 = Color32::from_rgb(176, 182, 188);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LibraryViewMode {
    FolderTree,
    RetroChronological,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ChronologicalGridRow {
    Section { label: String, photos: Vec<usize> },
    Photos(Vec<usize>),
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct InertialScrollState {
    offset: f32,
    velocity: f32,
    max_offset: f32,
}

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
    library_view_mode: LibraryViewMode,
    main_scroll: InertialScrollState,
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
            library_view_mode: LibraryViewMode::RetroChronological,
            main_scroll: InertialScrollState::default(),
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
            if ui.button("Dossiers").clicked() {
                self.library_view_mode = LibraryViewMode::FolderTree;
                self.status = "Vue dossiers".to_owned();
            }
            if ui.button("Chronologie").clicked() {
                self.library_view_mode = LibraryViewMode::RetroChronological;
                self.status = "Vue retrochronologique".to_owned();
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
                    RichText::new(match self.library_view_mode {
                        LibraryViewMode::FolderTree => "Phototheque",
                        LibraryViewMode::RetroChronological => "Chronologie",
                    })
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
            if ui
                .selectable_label(
                    self.library_view_mode == LibraryViewMode::FolderTree,
                    "Dossiers",
                )
                .clicked()
            {
                self.library_view_mode = LibraryViewMode::FolderTree;
            }
            if ui
                .selectable_label(
                    self.library_view_mode == LibraryViewMode::RetroChronological,
                    "Chronologie",
                )
                .clicked()
            {
                self.library_view_mode = LibraryViewMode::RetroChronological;
            }
            ui.separator();
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

        match self.library_view_mode {
            LibraryViewMode::FolderTree => self.ui_folder_grid(ui),
            LibraryViewMode::RetroChronological => self.ui_retrochronological_grid(ui),
        }
    }

    fn ui_folder_grid(&mut self, ui: &mut egui::Ui) {
        let grid_width = library_grid_width(ui.max_rect());
        let columns = columns_for_width(grid_width, TILE_WIDTH);
        let total_rows = row_count(self.photos.len(), columns);
        self.update_main_scroll(ui);

        let output = self.main_scroll_area().show_rows(
            ui,
            TILE_HEIGHT + TILE_PADDING,
            total_rows,
            |ui, row_range| {
                for row_index in row_range {
                    ui.horizontal(|ui| {
                        for photo_index in item_range_for_row(row_index, columns, self.photos.len())
                        {
                            let photo = self.photos[photo_index].clone();
                            self.photo_tile(ui, &photo);
                        }
                    });
                }
            },
        );
        self.sync_main_scroll_from_output(
            output.state.offset.y,
            output.content_size,
            output.inner_rect,
        );
    }

    fn ui_retrochronological_grid(&mut self, ui: &mut egui::Ui) {
        let grid_width = library_grid_width(ui.max_rect());
        let columns = columns_for_width(grid_width, TILE_WIDTH);
        let rows = retrochronological_grid_rows(&self.photos, columns);
        self.update_main_scroll(ui);

        let output = self.main_scroll_area().show_viewport(ui, |ui, viewport| {
            let row_layout = chronological_row_layout(&rows);
            let total_height = row_layout.last().map(|(_, bottom)| *bottom).unwrap_or(0.0);
            ui.set_min_height(total_height);

            let origin = ui.min_rect().min;
            for (row_index, (top, bottom)) in row_layout.iter().enumerate() {
                if *bottom < viewport.top() || *top > viewport.bottom() {
                    continue;
                }

                let row_rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x, origin.y + *top),
                    Vec2::new(grid_width, bottom - top),
                );
                match &rows[row_index] {
                    ChronologicalGridRow::Section { label, photos } => {
                        self.chronology_section_row(ui, row_rect, label, photos);
                    }
                    ChronologicalGridRow::Photos(photo_indices) => {
                        self.chronology_photo_row(ui, row_rect, photo_indices);
                    }
                }
            }
        });
        self.sync_main_scroll_from_output(
            output.state.offset.y,
            output.content_size,
            output.inner_rect,
        );
    }

    fn main_scroll_area(&self) -> ScrollArea {
        ScrollArea::vertical()
            .id_salt("main_library_grid")
            .auto_shrink([false, false])
            .scroll_source(ScrollSource {
                scroll_bar: true,
                drag: true,
                mouse_wheel: false,
            })
            .vertical_scroll_offset(self.main_scroll.offset)
    }

    fn update_main_scroll(&mut self, ui: &egui::Ui) {
        let hovered = ui
            .ctx()
            .input(|input| input.pointer.hover_pos())
            .is_some_and(|position| ui.max_rect().contains(position));
        let (wheel_delta, dt) = ui.ctx().input(|input| {
            (
                if hovered {
                    input.smooth_scroll_delta.y
                } else {
                    0.0
                },
                input.stable_dt.clamp(1.0 / 240.0, 0.1),
            )
        });

        if self.main_scroll.tick(wheel_delta, dt) {
            ui.ctx().request_repaint();
        }
    }

    fn sync_main_scroll_from_output(
        &mut self,
        output_offset: f32,
        content_size: Vec2,
        inner_rect: egui::Rect,
    ) {
        let max_offset = (content_size.y - inner_rect.height()).max(0.0);
        self.main_scroll
            .sync_from_scroll_area(output_offset, max_offset);
    }

    fn chronology_section_row(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        label: &str,
        photo_indices: &[usize],
    ) {
        let available = rect.width();
        let header_rect = egui::Rect::from_min_size(
            rect.min + Vec2::new(0.0, 2.0),
            Vec2::new(available, CHRONO_SECTION_HEIGHT),
        );
        ui.painter()
            .rect_filled(header_rect, 0.0, Color32::from_rgb(238, 239, 239));
        ui.painter().line_segment(
            [header_rect.left_bottom(), header_rect.right_bottom()],
            Stroke::new(1.0, CHROME_BORDER),
        );
        ui.painter().text(
            header_rect.left_center() + Vec2::new(10.0, 0.0),
            egui::Align2::LEFT_CENTER,
            label,
            egui::TextStyle::Heading.resolve(ui.style()),
            Color32::from_rgb(174, 112, 45),
        );

        let photos_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left(), header_rect.bottom() + TILE_PADDING),
            Vec2::new(available, TILE_HEIGHT),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(photos_rect)
                .layout(Layout::left_to_right(Align::Min)),
            |ui| {
                self.render_chronology_photos(ui, photo_indices);
            },
        );
    }

    fn chronology_photo_row(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        photo_indices: &[usize],
    ) {
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Min)),
            |ui| {
                self.render_chronology_photos(ui, photo_indices);
            },
        );
    }

    fn render_chronology_photos(&mut self, ui: &mut egui::Ui, photo_indices: &[usize]) {
        for photo_index in photo_indices {
            let photo = self.photos[*photo_index].clone();
            self.photo_tile(ui, &photo);
        }
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
        if !self.viewer.can_preload_around_current() {
            return;
        }

        let Some(current_id) = self.viewer.current_photo_id() else {
            return;
        };

        let neighbors = viewer_preload_candidates(&self.photos, current_id, VIEWER_PRELOAD_RADIUS);

        for photo in &neighbors {
            let _ = self.thumbnails.state_for(ctx, photo);
        }
        self.viewer.preload_photos(ctx, &neighbors);
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
            });
            if let Ok(catalog) = &self.catalog {
                let path = catalog.path().display().to_string();
                ui.add_space(2.0);
                ui.label(RichText::new(middle_truncate(&path, 80)).color(Color32::from_gray(120)))
                    .on_hover_text(path);
            }
        });
    }
}

pub fn library_grid_width(panel_rect: egui::Rect) -> f32 {
    panel_rect.width()
}

impl InertialScrollState {
    fn tick(&mut self, wheel_delta: f32, dt: f32) -> bool {
        if wheel_delta.abs() > f32::EPSILON {
            let applied_delta = -wheel_delta * INERTIAL_SCROLL_WHEEL_MULTIPLIER;
            self.offset = clamp_scroll_offset(self.offset + applied_delta, self.max_offset);
            self.velocity =
                applied_delta / dt.max(1.0 / 240.0) * INERTIAL_SCROLL_VELOCITY_MULTIPLIER;
            return true;
        }

        if self.velocity.abs() <= INERTIAL_SCROLL_STOP_SPEED {
            self.velocity = 0.0;
            return false;
        }

        self.offset = clamp_scroll_offset(self.offset + self.velocity * dt, self.max_offset);
        if self.offset <= 0.0 || self.offset >= self.max_offset {
            self.velocity = 0.0;
            return false;
        }

        self.velocity *= INERTIAL_SCROLL_FRICTION_PER_SECOND.powf(dt);
        true
    }

    fn sync_from_scroll_area(&mut self, offset: f32, max_offset: f32) {
        self.max_offset = max_offset.max(0.0);
        self.offset = clamp_scroll_offset(offset, self.max_offset);
        if self.offset <= 0.0 || self.offset >= self.max_offset {
            self.velocity = 0.0;
        }
    }
}

fn clamp_scroll_offset(offset: f32, max_offset: f32) -> f32 {
    offset.clamp(0.0, max_offset.max(0.0))
}

fn retrochronological_grid_rows(photos: &[Photo], columns: usize) -> Vec<ChronologicalGridRow> {
    let columns = columns.max(1);
    let mut indices: Vec<usize> = (0..photos.len()).collect();
    indices.sort_by(|left, right| {
        photo_time_key(&photos[*right])
            .cmp(&photo_time_key(&photos[*left]))
            .then_with(|| photos[*right].id.cmp(&photos[*left].id))
    });

    let mut rows = Vec::new();
    let mut current_section = None::<String>;
    let mut current_photos = Vec::new();

    for photo_index in indices {
        let section = photo_section_label(&photos[photo_index]);
        if current_section.as_deref() != Some(section.as_str()) {
            push_photo_rows(&mut rows, &mut current_photos, columns);
            rows.push(ChronologicalGridRow::Section {
                label: section.clone(),
                photos: Vec::new(),
            });
            current_section = Some(section);
        }
        current_photos.push(photo_index);
    }

    push_photo_rows(&mut rows, &mut current_photos, columns);
    rows
}

fn push_photo_rows(
    rows: &mut Vec<ChronologicalGridRow>,
    photo_indices: &mut Vec<usize>,
    columns: usize,
) {
    if let Some(first_row) = photo_indices.get(..photo_indices.len().min(columns)) {
        if let Some(ChronologicalGridRow::Section { photos, .. }) = rows.last_mut() {
            photos.extend_from_slice(first_row);
        }
    }

    for chunk in photo_indices[photo_indices.len().min(columns)..].chunks(columns) {
        rows.push(ChronologicalGridRow::Photos(chunk.to_vec()));
    }
    photo_indices.clear();
}

fn chrono_section_row_height() -> f32 {
    CHRONO_SECTION_HEIGHT + TILE_PADDING + TILE_HEIGHT
}

fn chrono_photo_row_height() -> f32 {
    TILE_HEIGHT + TILE_PADDING
}

fn chronological_row_height(row: &ChronologicalGridRow) -> f32 {
    match row {
        ChronologicalGridRow::Section { .. } => chrono_section_row_height(),
        ChronologicalGridRow::Photos(_) => chrono_photo_row_height(),
    }
}

fn chronological_row_layout(rows: &[ChronologicalGridRow]) -> Vec<(f32, f32)> {
    let mut top = 0.0;
    rows.iter()
        .map(|row| {
            let bottom = top + chronological_row_height(row);
            let bounds = (top, bottom);
            top = bottom;
            bounds
        })
        .collect()
}

fn photo_time_key(photo: &Photo) -> i64 {
    photo
        .captured_at
        .as_deref()
        .and_then(parse_photo_timestamp)
        .or(photo.modified_at)
        .unwrap_or_default()
}

fn parse_photo_timestamp(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|datetime| datetime.timestamp())
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y:%m:%d %H:%M:%S")
                .ok()
                .map(|datetime| datetime.and_utc().timestamp())
        })
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|datetime| datetime.and_utc().timestamp())
        })
}

fn photo_section_label(photo: &Photo) -> String {
    let timestamp = photo_time_key(photo);
    if timestamp <= 0 {
        return "Date inconnue".to_owned();
    }

    match Utc.timestamp_opt(timestamp, 0) {
        LocalResult::Single(datetime) => {
            format!("{} {}", month_name(datetime.month()), datetime.year())
        }
        _ => "Date inconnue".to_owned(),
    }
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "Janvier",
        2 => "Fevrier",
        3 => "Mars",
        4 => "Avril",
        5 => "Mai",
        6 => "Juin",
        7 => "Juillet",
        8 => "Aout",
        9 => "Septembre",
        10 => "Octobre",
        11 => "Novembre",
        12 => "Decembre",
        _ => "Date inconnue",
    }
}

fn viewer_preload_candidates(photos: &[Photo], current_id: i64, radius: usize) -> Vec<Photo> {
    let Some(current_index) = photos.iter().position(|photo| photo.id == current_id) else {
        return Vec::new();
    };

    let mut candidates = Vec::new();
    for offset in 1..=radius {
        if let Some(index) = current_index.checked_sub(offset) {
            candidates.push(photos[index].clone());
        }
        if let Some(photo) = photos.get(current_index + offset) {
            candidates.push(photo.clone());
        }
    }
    candidates
}

impl eframe::App for MyCasaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_frame_metrics();
        self.poll_background_work();
        if self.search_debouncer.should_run(Instant::now()) {
            self.refresh_photos();
        }

        if self.viewer.is_open() {
            self.viewer.show_docked(ctx);
        } else {
            self.viewer.poll_background_results();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_grid_width_uses_panel_rect_width() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1460.0, 900.0));

        assert_eq!(library_grid_width(rect), 1460.0);
    }

    #[test]
    fn inertial_scroll_adds_velocity_from_wheel_delta() {
        let mut scroll = InertialScrollState {
            max_offset: 1000.0,
            ..Default::default()
        };

        let active = scroll.tick(-120.0, 1.0 / 60.0);

        assert!(active);
        assert!(scroll.offset > 0.0);
        assert!(scroll.velocity > 0.0);
    }

    #[test]
    fn inertial_scroll_continues_and_slows_without_wheel_delta() {
        let mut scroll = InertialScrollState {
            offset: 100.0,
            velocity: 900.0,
            max_offset: 1000.0,
        };

        let active = scroll.tick(0.0, 1.0 / 60.0);

        assert!(active);
        assert!(scroll.offset > 100.0);
        assert!(scroll.velocity < 900.0);
    }

    #[test]
    fn inertial_scroll_clamps_at_bounds() {
        let mut scroll = InertialScrollState {
            offset: 990.0,
            velocity: 2000.0,
            max_offset: 1000.0,
        };

        let active = scroll.tick(0.0, 1.0 / 10.0);

        assert!(!active);
        assert_eq!(scroll.offset, 1000.0);
        assert_eq!(scroll.velocity, 0.0);
    }

    #[test]
    fn viewer_preload_candidates_cover_radius_around_current_photo() {
        let photos: Vec<Photo> = (1..=7).map(photo_for_test).collect();

        let candidates = viewer_preload_candidates(&photos, 4, 3);
        let ids: Vec<i64> = candidates.iter().map(|photo| photo.id).collect();

        assert_eq!(ids, vec![3, 5, 2, 6, 1, 7]);
    }

    #[test]
    fn viewer_preload_candidates_stop_at_edges() {
        let photos: Vec<Photo> = (1..=4).map(photo_for_test).collect();

        let candidates = viewer_preload_candidates(&photos, 1, 3);
        let ids: Vec<i64> = candidates.iter().map(|photo| photo.id).collect();

        assert_eq!(ids, vec![2, 3, 4]);
    }

    #[test]
    fn retrochronological_rows_sort_descending_and_group_by_month() {
        let mut older = photo_for_test(1);
        older.modified_at = Some(1_704_067_200); // 2024-01-01
        let mut newest = photo_for_test(2);
        newest.modified_at = Some(1_709_251_200); // 2024-03-01
        let mut same_month = photo_for_test(3);
        same_month.modified_at = Some(1_709_164_800); // 2024-02-20
        let photos = vec![older, newest, same_month];

        let rows = retrochronological_grid_rows(&photos, 2);

        assert_eq!(
            rows,
            vec![
                ChronologicalGridRow::Section {
                    label: "Mars 2024".to_owned(),
                    photos: vec![1],
                },
                ChronologicalGridRow::Section {
                    label: "Fevrier 2024".to_owned(),
                    photos: vec![2],
                },
                ChronologicalGridRow::Section {
                    label: "Janvier 2024".to_owned(),
                    photos: vec![0],
                },
            ]
        );
    }

    #[test]
    fn retrochronological_rows_use_captured_at_before_modified_at() {
        let mut captured_newer = photo_for_test(1);
        captured_newer.modified_at = Some(1);
        captured_newer.captured_at = Some("2025-04-03T10:00:00Z".to_owned());
        let mut modified_newer = photo_for_test(2);
        modified_newer.modified_at = Some(1_800_000_000);
        let photos = vec![captured_newer, modified_newer];

        let rows = retrochronological_grid_rows(&photos, 3);

        assert_eq!(
            rows,
            vec![
                ChronologicalGridRow::Section {
                    label: "Janvier 2027".to_owned(),
                    photos: vec![1],
                },
                ChronologicalGridRow::Section {
                    label: "Avril 2025".to_owned(),
                    photos: vec![0],
                },
            ]
        );
    }

    #[test]
    fn chronological_layout_uses_compact_section_rows_and_normal_photo_rows() {
        let rows = vec![
            ChronologicalGridRow::Section {
                label: "Mars 2024".to_owned(),
                photos: vec![0, 1],
            },
            ChronologicalGridRow::Photos(vec![2, 3]),
        ];

        let layout = chronological_row_layout(&rows);

        assert_eq!(layout[0], (0.0, chrono_section_row_height()));
        assert_eq!(
            layout[1],
            (
                chrono_section_row_height(),
                chrono_section_row_height() + chrono_photo_row_height()
            )
        );
        assert!(chrono_section_row_height() < (TILE_HEIGHT + TILE_PADDING) * 2.0);
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
