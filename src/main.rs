mod app;
mod catalog;
mod folders;
mod grid;
mod indexer;
mod picasa_db;
mod picasa_ini;
mod thumbnails;
mod ui_text;
mod viewer;

use app::MyCasaApp;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title("MyCasa")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "MyCasa",
        options,
        Box::new(|creation_context| Ok(Box::new(MyCasaApp::new(creation_context)))),
    )
}
