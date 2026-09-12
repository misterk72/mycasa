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
    register_extra_image_decoders();

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
            .with_app_id("mycasa")
            .with_icon(application_icon())
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

fn application_icon() -> egui::IconData {
    let image = image::load_from_memory_with_format(
        include_bytes!("../assets/mycasa.png"),
        image::ImageFormat::Png,
    )
    .expect("bundled MyCasa icon must be a valid PNG")
    .thumbnail(256, 256)
    .into_rgba8();
    egui::IconData {
        width: image.width(),
        height: image.height(),
        rgba: image.into_raw(),
    }
}

fn register_extra_image_decoders() {
    libheif_rs::integration::image::register_all_decoding_hooks();
}

#[cfg(test)]
mod desktop_tests {
    #[test]
    fn bundled_icon_is_valid_rgba() {
        let icon = super::application_icon();
        assert_eq!([icon.width, icon.height], [256, 256]);
        assert_eq!(icon.rgba.len(), 256 * 256 * 4);
        assert!(icon.rgba.chunks_exact(4).any(|pixel| pixel[3] == 255));
    }
}
