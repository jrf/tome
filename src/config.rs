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
    pub images: Option<bool>,
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
                images: None,
            })
        }
    }

    pub fn images_enabled(&self) -> bool {
        if let Some(v) = self.images {
            return v;
        }
        viuer::KittySupport::None != viuer::get_kitty_support()
            || viuer::is_iterm_supported()
            || std::env::var("TERM")
                .map(|t| t.contains("kitty"))
                .unwrap_or(false)
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
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("tome")
            .join("config.toml")
    }
}
