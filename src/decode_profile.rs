use std::{
    ffi::OsString,
    io::Cursor,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use image::{GenericImageView, ImageFormat, ImageReader};
use walkdir::WalkDir;

const DEFAULT_PROFILE_LIMIT: usize = 12;
const PROFILE_VIEWER_MAX_EDGE: u32 = 1600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeProfileOptions {
    pub roots: Vec<PathBuf>,
    pub limit: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DecodeProfileTimings {
    pub open_ms: f32,
    pub format_ms: f32,
    pub decode_ms: f32,
    pub resize_ms: f32,
    pub rgba_ms: f32,
    pub png_encode_ms: f32,
    pub png_decode_ms: f32,
    pub total_ms: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodeProfileSample {
    pub path: PathBuf,
    pub format: Option<ImageFormat>,
    pub source_dimensions: (u32, u32),
    pub preview_dimensions: (u32, u32),
    pub rgba_bytes: usize,
    pub png_bytes: usize,
    pub timings: DecodeProfileTimings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodeProfileFailure {
    pub path: PathBuf,
    pub total_ms: f32,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecodeProfileResult {
    Decoded(DecodeProfileSample),
    Failed(DecodeProfileFailure),
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DecodeProfileSummary {
    pub decoded: usize,
    pub failed: usize,
    pub total_ms: f32,
    pub decode_ms: f32,
    pub resize_ms: f32,
    pub rgba_ms: f32,
    pub png_encode_ms: f32,
    pub png_decode_ms: f32,
}

impl DecodeProfileSummary {
    pub fn record(&mut self, result: &DecodeProfileResult) {
        match result {
            DecodeProfileResult::Decoded(sample) => {
                self.decoded += 1;
                self.total_ms += sample.timings.total_ms;
                self.decode_ms += sample.timings.decode_ms;
                self.resize_ms += sample.timings.resize_ms;
                self.rgba_ms += sample.timings.rgba_ms;
                self.png_encode_ms += sample.timings.png_encode_ms;
                self.png_decode_ms += sample.timings.png_decode_ms;
            }
            DecodeProfileResult::Failed(failure) => {
                self.failed += 1;
                self.total_ms += failure.total_ms;
            }
        }
    }

    pub fn average_total_ms(&self) -> f32 {
        average(self.total_ms, self.decoded + self.failed)
    }

    pub fn average_decode_ms(&self) -> f32 {
        average(self.decode_ms, self.decoded)
    }

    pub fn average_resize_ms(&self) -> f32 {
        average(self.resize_ms, self.decoded)
    }

    pub fn average_rgba_ms(&self) -> f32 {
        average(self.rgba_ms, self.decoded)
    }

    pub fn average_png_encode_ms(&self) -> f32 {
        average(self.png_encode_ms, self.decoded)
    }

    pub fn average_png_decode_ms(&self) -> f32 {
        average(self.png_decode_ms, self.decoded)
    }

    pub fn average_display_ready_ms(&self) -> f32 {
        average(self.decode_ms + self.resize_ms + self.rgba_ms, self.decoded)
    }
}

pub fn run_cli(args: Vec<OsString>) -> Result<()> {
    let options = parse_profile_args(&args)?;
    let paths = collect_profile_paths(&options.roots, options.limit);

    if paths.is_empty() {
        return Err(anyhow!("aucune image trouvee dans les chemins fournis"));
    }

    println!(
        "Profilage decode viewer: {} image(s), preview max {} px",
        paths.len(),
        PROFILE_VIEWER_MAX_EDGE
    );
    println!(
        "path\tformat\tsource\tpreview\topen_ms\tformat_ms\tdecode_ms\tresize_ms\trgba_ms\tdisplay_ready_ms\tpng_encode_ms\tpng_decode_ms\ttotal_ms\tpng_kib\tstatus"
    );

    let mut summary = DecodeProfileSummary::default();
    for path in paths {
        let result = profile_decode_path(&path, PROFILE_VIEWER_MAX_EDGE);
        print_profile_result(&result);
        summary.record(&result);
    }

    println!(
        "SUMMARY decoded={} failed={} avg_display_ready_ms={:.1} avg_total_ms={:.1} avg_decode_ms={:.1} avg_resize_ms={:.1} avg_rgba_ms={:.1} avg_png_encode_ms={:.1} avg_png_decode_ms={:.1}",
        summary.decoded,
        summary.failed,
        summary.average_display_ready_ms(),
        summary.average_total_ms(),
        summary.average_decode_ms(),
        summary.average_resize_ms(),
        summary.average_rgba_ms(),
        summary.average_png_encode_ms(),
        summary.average_png_decode_ms()
    );

    Ok(())
}

pub fn parse_profile_args(args: &[OsString]) -> Result<DecodeProfileOptions> {
    let mut roots = Vec::new();
    let mut limit = DEFAULT_PROFILE_LIMIT;
    let mut index = 0;

    while index < args.len() {
        match args[index].to_string_lossy().as_ref() {
            "--limit" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| anyhow!("--limit attend une valeur"))?;
                limit = value
                    .to_string_lossy()
                    .parse::<usize>()
                    .context("--limit doit etre un entier")?;
            }
            "--help" | "-h" => {
                return Err(anyhow!(
                    "usage: mycasa --profile-decode [--limit N] <fichier-ou-dossier>..."
                ));
            }
            value if value.starts_with('-') => {
                return Err(anyhow!("option inconnue: {value}"));
            }
            _ => roots.push(PathBuf::from(&args[index])),
        }
        index += 1;
    }

    if roots.is_empty() {
        return Err(anyhow!(
            "usage: mycasa --profile-decode [--limit N] <fichier-ou-dossier>..."
        ));
    }

    Ok(DecodeProfileOptions { roots, limit })
}

pub fn collect_profile_paths(roots: &[PathBuf], limit: usize) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in roots {
        if paths.len() >= limit {
            break;
        }

        if root.is_file() {
            if is_profile_supported_path(root) {
                paths.push(root.clone());
            }
            continue;
        }

        if !root.is_dir() {
            continue;
        }

        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if paths.len() >= limit {
                break;
            }
            if entry.file_type().is_file() && is_profile_supported_path(entry.path()) {
                paths.push(entry.path().to_path_buf());
            }
        }
    }

    paths.sort();
    paths.truncate(limit);
    paths
}

pub fn is_profile_supported_path(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
        return false;
    };

    matches!(
        extension.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "heic" | "heif"
    )
}

pub fn profile_decode_path(path: &Path, max_edge: u32) -> DecodeProfileResult {
    let total_start = Instant::now();
    match profile_decode_path_result(path, max_edge, total_start) {
        Ok(sample) => DecodeProfileResult::Decoded(sample),
        Err(error) => DecodeProfileResult::Failed(DecodeProfileFailure {
            path: path.to_path_buf(),
            total_ms: elapsed_ms(total_start.elapsed()),
            error: error.to_string(),
        }),
    }
}

fn profile_decode_path_result(
    path: &Path,
    max_edge: u32,
    total_start: Instant,
) -> Result<DecodeProfileSample> {
    let step_start = Instant::now();
    let reader = ImageReader::open(path).with_context(|| format!("open {}", path.display()))?;
    let open_ms = elapsed_ms(step_start.elapsed());

    let step_start = Instant::now();
    let reader = reader
        .with_guessed_format()
        .with_context(|| format!("detect format {}", path.display()))?;
    let format = reader.format();
    let format_ms = elapsed_ms(step_start.elapsed());

    let step_start = Instant::now();
    let image = reader
        .decode()
        .with_context(|| format!("decode {}", path.display()))?;
    let decode_ms = elapsed_ms(step_start.elapsed());
    let source_dimensions = image.dimensions();

    let step_start = Instant::now();
    let preview = image.thumbnail(max_edge, max_edge);
    let resize_ms = elapsed_ms(step_start.elapsed());
    let preview_dimensions = preview.dimensions();

    let step_start = Instant::now();
    let rgba = preview.to_rgba8();
    let rgba_bytes = rgba.as_raw().len();
    let rgba_ms = elapsed_ms(step_start.elapsed());

    let step_start = Instant::now();
    let mut png_buffer = Cursor::new(Vec::new());
    preview
        .write_to(&mut png_buffer, ImageFormat::Png)
        .context("encode png cache")?;
    let png_encode_ms = elapsed_ms(step_start.elapsed());
    let png_data = png_buffer.into_inner();
    let png_bytes = png_data.len();

    let step_start = Instant::now();
    let cached_preview = image::load_from_memory_with_format(&png_data, ImageFormat::Png)
        .context("decode png cache")?;
    let _cached_rgba = cached_preview.to_rgba8();
    let png_decode_ms = elapsed_ms(step_start.elapsed());

    Ok(DecodeProfileSample {
        path: path.to_path_buf(),
        format,
        source_dimensions,
        preview_dimensions,
        rgba_bytes,
        png_bytes,
        timings: DecodeProfileTimings {
            open_ms,
            format_ms,
            decode_ms,
            resize_ms,
            rgba_ms,
            png_encode_ms,
            png_decode_ms,
            total_ms: elapsed_ms(total_start.elapsed()),
        },
    })
}

fn print_profile_result(result: &DecodeProfileResult) {
    match result {
        DecodeProfileResult::Decoded(sample) => {
            println!(
                "{}\t{}\t{}x{}\t{}x{}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\t{:.1}\tok",
                sample.path.display(),
                sample
                    .format
                    .map(|format| format!("{format:?}"))
                    .unwrap_or_else(|| "?".to_string()),
                sample.source_dimensions.0,
                sample.source_dimensions.1,
                sample.preview_dimensions.0,
                sample.preview_dimensions.1,
                sample.timings.open_ms,
                sample.timings.format_ms,
                sample.timings.decode_ms,
                sample.timings.resize_ms,
                sample.timings.rgba_ms,
                sample.timings.decode_ms + sample.timings.resize_ms + sample.timings.rgba_ms,
                sample.timings.png_encode_ms,
                sample.timings.png_decode_ms,
                sample.timings.total_ms,
                sample.png_bytes as f32 / 1024.0
            );
        }
        DecodeProfileResult::Failed(failure) => {
            println!(
                "{}\t?\t?\t?\t0.0\t0.0\t0.0\t0.0\t0.0\t0.0\t0.0\t0.0\t{:.1}\t0.0\tfailed: {}",
                failure.path.display(),
                failure.total_ms,
                failure.error
            );
        }
    }
}

fn average(total: f32, count: usize) -> f32 {
    if count == 0 {
        0.0
    } else {
        total / count as f32
    }
}

fn elapsed_ms(duration: Duration) -> f32 {
    duration.as_secs_f32() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_arg_parser_uses_limit_and_roots() {
        let args = vec![
            OsString::from("--limit"),
            OsString::from("3"),
            OsString::from("/photos"),
        ];

        let options = parse_profile_args(&args).unwrap();

        assert_eq!(options.limit, 3);
        assert_eq!(options.roots, vec![PathBuf::from("/photos")]);
    }

    #[test]
    fn profile_path_filter_includes_heic_for_failure_reporting() {
        assert!(is_profile_supported_path(Path::new("IMG_0001.HEIC")));
        assert!(is_profile_supported_path(Path::new("IMG_0001.jpg")));
        assert!(!is_profile_supported_path(Path::new("IMG_0001.txt")));
    }

    #[test]
    fn profile_summary_tracks_decoded_and_failed_images() {
        let mut summary = DecodeProfileSummary::default();
        summary.record(&DecodeProfileResult::Decoded(DecodeProfileSample {
            path: PathBuf::from("ok.jpg"),
            format: Some(ImageFormat::Jpeg),
            source_dimensions: (4000, 3000),
            preview_dimensions: (1600, 1200),
            rgba_bytes: 1600 * 1200 * 4,
            png_bytes: 100,
            timings: DecodeProfileTimings {
                total_ms: 30.0,
                decode_ms: 20.0,
                resize_ms: 5.0,
                rgba_ms: 1.0,
                png_encode_ms: 4.0,
                png_decode_ms: 2.0,
                ..DecodeProfileTimings::default()
            },
        }));
        summary.record(&DecodeProfileResult::Failed(DecodeProfileFailure {
            path: PathBuf::from("bad.heic"),
            total_ms: 2.0,
            error: "unsupported".to_string(),
        }));

        assert_eq!(summary.decoded, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.average_total_ms(), 16.0);
        assert_eq!(summary.average_decode_ms(), 20.0);
        assert_eq!(summary.average_display_ready_ms(), 26.0);
        assert_eq!(summary.average_png_decode_ms(), 2.0);
    }

    #[test]
    fn profile_decode_path_reports_preview_size() {
        let path = std::env::temp_dir().join(format!(
            "mycasa-decode-profile-{}-{}.png",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        image::RgbaImage::from_pixel(80, 40, image::Rgba([10, 20, 30, 255]))
            .save_with_format(&path, ImageFormat::Png)
            .unwrap();

        let result = profile_decode_path(&path, 20);

        let DecodeProfileResult::Decoded(sample) = result else {
            panic!("expected decoded image");
        };
        assert_eq!(sample.source_dimensions, (80, 40));
        assert_eq!(sample.preview_dimensions, (20, 10));
        assert_eq!(sample.rgba_bytes, 20 * 10 * 4);

        let _ = std::fs::remove_file(path);
    }
}
