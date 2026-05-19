mod app;
mod catalog;
mod debounce;
mod decode_profile;
mod folders;
mod grid;
mod indexer;
mod photo_limit;
mod picasa_db;
mod picasa_ini;
mod scan_state;
mod thumbnails;
mod ui_text;
mod viewer;

use app::MyCasaApp;

fn main() -> eframe::Result {
    let mut args = std::env::args_os();
    let _program = args.next();
    if matches!(
        args.next()
            .and_then(|arg| arg.into_string().ok())
            .as_deref(),
        Some("--profile-decode")
    ) {
        if let Err(error) = decode_profile::run_cli(args.collect()) {
            eprintln!("{error:#}");
            std::process::exit(2);
        }
        return Ok(());
    }

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
