use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use rusqlite::{Connection, params};

use crate::indexer::IndexedPhoto;

#[derive(Debug, Clone)]
pub struct Photo {
    pub id: i64,
    pub path: PathBuf,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub captured_at: Option<String>,
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
                "SELECT id, path, width, height, captured_at
                 FROM photos
                 ORDER BY imported_at DESC, id DESC
                 LIMIT ?1",
            )?
        } else {
            self.connection.prepare(
                "SELECT id, path, width, height, captured_at
                 FROM photos
                 WHERE file_name LIKE ?2 OR parent_path LIKE ?2 OR path LIKE ?2
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
                    .get::<_, Option<i64>>(2)?
                    .and_then(|value| value.try_into().ok());
                let height = row
                    .get::<_, Option<i64>>(3)?
                    .and_then(|value| value.try_into().ok());
                Ok(Photo {
                    id: row.get(0)?,
                    path: PathBuf::from(row.get::<_, String>(1)?),
                    width,
                    height,
                    captured_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(photos)
    }

    pub fn upsert_photo(&self, photo: &IndexedPhoto) -> Result<(), CatalogError> {
        self.connection.execute(
            "
            INSERT INTO photos (
                path, file_name, parent_path, file_size, modified_at, width, height
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(path) DO UPDATE SET
                file_name = excluded.file_name,
                parent_path = excluded.parent_path,
                file_size = excluded.file_size,
                modified_at = excluded.modified_at,
                width = excluded.width,
                height = excluded.height
            ",
            params![
                photo.path.to_string_lossy(),
                photo.file_name,
                photo.parent_path.to_string_lossy(),
                photo.file_size.map(|value| value as i64),
                photo.modified_at,
                photo.width.map(|value| value as i64),
                photo.height.map(|value| value as i64),
            ],
        )?;

        Ok(())
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
            ",
        )?;

        Ok(())
    }
}
