# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project overview

cairn is a CLI/TUI bookmark manager for web URLs, written in Rust. The filesystem is the source of truth — each bookmark is a directory containing an `info.toml` metadata file. SQLite (with FTS5) serves as a disposable full-text search index, reconciled at TUI startup and rebuildable via `cairn reindex`.

Architecturally, cairn closely mirrors [grimoire](../grimoire) — a paper reference manager — but adapted for web URLs instead of PDFs.

## Build commands

- `cargo build` — debug build
- `cargo build --release` — release build
- `cargo run -- <args>` — run CLI with arguments
- `cargo clippy` — lint
- `cargo fmt --check` — check formatting
- `just install` — release build + copy to `~/.local/bin/cairn` + codesign on macOS

## Architecture

### Module overview

- **main.rs** — CLI entry point (clap). Bare invocation opens the TUI; subcommands `add`, `reindex`, `validate`, `rm`, `restore`.
- **capture.rs** — Durable save-before-fetch workflow and metadata/article enrichment.
- **model.rs** — `Bookmark` struct. The core data type used by every other module.
- **tui.rs** — Interactive TUI (ratatui). Browse/Search modes, combined FTS/fuzzy filtering, tag/theme popups, dedup workflow, preview pane.
- **storage.rs** — Filesystem operations: create bookmark directories (`{site}-{title}`), list bookmark dirs.
- **metadata.rs** — Read/write `info.toml`.
- **index.rs** — Disposable SQLite FTS5 index over metadata and article text. Schema with triggers, ranked search, startup reconciliation.
- **fetch.rs** — Fetch HTML, extract OpenGraph / meta-tag metadata (title, description, author, site, year).
- **urls.rs** — HTTP(S) validation and canonical keys for duplicate detection.
- **config.rs** — Load `~/.config/cairn/config.toml`. Resolution order: env var > config file > default.
- **theme.rs** — Grimoire-compatible semantic theme system. Loads a configured theme path directly, uses an explicit theme catalog for the session-only picker, and falls back to terminal colors.
- **validate.rs** — Read-only library integrity checks with an explicit CLI auto-fix option.

### Core design principles

- **Filesystem is truth.** SQLite is disposable. TUI startup reconciles it, and `cairn reindex` rebuilds it on demand.
- **Capture before enrichment.** A valid URL is written before network metadata fetching, so an offline or blocked fetch does not lose the bookmark.
- **Defaults over config.** Library at `~/Bookmarks`, `$EDITOR` for editing, `open` for URLs. Config is optional.

### Filesystem layout (library)

```
~/Bookmarks/                       # configurable via $CAIRN_LIBRARY or config
  example-com-attention/
    info.toml                      # source of truth — human-editable metadata
    article.txt                    # readability-extracted text (optional)
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
