use anyhow::{Context, Result};
use std::path::Path;

use crate::model::Bookmark;

pub fn read_info(dir: &Path) -> Result<Bookmark> {
    let path = dir.join("info.toml");
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    Ok(toml::from_str(&contents)?)
}

pub fn write_info(dir: &Path, bookmark: &Bookmark) -> Result<()> {
    let path = dir.join("info.toml");
    let contents = toml::to_string_pretty(bookmark)?;
    std::fs::write(&path, contents)?;
    Ok(())
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}
