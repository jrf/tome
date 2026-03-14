mod editor;
mod notes;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Apple Notes from your terminal.")]
struct Cli {
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => tui::run(),
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
                let edited = editor::edit(&note.body, &format!("{}.md", note.name))?;
                if edited == note.body {
                    println!("No changes made.");
                } else {
                    notes::update_note_body(&note.name, &edited)?;
                    println!("Updated note: {}", note.name);
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
        },
    }
}
