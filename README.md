# bm

A fast TUI bookmark manager for web URLs.

The filesystem is the source of truth — each bookmark is a directory containing
an `info.toml` metadata file (and optionally a saved HTML snapshot). SQLite
(with FTS5) serves as a disposable search index, fully rebuildable from the
filesystem at any time via `bm reindex`.

## Install

Requires Rust.

```
cargo install --path .
```

Or with just:

```
just install
```

## Usage

```
bm                                  # browse library
bm rust                             # browse with "rust" pre-filled
bm add https://example.com/article  # fetch metadata, add bookmark
bm reindex                          # rebuild search index from filesystem
bm validate                         # check library integrity
bm validate --fix                   # auto-fix issues
```

### TUI keybindings

| Key      | Action                                |
|----------|---------------------------------------|
| `j / k`  | Move down / up                        |
| `g / G`  | Jump to top / bottom                  |
| `/ or i` | Enter search mode                     |
| `enter`  | Open URL in browser                   |
| `e`      | Edit info.toml                        |
| `y`      | Copy URL                              |
| `Y`      | Copy as markdown link                 |
| `a`      | Add bookmark (URL)                    |
| `r`      | Re-enrich selected (refetch metadata) |
| `R`      | Enrich all with missing fields        |
| `s`      | Cycle sort (added/site/title/year)    |
| `d`      | Deduplicate library                   |
| `I`      | Reindex library                       |
| `V`      | Validate library (auto-fix)           |
| `t`      | Browse tags                           |
| `T`      | Switch theme                          |
| `c`      | Clear search and tag filter           |
| `?`      | Help                                  |
| `q`      | Quit                                  |

## Library layout

```
~/Bookmarks/
  example-com-attention/
    info.toml
    snapshot.html        # optional
  arxiv-org-1706-03762/
    info.toml
```

Directory naming: `{site-slug}-{title-slug}`, with `-2` suffix on collision.

### info.toml

```toml
url = "https://example.com/article"
title = "Article Title"
description = "Brief description from page meta tags."
authors = ["Jane Doe"]
site = "example.com"
year = 2025
tags = ["rust", "tui"]
added = "2026-05-22"
files = []  # optional snapshot files
```

## Configuration

Optional. bm works without any config file.

`~/.config/bm/config.toml`:

```toml
library = "~/Bookmarks"     # default
editor = "hx"               # defaults to $EDITOR
browser = "open"            # defaults to $BM_BROWSER or "open"
theme = "tokyo-night-moon"  # default
```

Environment variables: `$BM_LIBRARY`, `$BM_BROWSER`, `$EDITOR`.
