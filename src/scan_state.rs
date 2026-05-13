use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

pub fn begin_scan(active_folders: &mut HashSet<PathBuf>, folder: &Path) -> bool {
    active_folders.insert(folder.to_path_buf())
}

pub fn finish_scan(active_folders: &mut HashSet<PathBuf>, folder: &Path) {
    active_folders.remove(folder);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_scan_rejects_duplicate_folder() {
        let mut active = HashSet::new();
        let folder = Path::new("/photos");

        assert!(begin_scan(&mut active, folder));
        assert!(!begin_scan(&mut active, folder));
        assert_eq!(active.len(), 1);
    }

    #[test]
    fn finish_scan_allows_folder_to_start_again() {
        let mut active = HashSet::new();
        let folder = Path::new("/photos");

        assert!(begin_scan(&mut active, folder));
        finish_scan(&mut active, folder);

        assert!(begin_scan(&mut active, folder));
    }
}
