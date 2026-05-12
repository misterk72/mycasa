use std::path::PathBuf;

pub fn add_folder_once(folders: &mut Vec<PathBuf>, folder: PathBuf) -> bool {
    if folders.iter().any(|existing| existing == &folder) {
        return false;
    }

    folders.push(folder);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_folder_only_once() {
        let mut folders = Vec::new();
        let folder = PathBuf::from("/photos");

        assert!(add_folder_once(&mut folders, folder.clone()));
        assert!(!add_folder_once(&mut folders, folder));
        assert_eq!(folders, vec![PathBuf::from("/photos")]);
    }
}
