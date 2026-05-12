use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Default)]
pub struct PicasaIniEntry {
    pub caption: Option<String>,
    pub keywords: Option<String>,
    pub starred: bool,
    pub filters: Option<String>,
    pub faces: Vec<PicasaFace>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PicasaFace {
    pub rect64: String,
    pub contact_id: String,
    pub name: Option<String>,
}

pub fn read_entry_for_photo(photo_path: &Path) -> Option<PicasaIniEntry> {
    let file_name = photo_path.file_name()?.to_str()?;
    let sidecar_path = photo_path.parent()?.join(".picasa.ini");
    let sections = read_sidecar(&sidecar_path).ok()?;
    let contacts = contacts_from_sections(&sections);
    sections
        .get(file_name)
        .map(|section| entry_from_section(section, &contacts))
}

fn read_sidecar(
    path: &PathBuf,
) -> Result<HashMap<String, HashMap<String, String>>, std::io::Error> {
    let content = std::fs::read_to_string(path)?;
    let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mut current_section: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }

        if let Some(section_name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            current_section = Some(section_name.to_owned());
            sections.entry(section_name.to_owned()).or_default();
            continue;
        }

        let Some(section_name) = &current_section else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        sections
            .entry(section_name.clone())
            .or_default()
            .insert(key.trim().to_ascii_lowercase(), value.trim().to_owned());
    }

    Ok(sections)
}

fn contacts_from_sections(
    sections: &HashMap<String, HashMap<String, String>>,
) -> HashMap<String, String> {
    sections
        .get("Contacts2")
        .map(|contacts| {
            contacts
                .iter()
                .filter_map(|(id, value)| {
                    let name = value.split(';').next()?.trim();
                    if name.is_empty() {
                        None
                    } else {
                        Some((id.clone(), name.to_owned()))
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn entry_from_section(
    section: &HashMap<String, String>,
    contacts: &HashMap<String, String>,
) -> PicasaIniEntry {
    PicasaIniEntry {
        caption: section.get("caption").cloned(),
        keywords: section.get("keywords").cloned(),
        starred: section
            .get("star")
            .or_else(|| section.get("favorite"))
            .map(|value| matches!(value.as_str(), "yes" | "true" | "1"))
            .unwrap_or(false),
        filters: section.get("filters").cloned(),
        faces: section
            .get("faces")
            .map(|faces| parse_faces(faces, contacts))
            .unwrap_or_default(),
    }
}

fn parse_faces(faces: &str, contacts: &HashMap<String, String>) -> Vec<PicasaFace> {
    faces
        .split(';')
        .filter_map(|face| {
            let face = face.trim();
            let rect64 = face.strip_prefix("rect64(")?.split_once(')')?.0.to_owned();
            let contact_id = face.split_once("),")?.1.trim().to_owned();
            let name = contacts.get(&contact_id).cloned();
            Some(PicasaFace {
                rect64,
                contact_id,
                name,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_photo_section_from_picasa_ini() {
        let root =
            std::env::temp_dir().join(format!("mycasa-picasa-ini-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let photo_path = root.join("IMG_0001.JPG");
        let sidecar_path = root.join(".picasa.ini");
        std::fs::write(
            sidecar_path,
            r#"
[Contacts2]
e251f092e07b1008=Christophe Kassabji;;

[IMG_0001.JPG]
caption=Sortie sciences
keywords=tribulations,atelier
star=yes
filters=crop64=1,12340000abcdffff;
faces=rect64(470713809bf5578a),e251f092e07b1008
"#,
        )
        .unwrap();

        let entry = read_entry_for_photo(&photo_path).unwrap();

        assert_eq!(entry.caption.as_deref(), Some("Sortie sciences"));
        assert_eq!(entry.keywords.as_deref(), Some("tribulations,atelier"));
        assert!(entry.starred);
        assert_eq!(entry.filters.as_deref(), Some("crop64=1,12340000abcdffff;"));
        assert_eq!(
            entry.faces,
            vec![PicasaFace {
                rect64: "470713809bf5578a".to_owned(),
                contact_id: "e251f092e07b1008".to_owned(),
                name: Some("Christophe Kassabji".to_owned()),
            }]
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reads_multiple_faces_and_keeps_unknown_contacts() {
        let root = std::env::temp_dir().join(format!(
            "mycasa-picasa-ini-multiple-faces-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let photo_path = root.join("Photo Avec Accent é.JPG");
        std::fs::write(
            root.join(".picasa.ini"),
            r#"
[Contacts2]
known=Known Person;;

[Photo Avec Accent é.JPG]
faces=rect64(1111222233334444),known;rect64(aaaabbbbccccdddd),missing
"#,
        )
        .unwrap();

        let entry = read_entry_for_photo(&photo_path).unwrap();

        assert_eq!(
            entry.faces,
            vec![
                PicasaFace {
                    rect64: "1111222233334444".to_owned(),
                    contact_id: "known".to_owned(),
                    name: Some("Known Person".to_owned()),
                },
                PicasaFace {
                    rect64: "aaaabbbbccccdddd".to_owned(),
                    contact_id: "missing".to_owned(),
                    name: None,
                }
            ]
        );

        let _ = std::fs::remove_dir_all(root);
    }
}
