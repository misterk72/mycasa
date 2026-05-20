use std::path::{Path, PathBuf};

use chrono::{Datelike, LocalResult, TimeZone, Utc};
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
        self.search_photos_page(search, limit, 0)
    }

    pub fn search_photos_page(
        &self,
        search: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Photo>, CatalogError> {
        self.search_photos_page_filtered(search, false, limit, offset)
    }

    pub fn search_photos_page_filtered(
        &self,
        search: &str,
        starred_only: bool,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Photo>, CatalogError> {
        let pattern = format!("%{}%", search.trim());
        let has_search = !search.trim().is_empty();
        let sql = format!(
            "SELECT id, path, file_size, modified_at, width, height, captured_at,
                        picasa_caption, picasa_keywords, picasa_starred,
                        (SELECT COUNT(*) FROM photo_faces WHERE photo_faces.photo_id = photos.id)
                 FROM photos
                 {}
                 ORDER BY COALESCE(captured_at, datetime(modified_at, 'unixepoch'), imported_at) DESC,
                          id DESC
                 LIMIT ?1 OFFSET ?2",
            photo_search_where_clause(has_search, starred_only, "?3")
        );
        let mut statement = self.connection.prepare(&sql)?;

        let params: &[&dyn rusqlite::ToSql] = if has_search {
            &[&(limit as i64), &(offset as i64), &pattern]
        } else {
            &[&(limit as i64), &(offset as i64)]
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

    pub fn chronology_months(&self, search: &str) -> Result<Vec<(i32, u32)>, CatalogError> {
        self.chronology_months_filtered(search, false)
    }

    pub fn chronology_months_filtered(
        &self,
        search: &str,
        starred_only: bool,
    ) -> Result<Vec<(i32, u32)>, CatalogError> {
        let mut months = self.search_photo_time_keys(search, starred_only)?;
        months.sort_unstable_by(|left, right| right.cmp(left));
        months.dedup();
        Ok(months)
    }

    pub fn photo_offset_for_month(
        &self,
        search: &str,
        year: i32,
        month: u32,
    ) -> Result<Option<usize>, CatalogError> {
        self.photo_offset_for_month_filtered(search, false, year, month)
    }

    pub fn photo_offset_for_month_filtered(
        &self,
        search: &str,
        starred_only: bool,
        year: i32,
        month: u32,
    ) -> Result<Option<usize>, CatalogError> {
        Ok(self
            .search_photo_time_keys(search, starred_only)?
            .into_iter()
            .position(|(photo_year, photo_month)| photo_year == year && photo_month == month))
    }

    fn search_photo_time_keys(
        &self,
        search: &str,
        starred_only: bool,
    ) -> Result<Vec<(i32, u32)>, CatalogError> {
        let pattern = format!("%{}%", search.trim());
        let has_search = !search.trim().is_empty();
        let sql = format!(
            "SELECT captured_at, modified_at
                 FROM photos
                 {}
                 ORDER BY COALESCE(captured_at, datetime(modified_at, 'unixepoch'), imported_at) DESC,
                          id DESC",
            photo_search_where_clause(has_search, starred_only, "?1")
        );
        let mut statement = self.connection.prepare(&sql)?;

        let params: &[&dyn rusqlite::ToSql] = if has_search { &[&pattern] } else { &[] };

        let keys = statement
            .query_map(params, |row| {
                let captured_at = row.get::<_, Option<String>>(0)?;
                let modified_at = row.get::<_, Option<i64>>(1)?;
                Ok(photo_time_month_key(captured_at.as_deref(), modified_at))
            })?
            .filter_map(Result::transpose)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(keys)
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

            CREATE INDEX IF NOT EXISTS idx_photos_imported_at ON photos(imported_at DESC, id DESC);
            CREATE INDEX IF NOT EXISTS idx_photos_file_name ON photos(file_name);
            CREATE INDEX IF NOT EXISTS idx_photos_parent_path ON photos(parent_path);
            CREATE INDEX IF NOT EXISTS idx_photo_faces_photo_id ON photo_faces(photo_id);
            ",
        )?;

        self.ensure_column("photos", "picasa_caption", "TEXT")?;
        self.ensure_column("photos", "picasa_keywords", "TEXT")?;
        self.ensure_column("photos", "picasa_starred", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column("photos", "picasa_filters", "TEXT")?;
        self.deduplicate_photos_by_path()?;
        self.connection.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_photos_path_unique ON photos(path)",
            [],
        )?;

        Ok(())
    }

    fn deduplicate_photos_by_path(&self) -> Result<(), CatalogError> {
        let mut statement = self.connection.prepare(
            "
            SELECT path
            FROM photos
            GROUP BY path
            HAVING COUNT(*) > 1
            ",
        )?;
        let duplicate_paths = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;

        for path in duplicate_paths {
            let ids = self.photo_ids_for_path(&path)?;
            let Some((&keeper_id, duplicate_ids)) = ids.split_first() else {
                continue;
            };

            for duplicate_id in duplicate_ids {
                self.merge_duplicate_photo(keeper_id, *duplicate_id)?;
            }
        }

        Ok(())
    }

    fn photo_ids_for_path(&self, path: &str) -> Result<Vec<i64>, CatalogError> {
        let mut statement = self.connection.prepare(
            "
            SELECT id
            FROM photos
            WHERE path = ?1
            ORDER BY id DESC
            ",
        )?;
        let ids = statement
            .query_map(params![path], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(ids)
    }

    fn merge_duplicate_photo(&self, keeper_id: i64, duplicate_id: i64) -> Result<(), CatalogError> {
        self.connection.execute(
            "
            INSERT OR IGNORE INTO album_photos (album_id, photo_id)
            SELECT album_id, ?1
            FROM album_photos
            WHERE photo_id = ?2
            ",
            params![keeper_id, duplicate_id],
        )?;
        self.connection.execute(
            "DELETE FROM album_photos WHERE photo_id = ?1",
            params![duplicate_id],
        )?;
        self.connection.execute(
            "UPDATE photo_faces SET photo_id = ?1 WHERE photo_id = ?2",
            params![keeper_id, duplicate_id],
        )?;
        self.connection
            .execute("DELETE FROM photos WHERE id = ?1", params![duplicate_id])?;

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

fn photo_time_month_key(captured_at: Option<&str>, modified_at: Option<i64>) -> Option<(i32, u32)> {
    let timestamp = captured_at
        .and_then(parse_photo_timestamp)
        .or(modified_at)?;

    match Utc.timestamp_opt(timestamp, 0) {
        LocalResult::Single(datetime) => Some((datetime.year(), datetime.month())),
        _ => None,
    }
}

fn parse_photo_timestamp(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|datetime| datetime.timestamp())
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y:%m:%d %H:%M:%S")
                .ok()
                .map(|datetime| datetime.and_utc().timestamp())
        })
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|datetime| datetime.and_utc().timestamp())
        })
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|datetime| datetime.and_utc().timestamp())
        })
}

