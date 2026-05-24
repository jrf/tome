use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use slug::slugify;

use crate::model::Bookmark;

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

pub fn delete_bookmark_dir(dir: &Path) -> Result<()> {
    std::fs::remove_dir_all(dir)
        .with_context(|| format!("Failed to delete {}", dir.display()))
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

    let title_word =
        first_meaningful_word(&bookmark.title).unwrap_or_else(|| "untitled".to_string());

    slugify(format!("{}-{}", site, title_word))
}

fn strip_site(site: &str) -> String {
    let s = site.trim().trim_start_matches("www.");
    if let Some((host, _)) = s.split_once('.') {
        host.to_string()
    } else {
        s.to_string()
    }
}

fn first_meaningful_word(title: &str) -> Option<String> {
    title
        .split_whitespace()
        .find(|w| {
            let lower = w.to_lowercase();
            !matches!(
                lower.as_str(),
                "a" | "an"
                    | "the"
                    | "on"
                    | "of"
                    | "for"
                    | "in"
                    | "to"
                    | "and"
                    | "with"
                    | "how"
                    | "why"
            )
        })
        .map(|s| s.to_string())
}
