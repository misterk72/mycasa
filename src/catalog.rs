use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use rusqlite::{Connection, params};

use crate::indexer::IndexedPhoto;

#[derive(Debug, Clone)]
pub struct Photo {
    pub id: i64,
    pub path: PathBuf,
    pub file_size: Option<u64>,
    pub modified_at: Option<i64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub captured_at: Option<String>,
    pub picasa_caption: Option<String>,
    pub picasa_keywords: Option<String>,
    pub picasa_starred: bool,
    pub picasa_face_count: usize,
}

pub struct Catalog {
    path: PathBuf,
    connection: Connection,
}

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("impossible de localiser le repertoire de donnees utilisateur")]
    MissingDataDirectory,
    #[error("erreur disque: {0}")]
    Io(#[from] std::io::Error),
    #[error("erreur SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

impl Catalog {
    pub fn open_default() -> Result<Self, CatalogError> {
        let project_dirs = ProjectDirs::from("org", "MyCasa", "MyCasa")
            .ok_or(CatalogError::MissingDataDirectory)?;
        let data_dir = project_dirs.data_local_dir();
        std::fs::create_dir_all(data_dir)?;
        Self::open(data_dir.join("catalog.sqlite3"))
    }

    pub fn open(path: PathBuf) -> Result<Self, CatalogError> {
        let connection = Connection::open(&path)?;
        let catalog = Self { path, connection };
        catalog.migrate()?;
        Ok(catalog)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load_recent_photos(&self, limit: usize) -> Result<Vec<Photo>, CatalogError> {
        self.search_photos("", limit)
    }

    pub fn search_photos(&self, search: &str, limit: usize) -> Result<Vec<Photo>, CatalogError> {
        let pattern = format!("%{}%", search.trim());
        let mut statement = if search.trim().is_empty() {
            self.connection.prepare(
                "SELECT id, path, file_size, modified_at, width, height, captured_at,
                        picasa_caption, picasa_keywords, picasa_starred,
                        (SELECT COUNT(*) FROM photo_faces WHERE photo_faces.photo_id = photos.id)
                 FROM photos
                 ORDER BY imported_at DESC, id DESC
                 LIMIT ?1",
            )?
        } else {
            self.connection.prepare(
                "SELECT id, path, file_size, modified_at, width, height, captured_at,
                        picasa_caption, picasa_keywords, picasa_starred,
                        (SELECT COUNT(*) FROM photo_faces WHERE photo_faces.photo_id = photos.id)
                 FROM photos
                 WHERE file_name LIKE ?2 OR parent_path LIKE ?2 OR path LIKE ?2
                    OR picasa_caption LIKE ?2 OR picasa_keywords LIKE ?2
                 ORDER BY imported_at DESC, id DESC
                 LIMIT ?1",
            )?
        };

        let params: &[&dyn rusqlite::ToSql] = if search.trim().is_empty() {
            &[&(limit as i64)]
        } else {
            &[&(limit as i64), &pattern]
        };

        let photos = statement
            .query_map(params, |row| {
                let width = row
                    .get::<_, Option<i64>>(4)?
                    .and_then(|value| value.try_into().ok());
                let height = row
                    .get::<_, Option<i64>>(5)?
                    .and_then(|value| value.try_into().ok());
                Ok(Photo {
                    id: row.get(0)?,
                    path: PathBuf::from(row.get::<_, String>(1)?),
                    file_size: row
                        .get::<_, Option<i64>>(2)?
                        .and_then(|value| value.try_into().ok()),
                    modified_at: row.get(3)?,
                    width,
                    height,
                    captured_at: row.get(6)?,
                    picasa_caption: row.get(7)?,
                    picasa_keywords: row.get(8)?,
                    picasa_starred: row.get::<_, i64>(9)? != 0,
                    picasa_face_count: row.get::<_, i64>(10)?.try_into().unwrap_or_default(),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(photos)
    }

    #[cfg(test)]
    pub fn upsert_photo(&self, photo: &IndexedPhoto) -> Result<(), CatalogError> {
        upsert_photo_on_connection(&self.connection, photo)
    }

    pub fn upsert_photos(&mut self, photos: &[IndexedPhoto]) -> Result<usize, CatalogError> {
        if photos.is_empty() {
            return Ok(0);
        }

        let transaction = self.connection.transaction()?;
        for photo in photos {
            upsert_photo_on_connection(&transaction, photo)?;
        }
        transaction.commit()?;

        Ok(photos.len())
    }

    pub fn load_folders(&self, limit: usize) -> Result<Vec<PathBuf>, CatalogError> {
        let mut statement = self.connection.prepare(
            "SELECT parent_path
             FROM photos
             GROUP BY parent_path
             ORDER BY MAX(imported_at) DESC
             LIMIT ?1",
        )?;

        let folders = statement
            .query_map(params![limit as i64], |row| {
                Ok(PathBuf::from(row.get::<_, String>(0)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(folders)
    }

    fn migrate(&self) -> Result<(), CatalogError> {
        self.connection.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS photos (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                file_name TEXT NOT NULL,
                parent_path TEXT NOT NULL,
                file_size INTEGER,
                modified_at INTEGER,
                imported_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                width INTEGER,
                height INTEGER,
                captured_at TEXT
            );

            CREATE TABLE IF NOT EXISTS albums (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS album_photos (
                album_id INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
                photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
                PRIMARY KEY (album_id, photo_id)
            );

            CREATE TABLE IF NOT EXISTS photo_faces (
                id INTEGER PRIMARY KEY,
                photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
                rect64 TEXT NOT NULL,
                contact_id TEXT NOT NULL,
                contact_name TEXT
            );
            ",
        )?;

        self.ensure_column("photos", "picasa_caption", "TEXT")?;
        self.ensure_column("photos", "picasa_keywords", "TEXT")?;
        self.ensure_column("photos", "picasa_starred", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column("photos", "picasa_filters", "TEXT")?;

        Ok(())
    }

    fn ensure_column(
        &self,
        table: &str,
        column: &str,
        definition: &str,
    ) -> Result<(), CatalogError> {
        let mut statement = self
            .connection
            .prepare(&format!("PRAGMA table_info({table})"))?;
        let exists = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .any(|existing| existing == column);

        if !exists {
            self.connection.execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                [],
            )?;
        }

        Ok(())
    }
}

fn upsert_photo_on_connection(
    connection: &Connection,
    photo: &IndexedPhoto,
) -> Result<(), CatalogError> {
    connection.execute(
        "
        INSERT INTO photos (
            path, file_name, parent_path, file_size, modified_at, width, height,
            picasa_caption, picasa_keywords, picasa_starred, picasa_filters
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(path) DO UPDATE SET
            file_name = excluded.file_name,
            parent_path = excluded.parent_path,
            file_size = excluded.file_size,
            modified_at = excluded.modified_at,
            width = excluded.width,
            height = excluded.height,
            picasa_caption = excluded.picasa_caption,
            picasa_keywords = excluded.picasa_keywords,
            picasa_starred = excluded.picasa_starred,
            picasa_filters = excluded.picasa_filters
        ",
        params![
            photo.path.to_string_lossy(),
            photo.file_name,
            photo.parent_path.to_string_lossy(),
            photo.file_size.map(|value| value as i64),
            photo.modified_at,
            photo.width.map(|value| value as i64),
            photo.height.map(|value| value as i64),
            photo
                .picasa
                .as_ref()
                .and_then(|entry| entry.caption.as_deref()),
            photo
                .picasa
                .as_ref()
                .and_then(|entry| entry.keywords.as_deref()),
            photo
                .picasa
                .as_ref()
                .map(|entry| i64::from(entry.starred))
                .unwrap_or(0),
            photo
                .picasa
                .as_ref()
                .and_then(|entry| entry.filters.as_deref()),
        ],
    )?;
    let photo_id: i64 = connection.query_row(
        "SELECT id FROM photos WHERE path = ?1",
        params![photo.path.to_string_lossy()],
        |row| row.get(0),
    )?;
    connection.execute(
        "DELETE FROM photo_faces WHERE photo_id = ?1",
        params![photo_id],
    )?;
    if let Some(picasa) = &photo.picasa {
        for face in &picasa.faces {
            connection.execute(
                "
                INSERT INTO photo_faces (photo_id, rect64, contact_id, contact_name)
                VALUES (?1, ?2, ?3, ?4)
                ",
                params![photo_id, face.rect64, face.contact_id, face.name],
            )?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picasa_ini::{PicasaFace, PicasaIniEntry};

    #[test]
    fn migrates_existing_catalog_without_losing_photos() {
        let path = test_db_path("migration");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "
                    CREATE TABLE photos (
                        id INTEGER PRIMARY KEY,
                        path TEXT NOT NULL UNIQUE,
                        file_name TEXT NOT NULL,
                        parent_path TEXT NOT NULL,
                        file_size INTEGER,
                        modified_at INTEGER,
                        imported_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                        width INTEGER,
                        height INTEGER,
                        captured_at TEXT
                    );
                    INSERT INTO photos (path, file_name, parent_path)
                    VALUES ('/tmp/photo.jpg', 'photo.jpg', '/tmp');
                    ",
                )
                .unwrap();
        }

        let catalog = Catalog::open(path.clone()).unwrap();
        let photos = catalog.load_recent_photos(10).unwrap();

        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].path, PathBuf::from("/tmp/photo.jpg"));
        assert!(!photos[0].picasa_starred);
        assert_eq!(photos[0].picasa_face_count, 0);
        assert!(has_column(&catalog.connection, "photos", "picasa_caption"));
        assert!(has_column(&catalog.connection, "photos", "picasa_filters"));
        assert!(has_table(&catalog.connection, "photo_faces"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn upsert_photos_batches_multiple_rows() {
        let path = test_db_path("upsert-batch");
        let mut catalog = Catalog::open(path.clone()).unwrap();
        let photos = vec![
            crate::indexer::IndexedPhoto::for_test(PathBuf::from("/photos/one.jpg")),
            crate::indexer::IndexedPhoto::for_test(PathBuf::from("/photos/two.jpg")),
        ];

        let written = catalog.upsert_photos(&photos).unwrap();
        let loaded = catalog.search_photos("/photos", 10).unwrap();

        assert_eq!(written, 2);
        assert_eq!(loaded.len(), 2);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn upsert_replaces_photo_metadata_and_faces() {
        let path = test_db_path("upsert");
        let catalog = Catalog::open(path.clone()).unwrap();
        let photo_path = PathBuf::from("/photos/IMG_0001.JPG");

        let mut indexed = crate::indexer::IndexedPhoto::for_test(photo_path.clone());
        indexed.picasa = Some(PicasaIniEntry {
            caption: Some("Initial caption".to_owned()),
            keywords: Some("one,two".to_owned()),
            starred: true,
            filters: Some("crop64=1,aaaa;".to_owned()),
            faces: vec![PicasaFace {
                rect64: "1111222233334444".to_owned(),
                contact_id: "contact-a".to_owned(),
                name: Some("Alice".to_owned()),
            }],
        });
        catalog.upsert_photo(&indexed).unwrap();

        indexed.width = Some(1024);
        indexed.height = Some(768);
        indexed.picasa = Some(PicasaIniEntry {
            caption: Some("Updated caption".to_owned()),
            keywords: Some("three".to_owned()),
            starred: false,
            filters: None,
            faces: vec![
                PicasaFace {
                    rect64: "aaaaaaaaaaaaaaaa".to_owned(),
                    contact_id: "contact-b".to_owned(),
                    name: Some("Bob".to_owned()),
                },
                PicasaFace {
                    rect64: "bbbbbbbbbbbbbbbb".to_owned(),
                    contact_id: "contact-c".to_owned(),
                    name: None,
                },
            ],
        });
        catalog.upsert_photo(&indexed).unwrap();

        let photos = catalog.search_photos("Updated", 10).unwrap();
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].path, photo_path);
        assert_eq!(photos[0].width, Some(1024));
        assert_eq!(photos[0].height, Some(768));
        assert_eq!(photos[0].picasa_caption.as_deref(), Some("Updated caption"));
        assert!(!photos[0].picasa_starred);
        assert_eq!(photos[0].picasa_face_count, 2);

        let photo_count: i64 = catalog
            .connection
            .query_row("SELECT COUNT(*) FROM photos", [], |row| row.get(0))
            .unwrap();
        let face_count: i64 = catalog
            .connection
            .query_row("SELECT COUNT(*) FROM photo_faces", [], |row| row.get(0))
            .unwrap();
        assert_eq!(photo_count, 1);
        assert_eq!(face_count, 2);

        let _ = std::fs::remove_file(path);
    }

    fn test_db_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "mycasa-catalog-{name}-{}-{}.sqlite3",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    fn has_column(connection: &Connection, table: &str, column: &str) -> bool {
        let mut statement = connection
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .any(|name| name.unwrap() == column)
    }

    fn has_table(connection: &Connection, table: &str) -> bool {
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                params![table],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    }
}
