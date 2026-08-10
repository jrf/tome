# TODO

## Now

## Next

- [ ] Import from browser bookmark exports (HTML/JSON) #feature
- [ ] `cairn refresh <bookmark>` to force re-fetch of metadata/article #feature

## Later

- [ ] Favicon column in list view #improvement
- [ ] Per-bookmark notes file alongside info.toml #feature
- [ ] Validate article.txt readability and consistency #improvement

## Scrapped

- HTML snapshot archive (`snapshot.html.gz` on add) — the gzipped page was never displayed in Cairn (no in-TUI HTML renderer, and `article.txt` covers the reading case), so the disk cost wasn't earning its keep.
- Image previews (og:image download + in-TUI rendering via viuer) — disabled-by-default then dropped entirely. Kept two non-trivial deps (`viuer`, `image`) and a fragile terminal-graphics path alive for a feature that wasn't load-bearing. If revisited, design it fresh as a lazy on-demand fetch gated on terminal capability.

## Done

- [x] Full-text search over metadata and article.txt with startup reconciliation #improvement
- [x] Recoverable bookmark removal (TUI `D`, `cairn rm`, and `cairn restore`) #feature
- [x] TUI improvements: SEARCH/BROWSE labels, always startup in browse mode, Ctrl-J/K scrolling, Tab/Ctrl-A toggle with URL fallback helper #improvement
- [x] Initial scaffold of tome bookmark TUI — port grimoire structure for URL bookmarks #feature
- [x] og:image preview download on add + kitty/iTerm rendering in preview pane via viuer #feature
- [x] HTML snapshot (gzipped) + readability article.txt saved on add #feature
- [x] Reader view in TUI (space key) showing extracted article text #feature
- [x] Library .gitignore auto-written on first add #chore
