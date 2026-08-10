use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::metadata;
use crate::storage;

const SCHEMA_VERSION: i64 = 2;

pub struct Index {
    conn: Connection,
}

impl Index {
    pub fn open(library: &Path) -> Result<Self> {
        std::fs::create_dir_all(library)?;
        let db_path = library.join(".cairn.db");
        let conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open index at {}", db_path.display()))?;

        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != SCHEMA_VERSION {
            conn.execute_batch(
                "DROP TRIGGER IF EXISTS bookmark_ai;
                 DROP TRIGGER IF EXISTS bookmark_ad;
                 DROP TRIGGER IF EXISTS bookmark_au;
                 DROP TABLE IF EXISTS bookmark_fts;
                 DROP TABLE IF EXISTS bookmark;",
            )?;
        }

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
                files TEXT NOT NULL DEFAULT '',
                article TEXT NOT NULL DEFAULT ''
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS bookmark_fts USING fts5(
                title, description, authors, site, tags, url, article,
                content='bookmark',
                content_rowid='rowid'
            );

            CREATE TRIGGER IF NOT EXISTS bookmark_ai AFTER INSERT ON bookmark BEGIN
                INSERT INTO bookmark_fts(
                    rowid, title, description, authors, site, tags, url, article
                ) VALUES (
                    new.rowid, new.title, new.description, new.authors, new.site,
                    new.tags, new.url, new.article
                );
            END;

            CREATE TRIGGER IF NOT EXISTS bookmark_ad AFTER DELETE ON bookmark BEGIN
                INSERT INTO bookmark_fts(
                    bookmark_fts, rowid, title, description, authors, site, tags,
                    url, article
                ) VALUES (
                    'delete', old.rowid, old.title, old.description, old.authors,
                    old.site, old.tags, old.url, old.article
                );
            END;

            CREATE TRIGGER IF NOT EXISTS bookmark_au AFTER UPDATE ON bookmark BEGIN
                INSERT INTO bookmark_fts(
                    bookmark_fts, rowid, title, description, authors, site, tags,
                    url, article
                ) VALUES (
                    'delete', old.rowid, old.title, old.description, old.authors,
                    old.site, old.tags, old.url, old.article
                );
                INSERT INTO bookmark_fts(
                    rowid, title, description, authors, site, tags, url, article
                ) VALUES (
                    new.rowid, new.title, new.description, new.authors, new.site,
                    new.tags, new.url, new.article
                );
            END;

            PRAGMA user_version = 2;",
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
                Ok(bookmark) => bookmark,
                Err(_) => continue,
            };
            let dir_name = dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let article = read_article(dir);
            let authors = bookmark.authors.join("; ");
            let tags = bookmark.tags.join(", ");
            let files = bookmark.files.join(", ");

            tx.execute(
                UPSERT_SQL,
                params![
                    dir_name,
                    bookmark.url,
                    bookmark.title,
                    bookmark.description,
                    authors,
                    bookmark.site,
                    bookmark.year,
                    tags,
                    bookmark.added,
                    files,
                    article,
                ],
            )?;
            count += 1;
        }

        tx.commit()?;
        Ok(count)
    }

    pub fn search(&self, query: &str) -> Result<Vec<String>> {
        let Some(query) = match_query(query) else {
            return Ok(Vec::new());
        };
        let mut statement = self.conn.prepare(
            "SELECT bookmark.dir_name
             FROM bookmark_fts
             JOIN bookmark ON bookmark.rowid = bookmark_fts.rowid
             WHERE bookmark_fts MATCH ?1
             ORDER BY bm25(bookmark_fts, 10.0, 3.0, 2.0, 4.0, 6.0, 2.0, 1.0)
             LIMIT 1000",
        )?;
        let rows = statement.query_map(params![query], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<Vec<String>>>()
            .map_err(Into::into)
    }

    pub fn delete(&self, dir_name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM bookmark WHERE dir_name = ?1",
            params![dir_name],
        )?;
        Ok(())
    }

    pub fn upsert(
        &self,
        dir_name: &str,
        bookmark: &crate::model::Bookmark,
        article: &str,
    ) -> Result<()> {
        let authors = bookmark.authors.join("; ");
        let tags = bookmark.tags.join(", ");
        let files = bookmark.files.join(", ");
        self.conn.execute(
            UPSERT_SQL,
            params![
                dir_name,
                bookmark.url,
                bookmark.title,
                bookmark.description,
                authors,
                bookmark.site,
                bookmark.year,
                tags,
                bookmark.added,
                files,
                article,
            ],
        )?;
        Ok(())
    }
}

const UPSERT_SQL: &str = "INSERT INTO bookmark (
        dir_name, url, title, description, authors, site, year, tags, added, files, article
     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
     ON CONFLICT(dir_name) DO UPDATE SET
        url=?2, title=?3, description=?4, authors=?5, site=?6, year=?7,
        tags=?8, added=?9, files=?10, article=?11";

fn read_article(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("article.txt")).unwrap_or_default()
}

fn match_query(query: &str) -> Option<String> {
    let terms: Vec<String> = query
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" AND "))
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::model::Bookmark;

    fn write_bookmark(library: &Path, slug: &str, title: &str, article: Option<&str>) {
        let dir = library.join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        let bookmark = Bookmark {
            url: format!("https://example.com/{}", slug),
            title: title.to_string(),
            description: Some("Synthetic description".to_string()),
            authors: vec!["Example Author".to_string()],
            site: Some("example.com".to_string()),
            year: Some(2026),
            tags: vec!["testing".to_string()],
            added: Some("2026-01-01".to_string()),
            files: vec![],
        };
        metadata::write_info(&dir, &bookmark).unwrap();
        if let Some(article) = article {
            std::fs::write(dir.join("article.txt"), article).unwrap();
        }
    }

    #[test]
    fn search_finds_metadata_and_article_text() {
        let temp = tempdir().unwrap();
        write_bookmark(
            temp.path(),
            "first",
            "Rust bookmark",
            Some("A uniquely searchable passage about cairns."),
        );
        let index = Index::open(temp.path()).unwrap();
        assert_eq!(index.reindex(temp.path()).unwrap(), 1);

        assert_eq!(index.search("Rust").unwrap(), vec!["first"]);
        assert_eq!(index.search("searchable passage").unwrap(), vec!["first"]);
    }

    #[test]
    fn reindex_removes_stale_entries() {
        let temp = tempdir().unwrap();
        write_bookmark(temp.path(), "first", "Temporary bookmark", None);
        let index = Index::open(temp.path()).unwrap();
        index.reindex(temp.path()).unwrap();
        std::fs::remove_dir_all(temp.path().join("first")).unwrap();
        index.reindex(temp.path()).unwrap();

        assert!(index.search("Temporary").unwrap().is_empty());
    }
}
