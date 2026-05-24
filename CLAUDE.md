# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project overview

tome is a CLI/TUI bookmark manager for web URLs, written in Rust. The filesystem is the source of truth — each bookmark is a directory containing an `info.toml` metadata file. SQLite (with FTS5) serves as a disposable search index, fully rebuildable from the filesystem at any time via `tome reindex`.

Architecturally, tome closely mirrors [grimoire](../grimoire) — a paper reference manager — but adapted for web URLs instead of PDFs.

## Build commands

- `cargo build` — debug build
- `cargo build --release` — release build
- `cargo run -- <args>` — run CLI with arguments
- `cargo clippy` — lint
- `cargo fmt --check` — check formatting
- `just install` — release build + copy to `~/.local/bin/tome` + codesign on macOS

## Architecture

### Module overview

- **main.rs** — CLI entry point (clap). Bare invocation opens the TUI; subcommands `add`, `reindex`, `validate`, `rm`.
- **model.rs** — `Bookmark` struct. The core data type used by every other module.
- **tui.rs** — Interactive TUI (ratatui). Browse/Search modes, fuzzy filtering, tag/theme popups, dedup workflow, preview pane.
- **storage.rs** — Filesystem operations: create bookmark directories (`{site}-{title}`), list bookmark dirs.
- **metadata.rs** — Read/write `info.toml`.
- **index.rs** — SQLite FTS5 index. Schema with triggers, ranked search.
- **fetch.rs** — Fetch HTML, extract OpenGraph / meta-tag metadata (title, description, author, site, year).
- **config.rs** — Load `~/.config/tome/config.toml`. Resolution order: env var > config file > default.
- **theme.rs** — Color theme system. Loads from `~/.config/tome/themes/{name}.toml`, defaults to Tokyo Night Moon.
- **validate.rs** — Library integrity checks with optional auto-fix.

### Core design principles

- **Filesystem is truth.** SQLite is disposable. `tome reindex` rebuilds from scratch.
- **Defaults over config.** Library at `~/Bookmarks`, `$EDITOR` for editing, `open` for URLs. Config is optional.

### Filesystem layout (library)

```
~/Bookmarks/                       # configurable via $TOME_LIBRARY or config
  example-com-attention/
    info.toml                      # source of truth — human-editable metadata
    snapshot.html                  # optional cached snapshot
```

Directory naming: `{site-slug}-{title-slug}`, with `-2` suffix on collision.

### TUI modes

The TUI has two input modes: **Browse** (single-key shortcuts) and **Search** (typing filters the list). The `LayoutMode` auto-detects terminal aspect ratio with manual `wide`/`tall` override.

### Metadata schema (info.toml)

```toml
url = "https://example.com/article"
title = "Article Title"
description = "From meta tags."
authors = ["Author Name"]
site = "example.com"
year = 2025
tags = ["tag1", "tag2"]
added = "2026-05-22"
files = []
```
