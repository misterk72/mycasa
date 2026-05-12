use egui::{Color32, RichText};

use crate::catalog::Photo;

#[derive(Default)]
pub struct ViewerState {
    current: Option<Photo>,
    zoom: f32,
}

impl ViewerState {
    pub fn open(&mut self, photo: Photo) {
        self.current = Some(photo);
        self.zoom = 1.0;
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let Some(photo) = self.current.clone() else {
            return;
        };

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
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("Apercu image a connecter").color(Color32::LIGHT_GRAY));
                });
                ui.separator();
                ui.label(photo.path.display().to_string());
                if let (Some(width), Some(height)) = (photo.width, photo.height) {
                    ui.label(format!("{width} x {height}"));
                }
                if let Some(captured_at) = &photo.captured_at {
                    ui.label(captured_at);
                }
            });

        if !open {
            self.current = None;
        }
    }
}
