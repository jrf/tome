use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub library: Option<String>,
    pub editor: Option<String>,
    pub browser: Option<String>,
    pub theme: Option<String>,
    pub layout: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if path.exists() {
            let contents = std::fs::read_to_string(&path)?;
            Ok(toml::from_str(&contents)?)
        } else {
            Ok(Self {
                library: None,
                editor: None,
                browser: None,
                theme: None,
                layout: None,
            })
        }
    }

    pub fn library_dir(&self) -> PathBuf {
        if let Ok(val) = std::env::var("TOME_LIBRARY") {
            return PathBuf::from(val);
        }
        if let Some(ref lib) = self.library {
            let expanded = shellexpand::tilde(lib);
            return PathBuf::from(expanded.as_ref());
        }
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Bookmarks")
    }

    pub fn editor(&self) -> String {
        std::env::var("EDITOR")
            .ok()
            .or_else(|| self.editor.clone())
            .unwrap_or_else(|| "vi".to_string())
    }

    pub fn browser(&self) -> String {
        std::env::var("TOME_BROWSER")
            .ok()
            .or_else(|| self.browser.clone())
            .unwrap_or_else(|| "open".to_string())
    }

    fn config_path() -> PathBuf {
        config_dir().join("config.toml")
    }
}

pub fn config_dir() -> PathBuf {
    for candidate in config_dir_candidates() {
        if candidate.exists() {
            return candidate;
        }
    }
    // Nothing exists yet — return the preferred default for this platform.
    config_dir_candidates()
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("."))
}

fn config_dir_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(val) = std::env::var("XDG_CONFIG_HOME") {
        out.push(PathBuf::from(val).join("tome"));
    }
    if let Some(home) = dirs::home_dir() {
        out.push(home.join(".config").join("tome"));
    }
    if let Some(native) = dirs::config_dir() {
        let native = native.join("tome");
        if !out.contains(&native) {
            out.push(native);
        }
    }
    out
}
