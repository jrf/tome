mod config;
mod fetch;
mod index;
mod metadata;
mod model;
mod storage;
mod theme;
mod tui;
mod validate;

use std::path::Path;

use anyhow::Result;
use clap::{Parser, Subcommand};

use config::Config;

#[derive(Parser)]
#[command(name = "tome", about = "A fast TUI bookmark manager")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Search query (pre-fills TUI filter)
    #[arg(global = false)]
    query: Vec<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch metadata for a URL and add it to the library
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
    /// Delete a bookmark by directory slug
    Rm {
        /// Bookmark directory slug (e.g. example-com-attention)
        slug: String,
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
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
        Some(Command::Validate { fix }) => validate::run(&library, fix),
        Some(Command::Rm { slug, yes }) => cmd_rm(&library, &slug, yes),
    }
}

pub fn cmd_add(library: &Path, input: &str) -> Result<()> {
    std::fs::create_dir_all(library)?;
    ensure_library_gitignore(library);

    let fetched = fetch::fetch_url(input)?;
    let mut bookmark = fetched.bookmark;

    let dir = storage::create_bookmark_dir(library, &bookmark)?;

    if let Err(e) = save_snapshot(&dir, &fetched.html) {
        eprintln!("warning: snapshot save failed: {}", e);
    }

    if let Some(text) = fetch::extract_article(&fetched.html, &bookmark.url) {
        let _ = std::fs::write(dir.join("article.txt"), text);
    }

    if let Some(ref img_url) = fetched.image_url {
        match fetch::download_preview(img_url) {
            Ok((bytes, ext)) => {
                let filename = format!("preview.{}", ext);
                let preview_path = dir.join(&filename);
                if std::fs::write(&preview_path, &bytes).is_ok() {
                    bookmark.preview = Some(filename);
                }
            }
            Err(e) => eprintln!("warning: preview image fetch failed: {}", e),
        }
    }

    metadata::write_info(&dir, &bookmark)?;
    index_bookmark(library, &dir, &bookmark);

    println!("Added: {}", bookmark.title);
    println!("  → {}", dir.display());
    Ok(())
}

fn save_snapshot(dir: &Path, html: &str) -> Result<()> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;
    let file = std::fs::File::create(dir.join("snapshot.html.gz"))?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(html.as_bytes())?;
    encoder.finish()?;
    Ok(())
}

fn ensure_library_gitignore(library: &Path) {
    let path = library.join(".gitignore");
    if path.exists() {
        return;
    }
    let contents = ".tome.db\n.tome.db-wal\n.tome.db-shm\n.trash/\n";
    let _ = std::fs::write(path, contents);
}

pub fn index_bookmark(library: &Path, dir: &Path, bookmark: &crate::model::Bookmark) {
    if let Ok(idx) = index::Index::open(library) {
        let dir_name = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let _ = idx.upsert(&dir_name, bookmark);
    }
}

fn cmd_rm(library: &Path, slug: &str, yes: bool) -> Result<()> {
    let dir = library.join(slug);
    if !dir.is_dir() || !dir.join("info.toml").exists() {
        anyhow::bail!("No bookmark found at {}", dir.display());
    }

    let title = metadata::read_info(&dir)
        .map(|b| b.title)
        .unwrap_or_else(|_| slug.to_string());

    if !yes {
        use std::io::Write;
        print!("Delete \"{}\" ({})? [y/N] ", title, slug);
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("Cancelled.");
            return Ok(());
        }
    }

    storage::delete_bookmark_dir(&dir)?;
    if let Ok(idx) = index::Index::open(library) {
        let _ = idx.delete(slug);
    }

    println!("Deleted: {}", title);
    Ok(())
}

fn cmd_reindex(library: &Path) -> Result<()> {
    let idx = index::Index::open(library)?;
    let count = idx.reindex(library)?;
    println!("Indexed {} bookmarks.", count);
    Ok(())
}
