# cairn

A fast TUI bookmark manager for web URLs.

The filesystem is the source of truth — each bookmark is a directory containing
an `info.toml` metadata file (and an optional `article.txt` of readability-extracted
text). SQLite (with FTS5) serves as a disposable full-text search index. Cairn
reconciles it from the filesystem when the TUI starts, and it remains manually
rebuildable via `cairn reindex`.

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
cairn add https://example.com/article  # save immediately, then fetch metadata
cairn reindex                          # rebuild search index from filesystem
cairn validate                         # check library integrity
cairn validate --fix                   # auto-fix issues
cairn rm <slug>                        # move bookmark to .trash
cairn rm <slug> --yes                  # skip confirmation
cairn restore <slug>                   # restore bookmark from .trash
```

Adding a valid HTTP(S) URL writes its `info.toml` before attempting a network
request. If metadata fetching fails, the bookmark remains saved and can be
enriched later with `r`. URLs are compared canonically, so common variants such
as tracking parameters, fragments, `www`, or HTTP versus HTTPS do not create
duplicate records.

Search combines fuzzy matching over visible metadata with FTS5 matches across
title, description, authors, site, tags, URL, and `article.txt` content.

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
| `D`      | Move selected bookmark to trash       |
| `I`      | Reindex library                       |
| `V`      | Validate library (read-only)           |
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
Directory names remain stable after enrichment or manual metadata edits.

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
files = []  # optional associated files
```

## Configuration

Optional. cairn works without any config file.

`~/.config/cairn/config.toml`:

```toml
library = "~/Bookmarks"     # default
editor = "hx"               # defaults to $EDITOR
browser = "open"            # defaults to $CAIRN_BROWSER or "open"
theme = "~/.config/themes/tokyo-night-moon.toml"
theme_catalog = "~/.config/themes/catalog.toml"
```

`theme` is loaded directly. `theme_catalog` contains an explicit `themes = [...]`
array used by the picker. Cairn never scans a theme directory. Picker changes
apply to the current session only and never rewrite `config.toml`; edit `theme`
directly to change the startup theme. The files in [`themes/`](themes/) are
examples. If `theme` is unset or its file cannot be loaded, Cairn uses terminal
colors.

The palette's `bg` color fills the interface background; `[ui].background` can
name a different palette color when needed. Cairn resolves the same shared
palette names and semantic UI roles as Grimoire, including `cursor_bg` taking
precedence over `selection`.

Environment variables: `$CAIRN_LIBRARY`, `$CAIRN_BROWSER`, `$EDITOR`.
