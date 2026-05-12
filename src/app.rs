use std::path::PathBuf;

use egui::{Align, Color32, Layout, RichText, ScrollArea, Sense, Stroke, Vec2};

use crate::catalog::{Catalog, CatalogError, Photo};
use crate::indexer::{IndexJob, Indexer};
use crate::thumbnails::{ThumbnailCache, ThumbnailState};
use crate::viewer::ViewerState;

const THUMBNAIL_SIZE: f32 = 144.0;
const TILE_PADDING: f32 = 10.0;
const MAX_INDEX_EVENTS_PER_FRAME: usize = 80;

pub struct MyCasaApp {
    catalog: Result<Catalog, CatalogError>,
    indexer: Indexer,
    thumbnails: ThumbnailCache,
    viewer: ViewerState,
    photos: Vec<Photo>,
    folders: Vec<PathBuf>,
    albums: Vec<String>,
    selected_photo: Option<i64>,
    status: String,
    search: String,
    imported_since_refresh: usize,
}

impl MyCasaApp {
    pub fn new(_creation_context: &eframe::CreationContext<'_>) -> Self {
        let catalog = Catalog::open_default();
        let mut status = String::from("Catalogue pret");
        let mut photos = Vec::new();
        let mut folders = Vec::new();

        if let Ok(catalog) = &catalog {
            match catalog.load_recent_photos(250) {
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
            folders,
            albums: vec![
                "Toutes les photos".to_owned(),
                "Favoris".to_owned(),
                "Import recent".to_owned(),
            ],
            selected_photo: None,
            status,
            search: String::new(),
            imported_since_refresh: 0,
        }
    }

    fn refresh_photos(&mut self) {
        if let Ok(catalog) = &self.catalog {
            match catalog.search_photos(&self.search, 500) {
                Ok(photos) => {
                    self.photos = photos;
                    self.status = format!("{} photo(s) dans le catalogue", self.photos.len());
                }
                Err(error) => self.status = format!("Erreur catalogue: {error}"),
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
                    if let Ok(catalog) = &self.catalog {
                        match catalog.upsert_photo(&photo) {
                            Ok(()) => {
                                self.imported_since_refresh += 1;
                                if self.imported_since_refresh % 25 == 0 {
                                    self.refresh_photos();
                                }
                                self.status = format!("Photo indexee: {}", photo.path.display());
                            }
                            Err(error) => {
                                self.status = format!("Ecriture catalogue impossible: {error}");
                            }
                        }
                    }
                }
                IndexJob::Finished {
                    folder,
                    photos_found,
                } => {
                    self.status = format!(
                        "Indexation terminee: {} photo(s) trouvee(s) dans {}",
                        photos_found,
                        folder.display()
                    );
                    if !self.folders.iter().any(|existing| existing == &folder) {
                        self.folders.push(folder);
                    }
                    self.refresh_photos();
                }
                IndexJob::Failed(message) => {
                    self.status = format!("Indexation impossible: {message}");
                }
            }
        }

        if processed == MAX_INDEX_EVENTS_PER_FRAME {
            self.status =
                format!("Indexation en cours: traitement par lots de {MAX_INDEX_EVENTS_PER_FRAME}");
        }
    }

    fn ui_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.heading("MyCasa");
        ui.add_space(8.0);

        if ui.button("Ajouter un dossier").clicked() {
            match rfd::FileDialog::new()
                .set_title("Ajouter un dossier photo")
                .pick_folder()
            {
                Some(path) => {
                    if !self.folders.iter().any(|existing| existing == &path) {
                        self.folders.push(path.clone());
                    }
                    self.indexer.scan_folder(path);
                }
                None => {
                    self.status = "Selection de dossier annulee".to_owned();
                }
            }
        }

        if ui.button("Charger dossiers Picasa").clicked() {
            let picasa_folders = crate::picasa_db::load_watched_folders();
            let mut added = 0;
            for folder in picasa_folders {
                if !self.folders.iter().any(|existing| existing == &folder) {
                    self.folders.push(folder);
                    added += 1;
                }
            }
            self.status = format!("{added} dossier(s) surveille(s) Picasa charges");
        }

