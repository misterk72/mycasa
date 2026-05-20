use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PicasaContact {
    pub id: String,
    pub name: String,
    pub modified_time: Option<String>,
}

pub fn load_watched_folders() -> Vec<PathBuf> {
    picasa_profile()
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

pub fn load_contacts() -> Vec<PicasaContact> {
    picasa_profile()
        .map(|profile| profile.join("contacts/contacts.xml"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|content| parse_contacts_xml(&content))
        .unwrap_or_default()
}

pub fn parse_contacts_xml(content: &str) -> Vec<PicasaContact> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if !line.starts_with("<contact ") {
                return None;
            }

            let id = xml_attr(line, "id")?;
            let name = xml_attr(line, "name")?;
            let modified_time = xml_attr(line, "modified_time");
            Some(PicasaContact {
                id,
                name,
                modified_time,
            })
        })
        .collect()
}

fn xml_attr(line: &str, attr: &str) -> Option<String> {
    let prefix = format!("{attr}=\"");
    let value_start = line.find(&prefix)? + prefix.len();
    let value = line[value_start..].split_once('"')?.0;
    Some(unescape_xml_attr(value))
}

fn unescape_xml_attr(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn picasa_profile() -> Option<PathBuf> {
    picasa_profile_candidates()
        .into_iter()
        .find(|profile| profile.exists())
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

    #[test]
    fn parses_contacts_xml() {
        let contacts = parse_contacts_xml(
            r#"
<contacts>
 <contact id="e251f092e07b1008" name="Alex Martin" modified_time="2022-09-22T23:52:05+02:00" local_contact="1"/>
 <contact id="43793bd32caffa75" name="Sam Lee" modified_time="2022-09-22T23:54:47+02:00" local_contact="1"/>
</contacts>
"#,
        );

        assert_eq!(
            contacts,
            vec![
                PicasaContact {
                    id: "e251f092e07b1008".to_owned(),
                    name: "Alex Martin".to_owned(),
                    modified_time: Some("2022-09-22T23:52:05+02:00".to_owned()),
                },
                PicasaContact {
                    id: "43793bd32caffa75".to_owned(),
                    name: "Sam Lee".to_owned(),
                    modified_time: Some("2022-09-22T23:54:47+02:00".to_owned()),
                },
            ]
        );
    }
}
