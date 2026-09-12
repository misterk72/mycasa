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

## Install From A GitHub Release

Release builds are published from version tags named `X.Y.Z` or `vX.Y.Z`, for example `1.1.0`.

1. Open the repository Releases page on GitHub.
2. Download the archive for your platform:
   - `mycasa-linux-x86_64.tar.gz`
   - `mycasa-windows-x86_64.zip`
3. Extract the archive.
4. Run the executable inside the extracted folder:
   - Linux: `./mycasa`
   - Windows: `mycasa.exe`

Linux may require the same runtime libraries as the development build, notably graphics stack libraries and `libheif`. On Ubuntu, install the packages listed above if the binary does not start.

Windows archives include the executable and the native DLLs collected from the CI vcpkg install. The app is portable for now: there is no installer, Start Menu shortcut, or file association.

## Publish A Release

The GitHub Actions workflow builds Linux and Windows release archives on every push. It creates a downloadable GitHub Release only when a version tag is pushed.

```bash
git tag 1.1.0
git push origin 1.1.0
```

After the workflow finishes, GitHub will create `MyCasa 1.1.0` and attach:

- `mycasa-linux-x86_64.tar.gz`
- `mycasa-windows-x86_64.zip`

For a replacement build of the same version, delete the GitHub Release and tag first, then push a corrected tag. Prefer creating a new patch tag such as `1.1.1` once a release has been shared.

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

## Linux Desktop Integration (Cinnamon)

The app includes a window icon. To also add MyCasa to the application menu and
associate pinned panel launchers with its window, run from the checkout:

```bash
cargo build --release
python3 scripts/install-desktop.py
```

The launcher points to `target/release/mycasa`; keep this checkout in place.
You can pass another executable path to the installer as its first argument.
Restart MyCasa after rebuilding, then right-click its Cinnamon panel icon to pin it.

## Data And Privacy

MyCasa stores its catalog and generated caches locally in the user profile. Original photos remain where they are and are not copied into the catalog. The current Picasa integration reads compatible files but does not automatically write `.picasa.ini` files.

## Repository Notes

The repository intentionally includes design reference screenshots under `docs/design-references/picasa/` to guide UI compatibility work. Build output, distribution archives, and local agent/tooling folders are ignored.

## License

MIT. See [LICENSE](LICENSE).