        if ui.button("Scanner le dossier courant").clicked() {
            match std::env::current_dir() {
                Ok(path) => {
                    self.folders.push(path.clone());
                    self.indexer.scan_folder(path);
                }
                Err(error) => self.status = format!("Dossier courant introuvable: {error}"),
            }
        }

        ui.separator();
        ui.label(RichText::new("Dossiers").strong());
        if self.folders.is_empty() {
            ui.label("Aucun dossier indexe");
        } else {
            for folder in &self.folders {
                ui.label(folder.display().to_string());
            }
        }

        ui.separator();
        ui.label(RichText::new("Albums").strong());
        for album in &self.albums {
            let _ = ui.selectable_label(album == "Toutes les photos", album);
        }
    }

    fn ui_top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Recherche");
            let response = ui.text_edit_singleline(&mut self.search);
            if response.changed() {
                self.refresh_photos();
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(format!("{} photo(s)", self.photos.len()));
            });
        });
    }

    fn ui_grid(&mut self, ui: &mut egui::Ui) {
        if self.photos.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label("Aucune photo indexee pour le moment");
            });
            return;
        }

        let available_width = ui.available_width().max(THUMBNAIL_SIZE);
        let tile_width = THUMBNAIL_SIZE + TILE_PADDING * 2.0;
        let columns = (available_width / tile_width).floor().max(1.0) as usize;

        let rows: Vec<Vec<Photo>> = self
            .photos
            .chunks(columns)
            .map(|row| row.to_vec())
            .collect();

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for row in rows {
                    ui.horizontal(|ui| {
                        for photo in &row {
                            self.photo_tile(ui, photo);
                        }
                    });
                }
            });
    }

    fn photo_tile(&mut self, ui: &mut egui::Ui, photo: &Photo) {
        let selected = self.selected_photo == Some(photo.id);
        let desired_size = Vec2::new(THUMBNAIL_SIZE + TILE_PADDING, THUMBNAIL_SIZE + 34.0);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());

        if response.clicked() {
            self.selected_photo = Some(photo.id);
            self.viewer.open(photo.clone());
        }

        let fill = if selected {
            Color32::from_rgb(64, 95, 145)
        } else if response.hovered() {
            Color32::from_rgb(46, 50, 58)
        } else {
            Color32::from_rgb(34, 37, 43)
        };

        ui.painter().rect_filled(rect, 6.0, fill);
        ui.painter().rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, Color32::from_rgb(68, 74, 84)),
            egui::StrokeKind::Inside,
        );

        let thumb_rect = egui::Rect::from_min_size(
            rect.min + Vec2::new(TILE_PADDING / 2.0, TILE_PADDING / 2.0),
            Vec2::splat(THUMBNAIL_SIZE),
        );

        match self.thumbnails.state_for(ui.ctx(), photo) {
            ThumbnailState::Pending => {
                ui.painter()
                    .rect_filled(thumb_rect, 4.0, Color32::from_rgb(24, 26, 31));
                ui.painter().text(
                    thumb_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "miniature",
                    egui::TextStyle::Small.resolve(ui.style()),
                    Color32::from_gray(150),
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
                    .rect_filled(thumb_rect, 4.0, Color32::from_rgb(20, 22, 26));
                ui.painter().image(
                    texture.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        }

        let name = photo
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("photo");
        ui.painter().text(
            rect.center_bottom() - Vec2::new(0.0, 14.0),
            egui::Align2::CENTER_CENTER,
            name,
            egui::TextStyle::Small.resolve(ui.style()),
            Color32::from_gray(220),
        );
    }
}

impl eframe::App for MyCasaApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_background_work();

        egui::SidePanel::left("sidebar")
            .resizable(true)
            .default_width(230.0)
            .show(ctx, |ui| self.ui_sidebar(ui));

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                if let Ok(catalog) = &self.catalog {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(catalog.path().display().to_string());
                    });
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.ui_top_bar(ui);
            ui.separator();
            self.ui_grid(ui);
        });

        self.viewer.show(ctx);
    }
}
