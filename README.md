# MyCasa

MyCasa is a native desktop photo library and viewer inspired by Picasa. It keeps original photos in place, builds a local SQLite catalog, and focuses on fast browsing of large local or NAS-backed photo folders.

The project is currently early-stage and developed incrementally with tests. It is usable as an experimental photo browser, not yet as a full Picasa replacement.

## Features

- Native Windows/Linux desktop app built with Rust, `eframe`/`egui`, and `wgpu`.
- Folder scanning with a local SQLite catalog.
- Virtualized chronological photo grid for large libraries.
- Progressive thumbnail loading with local disk cache.
- Viewer with 1600 px preview cache, neighboring-photo preload, and keyboard/wheel navigation.
- JPEG, PNG, WebP, GIF, BMP, TIFF, HEIC and HEIF indexing/display support.
- Pragmatic Picasa import from stable sidecar/readable files:
  - `.picasa.ini` captions, keywords, favorites, filters and face rectangles.
  - Picasa `contacts.xml` names when a compatible Wine/PlayOnLinux profile is detected.
  - watched folder paths from Picasa when available.

## Status

Implemented pieces include the catalog, folder scan, thumbnail cache, virtualized grid, chronological view, Picasa favorite/face filters, basic people panel, and image viewer. Editing tools, albums, tags, metadata writing, and full Picasa effect compatibility are not complete yet.

Development rules and current backlog are documented in [docs/development.md](docs/development.md). Performance notes are in [docs/performance-plan.md](docs/performance-plan.md). Picasa compatibility decisions are in [docs/picasa-compatibility.md](docs/picasa-compatibility.md).

## Requirements

- Rust stable.
- Linux native packages for GUI/image dependencies, including OpenGL/Wayland/X11 development headers, `pkg-config`, and `libheif`.
- Windows builds use vcpkg for `libheif`.

On Ubuntu 24.04, the CI installs:

```bash
sudo apt-get install -y \
  libegl1-mesa-dev \
  libgl1-mesa-dev \
  libheif-dev \
  libwayland-dev \
  libx11-dev \
  libxcb-render0-dev \
  libxcb-shape0-dev \
  libxcb-xfixes0-dev \
  libxcb1-dev \
  libxkbcommon-dev \
  pkg-config
```

## Build And Run

```bash
cargo run --release
```

For day-to-day development:

```bash
cargo check
cargo test
```

Decode profiling can be run without opening the UI:

```bash
cargo run -- --profile-decode --limit 12 /path/to/photos
```

## Data And Privacy

MyCasa stores its catalog and generated caches locally in the user profile. Original photos remain where they are and are not copied into the catalog. The current Picasa integration reads compatible files but does not automatically write `.picasa.ini` files.

## Repository Notes

The repository intentionally includes design reference screenshots under `docs/design-references/picasa/` to guide UI compatibility work. Build output, distribution archives, and local agent/tooling folders are ignored.

## License

MIT. See [LICENSE](LICENSE).
