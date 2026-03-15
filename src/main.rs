mod config;
mod editor;
mod notes;
mod theme;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Apple Notes from your terminal.")]
struct Cli {
    /// Color theme (synthwave, monochrome, ocean, sunset, forest, tokyo night moon)
    #[arg(long, default_value = "synthwave")]
    theme: String,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// List all notes
    List {
        /// Filter by folder name
        #[arg(short, long)]
        folder: Option<String>,
    },
    /// Show a note's contents
    Show {
        /// Note title (or partial match)
        name: String,
    },
    /// Full-text search across all notes
    Search {
        /// Search query
        query: String,
    },
    /// Create a new note
    New {
        /// Note title
        title: String,
        /// Folder to create the note in
        #[arg(short, long)]
        folder: Option<String>,
        /// Open in $EDITOR instead of reading from stdin
        #[arg(short, long)]
        edit: bool,
    },
    /// Edit a note in $EDITOR
    Edit {
        /// Note title (or partial match)
        name: String,
    },
    /// List all folders
    Folders,
    /// Manage folders
    Folder {
        #[command(subcommand)]
        action: FolderAction,
    },
}

#[derive(Subcommand)]
enum FolderAction {
    /// Create a new folder
    New {
        /// Folder name
        name: String,
    },
    /// Rename a folder
    Rename {
        /// Current folder name
        old: String,
        /// New folder name
        new: String,
    },
    /// Delete a folder
    Delete {
        /// Folder name
        name: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let cfg = config::load();

    // CLI --theme flag overrides config, but only if explicitly provided
    let theme_name = if cli.theme != "synthwave" {
        // User explicitly passed --theme
        cli.theme.clone()
    } else if let Some(ref saved) = cfg.theme {
        saved.clone()
    } else {
        cli.theme.clone()
    };

    let theme = theme::find_theme(&theme_name).unwrap_or_else(|| {
        eprintln!("Unknown theme '{}', using synthwave", theme_name);
        theme::default_theme()
    });

    match cli.command {
        None => tui::run(theme),
        Some(cmd) => match cmd {
            Commands::List { folder } => {
                let notes = notes::list_notes(folder.as_deref())?;
                if notes.is_empty() {
                    println!("No notes found.");
                } else {
                    for note in notes {
                        println!("  {}/{}", note.folder, note.name);
                    }
                }
                Ok(())
            }
            Commands::Show { name } => {
                let note = notes::get_note(&name)?;
                println!("\x1b[1m{}\x1b[0m", note.name);
                println!("\x1b[2m{}\x1b[0m", note.folder);
                println!();
                println!("{}", note.body);
                Ok(())
            }
            Commands::Search { query } => {
                let results = notes::search_notes(&query)?;
                if results.is_empty() {
                    println!("No notes matching \"{}\".", query);
                } else {
                    println!("Found {} note(s):", results.len());
                    for note in results {
                        println!("  {}/{}", note.folder, note.name);
                    }
                }
                Ok(())
            }
            Commands::New {
                title,
                folder,
                edit,
            } => {
                let body = if edit {
                    editor::edit("", &format!("{title}.md"))?
                } else {
                    println!("Enter note body (Ctrl-D to finish):");
                    let mut buf = String::new();
                    use std::io::Read;
                    std::io::stdin().read_to_string(&mut buf)?;
                    buf
                };
                notes::create_note(&title, &body, folder.as_deref())?;
                println!("Created note: {title}");
                Ok(())
            }
            Commands::Edit { name } => {
                let note = notes::get_note(&name)?;
                let content = format!("{}\n\n{}", note.name, note.body);
                let edited = editor::edit(&content, &format!("{}.md", note.name))?;
                if edited == content {
                    println!("No changes made.");
                } else {
                    let (new_title, new_body) = edited.split_once('\n')
                        .map(|(t, b)| (t.trim(), b.trim_start()))
                        .unwrap_or((&edited, ""));
                    notes::update_note(&note.name, new_title, new_body)?;
                    println!("Updated note: {}", new_title);
                }
                Ok(())
            }
            Commands::Folders => {
                let folders = notes::list_folders()?;
                for folder in folders {
                    println!("  {folder}");
                }
                Ok(())
            }
            Commands::Folder { action } => match action {
                FolderAction::New { name } => {
                    notes::create_folder(&name)?;
                    println!("Created folder: {name}");
                    Ok(())
                }
                FolderAction::Rename { old, new } => {
                    notes::rename_folder(&old, &new)?;
                    println!("Renamed folder: {old} -> {new}");
                    Ok(())
                }
                FolderAction::Delete { name } => {
                    notes::delete_folder(&name)?;
                    println!("Deleted folder: {name}");
                    Ok(())
                }
            },
        },
    }
}
