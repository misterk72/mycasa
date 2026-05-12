use std::path::{Path, PathBuf};

pub fn load_watched_folders() -> Vec<PathBuf> {
    picasa_profile_candidates()
        .into_iter()
        .find(|profile| profile.exists())
        .map(|profile| profile.join("../Picasa2Albums/watchedfolders.txt"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|content| {
            content
                .lines()
                .filter_map(wine_path_to_linux)
                .filter(|path| path.exists())
                .collect()
        })
        .unwrap_or_default()
}

fn picasa_profile_candidates() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let user_name = home
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("user");
    let relative_profile = Path::new("drive_c")
        .join("users")
        .join(user_name)
        .join("Local Settings")
        .join("Application Data")
        .join("Google")
        .join("Picasa2");

    vec![
        home.join(".PlayOnLinux")
            .join("wineprefix")
            .join("Picasa")
            .join(&relative_profile),
        home.join(".pki")
            .join(".PlayOnLinux")
            .join("wineprefix")
            .join("Picasa")
            .join(relative_profile),
    ]
}

fn wine_path_to_linux(path: &str) -> Option<PathBuf> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }

    let normalized = path.replace('\\', "/");
    if let Some(path) = normalized.strip_prefix("Z:") {
        return Some(PathBuf::from(path));
    }

    Some(PathBuf::from(normalized))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_wine_z_drive_path_to_linux_path() {
        assert_eq!(
            wine_path_to_linux(r"Z:\mnt\nas_Media\Photos_sorted\").unwrap(),
            PathBuf::from("/mnt/nas_Media/Photos_sorted/")
        );
    }
}
