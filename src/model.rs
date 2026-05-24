use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    pub site: Option<String>,
    pub year: Option<u16>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub added: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}
