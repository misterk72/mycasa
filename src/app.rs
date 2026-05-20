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
use crate::photo_limit::{
    INITIAL_PHOTO_LIMIT, next_photo_limit, photo_window_trim_count, previous_photo_page,
    should_extend_photo_window, should_prepend_photo_window,
};
use crate::scan_state::{begin_scan, finish_scan};
use crate::thumbnails::{ThumbnailCache, ThumbnailState};
use crate::ui_text::middle_truncate;
use crate::viewer::{NavigationDirection, ViewerNavigationRequest, ViewerState, adjacent_photo_id};

const THUMBNAIL_SIZE: f32 = 132.0;
const SMALL_THUMBNAIL_SIZE: f32 = 92.0;
const TILE_PADDING: f32 = 8.0;
#[cfg(test)]
const TILE_WIDTH: f32 = THUMBNAIL_SIZE + TILE_PADDING * 2.0;
#[cfg(test)]
const TILE_HEIGHT: f32 = THUMBNAIL_SIZE + 28.0;
const CHRONO_SECTION_HEIGHT: f32 = 44.0;
const TOP_CHROME_HEIGHT: f32 = 86.0;
const INERTIAL_SCROLL_WHEEL_MULTIPLIER: f32 = 0.9;
const INERTIAL_SCROLL_VELOCITY_MULTIPLIER: f32 = 0.32;
const INERTIAL_SCROLL_MIN_FLING_SPEED: f32 = 600.0;
const INERTIAL_SCROLL_MAX_SPEED: f32 = 5200.0;
const INERTIAL_SCROLL_FRICTION_PER_SECOND: f32 = 0.22;
const INERTIAL_SCROLL_STOP_SPEED: f32 = 18.0;
const INERTIAL_SCROLL_EXTERNAL_SYNC_EPSILON: f32 = 1.0;
const VIEWER_WHEEL_NAVIGATION_THRESHOLD: f32 = 90.0;
const VIEWER_WHEEL_NAVIGATION_COOLDOWN_SECONDS: f32 = 0.16;
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ThumbnailSizeMode {
    Small,
    #[default]
    Normal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PicasaFilterState {
    starred_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ChronologicalGridRow {
    Section {
        label: String,
        key: Option<ChronologySectionKey>,
        photos: Vec<usize>,
    },
    Photos(Vec<usize>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChronologySectionKey {
    year: i32,
    month: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ChronologicalSidebarSection {
    year: i32,
    months: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ChronologicalStickySection<'a> {
    label: &'a str,
    key: Option<ChronologySectionKey>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct InertialScrollState {
    offset: f32,
    velocity: f32,
    max_offset: f32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
struct ViewerWheelNavigationState {
    accumulated_delta: f32,
    cooldown_seconds: f32,
}

pub struct MyCasaApp {
    catalog: Result<Catalog, CatalogError>,
    indexer: Indexer,
    thumbnails: ThumbnailCache,
    viewer: ViewerState,
    photos: Vec<Photo>,
    photo_limit: usize,
    photo_window_start: usize,
    all_photos_loaded: bool,
    folders: Vec<PathBuf>,
    albums: Vec<String>,
    chronology_sections: Vec<ChronologicalSidebarSection>,
    selected_photo: Option<i64>,
    library_view_mode: LibraryViewMode,
    active_chronology_section: Option<ChronologySectionKey>,
    pending_chronology_scroll: Option<ChronologySectionKey>,
    sidebar_auto_scroll_target: Option<ChronologySectionKey>,
    thumbnail_size_mode: ThumbnailSizeMode,
    main_scroll: InertialScrollState,
    viewer_wheel_navigation: ViewerWheelNavigationState,
    status: String,
    search: String,
    search_debouncer: Debouncer,
    imported_since_refresh: usize,
    pending_catalog_writes: Vec<IndexedPhoto>,
    active_scan_folders: HashSet<PathBuf>,
    photo_filters: PicasaFilterState,
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
        let mut chronology_sections = Vec::new();
        let mut all_photos_loaded = true;

        if let Ok(catalog) = &catalog {
            match catalog.load_recent_photos(INITIAL_PHOTO_LIMIT) {
                Ok(loaded) => {
                    all_photos_loaded = loaded.len() < INITIAL_PHOTO_LIMIT;
                    photos = loaded;
                }
                Err(error) => {
                    status = format!("Catalogue ouvert, lecture photos impossible: {error}")
                }
            }
            if let Ok(loaded_folders) = catalog.load_folders(50) {
                folders = loaded_folders;
            }
            if let Ok(months) = catalog.chronology_months("") {
                chronology_sections = chronological_sidebar_sections_from_months(months);
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
            photo_window_start: 0,
            all_photos_loaded,
            folders,
            albums: vec![
                "Toutes les photos".to_owned(),
                "Favoris".to_owned(),
                "Import recent".to_owned(),
            ],
            chronology_sections,
            selected_photo: None,
            library_view_mode: LibraryViewMode::RetroChronological,
            active_chronology_section: None,
            pending_chronology_scroll: None,
            sidebar_auto_scroll_target: None,
            thumbnail_size_mode: ThumbnailSizeMode::Normal,
            main_scroll: InertialScrollState::default(),
            viewer_wheel_navigation: ViewerWheelNavigationState::default(),
            status,
            search: String::new(),
            search_debouncer: Debouncer::new(SEARCH_DEBOUNCE),
            imported_since_refresh: 0,
            pending_catalog_writes: Vec::new(),
            active_scan_folders: HashSet::new(),
            photo_filters: PicasaFilterState::default(),
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

    fn extend_photo_window(&mut self, columns: usize, chronological: bool) {
        if self.all_photos_loaded {
            return;
        }

        let previous_loaded_count = self.photos.len();
        let page_offset = self.photo_window_start + self.photos.len();
        let next_limit = next_photo_limit(self.photo_limit);
        let page_size = next_limit.saturating_sub(self.photo_limit);
        self.photo_limit = next_limit;

        if let Ok(catalog) = &self.catalog {
            match catalog.search_photos_page_filtered(
                &self.search,
                self.photo_filters.starred_only,
                page_size,
                page_offset,
            ) {
                Ok(mut photos) => {
                    self.all_photos_loaded = photos.len() < page_size;
                    self.photos.append(&mut photos);
                    let evicted = self.trim_photo_window(columns, chronological);
                    self.status = if self.all_photos_loaded {
                        format!("{} photo(s) dans le catalogue", self.photos.len())
                    } else if evicted > 0 {
                        format!(
                            "Chargement continu: fenetre {}-{} ({} evincees)",
                            self.photo_window_start + 1,
                            self.photo_window_start + self.photos.len(),
                            evicted
                        )
                    } else {
                        format!(
                            "Chargement continu: {} -> {} photo(s)",
                            previous_loaded_count,
                            self.photos.len()
                        )
                    };
                }
                Err(error) => self.status = format!("Erreur catalogue: {error}"),
            }
        }
    }

    fn trim_photo_window(&mut self, columns: usize, chronological: bool) -> usize {
        let evict_count = photo_window_trim_count(self.photos.len());
        if evict_count == 0 {
            return 0;
        }

        let scroll_delta = if chronological {
            chronological_removed_prefix_height(
                &self.photos,
                evict_count,
                columns,
                self.thumbnail_size_mode,
            )
        } else {
            folder_removed_prefix_height(evict_count, columns, self.thumbnail_size_mode)
        };
        self.photos.drain(..evict_count);
        self.photo_window_start += evict_count;
        self.main_scroll.offset = clamp_scroll_offset(
            self.main_scroll.offset - scroll_delta,
            self.main_scroll.max_offset,
        );
        evict_count
    }

    fn prepend_photo_window(&mut self, columns: usize, chronological: bool) {
        let Some((page_offset, page_size)) = previous_photo_page(self.photo_window_start) else {
            return;
        };

        if let Ok(catalog) = &self.catalog {
            match catalog.search_photos_page_filtered(
                &self.search,
                self.photo_filters.starred_only,
                page_size,
                page_offset,
            ) {
                Ok(mut photos) => {
                    if photos.is_empty() {
                        self.photo_window_start = 0;
                        return;
                    }

                    let scroll_delta = if chronological {
                        chronological_prepended_prefix_height(
                            &photos,
                            &self.photos,
                            columns,
                            self.thumbnail_size_mode,
                        )
                    } else {
                        folder_removed_prefix_height(
                            photos.len(),
                            columns,
                            self.thumbnail_size_mode,
                        )
                    };
                    let prepended = photos.len();
                    photos.append(&mut self.photos);
                    self.photos = photos;
                    self.photo_window_start = page_offset;
                    self.main_scroll.offset =
                        clamp_scroll_offset(self.main_scroll.offset + scroll_delta, f32::MAX);

                    let evicted_tail = self.trim_photo_window_tail();
                    if evicted_tail > 0 {
                        self.all_photos_loaded = false;
                        self.photo_limit = self.photo_window_start + self.photos.len();
                    }
                    self.status = format!(
                        "Chargement precedent: {} photo(s), fenetre {}-{}",
                        prepended,
                        self.photo_window_start + 1,
                        self.photo_window_start + self.photos.len()
                    );
                }
                Err(error) => self.status = format!("Erreur catalogue: {error}"),
            }
        }
    }

    fn trim_photo_window_tail(&mut self) -> usize {
        let evict_count = photo_window_trim_count(self.photos.len());
        if evict_count == 0 {
            return 0;
        }

        let keep = self.photos.len() - evict_count;
        self.photos.truncate(keep);
        evict_count
    }

    fn refresh_photos(&mut self) {
        if let Ok(catalog) = &self.catalog {
            let photos_result = catalog.search_photos_page_filtered(
                &self.search,
                self.photo_filters.starred_only,
                self.photo_limit,
                0,
            );
            let chronology_result =
                catalog.chronology_months_filtered(&self.search, self.photo_filters.starred_only);
            match photos_result {
                Ok(photos) => {
                    self.all_photos_loaded = photos.len() < self.photo_limit;
                    self.photos = photos;
                    if let Ok(months) = chronology_result {
                        self.chronology_sections =
                            chronological_sidebar_sections_from_months(months);
                    }
                    self.photo_window_start = 0;
                    self.status = if self.all_photos_loaded {
                        format!("{} photo(s) dans le catalogue", self.photos.len())
                    } else {
                        format!("{} photo(s) chargee(s), suite au scroll", self.photos.len())
                    };
                }
                Err(error) => self.status = format!("Erreur catalogue: {error}"),
            }
        }
    }

    fn select_chronology_section(&mut self, key: ChronologySectionKey) {
        self.library_view_mode = LibraryViewMode::RetroChronological;
        self.active_chronology_section = Some(key);
        self.pending_chronology_scroll = Some(key);
        self.load_photo_window_at_month(key);
    }

    fn load_photo_window_at_month(&mut self, key: ChronologySectionKey) {
        if let Ok(catalog) = &self.catalog {
            let offset_result = if self.photo_filters.starred_only {
                catalog.photo_offset_for_month_filtered(&self.search, true, key.year, key.month)
            } else {
                catalog.photo_offset_for_month(&self.search, key.year, key.month)
            };
            let Ok(Some(offset)) = offset_result else {
                return;
            };
            match catalog.search_photos_page_filtered(
                &self.search,
                self.photo_filters.starred_only,
                INITIAL_PHOTO_LIMIT,
                offset,
            ) {
                Ok(photos) => {
                    self.all_photos_loaded = photos.len() < INITIAL_PHOTO_LIMIT;
                    self.photo_window_start = offset;
                    self.photo_limit = offset + photos.len();
                    self.photos = photos;
                    self.main_scroll.offset = 0.0;
                    self.main_scroll.velocity = 0.0;
                }
                Err(error) => self.status = format!("Erreur catalogue: {error}"),
            }
        }
    }

    fn reset_photo_window(&mut self) {
        self.photo_limit = INITIAL_PHOTO_LIMIT;
        self.photo_window_start = 0;
        self.all_photos_loaded = false;
        self.main_scroll.offset = 0.0;
        self.main_scroll.velocity = 0.0;
    }

    fn maybe_extend_photo_window_after_scroll(&mut self, columns: usize, chronological: bool) {
        if should_extend_photo_window(
            self.main_scroll.offset,
            self.main_scroll.max_offset,
            self.photos.len(),
            self.all_photos_loaded,
        ) {
            self.extend_photo_window(columns, chronological);
        }
    }

    fn maybe_prepend_photo_window_after_scroll(&mut self, columns: usize, chronological: bool) {
        if should_prepend_photo_window(
            self.main_scroll.offset,
            self.photos.len(),
            self.photo_window_start,
        ) {
            self.prepend_photo_window(columns, chronological);
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
        let sidebar_hovered = ui
            .ctx()
            .input(|input| input.pointer.hover_pos())
            .is_some_and(|position| ui.max_rect().contains(position));
        if sidebar_hovered {
            self.sidebar_auto_scroll_target = None;
        }
        ScrollArea::vertical()
            .id_salt("sidebar_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
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

                let chronology_sections = self.chronology_sections.clone();
                egui::CollapsingHeader::new(format!("Chronologie ({})", chronology_sections.len()))
                    .default_open(self.library_view_mode == LibraryViewMode::RetroChronological)
                    .show(ui, |ui| {
                        if chronology_sections.is_empty() {
                            ui.label("Aucune date");
                        } else {
                            for section in chronology_sections {
                                let year_active = self.library_view_mode
                                    == LibraryViewMode::RetroChronological
                                    && self
                                        .active_chronology_section
                                        .is_some_and(|active| active.year == section.year);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("▾")
                                            .size(12.0)
                                            .color(Color32::from_gray(110)),
                                    );
                                    if ui
                                        .selectable_label(year_active, section.year.to_string())
                                        .clicked()
                                    {
                                        if let Some(key) = section.months.first().map(|month| {
                                            ChronologySectionKey {
                                                year: section.year,
                                                month: *month,
                                            }
                                        }) {
                                            self.select_chronology_section(key);
                                        }
                                        self.status = format!("Chronologie: {}", section.year);
                                    }
                                });
                                for month in section.months {
                                    ui.horizontal(|ui| {
                                        ui.add_space(18.0);
                                        ui.label(
                                            RichText::new("▫")
                                                .size(12.0)
                                                .color(Color32::from_gray(120)),
                                        );
                                        let month_label = month_name(month);
                                        let month_key = ChronologySectionKey {
                                            year: section.year,
                                            month,
                                        };
                                        let month_active = self.library_view_mode
                                            == LibraryViewMode::RetroChronological
                                            && self.active_chronology_section == Some(month_key);
                                        let response = ui
                                            .selectable_label(month_active, month_label)
                                            .on_hover_text(format!(
                                                "{month_label} {}",
                                                section.year
                                            ));
                                        let (should_scroll, next_target) =
                                            sidebar_auto_scroll_action(
                                                self.sidebar_auto_scroll_target,
                                                Some(month_key),
                                                sidebar_hovered,
                                            );
                                        if month_active && should_scroll {
                                            response.scroll_to_me(Some(Align::Center));
                                        }
                                        self.sidebar_auto_scroll_target = next_target;
                                        if response.clicked() {
                                            self.select_chronology_section(month_key);
                                            self.status = format!(
                                                "Chronologie: {month_label} {}",
                                                section.year
                                            );
                                        }
                                    });
                                }
                            }
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
                                    ui.label(
                                        RichText::new("▸")
                                            .size(12.0)
                                            .color(Color32::from_gray(110)),
                                    );
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
            });
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
            for label in picasa_top_toolbar_button_labels() {
                let enabled = picasa_top_toolbar_button_enabled(label);
                let response = ui.add_enabled(enabled, egui::Button::new(label));
                if !enabled {
                    response.on_hover_text("Fonctionnalite pas encore implementee");
                    continue;
                }

                if response.clicked() {
                    match label {
                        "Importer" => self.pick_folder_and_scan(),
                        "Dossiers" => {
                            self.library_view_mode = LibraryViewMode::FolderTree;
                            self.status = "Vue dossiers".to_owned();
                        }
                        "Chronologie" => {
                            self.library_view_mode = LibraryViewMode::RetroChronological;
                            self.status = "Vue retrochronologique".to_owned();
                        }
                        "Scanner dossier courant" => self.scan_current_dir(),
                        _ => {}
                    }
                }
            }
        });
        ui.add_space(2.0);
        self.ui_filter_strip(ui);
    }

    fn ui_filter_strip(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_space(260.0);
            ui.label(RichText::new("Filtres").color(Color32::from_gray(95)));
            for label in picasa_filter_button_labels() {
                if picasa_filter_button_enabled(label) {
                    if ui
                        .selectable_label(self.photo_filters.starred_only, label)
                        .on_hover_text("Afficher uniquement les favoris Picasa")
                        .clicked()
                    {
                        self.photo_filters.starred_only = !self.photo_filters.starred_only;
                        self.reset_photo_window();
                        self.refresh_photos();
                        self.status = picasa_filter_status_text(self.photo_filters).to_owned();
                    }
                } else {
                    ui.add_enabled(false, egui::Button::new(label).small())
                        .on_hover_text("Filtre pas encore implemente");
                }
            }
            let mut filter_strength = 0.0_f32;
            ui.add_enabled_ui(false, |ui| {
                let _ = ui.add_sized(
                    [96.0, 18.0],
                    egui::Slider::new(&mut filter_strength, 0.0..=1.0),
                );
            })
            .response
            .on_hover_text("Intensite de filtre pas encore implementee");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let response = ui.add_sized(
                    [360.0, 22.0],
                    egui::TextEdit::singleline(&mut self.search).hint_text("Rechercher"),
                );
                if response.changed() {
                    self.search_debouncer.mark_changed(Instant::now());
                }
                if ui.small_button("🔎").clicked() {
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
                let loading_status = if self.all_photos_loaded {
                    "catalogue complet"
                } else {
                    "scroll continu"
                };
                ui.label(RichText::new(loading_status).color(Color32::from_gray(115)));
            });
        });
        ui.add_space(4.0);
        self.ui_collection_actions(ui);
        ui.add_space(4.0);
        ui.label(RichText::new("Ajouter une description").color(Color32::from_gray(170)));
        ui.add_space(6.0);
    }

    fn ui_collection_actions(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for action in picasa_collection_action_labels() {
                ui.add_enabled_ui(false, |ui| {
                    let _ = ui.add_sized(
                        [picasa_collection_action_width(action), 24.0],
                        egui::Button::new(action),
                    );
                })
                .response
                .on_hover_text("Action pas encore implementee");
            }
        });
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
            if ui
                .selectable_label(
                    self.thumbnail_size_mode == ThumbnailSizeMode::Small,
                    "Petites vignettes",
                )
                .clicked()
            {
                self.thumbnail_size_mode = ThumbnailSizeMode::Small;
                self.main_scroll.offset = 0.0;
                self.main_scroll.velocity = 0.0;
                self.status = thumbnail_mode_status_text(self.thumbnail_size_mode).to_owned();
            }
            if ui
                .selectable_label(
                    self.thumbnail_size_mode == ThumbnailSizeMode::Normal,
                    RichText::new("Vignettes normales").color(PICASA_BLUE),
                )
                .clicked()
            {
                self.thumbnail_size_mode = ThumbnailSizeMode::Normal;
                self.main_scroll.offset = 0.0;
                self.main_scroll.velocity = 0.0;
                self.status = thumbnail_mode_status_text(self.thumbnail_size_mode).to_owned();
            }
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
        let tile_width = tile_width(self.thumbnail_size_mode);
        let columns = columns_for_width(grid_width, tile_width);
        let row_left_padding = centered_grid_left_padding(grid_width, columns, tile_width);
        let total_rows = row_count(self.photos.len(), columns);
        let row_height = photo_row_height(self.thumbnail_size_mode);
        self.update_main_scroll(ui, total_rows as f32 * row_height);

        let output =
            self.main_scroll_area()
                .show_rows(ui, row_height, total_rows, |ui, row_range| {
                    for row_index in row_range {
                        ui.horizontal(|ui| {
                            ui.add_space(row_left_padding);
                            for photo_index in
                                item_range_for_row(row_index, columns, self.photos.len())
                            {
                                let photo = self.photos[photo_index].clone();
                                self.photo_tile(ui, &photo);
                            }
                        });
                    }
                });
        self.sync_main_scroll_from_output(
            ui,
            output.state.offset.y,
            output.content_size,
            output.inner_rect,
        );
        self.maybe_prepend_photo_window_after_scroll(columns, false);
        self.maybe_extend_photo_window_after_scroll(columns, false);
    }

    fn ui_retrochronological_grid(&mut self, ui: &mut egui::Ui) {
        let grid_width = library_grid_width(ui.max_rect());
        let tile_width = tile_width(self.thumbnail_size_mode);
        let columns = columns_for_width(grid_width, tile_width);
        let row_left_padding = centered_grid_left_padding(grid_width, columns, tile_width);
        let rows = retrochronological_grid_rows(&self.photos, columns);
        let row_layout = chronological_row_layout(&rows, self.thumbnail_size_mode);
        let total_height = row_layout.last().map(|(_, bottom)| *bottom).unwrap_or(0.0);
        self.update_main_scroll(ui, total_height);
        if let Some(target) = self.pending_chronology_scroll.take() {
            if let Some(offset) = chronological_section_offset(&rows, &row_layout, target) {
                self.main_scroll.offset = clamp_scroll_offset(offset, self.main_scroll.max_offset);
                self.main_scroll.velocity = 0.0;
            }
        }

        let previous_active_chronology_section = self.active_chronology_section;
        let mut active_chronology_section = self.active_chronology_section;
        let output = self.main_scroll_area().show_viewport(ui, |ui, viewport| {
            ui.set_min_height(total_height);
            active_chronology_section =
                chronological_sticky_section(&rows, &row_layout, viewport.top())
                    .and_then(|section| section.key);

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
                    ChronologicalGridRow::Section { label, photos, .. } => {
                        self.chronology_section_row(ui, row_rect, row_left_padding, label, photos);
                    }
                    ChronologicalGridRow::Photos(photo_indices) => {
                        self.chronology_photo_row(ui, row_rect, row_left_padding, photo_indices);
                    }
                }
            }
            if let Some(section) = chronological_sticky_section(&rows, &row_layout, viewport.top())
            {
                let sticky_rect = egui::Rect::from_min_size(
                    egui::pos2(origin.x, origin.y + viewport.top()),
                    Vec2::new(grid_width, CHRONO_SECTION_HEIGHT),
                );
                draw_chronology_header(ui, sticky_rect, section.label);
            }
        });
        self.active_chronology_section = active_chronology_section;
        if active_chronology_section != previous_active_chronology_section {
            self.sidebar_auto_scroll_target = active_chronology_section;
        }
        self.sync_main_scroll_from_output(
            ui,
            output.state.offset.y,
            output.content_size,
            output.inner_rect,
        );
        self.maybe_prepend_photo_window_after_scroll(columns, true);
        self.maybe_extend_photo_window_after_scroll(columns, true);
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

    fn update_main_scroll(&mut self, ui: &egui::Ui, content_height: f32) {
        self.main_scroll
            .set_max_offset(content_height - ui.available_height());
        let hovered = ui
            .ctx()
            .input(|input| input.pointer.hover_pos())
            .is_some_and(|position| ui.max_rect().contains(position));
        let (wheel_delta, dt) = ui.ctx().input(|input| {
            let wheel_delta = if hovered {
                effective_wheel_delta(input.raw_scroll_delta.y, input.smooth_scroll_delta.y)
            } else {
                0.0
            };
            (wheel_delta, input.stable_dt.clamp(1.0 / 240.0, 0.1))
        });

        if self.main_scroll.tick(wheel_delta, dt) {
            ui.ctx().request_repaint();
        }
    }

    fn sync_main_scroll_from_output(
        &mut self,
        ui: &egui::Ui,
        output_offset: f32,
        content_size: Vec2,
        inner_rect: egui::Rect,
    ) {
        let max_offset = (content_size.y - inner_rect.height()).max(0.0);
        let external_scroll_active = ui.ctx().input(|input| input.pointer.any_down());
        self.main_scroll
            .sync_from_scroll_area(output_offset, max_offset, external_scroll_active);
    }

    fn chronology_section_row(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        row_left_padding: f32,
        label: &str,
        photo_indices: &[usize],
    ) {
        let available = rect.width();
        let header_rect = egui::Rect::from_min_size(
            rect.min + Vec2::new(0.0, 2.0),
            Vec2::new(available, CHRONO_SECTION_HEIGHT),
        );
        draw_chronology_header(ui, header_rect, label);

        let photos_rect = egui::Rect::from_min_size(
            egui::pos2(
                rect.left() + row_left_padding,
                header_rect.bottom() + TILE_PADDING,
            ),
            Vec2::new(
                (available - row_left_padding).max(0.0),
                tile_height(self.thumbnail_size_mode),
            ),
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
        row_left_padding: f32,
        photo_indices: &[usize],
    ) {
        let photos_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + row_left_padding, rect.top()),
            Vec2::new((rect.width() - row_left_padding).max(0.0), rect.height()),
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
            self.viewer_wheel_navigation.reset();
            return;
        }

        if let Some(request) = self.viewer.take_navigation_request() {
            self.handle_viewer_navigation_request(request);
            return;
        }

        let direction = ctx.input(|input| {
            if input.key_pressed(egui::Key::ArrowLeft) {
                Some(NavigationDirection::Previous)
            } else if input.key_pressed(egui::Key::ArrowRight) {
                Some(NavigationDirection::Next)
            } else {
                let wheel_delta =
                    effective_wheel_delta(input.raw_scroll_delta.y, input.smooth_scroll_delta.y);
                self.viewer_wheel_navigation
                    .tick(wheel_delta, input.stable_dt.clamp(1.0 / 240.0, 0.1))
            }
        });

        let Some(direction) = direction else {
            return;
        };
        self.navigate_viewer(current_id, direction);
    }

    fn handle_viewer_navigation_request(&mut self, request: ViewerNavigationRequest) {
        match request {
            ViewerNavigationRequest::Direction(direction) => {
                let Some(current_id) = self.viewer.current_photo_id() else {
                    return;
                };
                self.navigate_viewer(current_id, direction);
            }
            ViewerNavigationRequest::Photo(photo_id) => {
                self.viewer_wheel_navigation.reset();
                let direction = self.viewer.current_photo_id().and_then(|current_id| {
                    viewer_direction_between_photos(&self.photos, current_id, photo_id)
                });
                self.open_viewer_photo_internal(photo_id, direction);
            }
        }
    }

    fn navigate_viewer(&mut self, current_id: i64, direction: NavigationDirection) {
        let Some(next_id) = adjacent_photo_id(&self.photos, current_id, direction) else {
            return;
        };
        self.open_viewer_photo_with_direction(next_id, direction);
    }

    fn open_viewer_photo_with_direction(&mut self, photo_id: i64, direction: NavigationDirection) {
        self.open_viewer_photo_internal(photo_id, Some(direction));
    }

    fn open_viewer_photo_internal(
        &mut self,
        photo_id: i64,
        direction: Option<NavigationDirection>,
    ) {
        let Some(photo) = self
            .photos
            .iter()
            .find(|photo| photo.id == photo_id)
            .cloned()
        else {
            return;
        };

        self.selected_photo = Some(photo.id);
        if let Some(direction) = direction {
            self.viewer.open_with_direction(photo, direction);
        } else {
            self.viewer.open(photo);
        }
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
        let thumbnail_size = thumbnail_size(self.thumbnail_size_mode);
        let desired_size = Vec2::new(
            thumbnail_size + TILE_PADDING,
            tile_height(self.thumbnail_size_mode),
        );
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
            Vec2::splat(thumbnail_size - TILE_PADDING),
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
                ui.add_enabled(false, egui::Button::new("⚙").small())
                    .on_hover_text("Reglages personnes pas encore implementes");
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
                        "{} photos | win:{}-{} | cache:{} gen:{} pending:{} active:{} fps:{:.0}",
                        self.photos.len(),
                        self.photo_window_start + 1,
                        self.photo_window_start + self.photos.len(),
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
                for label in picasa_bottom_action_labels() {
                    ui.add_enabled_ui(false, |ui| {
                        let _ = ui.add_sized([82.0, 24.0], egui::Button::new(label));
                    })
                    .response
                    .on_hover_text("Action pas encore implementee");
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

fn centered_grid_left_padding(available_width: f32, columns: usize, tile_width: f32) -> f32 {
    let used_width = columns.max(1) as f32 * tile_width;
    ((available_width - used_width) / 2.0).max(0.0)
}

impl InertialScrollState {
    fn set_max_offset(&mut self, max_offset: f32) {
        self.max_offset = max_offset.max(0.0);
        self.offset = clamp_scroll_offset(self.offset, self.max_offset);
        if self.max_offset <= 0.0 {
            self.velocity = 0.0;
        }
    }

    fn tick(&mut self, wheel_delta: f32, dt: f32) -> bool {
        if wheel_delta.abs() > f32::EPSILON {
            let applied_delta = -wheel_delta * INERTIAL_SCROLL_WHEEL_MULTIPLIER;
            self.velocity = wheel_fling_velocity(self.velocity, applied_delta, dt);
            return self.advance_with_velocity(dt);
        }

        self.advance_with_velocity(dt)
    }

    fn advance_with_velocity(&mut self, dt: f32) -> bool {
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

    fn sync_from_scroll_area(
        &mut self,
        offset: f32,
        max_offset: f32,
        external_scroll_active: bool,
    ) {
        self.max_offset = max_offset.max(0.0);
        let scroll_area_offset = clamp_scroll_offset(offset, self.max_offset);
        let external_scroll_changed =
            (scroll_area_offset - self.offset).abs() > INERTIAL_SCROLL_EXTERNAL_SYNC_EPSILON;

        if external_scroll_active && external_scroll_changed {
            self.offset = scroll_area_offset;
            self.velocity = 0.0;
        } else if self.velocity.abs() <= INERTIAL_SCROLL_STOP_SPEED {
            self.offset = scroll_area_offset;
        } else {
            self.offset = clamp_scroll_offset(self.offset, self.max_offset);
        }

        if self.offset <= 0.0 || self.offset >= self.max_offset {
            self.velocity = 0.0;
        }
    }
}

impl ViewerWheelNavigationState {
    fn tick(&mut self, wheel_delta: f32, dt: f32) -> Option<NavigationDirection> {
        self.cooldown_seconds = (self.cooldown_seconds - dt).max(0.0);

        if wheel_delta.abs() <= f32::EPSILON {
            return None;
        }

        if self.accumulated_delta.signum() != wheel_delta.signum() {
            self.accumulated_delta = 0.0;
        }
        self.accumulated_delta += wheel_delta;

        if self.cooldown_seconds > 0.0 {
            return None;
        }

        let direction = viewer_wheel_navigation_direction(self.accumulated_delta)?;
        self.accumulated_delta = 0.0;
        self.cooldown_seconds = VIEWER_WHEEL_NAVIGATION_COOLDOWN_SECONDS;
        Some(direction)
    }

    fn reset(&mut self) {
        self.accumulated_delta = 0.0;
        self.cooldown_seconds = 0.0;
    }
}

fn wheel_fling_velocity(current_velocity: f32, applied_delta: f32, dt: f32) -> f32 {
    let raw_velocity = applied_delta / dt.max(1.0 / 240.0) * INERTIAL_SCROLL_VELOCITY_MULTIPLIER;
    let direction = raw_velocity.signum();
    if direction == 0.0 {
        return current_velocity;
    }

    let current_same_direction = if current_velocity.signum() == direction {
        current_velocity
    } else {
        0.0
    };
    let boosted = current_same_direction + raw_velocity;
    let minimum = direction * INERTIAL_SCROLL_MIN_FLING_SPEED;

    if boosted.abs() < INERTIAL_SCROLL_MIN_FLING_SPEED {
        minimum
    } else {
        boosted.clamp(-INERTIAL_SCROLL_MAX_SPEED, INERTIAL_SCROLL_MAX_SPEED)
    }
}

fn effective_wheel_delta(raw_scroll_delta: f32, smooth_scroll_delta: f32) -> f32 {
    if raw_scroll_delta.abs() > f32::EPSILON {
        raw_scroll_delta
    } else {
        smooth_scroll_delta
    }
}

fn viewer_wheel_navigation_direction(wheel_delta: f32) -> Option<NavigationDirection> {
    if wheel_delta <= -VIEWER_WHEEL_NAVIGATION_THRESHOLD {
        Some(NavigationDirection::Next)
    } else if wheel_delta >= VIEWER_WHEEL_NAVIGATION_THRESHOLD {
        Some(NavigationDirection::Previous)
    } else {
        None
    }
}

fn viewer_direction_between_photos(
    photos: &[Photo],
    current_id: i64,
    target_id: i64,
) -> Option<NavigationDirection> {
    let current_index = photos.iter().position(|photo| photo.id == current_id)?;
    let target_index = photos.iter().position(|photo| photo.id == target_id)?;
    match target_index.cmp(&current_index) {
        std::cmp::Ordering::Less => Some(NavigationDirection::Previous),
        std::cmp::Ordering::Greater => Some(NavigationDirection::Next),
        std::cmp::Ordering::Equal => None,
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
                key: photo_section_key(&photos[photo_index]),
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

fn chrono_section_row_height(mode: ThumbnailSizeMode) -> f32 {
    CHRONO_SECTION_HEIGHT + photo_row_height(mode)
}

fn chrono_photo_row_height(mode: ThumbnailSizeMode) -> f32 {
    photo_row_height(mode)
}

fn chronological_row_height(row: &ChronologicalGridRow, mode: ThumbnailSizeMode) -> f32 {
    match row {
        ChronologicalGridRow::Section { .. } => chrono_section_row_height(mode),
        ChronologicalGridRow::Photos(_) => chrono_photo_row_height(mode),
    }
}

fn chronological_row_layout(
    rows: &[ChronologicalGridRow],
    mode: ThumbnailSizeMode,
) -> Vec<(f32, f32)> {
    let mut top = 0.0;
    rows.iter()
        .map(|row| {
            let bottom = top + chronological_row_height(row, mode);
            let bounds = (top, bottom);
            top = bottom;
            bounds
        })
        .collect()
}

fn chronological_sticky_section<'a>(
    rows: &'a [ChronologicalGridRow],
    row_layout: &[(f32, f32)],
    viewport_top: f32,
) -> Option<ChronologicalStickySection<'a>> {
    let mut active_label = None;
    for (row, (top, _)) in rows.iter().zip(row_layout.iter()) {
        if *top > viewport_top {
            break;
        }
        if let ChronologicalGridRow::Section { label, key, .. } = row {
            active_label = Some(ChronologicalStickySection {
                label: label.as_str(),
                key: *key,
            });
        }
    }

    active_label.or_else(|| {
        rows.iter().find_map(|row| match row {
            ChronologicalGridRow::Section { label, key, .. } => Some(ChronologicalStickySection {
                label: label.as_str(),
                key: *key,
            }),
            ChronologicalGridRow::Photos(_) => None,
        })
    })
}

fn chronological_section_offset(
    rows: &[ChronologicalGridRow],
    row_layout: &[(f32, f32)],
    target: ChronologySectionKey,
) -> Option<f32> {
    rows.iter()
        .zip(row_layout.iter())
        .find_map(|(row, (top, _))| match row {
            ChronologicalGridRow::Section { key, .. } if *key == Some(target) => Some(*top),
            _ => None,
        })
}

fn sidebar_auto_scroll_action(
    target: Option<ChronologySectionKey>,
    row: Option<ChronologySectionKey>,
    sidebar_hovered: bool,
) -> (bool, Option<ChronologySectionKey>) {
    if sidebar_hovered {
        return (false, None);
    }

    match (target, row) {
        (Some(target), Some(row)) if target == row => (true, None),
        _ => (false, target),
    }
}

fn folder_removed_prefix_height(
    removed_photos: usize,
    columns: usize,
    mode: ThumbnailSizeMode,
) -> f32 {
    let removed_rows = row_count(removed_photos, columns.max(1));
    removed_rows as f32 * photo_row_height(mode)
}

fn chronological_removed_prefix_height(
    photos: &[Photo],
    removed_photos: usize,
    columns: usize,
    mode: ThumbnailSizeMode,
) -> f32 {
    let prefix_len = removed_photos.min(photos.len());
    let rows = retrochronological_grid_rows(&photos[..prefix_len], columns);
    chronological_row_layout(&rows, mode)
        .last()
        .map(|(_, bottom)| *bottom)
        .unwrap_or(0.0)
}

fn chronological_prepended_prefix_height(
    prepended: &[Photo],
    existing: &[Photo],
    columns: usize,
    mode: ThumbnailSizeMode,
) -> f32 {
    let mut combined = Vec::with_capacity(prepended.len() + existing.len());
    combined.extend_from_slice(prepended);
    combined.extend_from_slice(existing);

    chronological_total_height(&combined, columns, mode)
        - chronological_total_height(existing, columns, mode)
}

fn chronological_total_height(photos: &[Photo], columns: usize, mode: ThumbnailSizeMode) -> f32 {
    let rows = retrochronological_grid_rows(photos, columns);
    chronological_row_layout(&rows, mode)
        .last()
        .map(|(_, bottom)| *bottom)
        .unwrap_or(0.0)
}

fn draw_chronology_header(ui: &egui::Ui, rect: egui::Rect, label: &str) {
    ui.painter()
        .rect_filled(rect, 0.0, Color32::from_rgb(238, 239, 239));
    ui.painter().line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, CHROME_BORDER),
    );
    ui.painter().text(
        rect.left_center() + Vec2::new(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::TextStyle::Heading.resolve(ui.style()),
        Color32::from_rgb(174, 112, 45),
    );
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
    match photo_section_key(photo) {
        Some(key) => format!("{} {}", month_name(key.month), key.year),
        None => "Date inconnue".to_owned(),
    }
}

fn photo_section_key(photo: &Photo) -> Option<ChronologySectionKey> {
    let timestamp = photo_time_key(photo);
    if timestamp <= 0 {
        return None;
    }

    match Utc.timestamp_opt(timestamp, 0) {
        LocalResult::Single(datetime) => Some(ChronologySectionKey {
            year: datetime.year(),
            month: datetime.month(),
        }),
        _ => None,
    }
}

#[cfg(test)]
fn chronological_sidebar_years(photos: &[Photo]) -> Vec<i32> {
    chronological_sidebar_sections(photos)
        .into_iter()
        .map(|section| section.year)
        .collect()
}

#[cfg(test)]
fn chronological_sidebar_sections(photos: &[Photo]) -> Vec<ChronologicalSidebarSection> {
    let entries: Vec<(i32, u32)> = photos
        .iter()
        .filter_map(|photo| {
            let timestamp = photo_time_key(photo);
            if timestamp <= 0 {
                return None;
            }
            match Utc.timestamp_opt(timestamp, 0) {
                LocalResult::Single(datetime) => Some((datetime.year(), datetime.month())),
                _ => None,
            }
        })
        .collect();
    chronological_sidebar_sections_from_months(entries)
}

fn chronological_sidebar_sections_from_months(
    mut entries: Vec<(i32, u32)>,
) -> Vec<ChronologicalSidebarSection> {
    entries.sort_unstable_by(|left, right| right.cmp(left));
    entries.dedup();

    let mut sections: Vec<ChronologicalSidebarSection> = Vec::new();
    for (year, month) in entries {
        if let Some(index) = sections.iter().position(|section| section.year == year) {
            sections[index].months.push(month);
        } else {
            sections.push(ChronologicalSidebarSection {
                year,
                months: vec![month],
            });
        }
    }
    sections
}

fn picasa_filter_button_labels() -> [&'static str; 5] {
    ["★", "↑", "👤", "▦", "⌖"]
}

fn picasa_filter_button_enabled(label: &str) -> bool {
    label == "★"
}

fn picasa_filter_status_text(filters: PicasaFilterState) -> &'static str {
    if filters.starred_only {
        "Filtre favoris active"
    } else {
        "Filtre favoris desactive"
    }
}

fn thumbnail_size(mode: ThumbnailSizeMode) -> f32 {
    match mode {
        ThumbnailSizeMode::Small => SMALL_THUMBNAIL_SIZE,
        ThumbnailSizeMode::Normal => THUMBNAIL_SIZE,
    }
}

fn tile_width(mode: ThumbnailSizeMode) -> f32 {
    thumbnail_size(mode) + TILE_PADDING * 2.0
}

fn tile_height(mode: ThumbnailSizeMode) -> f32 {
    thumbnail_size(mode) + 28.0
}

fn photo_row_height(mode: ThumbnailSizeMode) -> f32 {
    tile_height(mode) + TILE_PADDING
}

fn thumbnail_mode_status_text(mode: ThumbnailSizeMode) -> &'static str {
    match mode {
        ThumbnailSizeMode::Small => "Petites vignettes",
        ThumbnailSizeMode::Normal => "Vignettes normales",
    }
}

fn picasa_top_toolbar_button_labels() -> [&'static str; 6] {
    [
        "Importer",
        "Diaporama",
        "Dossiers",
        "Chronologie",
        "CD cadeau",
        "Scanner dossier courant",
    ]
}

fn picasa_top_toolbar_button_enabled(label: &str) -> bool {
    matches!(
        label,
        "Importer" | "Dossiers" | "Chronologie" | "Scanner dossier courant"
    )
}

fn picasa_collection_action_labels() -> [&'static str; 6] {
    ["Lecture", "Photo", "Film", "Favori", "Partager", "Menu"]
}

fn picasa_collection_action_width(label: &str) -> f32 {
    match label {
        "Partager" => 96.0,
        "Lecture" => 74.0,
        _ => 58.0,
    }
}

fn picasa_bottom_action_labels() -> [&'static str; 7] {
    [
        "Album Web",
        "E-mail",
        "Imprimer",
        "Commander",
        "BlogThis!",
        "Montage",
        "Exporter",
    ]
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
            self.reset_photo_window();
            self.refresh_photos();
        }

        if self.viewer.is_open() {
            self.viewer
                .show_docked(ctx, &self.photos, &mut self.thumbnails);
        } else {
            self.viewer.poll_background_results();
            egui::TopBottomPanel::top("picasa_top_chrome")
                .exact_height(TOP_CHROME_HEIGHT)
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
    fn centered_grid_left_padding_splits_unused_grid_width() {
        assert_eq!(centered_grid_left_padding(1000.0, 6, 150.0), 50.0);
    }

    #[test]
    fn centered_grid_left_padding_never_goes_negative() {
        assert_eq!(centered_grid_left_padding(300.0, 3, 150.0), 0.0);
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
        assert!(scroll.velocity >= INERTIAL_SCROLL_MIN_FLING_SPEED);
        assert!(scroll.velocity <= INERTIAL_SCROLL_MAX_SPEED);
    }

    #[test]
    fn inertial_scroll_wheel_tick_uses_velocity_step_instead_of_full_wheel_jump() {
        let mut scroll = InertialScrollState {
            max_offset: 1000.0,
            ..Default::default()
        };

        scroll.tick(-120.0, 1.0 / 60.0);

        assert!(scroll.offset < 120.0 * INERTIAL_SCROLL_WHEEL_MULTIPLIER);
    }

    #[test]
    fn inertial_scroll_initializes_bounds_before_first_wheel_tick() {
        let mut scroll = InertialScrollState::default();

        scroll.set_max_offset(1000.0);
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
    fn inertial_scroll_wheel_tick_during_motion_keeps_same_direction_momentum() {
        let mut scroll = InertialScrollState {
            offset: 100.0,
            velocity: 700.0,
            max_offset: 1000.0,
        };

        let active = scroll.tick(-8.0, 1.0 / 60.0);

        assert!(active);
        assert!(scroll.offset > 100.0);
        assert!(scroll.velocity > 700.0);
    }

    #[test]
    fn wheel_fling_velocity_accumulates_same_direction_notches() {
        let first = wheel_fling_velocity(0.0, 10.0, 1.0 / 60.0);
        let second = wheel_fling_velocity(first, 10.0, 1.0 / 60.0);

        assert!(first >= INERTIAL_SCROLL_MIN_FLING_SPEED);
        assert!(second > first);
    }

    #[test]
    fn wheel_fling_velocity_standard_notch_leaves_headroom_for_acceleration() {
        let applied_delta = 120.0 * INERTIAL_SCROLL_WHEEL_MULTIPLIER;

        let first = wheel_fling_velocity(0.0, applied_delta, 1.0 / 60.0);
        let second = wheel_fling_velocity(first, applied_delta, 1.0 / 60.0);
        let third = wheel_fling_velocity(second, applied_delta, 1.0 / 60.0);

        assert!(first < INERTIAL_SCROLL_MAX_SPEED * 0.6);
        assert!(second > first);
        assert!(third > second);
        assert!(third <= INERTIAL_SCROLL_MAX_SPEED);
    }

    #[test]
    fn wheel_fling_velocity_resets_when_direction_changes() {
        let first = wheel_fling_velocity(5000.0, -10.0, 1.0 / 60.0);

        assert!(first <= -INERTIAL_SCROLL_MIN_FLING_SPEED);
    }

    #[test]
    fn inertial_scroll_sync_preserves_active_motion_when_offsets_match() {
        let mut scroll = InertialScrollState {
            offset: 180.0,
            velocity: 900.0,
            max_offset: 1000.0,
        };

        scroll.sync_from_scroll_area(180.0, 1000.0, false);

        assert_eq!(scroll.offset, 180.0);
        assert_eq!(scroll.velocity, 900.0);
    }

    #[test]
    fn inertial_scroll_sync_preserves_active_motion_when_output_lags() {
        let mut scroll = InertialScrollState {
            offset: 180.0,
            velocity: 900.0,
            max_offset: 1000.0,
        };

        scroll.sync_from_scroll_area(0.0, 1000.0, false);

        assert_eq!(scroll.offset, 180.0);
        assert_eq!(scroll.velocity, 900.0);
    }

    #[test]
    fn inertial_scroll_sync_adopts_external_drag_offset() {
        let mut scroll = InertialScrollState {
            offset: 180.0,
            velocity: 900.0,
            max_offset: 1000.0,
        };

        scroll.sync_from_scroll_area(260.0, 1000.0, true);

        assert_eq!(scroll.offset, 260.0);
        assert_eq!(scroll.velocity, 0.0);
    }

    #[test]
    fn wheel_delta_prefers_raw_input_over_smooth_input() {
        assert_eq!(effective_wheel_delta(-120.0, -8.0), -120.0);
        assert_eq!(effective_wheel_delta(0.0, -8.0), -8.0);
    }

    #[test]
    fn viewer_wheel_navigation_maps_wheel_direction_to_photos() {
        assert_eq!(
            viewer_wheel_navigation_direction(-120.0),
            Some(NavigationDirection::Next)
        );
        assert_eq!(
            viewer_wheel_navigation_direction(120.0),
            Some(NavigationDirection::Previous)
        );
        assert_eq!(viewer_wheel_navigation_direction(-4.0), None);
    }

    #[test]
    fn viewer_direction_between_photos_tracks_target_position() {
        let photos: Vec<Photo> = (10..=12).map(photo_for_test).collect();

        assert_eq!(
            viewer_direction_between_photos(&photos, 11, 12),
            Some(NavigationDirection::Next)
        );
        assert_eq!(
            viewer_direction_between_photos(&photos, 11, 10),
            Some(NavigationDirection::Previous)
        );
        assert_eq!(viewer_direction_between_photos(&photos, 11, 11), None);
        assert_eq!(viewer_direction_between_photos(&photos, 11, 99), None);
    }

    #[test]
    fn viewer_wheel_navigation_accumulates_smooth_wheel_deltas() {
        let mut wheel = ViewerWheelNavigationState::default();

        assert_eq!(wheel.tick(-30.0, 1.0 / 60.0), None);
        assert_eq!(wheel.tick(-30.0, 1.0 / 60.0), None);
        assert_eq!(
            wheel.tick(-30.0, 1.0 / 60.0),
            Some(NavigationDirection::Next)
        );
    }

    #[test]
    fn viewer_wheel_navigation_rate_limits_repeated_notches() {
        let mut wheel = ViewerWheelNavigationState::default();

        assert_eq!(
            wheel.tick(-120.0, 1.0 / 60.0),
            Some(NavigationDirection::Next)
        );
        assert_eq!(wheel.tick(-120.0, 1.0 / 60.0), None);
        assert_eq!(
            wheel.tick(-120.0, VIEWER_WHEEL_NAVIGATION_COOLDOWN_SECONDS),
            Some(NavigationDirection::Next)
        );
    }

    #[test]
    fn viewer_wheel_navigation_direction_change_resets_accumulator() {
        let mut wheel = ViewerWheelNavigationState::default();

        assert_eq!(wheel.tick(-60.0, 1.0 / 60.0), None);
        assert_eq!(wheel.tick(60.0, 1.0 / 60.0), None);
        assert_eq!(
            wheel.tick(60.0, 1.0 / 60.0),
            Some(NavigationDirection::Previous)
        );
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
                    key: Some(ChronologySectionKey {
                        year: 2024,
                        month: 3,
                    }),
                    photos: vec![1],
                },
                ChronologicalGridRow::Section {
                    label: "Fevrier 2024".to_owned(),
                    key: Some(ChronologySectionKey {
                        year: 2024,
                        month: 2,
                    }),
                    photos: vec![2],
                },
                ChronologicalGridRow::Section {
                    label: "Janvier 2024".to_owned(),
                    key: Some(ChronologySectionKey {
                        year: 2024,
                        month: 1,
                    }),
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
                    key: Some(ChronologySectionKey {
                        year: 2027,
                        month: 1,
                    }),
                    photos: vec![1],
                },
                ChronologicalGridRow::Section {
                    label: "Avril 2025".to_owned(),
                    key: Some(ChronologySectionKey {
                        year: 2025,
                        month: 4,
                    }),
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
                key: Some(ChronologySectionKey {
                    year: 2024,
                    month: 3,
                }),
                photos: vec![0, 1],
            },
            ChronologicalGridRow::Photos(vec![2, 3]),
        ];

        let layout = chronological_row_layout(&rows, ThumbnailSizeMode::Normal);

        assert_eq!(
            layout[0],
            (0.0, chrono_section_row_height(ThumbnailSizeMode::Normal))
        );
        assert_eq!(
            layout[1],
            (
                chrono_section_row_height(ThumbnailSizeMode::Normal),
                chrono_section_row_height(ThumbnailSizeMode::Normal)
                    + chrono_photo_row_height(ThumbnailSizeMode::Normal)
            )
        );
        assert!(
            chrono_section_row_height(ThumbnailSizeMode::Normal)
                < (TILE_HEIGHT + TILE_PADDING) * 2.0
        );
    }

    #[test]
    fn chronological_sticky_label_keeps_month_visible_inside_photo_rows() {
        let rows = vec![
            ChronologicalGridRow::Section {
                label: "Avril 2026".to_owned(),
                key: Some(ChronologySectionKey {
                    year: 2026,
                    month: 4,
                }),
                photos: vec![0, 1],
            },
            ChronologicalGridRow::Photos(vec![2, 3]),
            ChronologicalGridRow::Section {
                label: "Mars 2026".to_owned(),
                key: Some(ChronologySectionKey {
                    year: 2026,
                    month: 3,
                }),
                photos: vec![4, 5],
            },
        ];
        let layout = chronological_row_layout(&rows, ThumbnailSizeMode::Normal);

        assert_eq!(
            chronological_sticky_section(&rows, &layout, layout[1].0 + 8.0),
            Some(ChronologicalStickySection {
                label: "Avril 2026",
                key: Some(ChronologySectionKey {
                    year: 2026,
                    month: 4,
                }),
            })
        );
        assert_eq!(
            chronological_sticky_section(&rows, &layout, layout[2].0),
            Some(ChronologicalStickySection {
                label: "Mars 2026",
                key: Some(ChronologySectionKey {
                    year: 2026,
                    month: 3,
                }),
            })
        );
    }

    #[test]
    fn chronological_section_offset_finds_month_start_for_sidebar_navigation() {
        let rows = vec![
            ChronologicalGridRow::Section {
                label: "Mai 2026".to_owned(),
                key: Some(ChronologySectionKey {
                    year: 2026,
                    month: 5,
                }),
                photos: vec![0, 1],
            },
            ChronologicalGridRow::Photos(vec![2, 3]),
            ChronologicalGridRow::Section {
                label: "Avril 2026".to_owned(),
                key: Some(ChronologySectionKey {
                    year: 2026,
                    month: 4,
                }),
                photos: vec![4, 5],
            },
        ];
        let layout = chronological_row_layout(&rows, ThumbnailSizeMode::Normal);

        assert_eq!(
            chronological_section_offset(
                &rows,
                &layout,
                ChronologySectionKey {
                    year: 2026,
                    month: 4,
                },
            ),
            Some(layout[2].0)
        );
        assert_eq!(
            chronological_section_offset(
                &rows,
                &layout,
                ChronologySectionKey {
                    year: 2026,
                    month: 3,
                },
            ),
            None
        );
    }

    #[test]
    fn sidebar_auto_scroll_runs_once_for_matching_active_month() {
        let april = ChronologySectionKey {
            year: 2026,
            month: 4,
        };
        let march = ChronologySectionKey {
            year: 2026,
            month: 3,
        };

        assert_eq!(
            sidebar_auto_scroll_action(Some(april), Some(march), false),
            (false, Some(april))
        );
        assert_eq!(
            sidebar_auto_scroll_action(Some(april), Some(april), false),
            (true, None)
        );
    }

    #[test]
    fn sidebar_auto_scroll_is_cancelled_while_sidebar_is_hovered() {
        let april = ChronologySectionKey {
            year: 2026,
            month: 4,
        };

        assert_eq!(
            sidebar_auto_scroll_action(Some(april), Some(april), true),
            (false, None)
        );
    }

    #[test]
    fn removed_folder_prefix_height_preserves_scroll_position_after_eviction() {
        assert_eq!(
            folder_removed_prefix_height(10, 4, ThumbnailSizeMode::Normal),
            3.0 * (TILE_HEIGHT + TILE_PADDING)
        );
    }

    #[test]
    fn removed_chronological_prefix_height_accounts_for_month_headers() {
        let mut april_1 = photo_for_test(1);
        april_1.captured_at = Some("2026-04-10T10:00:00Z".to_owned());
        let mut april_2 = photo_for_test(2);
        april_2.captured_at = Some("2026-04-09T10:00:00Z".to_owned());
        let mut march = photo_for_test(3);
        march.captured_at = Some("2026-03-01T10:00:00Z".to_owned());

        let photos = vec![april_1, april_2, march];

        assert_eq!(
            chronological_removed_prefix_height(&photos, 2, 2, ThumbnailSizeMode::Normal),
            chrono_section_row_height(ThumbnailSizeMode::Normal)
        );
    }

    #[test]
    fn prepended_chronological_height_accounts_for_merged_month_header() {
        let mut newest_april = photo_for_test(1);
        newest_april.captured_at = Some("2026-04-12T10:00:00Z".to_owned());
        let mut existing_april_1 = photo_for_test(2);
        existing_april_1.captured_at = Some("2026-04-10T10:00:00Z".to_owned());
        let mut existing_april_2 = photo_for_test(3);
        existing_april_2.captured_at = Some("2026-04-09T10:00:00Z".to_owned());

        let prepended = vec![newest_april];
        let existing = vec![existing_april_1, existing_april_2];

        assert_eq!(
            chronological_prepended_prefix_height(
                &prepended,
                &existing,
                2,
                ThumbnailSizeMode::Normal
            ),
            chrono_photo_row_height(ThumbnailSizeMode::Normal)
        );
    }

    #[test]
    fn chronological_sidebar_years_are_sorted_descending_and_unique() {
        let mut photo_2024 = photo_for_test(1);
        photo_2024.modified_at = Some(1_704_067_200);
        let mut photo_2025 = photo_for_test(2);
        photo_2025.captured_at = Some("2025-04-03T10:00:00Z".to_owned());
        let mut same_2025 = photo_for_test(3);
        same_2025.modified_at = Some(1_746_144_000);

        assert_eq!(
            chronological_sidebar_years(&[photo_2024, photo_2025, same_2025]),
            vec![2025, 2024]
        );
    }

    #[test]
    fn chronological_sidebar_sections_include_months_per_year() {
        let mut april_2026 = photo_for_test(1);
        april_2026.captured_at = Some("2026-04-03T10:00:00Z".to_owned());
        let mut duplicate_april_2026 = photo_for_test(2);
        duplicate_april_2026.captured_at = Some("2026-04-10T10:00:00Z".to_owned());
        let mut march_2026 = photo_for_test(3);
        march_2026.captured_at = Some("2026-03-01T10:00:00Z".to_owned());
        let mut december_2025 = photo_for_test(4);
        december_2025.captured_at = Some("2025-12-24T10:00:00Z".to_owned());

        assert_eq!(
            chronological_sidebar_sections(&[
                march_2026,
                april_2026,
                december_2025,
                duplicate_april_2026,
            ]),
            vec![
                ChronologicalSidebarSection {
                    year: 2026,
                    months: vec![4, 3],
                },
                ChronologicalSidebarSection {
                    year: 2025,
                    months: vec![12],
                },
            ]
        );
    }

    #[test]
    fn picasa_filter_strip_uses_reference_style_button_count() {
        assert_eq!(picasa_filter_button_labels().len(), 5);
    }

    #[test]
    fn only_star_filter_is_enabled_for_now() {
        assert!(picasa_filter_button_enabled("★"));
        assert!(!picasa_filter_button_enabled("↑"));
        assert!(!picasa_filter_button_enabled("👤"));
    }

    #[test]
    fn favorite_filter_status_reflects_toggle_state() {
        assert_eq!(
            picasa_filter_status_text(PicasaFilterState { starred_only: true }),
            "Filtre favoris active"
        );
        assert_eq!(
            picasa_filter_status_text(PicasaFilterState {
                starred_only: false
            }),
            "Filtre favoris desactive"
        );
    }

    #[test]
    fn thumbnail_size_modes_keep_normal_layout_and_add_compact_layout() {
        assert_eq!(thumbnail_size(ThumbnailSizeMode::Normal), THUMBNAIL_SIZE);
        assert_eq!(tile_width(ThumbnailSizeMode::Normal), TILE_WIDTH);
        assert_eq!(tile_height(ThumbnailSizeMode::Normal), TILE_HEIGHT);
        assert!(tile_width(ThumbnailSizeMode::Small) < tile_width(ThumbnailSizeMode::Normal));
        assert!(
            photo_row_height(ThumbnailSizeMode::Small)
                < photo_row_height(ThumbnailSizeMode::Normal)
        );
    }

    #[test]
    fn thumbnail_mode_status_text_names_selected_mode() {
        assert_eq!(
            thumbnail_mode_status_text(ThumbnailSizeMode::Small),
            "Petites vignettes"
        );
        assert_eq!(
            thumbnail_mode_status_text(ThumbnailSizeMode::Normal),
            "Vignettes normales"
        );
    }

    #[test]
    fn top_toolbar_marks_only_real_actions_enabled() {
        let labels = picasa_top_toolbar_button_labels();

        assert_eq!(
            labels
                .iter()
                .copied()
                .filter(|label| picasa_top_toolbar_button_enabled(label))
                .collect::<Vec<_>>(),
            [
                "Importer",
                "Dossiers",
                "Chronologie",
                "Scanner dossier courant"
            ]
        );
        assert!(!picasa_top_toolbar_button_enabled("Diaporama"));
        assert!(!picasa_top_toolbar_button_enabled("CD cadeau"));
    }

    #[test]
    fn picasa_collection_actions_match_reference_header_controls() {
        assert_eq!(
            picasa_collection_action_labels(),
            ["Lecture", "Photo", "Film", "Favori", "Partager", "Menu"]
        );
        assert!(
            picasa_collection_action_width("Partager") > picasa_collection_action_width("Menu")
        );
    }

    #[test]
    fn picasa_bottom_tray_keeps_reference_action_count() {
        assert_eq!(
            picasa_bottom_action_labels(),
            [
                "Album Web",
                "E-mail",
                "Imprimer",
                "Commander",
                "BlogThis!",
                "Montage",
                "Exporter"
            ]
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