fn photo_search_where_clause(
    has_search: bool,
    starred_only: bool,
    search_placeholder: &str,
) -> String {
    let search_clause = format!(
        "(file_name LIKE {0} OR parent_path LIKE {0} OR path LIKE {0}
          OR picasa_caption LIKE {0} OR picasa_keywords LIKE {0})",
        search_placeholder
    );

    match (has_search, starred_only) {
        (false, false) => String::new(),
        (false, true) => "WHERE picasa_starred = 1".to_owned(),
        (true, false) => format!("WHERE {search_clause}"),
        (true, true) => format!("WHERE picasa_starred = 1 AND {search_clause}"),
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
        assert!(has_index(&catalog.connection, "idx_photos_imported_at"));
        assert!(has_index(&catalog.connection, "idx_photos_file_name"));
        assert!(has_index(&catalog.connection, "idx_photos_parent_path"));
        assert!(has_index(&catalog.connection, "idx_photos_path_unique"));
        assert!(has_index(&catalog.connection, "idx_photo_faces_photo_id"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn migration_deduplicates_existing_photos_by_path() {
        let path = test_db_path("dedupe");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "
                    PRAGMA foreign_keys = ON;
                    CREATE TABLE photos (
                        id INTEGER PRIMARY KEY,
                        path TEXT NOT NULL,
                        file_name TEXT NOT NULL,
                        parent_path TEXT NOT NULL,
                        file_size INTEGER,
                        modified_at INTEGER,
                        imported_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                        width INTEGER,
                        height INTEGER,
                        captured_at TEXT
                    );
                    CREATE TABLE albums (
                        id INTEGER PRIMARY KEY,
                        name TEXT NOT NULL UNIQUE,
                        created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                    );
                    CREATE TABLE album_photos (
                        album_id INTEGER NOT NULL REFERENCES albums(id) ON DELETE CASCADE,
                        photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
                        PRIMARY KEY (album_id, photo_id)
                    );
                    CREATE TABLE photo_faces (
                        id INTEGER PRIMARY KEY,
                        photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
                        rect64 TEXT NOT NULL,
                        contact_id TEXT NOT NULL,
                        contact_name TEXT
                    );
                    INSERT INTO photos (id, path, file_name, parent_path, width)
                    VALUES
                        (1, '/photos/duplicate.jpg', 'duplicate.jpg', '/photos', 800),
                        (2, '/photos/duplicate.jpg', 'duplicate.jpg', '/photos', 1024);
                    INSERT INTO albums (id, name) VALUES (1, 'Album');
                    INSERT INTO album_photos (album_id, photo_id) VALUES (1, 1);
                    INSERT INTO photo_faces (photo_id, rect64, contact_id, contact_name)
                    VALUES (1, 'face-old', '1', 'Old'), (2, 'face-new', '2', 'New');
                    ",
                )
                .unwrap();
        }

        let catalog = Catalog::open(path.clone()).unwrap();
        let photos = catalog.search_photos("duplicate", 10).unwrap();

        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0].path, PathBuf::from("/photos/duplicate.jpg"));
        assert_eq!(photos[0].id, 2);
        assert_eq!(photos[0].picasa_face_count, 2);
        assert!(has_index(&catalog.connection, "idx_photos_path_unique"));

        catalog
            .upsert_photo(&crate::indexer::IndexedPhoto::for_test(PathBuf::from(
                "/photos/duplicate.jpg",
            )))
            .unwrap();
        let photos_after_reimport = catalog.search_photos("duplicate", 10).unwrap();
        assert_eq!(photos_after_reimport.len(), 1);

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
    fn search_photos_orders_by_photo_time_descending() {
        let path = test_db_path("chrono-order");
        let catalog = Catalog::open(path.clone()).unwrap();
        let mut older = crate::indexer::IndexedPhoto::for_test(PathBuf::from("/photos/older.jpg"));
        older.modified_at = Some(1_700_000_000);
        let mut newer = crate::indexer::IndexedPhoto::for_test(PathBuf::from("/photos/newer.jpg"));
        newer.modified_at = Some(1_800_000_000);

        catalog.upsert_photo(&older).unwrap();
        catalog.upsert_photo(&newer).unwrap();

        let loaded = catalog.search_photos("/photos", 10).unwrap();

        assert_eq!(loaded[0].path, PathBuf::from("/photos/newer.jpg"));
        assert_eq!(loaded[1].path, PathBuf::from("/photos/older.jpg"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn search_photos_page_loads_next_slice_without_reloading_first_rows() {
        let path = test_db_path("paged-search");
        let catalog = Catalog::open(path.clone()).unwrap();
        for index in 1..=3 {
            let mut photo = crate::indexer::IndexedPhoto::for_test(PathBuf::from(format!(
                "/photos/{index}.jpg"
            )));
            photo.modified_at = Some(1_700_000_000 + index);
            catalog.upsert_photo(&photo).unwrap();
        }

        let first_page = catalog.search_photos_page("/photos", 2, 0).unwrap();
        let second_page = catalog.search_photos_page("/photos", 2, 2).unwrap();

        assert_eq!(first_page.len(), 2);
        assert_eq!(second_page.len(), 1);
        assert_eq!(first_page[0].path, PathBuf::from("/photos/3.jpg"));
        assert_eq!(first_page[1].path, PathBuf::from("/photos/2.jpg"));
        assert_eq!(second_page[0].path, PathBuf::from("/photos/1.jpg"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn search_starred_photos_filters_catalog_before_paging() {
        let path = test_db_path("starred-search");
        let catalog = Catalog::open(path.clone()).unwrap();
        for (index, starred) in [(1, false), (2, true), (3, true)] {
            let mut photo = crate::indexer::IndexedPhoto::for_test(PathBuf::from(format!(
                "/photos/{index}.jpg"
            )));
            photo.modified_at = Some(1_700_000_000 + index);
            photo.picasa = Some(PicasaIniEntry {
                caption: Some("family".to_owned()),
                keywords: None,
                starred,
                filters: None,
                faces: Vec::new(),
            });
            catalog.upsert_photo(&photo).unwrap();
        }

        let first_page = catalog
            .search_photos_page_filtered("family", true, 1, 0)
            .unwrap();
        let second_page = catalog
            .search_photos_page_filtered("family", true, 1, 1)
            .unwrap();

        assert_eq!(first_page.len(), 1);
        assert_eq!(second_page.len(), 1);
        assert!(first_page[0].picasa_starred);
        assert!(second_page[0].picasa_starred);
        assert_eq!(first_page[0].path, PathBuf::from("/photos/3.jpg"));
        assert_eq!(second_page[0].path, PathBuf::from("/photos/2.jpg"));

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn chronology_months_can_be_filtered_to_starred_photos() {
        let path = test_db_path("starred-months");
        let catalog = Catalog::open(path.clone()).unwrap();
        for (index, captured_at, starred) in [
            (1, "2026-04-03T10:00:00Z", true),
            (2, "2026-03-01T10:00:00Z", false),
            (3, "2025-12-24T10:00:00Z", true),
        ] {
            let mut photo = crate::indexer::IndexedPhoto::for_test(PathBuf::from(format!(
                "/photos/{index}.jpg"
            )));
            photo.picasa = Some(PicasaIniEntry {
                caption: None,
                keywords: None,
                starred,
                filters: None,
                faces: Vec::new(),
            });
            catalog.upsert_photo(&photo).unwrap();
            catalog
                .connection
                .execute(
                    "UPDATE photos SET captured_at = ?1 WHERE path = ?2",
                    params![captured_at, format!("/photos/{index}.jpg")],
                )
                .unwrap();
        }

        assert_eq!(
            catalog.chronology_months_filtered("", true).unwrap(),
            vec![(2026, 4), (2025, 12)]
        );
        assert_eq!(
            catalog
                .photo_offset_for_month_filtered("", true, 2025, 12)
                .unwrap(),
            Some(1)
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn chronology_months_include_all_catalog_dates_beyond_loaded_page() {
        let path = test_db_path("chronology-months");
        let catalog = Catalog::open(path.clone()).unwrap();
        for (index, captured_at) in [
            (1, "2026-04-03T10:00:00Z"),
            (2, "2026-03-01T10:00:00Z"),
            (3, "2025-12-24T10:00:00Z"),
        ] {
            let mut photo = crate::indexer::IndexedPhoto::for_test(PathBuf::from(format!(
                "/photos/{index}.jpg"
            )));
            photo.modified_at = Some(1);
            photo.picasa = None;
            catalog.upsert_photo(&photo).unwrap();
            catalog
                .connection
                .execute(
                    "UPDATE photos SET captured_at = ?1 WHERE path = ?2",
                    params![captured_at, format!("/photos/{index}.jpg")],
                )
                .unwrap();
        }

        let loaded_page = catalog.search_photos("/photos", 1).unwrap();
        let months = catalog.chronology_months("/photos").unwrap();

        assert_eq!(loaded_page.len(), 1);
        assert_eq!(months, vec![(2026, 4), (2026, 3), (2025, 12)]);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn photo_offset_for_month_finds_first_photo_in_sorted_catalog() {
        let path = test_db_path("month-offset");
        let catalog = Catalog::open(path.clone()).unwrap();
        for (index, captured_at) in [
            (1, "2026-05-01T10:00:00Z"),
            (2, "2026-04-03T10:00:00Z"),
            (3, "2026-04-01T10:00:00Z"),
            (4, "2026-03-01T10:00:00Z"),
        ] {
            let mut photo = crate::indexer::IndexedPhoto::for_test(PathBuf::from(format!(
                "/photos/{index}.jpg"
            )));
            photo.picasa = None;
            catalog.upsert_photo(&photo).unwrap();
            catalog
                .connection
                .execute(
                    "UPDATE photos SET captured_at = ?1 WHERE path = ?2",
                    params![captured_at, format!("/photos/{index}.jpg")],
                )
                .unwrap();
        }

        assert_eq!(
            catalog.photo_offset_for_month("/photos", 2026, 4).unwrap(),
            Some(1)
        );
        assert_eq!(
            catalog.photo_offset_for_month("/photos", 2025, 4).unwrap(),
            None
        );

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

    fn has_index(connection: &Connection, index: &str) -> bool {
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = ?1 AND name = ?2",
                params!["index", index],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    }
}
