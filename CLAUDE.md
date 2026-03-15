# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Tome?

Tome is a macOS-only CLI for managing Apple Notes from the terminal. It provides subcommands for listing, searching, creating, and editing notes, plus an interactive TUI mode (default when run with no arguments). All Notes interactions happen via AppleScript executed through `osascript`.

## Build & Development Commands

```bash
# Build / run
cargo build                  # debug build
cargo build --release        # release build
cargo run                    # run TUI mode
cargo run -- <command>       # run a specific subcommand

# Lint & format
cargo fmt                    # format code
cargo fmt --check            # check formatting
cargo clippy                 # run lints

# Task runner (justfile)
just build                   # debug build
just release                 # release build
just run                     # run dev build
just install                 # install to ~/.local/bin
just clean                   # clean artifacts
```

No automated tests exist yet — testing is manual against the macOS Notes app.

## Architecture

Six source files in `src/`:

- **main.rs** — CLI entry point using clap derive macros. Defines subcommands (`list`, `show`, `search`, `new`, `edit`, `folders`, `folder {new,rename,delete}`) and dispatches to the appropriate module. No subcommand launches the TUI. Resolves theme from CLI `--theme` flag → config file → default ("synthwave").
- **notes.rs** — Apple Notes backend. All interaction goes through `run_applescript()` which shells out to `osascript`. Notes are parsed from AppleScript output using `|||` as a delimiter. Writes convert plaintext to HTML (`\n` → `<br>`). String escaping via `escape_applescript()` is critical for correctness.
- **editor.rs** — Spawns `$EDITOR` (default: vim) with a temp file (`{temp_dir}/tome_{filename}`), reads back the result, then cleans up.
- **tui.rs** — Interactive browse/search mode built with ratatui + crossterm. Two modes: Browse (j/k navigation) and Search (incremental filtering). Manages raw mode and alternate screen, restoring terminal state before launching the editor and on exit.
- **config.rs** — Loads/saves TOML config from `$XDG_CONFIG_HOME/tome/config.toml` (or `~/.config/tome/config.toml`). Currently stores the preferred theme.
- **theme.rs** — Defines `Theme` struct (border, accent, text colors) and a static `ALL_THEMES` table. Available themes: synthwave (default), monochrome, ocean, sunset, forest, tokyo night moon.

## Key Patterns

- **Error handling**: `anyhow::Result` throughout, with `.context()` for user-facing messages and `bail!()` for early returns.
- **Data flow**: CLI → `notes.rs` (AppleScript via osascript) → parse `|||`-delimited output → `Note { id, name, folder, body }`.
- **TUI lifecycle**: Enter raw mode / alternate screen → event loop → restore terminal before editor or exit. Terminal restoration must happen even on errors.
- **Theme resolution**: CLI `--theme` flag takes priority over config file, which takes priority over the "synthwave" default.

## Why osascript (not a Swift bridge)

Unlike Reminders/Calendar (which have EventKit), Apple Notes has no public framework API. AppleScript via `osascript` is the only viable programmatic interface. This means slower per-operation performance (process spawn each call), fragile string escaping, and brittle delimiter-based parsing — but there is no better alternative until Apple ships a Notes framework.

## Requirements

- macOS 13+ (depends on Apple Notes app and AppleScript)
- Rust 2024 edition
