use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::UNIX_EPOCH,
};

use walkdir::WalkDir;

use crate::picasa_ini::PicasaIniEntry;

#[derive(Debug)]
pub enum IndexJob {
    Started(PathBuf),
    FoundPhoto(IndexedPhoto),
    Finished {
        folder: PathBuf,
        photos_found: usize,
    },
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct IndexedPhoto {
    pub path: PathBuf,
    pub file_name: String,
    pub parent_path: PathBuf,
    pub file_size: Option<u64>,
    pub modified_at: Option<i64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub picasa: Option<PicasaIniEntry>,
}

pub struct Indexer {
    sender: Sender<IndexJob>,
    receiver: Receiver<IndexJob>,
}

impl Indexer {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self { sender, receiver }
    }

    pub fn scan_folder(&self, folder: PathBuf) {
        let sender = self.sender.clone();
        thread::spawn(move || {
            if sender.send(IndexJob::Started(folder.clone())).is_err() {
                return;
            }

            let mut photos_found = 0;
            for entry in WalkDir::new(&folder).follow_links(false).into_iter() {
                match entry {
                    Ok(entry)
                        if entry.file_type().is_file() && is_supported_image(entry.path()) =>
                    {
                        match IndexedPhoto::from_path(entry.path()) {
                            Ok(photo) => {
                                photos_found += 1;
                                let _ = sender.send(IndexJob::FoundPhoto(photo));
                            }
                            Err(error) => {
                                let _ = sender.send(IndexJob::Failed(error));
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        let _ = sender.send(IndexJob::Failed(error.to_string()));
                    }
                }
            }

            let _ = sender.send(IndexJob::Finished {
                folder,
                photos_found,
            });
        });
    }

    pub fn try_recv(&self) -> Option<IndexJob> {
        self.receiver.try_recv().ok()
    }
}

impl IndexedPhoto {
    #[cfg(test)]
    pub fn for_test(path: PathBuf) -> Self {
        Self {
            file_name: path
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .unwrap_or("photo")
                .to_owned(),
            parent_path: path.parent().unwrap_or_else(|| Path::new("")).to_path_buf(),
            path,
            file_size: Some(42),
            modified_at: Some(1_700_000_000),
            width: Some(800),
            height: Some(600),
            picasa: None,
        }
    }

    fn from_path(path: &Path) -> Result<Self, String> {
        let metadata =
            std::fs::metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let (width, height) = match image::image_dimensions(path) {
            Ok((width, height)) => (Some(width), Some(height)),
            Err(_) => (None, None),
        };

        Ok(Self {
            path: path.to_path_buf(),
            file_name: path
                .file_name()
                .and_then(|file_name| file_name.to_str())
                .unwrap_or("photo")
                .to_owned(),
            parent_path: path.parent().unwrap_or_else(|| Path::new("")).to_path_buf(),
            file_size: Some(metadata.len()),
            modified_at: metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs() as i64),
            width,
            height,
            picasa: crate::picasa_ini::read_entry_for_photo(path),
        })
    }
}

fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp" | "tif" | "tiff"
            )
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_image_extensions_case_insensitively() {
        assert!(is_supported_image(Path::new("photo.JPG")));
        assert!(is_supported_image(Path::new("scan.tiff")));
        assert!(is_supported_image(Path::new("web.webp")));
        assert!(!is_supported_image(Path::new("notes.txt")));
        assert!(!is_supported_image(Path::new("no-extension")));
    }
}
