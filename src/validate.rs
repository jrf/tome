use std::path::Path;

use anyhow::Result;

use crate::metadata;
use crate::storage;

pub struct ValidateResult {
    pub total: usize,
    pub fixed: usize,
    pub issues: Vec<String>,
}

impl ValidateResult {
    pub fn summary(&self) -> String {
        if self.issues.is_empty() && self.fixed == 0 {
            format!("Library OK — {} bookmarks", self.total)
        } else if self.fixed > 0 && self.issues.is_empty() {
            format!("Fixed {} issues", self.fixed)
        } else if self.fixed > 0 {
            format!(
                "{} issues, fixed {}",
                self.issues.len() + self.fixed,
                self.fixed
            )
        } else {
            format!("{} issues found", self.issues.len())
        }
    }
}

pub fn validate(library: &Path, fix: bool) -> Result<ValidateResult> {
    let dirs = storage::list_bookmark_dirs(library)?;
    let total = dirs.len();
    let mut issues: Vec<String> = Vec::new();
    let mut fixed = 0u32;

    for dir in &dirs {
        let dir_name = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let mut bookmark = match metadata::read_info(dir) {
            Ok(b) => b,
            Err(e) => {
                issues.push(format!("unreadable info.toml: {}: {}", dir_name, e));
                continue;
            }
        };

        if bookmark.url.is_empty() {
            issues.push(format!("missing url: {}", dir_name));
        }
        if bookmark.title.is_empty() {
            issues.push(format!("missing title: {}", dir_name));
        }

        if bookmark.added.is_none() {
            if fix {
                bookmark.added = Some(metadata::today());
                metadata::write_info(dir, &bookmark)?;
                fixed += 1;
            } else {
                issues.push(format!("missing added date: {}", dir_name));
            }
        }

        for listed in &bookmark.files {
            let p = dir.join(listed);
            if !p.exists() {
                issues.push(format!("listed but missing: {}/{}", dir_name, listed));
            }
        }
    }

    Ok(ValidateResult {
        total,
        fixed: fixed as usize,
        issues,
    })
}

pub fn run(library: &Path, fix: bool) -> Result<()> {
    let result = validate(library, fix)?;
    if result.issues.is_empty() {
        println!("{}", result.summary());
    } else {
        println!("{}", result.summary());
        for issue in &result.issues {
            println!("  {}", issue);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::model::Bookmark;

    #[test]
    fn validation_is_read_only_without_fix() {
        let temp = tempdir().unwrap();
        let dir = temp.path().join("synthetic-bookmark");
        std::fs::create_dir_all(&dir).unwrap();
        let bookmark = Bookmark {
            url: "https://example.com/synthetic".to_string(),
            title: "Synthetic bookmark".to_string(),
            description: None,
            authors: vec![],
            site: Some("example.com".to_string()),
            year: None,
            tags: vec![],
            added: None,
            files: vec![],
        };
        metadata::write_info(&dir, &bookmark).unwrap();

        let result = validate(temp.path(), false).unwrap();
        assert_eq!(result.fixed, 0);
        assert!(
            result
                .issues
                .iter()
                .any(|issue| issue.contains("missing added"))
        );
        assert!(metadata::read_info(&dir).unwrap().added.is_none());
    }
}
