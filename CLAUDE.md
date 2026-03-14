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

Four source files in `src/`:

- **main.rs** — CLI entry point using clap derive macros. Defines subcommands (`list`, `show`, `search`, `new`, `edit`, `folders`) and dispatches to the appropriate module. No subcommand launches the TUI.
- **notes.rs** — Apple Notes backend. All interaction goes through `run_applescript()` which shells out to `osascript`. Notes are parsed from AppleScript output using `|||` as a delimiter. Writes convert plaintext to HTML (`\n` → `<br>`). String escaping via `escape_applescript()` is critical for correctness.
- **editor.rs** — Spawns `$EDITOR` (default: vim) with a temp file (`{temp_dir}/tome_{filename}`), reads back the result, then cleans up.
- **tui.rs** — Interactive browse/search mode built with ratatui + crossterm. Two modes: Browse (j/k navigation) and Search (incremental filtering). Manages raw mode and alternate screen, restoring terminal state before launching the editor and on exit.

## Key Patterns

- **Error handling**: `anyhow::Result` throughout, with `.context()` for user-facing messages and `bail!()` for early returns.
- **Data flow**: CLI → `notes.rs` (AppleScript via osascript) → parse `|||`-delimited output → `Note { id, name, folder, body }`.
- **TUI lifecycle**: Enter raw mode / alternate screen → event loop → restore terminal before editor or exit. Terminal restoration must happen even on errors.

## Requirements

- macOS 13+ (depends on Apple Notes app and AppleScript)
- Rust 2024 edition
