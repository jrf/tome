use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::metadata;
use crate::storage;

pub struct Index {
    conn: Connection,
}

impl Index {
    pub fn open(library: &Path) -> Result<Self> {
        let db_path = library.join(".cairn.db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open index at {}", db_path.display()))?;

        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS bookmark (
                dir_name TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                title TEXT NOT NULL,
                description TEXT,
                authors TEXT NOT NULL DEFAULT '',
                site TEXT,
                year INTEGER,
                tags TEXT NOT NULL DEFAULT '',
                added TEXT,
                files TEXT NOT NULL DEFAULT ''
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS bookmark_fts USING fts5(
                title, description, authors, site, tags, url,
                content='bookmark',
                content_rowid='rowid'
            );

            CREATE TRIGGER IF NOT EXISTS bookmark_ai AFTER INSERT ON bookmark BEGIN
                INSERT INTO bookmark_fts(rowid, title, description, authors, site, tags, url)
                VALUES (new.rowid, new.title, new.description, new.authors, new.site, new.tags, new.url);
            END;

            CREATE TRIGGER IF NOT EXISTS bookmark_ad AFTER DELETE ON bookmark BEGIN
                INSERT INTO bookmark_fts(bookmark_fts, rowid, title, description, authors, site, tags, url)
                VALUES ('delete', old.rowid, old.title, old.description, old.authors, old.site, old.tags, old.url);
            END;

            CREATE TRIGGER IF NOT EXISTS bookmark_au AFTER UPDATE ON bookmark BEGIN
                INSERT INTO bookmark_fts(bookmark_fts, rowid, title, description, authors, site, tags, url)
                VALUES ('delete', old.rowid, old.title, old.description, old.authors, old.site, old.tags, old.url);
                INSERT INTO bookmark_fts(rowid, title, description, authors, site, tags, url)
                VALUES (new.rowid, new.title, new.description, new.authors, new.site, new.tags, new.url);
            END;",
        )?;

        Ok(Self { conn })
    }

    pub fn reindex(&self, library: &Path) -> Result<usize> {
        let dirs = storage::list_bookmark_dirs(library)?;

        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM bookmark", [])?;

        let mut count = 0;
        for dir in &dirs {
            let bookmark = match metadata::read_info(dir) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let dir_name = dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            tx.execute(
                "INSERT INTO bookmark (dir_name, url, title, description, authors, site, year, tags, added, files)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(dir_name) DO UPDATE SET
                    url=?2, title=?3, description=?4, authors=?5, site=?6, year=?7, tags=?8, added=?9, files=?10",
                params![
                    dir_name,
                    bookmark.url,
                    bookmark.title,
                    bookmark.description,
                    bookmark.authors.join("; "),
                    bookmark.site,
                    bookmark.year,
                    bookmark.tags.join(", "),
                    bookmark.added,
                    bookmark.files.join(", "),
                ],
            )?;
            count += 1;
        }

        tx.commit()?;
        Ok(count)
    }

    pub fn delete(&self, dir_name: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM bookmark WHERE dir_name = ?1", params![dir_name])?;
        Ok(())
    }

    pub fn upsert(&self, dir_name: &str, b: &crate::model::Bookmark) -> Result<()> {
        self.conn.execute(
            "INSERT INTO bookmark (dir_name, url, title, description, authors, site, year, tags, added, files)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(dir_name) DO UPDATE SET
                url=?2, title=?3, description=?4, authors=?5, site=?6, year=?7, tags=?8, added=?9, files=?10",
            params![
                dir_name,
                b.url,
                b.title,
                b.description,
                b.authors.join("; "),
                b.site,
                b.year,
                b.tags.join(", "),
                b.added,
                b.files.join(", "),
            ],
        )?;
        Ok(())
    }
}
