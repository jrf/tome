use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use slug::slugify;

use crate::model::Bookmark;
use crate::{metadata, urls};

pub fn create_bookmark_dir(library: &Path, bookmark: &Bookmark) -> Result<PathBuf> {
    let dir_name = make_dir_name(bookmark);
    let mut dir = library.join(&dir_name);

    if dir.exists() {
        let mut n = 2;
        loop {
            let candidate = library.join(format!("{}-{}", dir_name, n));
            if !candidate.exists() {
                dir = candidate;
                break;
            }
            n += 1;
        }
    }

    std::fs::create_dir_all(&dir).with_context(|| format!("Failed to create {}", dir.display()))?;
    Ok(dir)
}

pub fn bookmark_dir(library: &Path, slug: &str) -> Result<PathBuf> {
    validate_slug(slug)?;
    Ok(library.join(slug))
}

pub fn find_bookmark_by_url(library: &Path, url: &str) -> Result<Option<PathBuf>> {
    let Some(target) = urls::canonical_key(url) else {
        return Ok(None);
    };

    for dir in list_bookmark_dirs(library)? {
        let Ok(bookmark) = metadata::read_info(&dir) else {
            continue;
        };
        if urls::canonical_key(&bookmark.url).as_deref() == Some(target.as_str()) {
            return Ok(Some(dir));
        }
    }
    Ok(None)
}

pub fn trash_bookmark_dir(library: &Path, dir: &Path) -> Result<PathBuf> {
    if dir.parent() != Some(library) {
        anyhow::bail!("Bookmark must be an immediate child of the library");
    }
    let dir_name = dir
        .file_name()
        .context("Bookmark directory has no name")?
        .to_string_lossy();
    let trash = library.join(".trash");
    std::fs::create_dir_all(&trash)?;

    let destination = available_destination(&trash, &dir_name);
    std::fs::rename(dir, &destination).with_context(|| {
        format!(
            "Failed to move {} to {}",
            dir.display(),
            destination.display()
        )
    })?;
    Ok(destination)
}

pub fn restore_bookmark_dir(library: &Path, slug: &str) -> Result<PathBuf> {
    validate_slug(slug)?;
    let source = library.join(".trash").join(slug);
    if !source.is_dir() || !source.join("info.toml").is_file() {
        anyhow::bail!("No trashed bookmark found at {}", source.display());
    }

    let destination = library.join(slug);
    if destination.exists() {
        anyhow::bail!(
            "Cannot restore because {} already exists",
            destination.display()
        );
    }

    std::fs::rename(&source, &destination).with_context(|| {
        format!(
            "Failed to restore {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(destination)
}

fn available_destination(parent: &Path, name: &str) -> PathBuf {
    let initial = parent.join(name);
    if !initial.exists() {
        return initial;
    }
    for suffix in 2.. {
        let candidate = parent.join(format!("{}-{}", name, suffix));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn validate_slug(slug: &str) -> Result<()> {
    let mut components = Path::new(slug).components();
    if !matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    ) {
        anyhow::bail!("Bookmark slug must be a single directory name");
    }
    Ok(())
}

pub fn list_bookmark_dirs(library: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    if !library.exists() {
        return Ok(dirs);
    }
    for entry in std::fs::read_dir(library)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && path.join("info.toml").exists() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn make_dir_name(bookmark: &Bookmark) -> String {
    let site = bookmark
        .site
        .as_deref()
        .map(strip_site)
        .unwrap_or_else(|| "site".to_string());

    let title_source = if bookmark.title == bookmark.url {
        url::Url::parse(&bookmark.url)
            .ok()
            .and_then(|url| {
                url.path_segments()?
                    .rfind(|segment| !segment.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "home".to_string())
    } else {
        bookmark.title.clone()
    };
    let title = slugify(title_source);
    let title = if title.is_empty() {
        "untitled"
    } else {
        title.get(..title.len().min(48)).unwrap_or(&title)
    };

    slugify(format!("{}-{}", site, title))
}

fn strip_site(site: &str) -> String {
    let s = site.trim().trim_start_matches("www.");
    if let Some((host, _)) = s.split_once('.') {
        host.to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::metadata;

    fn bookmark(url: &str) -> Bookmark {
        Bookmark {
            url: url.to_string(),
            title: "Synthetic bookmark".to_string(),
            description: None,
            authors: vec![],
            site: Some("example.com".to_string()),
            year: None,
            tags: vec![],
            added: Some("2026-01-01".to_string()),
            files: vec![],
        }
    }

    #[test]
    fn trash_and_restore_round_trip() {
        let temp = tempdir().unwrap();
        let dir = create_bookmark_dir(temp.path(), &bookmark("https://example.com")).unwrap();
        metadata::write_info(&dir, &bookmark("https://example.com")).unwrap();
        let slug = dir.file_name().unwrap().to_string_lossy().to_string();

        let trashed = trash_bookmark_dir(temp.path(), &dir).unwrap();
        assert!(trashed.is_dir());
        assert!(!dir.exists());

        let restored = restore_bookmark_dir(temp.path(), &slug).unwrap();
        assert_eq!(restored, dir);
        assert!(restored.join("info.toml").is_file());
    }

    #[test]
    fn restore_rejects_path_traversal() {
        let temp = tempdir().unwrap();
        assert!(restore_bookmark_dir(temp.path(), "../outside").is_err());
        assert!(restore_bookmark_dir(temp.path(), "/").is_err());
    }

    #[test]
    fn placeholder_title_uses_url_path_for_directory_name() {
        let temp = tempdir().unwrap();
        let mut bookmark = bookmark("https://example.com/articles/synthetic-cairn");
        bookmark.title = bookmark.url.clone();

        let dir = create_bookmark_dir(temp.path(), &bookmark).unwrap();
        assert_eq!(
            dir.file_name().unwrap().to_string_lossy(),
            "example-synthetic-cairn"
        );
    }
}
