use std::path::{Path, PathBuf};

use anyhow::Result;
use url::Url;

use crate::fetch::{self, FetchResult};
use crate::metadata;
use crate::model::Bookmark;
use crate::storage;
use crate::urls;

pub enum CaptureResult {
    Created { dir: PathBuf, bookmark: Bookmark },
    Existing { dir: PathBuf, bookmark: Bookmark },
}

pub fn capture_url(library: &Path, input: &str) -> Result<CaptureResult> {
    std::fs::create_dir_all(library)?;
    ensure_library_gitignore(library);

    let normalized = urls::normalize(input)?;
    if let Some(dir) = storage::find_bookmark_by_url(library, &normalized)? {
        let bookmark = metadata::read_info(&dir)?;
        return Ok(CaptureResult::Existing { dir, bookmark });
    }

    let parsed = Url::parse(&normalized)?;
    let bookmark = Bookmark {
        url: normalized.clone(),
        title: normalized,
        description: None,
        authors: vec![],
        site: parsed.host_str().map(str::to_string),
        year: None,
        tags: vec![],
        added: Some(metadata::today()),
        files: vec![],
    };

    let dir = storage::create_bookmark_dir(library, &bookmark)?;
    metadata::write_info(&dir, &bookmark)?;
    crate::index_bookmark(library, &dir, &bookmark);

    Ok(CaptureResult::Created { dir, bookmark })
}

pub fn enrich_saved_bookmark(library: &Path, dir: &Path) -> Result<Bookmark> {
    enrich_saved_bookmark_with(library, dir, fetch::fetch_url)
}

fn enrich_saved_bookmark_with<F>(library: &Path, dir: &Path, fetcher: F) -> Result<Bookmark>
where
    F: FnOnce(&str) -> Result<FetchResult>,
{
    let current = metadata::read_info(dir)?;
    let fetched = fetcher(&current.url)?;
    let mut updated = current.clone();

    if updated.title.is_empty() || updated.title == updated.url {
        updated.title = fetched.bookmark.title;
    }
    if updated.description.is_none() {
        updated.description = fetched.bookmark.description;
    }
    if updated.authors.is_empty() {
        updated.authors = fetched.bookmark.authors;
    }
    if updated.site.is_none() {
        updated.site = fetched.bookmark.site;
    }
    if updated.year.is_none() {
        updated.year = fetched.bookmark.year;
    }

    if let Some(text) = fetch::extract_article(&fetched.html, &updated.url) {
        std::fs::write(dir.join("article.txt"), text)?;
    }

    metadata::write_info(dir, &updated)?;
    crate::index_bookmark(library, dir, &updated);
    Ok(updated)
}

fn ensure_library_gitignore(library: &Path) {
    let path = library.join(".gitignore");
    if path.exists() {
        return;
    }
    let contents = ".cairn.db\n.cairn.db-wal\n.cairn.db-shm\n.trash/\n";
    let _ = std::fs::write(path, contents);
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn capture_survives_enrichment_failure() {
        let temp = tempdir().unwrap();
        let CaptureResult::Created { dir, .. } =
            capture_url(temp.path(), "https://example.com/article").unwrap()
        else {
            panic!("expected a new bookmark");
        };

        let result = enrich_saved_bookmark_with(temp.path(), &dir, |_| Err(anyhow!("offline")));
        assert!(result.is_err());
        assert!(dir.join("info.toml").is_file());
        assert_eq!(
            metadata::read_info(&dir).unwrap().url,
            "https://example.com/article"
        );
    }

    #[test]
    fn capture_returns_existing_canonical_duplicate() {
        let temp = tempdir().unwrap();
        let first = capture_url(temp.path(), "https://www.example.com/article/").unwrap();
        let first_dir = match first {
            CaptureResult::Created { dir, .. } => dir,
            CaptureResult::Existing { .. } => panic!("expected a new bookmark"),
        };

        let second = capture_url(
            temp.path(),
            "http://example.com/article?utm_source=newsletter",
        )
        .unwrap();
        let second_dir = match second {
            CaptureResult::Existing { dir, .. } => dir,
            CaptureResult::Created { .. } => panic!("expected an existing bookmark"),
        };

        assert_eq!(first_dir, second_dir);
        assert_eq!(storage::list_bookmark_dirs(temp.path()).unwrap().len(), 1);
    }
}
