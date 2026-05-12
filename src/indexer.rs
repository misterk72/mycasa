use std::{
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use walkdir::WalkDir;

#[derive(Debug)]
pub enum IndexJob {
    Started(PathBuf),
    FoundPhoto(PathBuf),
    Finished {
        folder: PathBuf,
        photos_found: usize,
    },
    Failed(String),
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
                        photos_found += 1;
                        let _ = sender.send(IndexJob::FoundPhoto(entry.path().to_path_buf()));
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
