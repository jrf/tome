# cairn

A fast TUI bookmark manager for web URLs.

The filesystem is the source of truth — each bookmark is a directory containing
an `info.toml` metadata file (and an optional `article.txt` of readability-extracted
text). SQLite (with FTS5) serves as a disposable search index, fully rebuildable
from the filesystem at any time via `cairn reindex`.

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
cairn                                  # browse library
cairn rust                             # browse with "rust" pre-filled
cairn add https://example.com/article  # fetch metadata, add bookmark
cairn reindex                          # rebuild search index from filesystem
cairn validate                         # check library integrity
cairn validate --fix                   # auto-fix issues
cairn rm <slug>                        # delete bookmark by directory slug
cairn rm <slug> --yes                  # skip confirmation
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
| `D`      | Delete selected bookmark              |
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
    article.txt          # readability-extracted text (optional)
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

Optional. cairn works without any config file.

`~/.config/cairn/config.toml`:

```toml
library = "~/Bookmarks"     # default
editor = "hx"               # defaults to $EDITOR
browser = "open"            # defaults to $CAIRN_BROWSER or "open"
theme = "tokyo-night-moon"  # default
```

Environment variables: `$CAIRN_LIBRARY`, `$CAIRN_BROWSER`, `$EDITOR`.
