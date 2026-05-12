use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use rusqlite::{Connection, params};

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
        let mut statement = self.connection.prepare(
            "SELECT id, path, width, height, captured_at
             FROM photos
             ORDER BY imported_at DESC, id DESC
             LIMIT ?1",
        )?;

        let photos = statement
            .query_map(params![limit as i64], |row| {
                Ok(Photo {
                    id: row.get(0)?,
                    path: PathBuf::from(row.get::<_, String>(1)?),
                    width: row.get::<_, Option<u32>>(2)?,
                    height: row.get::<_, Option<u32>>(3)?,
                    captured_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(photos)
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
