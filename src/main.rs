mod capture;
mod config;
mod fetch;
mod index;
mod metadata;
mod model;
mod storage;
mod theme;
mod tui;
mod urls;
mod validate;

use std::path::Path;

use anyhow::Result;
use clap::{Parser, Subcommand};

use config::Config;

#[derive(Parser)]
#[command(name = "cairn", about = "A fast TUI bookmark manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Search query (pre-fills TUI filter)
    #[arg(global = false)]
    query: Vec<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Save a URL and fetch metadata when available
    Add {
        /// URL to bookmark
        url: String,
    },
    /// Rebuild the search index from filesystem
    Reindex,
    /// Validate library integrity
    Validate {
        /// Automatically fix issues
        #[arg(short, long)]
        fix: bool,
    },
    /// Move a bookmark to the library trash
    Rm {
        /// Bookmark directory slug (e.g. example-com-attention)
        slug: String,
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// Restore a bookmark from the library trash
    Restore {
        /// Bookmark directory slug in .trash
        slug: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;
    let library = config.library_dir();

    match cli.command {
        None => {
            let initial = if cli.query.is_empty() {
                None
            } else {
                Some(cli.query.join(" "))
            };
            tui::browse(&config, &library, initial.as_deref())
        }
        Some(Command::Add { url }) => cmd_add(&library, &url),
        Some(Command::Reindex) => cmd_reindex(&library),
        Some(Command::Validate { fix }) => cmd_validate(&library, fix),
        Some(Command::Rm { slug, yes }) => cmd_rm(&library, &slug, yes),
        Some(Command::Restore { slug }) => cmd_restore(&library, &slug),
    }
}

pub fn cmd_add(library: &Path, input: &str) -> Result<()> {
    match capture::capture_url(library, input)? {
        capture::CaptureResult::Existing { dir, bookmark } => {
            println!("Already saved: {}", bookmark.title);
            println!("  → {}", dir.display());
        }
        capture::CaptureResult::Created { dir, bookmark } => {
            match capture::enrich_saved_bookmark(library, &dir) {
                Ok(enriched) => println!("Added: {}", enriched.title),
                Err(error) => {
                    println!("Saved: {}", bookmark.url);
                    eprintln!("Metadata fetch failed: {}", error);
                }
            }
            println!("  → {}", dir.display());
        }
    }
    Ok(())
}

pub fn index_bookmark(library: &Path, dir: &Path, bookmark: &crate::model::Bookmark) {
    if let Ok(idx) = index::Index::open(library) {
        let dir_name = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let article = std::fs::read_to_string(dir.join("article.txt")).unwrap_or_default();
        let _ = idx.upsert(&dir_name, bookmark, &article);
    }
}

fn cmd_rm(library: &Path, slug: &str, yes: bool) -> Result<()> {
    let dir = storage::bookmark_dir(library, slug)?;
    if !dir.is_dir() || !dir.join("info.toml").exists() {
        anyhow::bail!("No bookmark found at {}", dir.display());
    }

    let title = metadata::read_info(&dir)
        .map(|b| b.title)
        .unwrap_or_else(|_| slug.to_string());

    if !yes {
        use std::io::Write;
        print!("Move \"{}\" ({}) to trash? [y/N] ", title, slug);
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    let trashed = storage::trash_bookmark_dir(library, &dir)?;
    if let Ok(idx) = index::Index::open(library) {
        let _ = idx.delete(slug);
    }

    println!("Moved to trash: {}", title);
    println!("  → {}", trashed.display());
    Ok(())
}

fn cmd_restore(library: &Path, slug: &str) -> Result<()> {
    let dir = storage::restore_bookmark_dir(library, slug)?;
    let bookmark = metadata::read_info(&dir)?;
    index_bookmark(library, &dir, &bookmark);
    println!("Restored: {}", bookmark.title);
    println!("  → {}", dir.display());
    Ok(())
}

fn cmd_reindex(library: &Path) -> Result<()> {
    let idx = index::Index::open(library)?;
    let count = idx.reindex(library)?;
    println!("Indexed {} bookmarks.", count);
    Ok(())
}

fn cmd_validate(library: &Path, fix: bool) -> Result<()> {
    validate::run(library, fix)?;
    if fix {
        let index = index::Index::open(library)?;
        index.reindex(library)?;
    }
    Ok(())
}
