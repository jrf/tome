use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use anyhow::Result;
use crossterm::ExecutableCommand;
use crossterm::cursor::Show;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::style::ResetColor;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{Frame, Terminal};

use crate::config::Config as AppConfig;
use crate::index;
use crate::metadata;
use crate::model::Bookmark;
use crate::storage;
use crate::theme::{self, Theme};
use crate::validate;

#[derive(Clone, Copy, PartialEq)]
enum SortMode {
    Added,
    Site,
    Title,
    Year,
}

impl SortMode {
    fn next(self) -> Self {
        match self {
            SortMode::Added => SortMode::Site,
            SortMode::Site => SortMode::Title,
            SortMode::Title => SortMode::Year,
            SortMode::Year => SortMode::Added,
        }
    }

    fn label(self) -> &'static str {
        match self {
            SortMode::Added => "added",
            SortMode::Site => "site",
            SortMode::Title => "title",
            SortMode::Year => "year",
        }
    }
}

struct Entry {
    dir: PathBuf,
    dir_name: String,
    bookmark: Bookmark,
    display: String,
}

pub struct App {
    entries: Vec<Entry>,
    filtered_indices: Vec<usize>,
    filter: String,
    list_state: ListState,
    config: AppConfig,
    theme: Theme,
    input_mode: InputMode,
    should_quit: bool,
    tag_filter: Option<String>,
    all_tags: Vec<String>,
    tag_popup: Option<TagPopup>,
    theme_popup: Option<ThemePopup>,
    layout: LayoutMode,
    flash: Option<(String, std::time::Instant)>,
    preview_scroll: u16,
    show_help: bool,
    list_height: usize,
    add_input: Option<String>,
    enrich_preview: Option<EnrichPreview>,
    enrich_rx: Option<mpsc::Receiver<Vec<EnrichItem>>>,
    sort_mode: SortMode,
    validate_popup: Option<ValidatePopup>,
    preview_image_path: Option<std::path::PathBuf>,
    preview_image_area: Option<ratatui::layout::Rect>,
    last_blit_key: Option<(std::path::PathBuf, ratatui::layout::Rect)>,
    reader_view: Option<ReaderView>,
    delete_confirm: Option<DeleteConfirm>,
}

struct DeleteConfirm {
    idx: usize,
    title: String,
    dir_name: String,
}

struct ReaderView {
    title: String,
    text: String,
    scroll: u16,
}

struct ValidatePopup {
    summary: String,
    issues: Vec<String>,
    scroll: u16,
}

type FieldDiff = (String, String, String);
type EnrichItem = (usize, Bookmark, Vec<FieldDiff>);

struct EnrichPreview {
    idx: usize,
    updated: Bookmark,
    diffs: Vec<FieldDiff>,
    scroll: u16,
    batch_queue: Vec<EnrichItem>,
    applied: usize,
    skipped: usize,
}

struct TagPopup {
    filter: String,
    filtered_tags: Vec<String>,
    counts: std::collections::BTreeMap<String, usize>,
    total: usize,
    selected: usize,
    scroll: usize,
    prev_tag_filter: Option<String>,
}

impl TagPopup {
    fn new(all_tags: &[String], entries: &[Entry], current_tag_filter: &Option<String>) -> Self {
        let mut counts = std::collections::BTreeMap::new();
        for e in entries {
            for tag in &e.bookmark.tags {
                *counts.entry(tag.clone()).or_insert(0) += 1;
            }
        }
        let total = entries.len();
        let mut tags = vec!["(all)".to_string()];
        tags.extend(all_tags.iter().cloned());
        Self {
            filter: String::new(),
            filtered_tags: tags,
            counts,
            total,
            selected: 0,
            scroll: 0,
            prev_tag_filter: current_tag_filter.clone(),
        }
    }

    fn rebuild(&mut self, all_tags: &[String]) {
        let mut tags = vec!["(all)".to_string()];
        tags.extend(all_tags.iter().cloned());
        if self.filter.is_empty() {
            self.filtered_tags = tags;
        } else {
            let f = self.filter.to_lowercase();
            self.filtered_tags = tags
                .into_iter()
                .filter(|t| t.to_lowercase().contains(&f))
                .collect();
        }
        if self.selected >= self.filtered_tags.len() {
            self.selected = self.filtered_tags.len().saturating_sub(1);
        }
    }

    fn count_for(&self, tag: &str) -> usize {
        if tag == "(all)" {
            self.total
        } else {
            self.counts.get(tag).copied().unwrap_or(0)
        }
    }

    fn selected_as_filter(&self) -> Option<String> {
        match self.selected_tag() {
            Some("(all)") | None => None,
            Some(t) => Some(t.to_string()),
        }
    }

    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn move_down(&mut self) {
        if !self.filtered_tags.is_empty() {
            self.selected = (self.selected + 1).min(self.filtered_tags.len() - 1);
        }
    }

    fn page_down(&mut self) {
        if !self.filtered_tags.is_empty() {
            self.selected = (self.selected + 20).min(self.filtered_tags.len() - 1);
        }
    }

    fn page_up(&mut self) {
        self.selected = self.selected.saturating_sub(20);
    }

    fn clamp_scroll(&mut self, visible: usize) {
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible {
            self.scroll = self.selected - visible + 1;
        }
    }

    fn selected_tag(&self) -> Option<&str> {
        self.filtered_tags.get(self.selected).map(|s| s.as_str())
    }
}

struct ThemePopup {
    names: Vec<String>,
    selected: usize,
}

impl ThemePopup {
    fn new() -> Self {
        let mut names = Vec::new();
        let theme_dir = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("tome")
            .join("themes");
        if let Ok(entries) = std::fs::read_dir(&theme_dir) {
            names = entries
                .flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.strip_suffix(".toml").map(|s| s.to_string())
                })
                .collect();
            names.sort();
        }
        Self { names, selected: 0 }
    }

    fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn move_down(&mut self) {
        if !self.names.is_empty() {
            self.selected = (self.selected + 1).min(self.names.len() - 1);
        }
    }

    fn selected_name(&self) -> Option<&str> {
        self.names.get(self.selected).map(|s| s.as_str())
    }
}

#[derive(Clone, Copy)]
enum LayoutMode {
    Wide,
    Tall,
    Auto,
}

impl LayoutMode {
    fn from_config(s: Option<&str>) -> Self {
        match s {
            Some("wide") => Self::Wide,
            Some("tall") => Self::Tall,
            _ => Self::Auto,
        }
    }

    fn resolve(self, width: u16, height: u16) -> ResolvedLayout {
        match self {
            Self::Wide => ResolvedLayout::Wide,
            Self::Tall => ResolvedLayout::Tall,
            Self::Auto => {
                if width as u32 >= height as u32 * 2 {
                    ResolvedLayout::Wide
                } else {
                    ResolvedLayout::Tall
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ResolvedLayout {
    Wide,
    Tall,
}

#[derive(Clone, Copy, PartialEq)]
enum InputMode {
    Browse,
    Search,
}

pub fn browse(config: &AppConfig, library: &Path, initial_query: Option<&str>) -> Result<()> {
    let app = App::new(config, library, initial_query)?;
    run_app(app)
}

fn run_app(mut app: App) -> Result<()> {
    let tty = File::options().read(true).write(true).open("/dev/tty")?;
    let mut tty_ctl = tty.try_clone()?;

    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = terminal::disable_raw_mode();
        if let Ok(mut f) = File::options().write(true).open("/dev/tty") {
            let _ = f.execute(LeaveAlternateScreen);
            let _ = f.execute(ResetColor);
            let _ = f.execute(Show);
        }
        prev_hook(info);
    }));

    tty_ctl.execute(EnterAlternateScreen)?;
    terminal::enable_raw_mode()?;

    let backend = CrosstermBackend::new(BufWriter::new(tty.try_clone()?));
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = run_event_loop(&mut terminal, &mut app, &mut tty_ctl);

    terminal::disable_raw_mode()?;
    tty_ctl.execute(LeaveAlternateScreen)?;
    tty_ctl.execute(ResetColor)?;
    tty_ctl.execute(Show)?;

    result
}

type Term = Terminal<CrosstermBackend<BufWriter<File>>>;

fn run_event_loop(terminal: &mut Term, app: &mut App, tty_ctl: &mut File) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;
        blit_preview_image(app, tty_ctl);

        if app.should_quit {
            return Ok(());
        }

        if let Some(ref rx) = app.enrich_rx {
            match rx.try_recv() {
                Ok(items) => {
                    app.enrich_rx = None;
                    if items.is_empty() {
                        app.flash =
                            Some(("Nothing to enrich".to_string(), std::time::Instant::now()));
                    } else {
                        let mut queue = items;
                        let (idx, updated, diffs) = queue.remove(0);
                        app.jump_to_entry(idx);
                        app.enrich_preview = Some(EnrichPreview {
                            idx,
                            updated,
                            diffs,
                            scroll: 0,
                            batch_queue: queue,
                            applied: 0,
                            skipped: 0,
                        });
                    }
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    app.enrich_rx = None;
                    app.flash = Some(("Enrich failed".to_string(), std::time::Instant::now()));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        let timeout = if app.flash.is_some() || app.enrich_rx.is_some() {
            std::time::Duration::from_millis(100)
        } else {
            std::time::Duration::from_secs(60)
        };
        if !event::poll(timeout)? {
            if app.enrich_rx.is_some() {
                app.flash = Some(("Fetching...".to_string(), std::time::Instant::now()));
            } else if app.flash_message().is_none() {
                app.flash = None;
            }
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if app.show_help {
                app.show_help = false;
                continue;
            }

            if let Some(ref mut rv) = app.reader_view {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        app.reader_view = None;
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        rv.scroll = rv.scroll.saturating_add(1);
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        rv.scroll = rv.scroll.saturating_sub(1);
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        rv.scroll = rv.scroll.saturating_add(10);
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        rv.scroll = rv.scroll.saturating_sub(10);
                    }
                    KeyCode::Char(' ') => {
                        rv.scroll = rv.scroll.saturating_add(20);
                    }
                    KeyCode::Char('g') => {
                        rv.scroll = 0;
                    }
                    _ => {}
                }
                continue;
            }

            if let Some(ref mut vp) = app.validate_popup {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
                        app.validate_popup = None;
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        vp.scroll = vp.scroll.saturating_add(1);
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        vp.scroll = vp.scroll.saturating_sub(1);
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        vp.scroll = vp.scroll.saturating_add(10);
                    }
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        vp.scroll = vp.scroll.saturating_sub(10);
                    }
                    _ => {}
                }
                continue;
            }

            if app.delete_confirm.is_some() {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        app.confirm_delete();
                    }
                    _ => {
                        app.delete_confirm = None;
                    }
                }
                continue;
            }

            if app.add_input.is_some() {
                match key.code {
                    KeyCode::Esc => {
                        app.add_input = None;
                    }
                    KeyCode::Enter => {
                        app.submit_add();
                    }
                    KeyCode::Tab => {
                        if let Some(text) = app.add_input.take() {
                            app.filter = text;
                            app.rebuild_filter();
                        }
                        app.input_mode = InputMode::Search;
                    }
                    KeyCode::Char('a')
                        if key.modifiers.contains(KeyModifiers::ALT)
                            || key.modifiers.contains(KeyModifiers::CONTROL) =>
                    {
                        if let Some(text) = app.add_input.take() {
                            app.filter = text;
                            app.rebuild_filter();
                        }
                        app.input_mode = InputMode::Search;
                    }
                    KeyCode::Backspace => {
                        if let Some(ref mut s) = app.add_input {
                            s.pop();
                        }
                    }
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        if let Some(ref mut s) = app.add_input {
                            s.push(c);
                        }
                    }
                    _ => {}
                }
                continue;
            }

            if app.enrich_preview.is_some() {
                match key.code {
                    KeyCode::Enter | KeyCode::Char('y') => {
                        app.apply_enrich();
                    }
                    KeyCode::Char('n') | KeyCode::Char('s') => {
                        app.skip_enrich();
                    }
                    KeyCode::Esc | KeyCode::Char('q') => {
                        app.finish_enrich();
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        if let Some(ref mut ep) = app.enrich_preview {
                            ep.scroll = ep.scroll.saturating_add(1);
                        }
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        if let Some(ref mut ep) = app.enrich_preview {
                            ep.scroll = ep.scroll.saturating_sub(1);
                        }
                    }
                    _ => {}
                }
                continue;
            }

            if app.tag_popup.is_some() {
                match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) => {
                        let prev = app.tag_popup.as_ref().unwrap().prev_tag_filter.clone();
                        app.tag_popup = None;
                        app.tag_filter = prev;
                        app.rebuild_filter();
                    }
                    (KeyCode::Up, _) => {
                        app.tag_popup.as_mut().unwrap().move_up();
                        app.tag_filter = app.tag_popup.as_ref().unwrap().selected_as_filter();
                        app.rebuild_filter();
                    }
                    (KeyCode::Down, _) => {
                        app.tag_popup.as_mut().unwrap().move_down();
                        app.tag_filter = app.tag_popup.as_ref().unwrap().selected_as_filter();
                        app.rebuild_filter();
                    }
                    (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                        app.tag_popup.as_mut().unwrap().page_down();
                        app.tag_filter = app.tag_popup.as_ref().unwrap().selected_as_filter();
                        app.rebuild_filter();
                    }
                    (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
                        app.tag_popup.as_mut().unwrap().page_up();
                        app.tag_filter = app.tag_popup.as_ref().unwrap().selected_as_filter();
                        app.rebuild_filter();
                    }
                    (KeyCode::Backspace, _) => {
                        let popup = app.tag_popup.as_mut().unwrap();
                        popup.filter.pop();
                        popup.rebuild(&app.all_tags);
                        app.tag_filter = app.tag_popup.as_ref().unwrap().selected_as_filter();
                        app.rebuild_filter();
                    }
                    (KeyCode::Char(c), _) => {
                        let popup = app.tag_popup.as_mut().unwrap();
                        popup.filter.push(c);
                        popup.rebuild(&app.all_tags);
                        app.tag_filter = app.tag_popup.as_ref().unwrap().selected_as_filter();
                        app.rebuild_filter();
                    }
                    (KeyCode::Enter, _) => {
                        app.tag_popup = None;
                    }
                    _ => {}
                }
                continue;
            }

            if app.theme_popup.is_some() {
                match key.code {
                    KeyCode::Esc => {
                        app.theme_popup = None;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.theme_popup.as_mut().unwrap().move_up();
                        if let Some(name) = app.theme_popup.as_ref().unwrap().selected_name() {
                            let theme_name = if name == "default" { None } else { Some(name) };
                            app.theme = theme::load_theme(theme_name);
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        app.theme_popup.as_mut().unwrap().move_down();
                        if let Some(name) = app.theme_popup.as_ref().unwrap().selected_name() {
                            let theme_name = if name == "default" { None } else { Some(name) };
                            app.theme = theme::load_theme(theme_name);
                        }
                    }
                    KeyCode::Enter => {
                        app.theme_popup = None;
                    }
                    _ => {}
                }
                continue;
            }

            match app.input_mode {
                InputMode::Search => match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) => {
                        app.input_mode = InputMode::Browse;
                    }
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => app.should_quit = true,

                    (KeyCode::Char('a'), KeyModifiers::ALT)
                    | (KeyCode::Char('a'), KeyModifiers::CONTROL)
                    | (KeyCode::Tab, _) => {
                        app.add_input = Some(app.filter.clone());
                        app.filter.clear();
                        app.rebuild_filter();
                        app.input_mode = InputMode::Browse;
                    }

                    (KeyCode::Char(c), KeyModifiers::NONE | KeyModifiers::SHIFT) => {
                        app.filter.push(c);
                        app.rebuild_filter();
                    }
                    (KeyCode::Backspace, _) => {
                        app.filter.pop();
                        app.rebuild_filter();
                    }
                    (KeyCode::Up, _)
                    | (KeyCode::Char('p'), KeyModifiers::CONTROL)
                    | (KeyCode::Char('k'), KeyModifiers::CONTROL) => app.move_up(),
                    (KeyCode::Down, _)
                    | (KeyCode::Char('n'), KeyModifiers::CONTROL)
                    | (KeyCode::Char('j'), KeyModifiers::CONTROL) => app.move_down(),
                    (KeyCode::Char('d'), KeyModifiers::CONTROL) => app.half_page_down(),
                    (KeyCode::Char('u'), KeyModifiers::CONTROL) => app.half_page_up(),
                    (KeyCode::Char('f'), KeyModifiers::CONTROL) => app.page_down(),
                    (KeyCode::Char('b'), KeyModifiers::CONTROL) => app.page_up(),
                    (KeyCode::Char('t'), KeyModifiers::CONTROL) => {
                        app.tag_popup =
                            Some(TagPopup::new(&app.all_tags, &app.entries, &app.tag_filter));
                    }
                    (KeyCode::Enter, _) => {
                        app.input_mode = InputMode::Browse;
                    }
                    _ => {}
                },
                InputMode::Browse => match (key.code, key.modifiers) {
                    (KeyCode::Esc, _) | (KeyCode::Char('q'), KeyModifiers::NONE) => {
                        app.should_quit = true;
                    }
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => app.should_quit = true,
                    (KeyCode::Char('/'), KeyModifiers::NONE)
                    | (KeyCode::Char('i'), KeyModifiers::NONE) => {
                        app.input_mode = InputMode::Search;
                    }
                    (KeyCode::Char('j'), KeyModifiers::NONE)
                    | (KeyCode::Down, _)
                    | (KeyCode::Char('n'), KeyModifiers::CONTROL)
                    | (KeyCode::Char('j'), KeyModifiers::CONTROL) => app.move_down(),
                    (KeyCode::Char('k'), KeyModifiers::NONE)
                    | (KeyCode::Up, _)
                    | (KeyCode::Char('p'), KeyModifiers::CONTROL)
                    | (KeyCode::Char('k'), KeyModifiers::CONTROL) => app.move_up(),
                    (KeyCode::Char('d'), KeyModifiers::CONTROL) => app.half_page_down(),
                    (KeyCode::Char('u'), KeyModifiers::CONTROL) => app.half_page_up(),
                    (KeyCode::Char('f'), KeyModifiers::CONTROL) => app.page_down(),
                    (KeyCode::Char('b'), KeyModifiers::CONTROL) => app.page_up(),
                    (KeyCode::Char('g'), KeyModifiers::NONE) => app.move_to_top(),
                    (KeyCode::Char('G'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        app.move_to_bottom();
                    }
                    (KeyCode::Char('t'), KeyModifiers::NONE) => {
                        app.tag_popup =
                            Some(TagPopup::new(&app.all_tags, &app.entries, &app.tag_filter));
                    }
                    (KeyCode::Char('T'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        app.theme_popup = Some(ThemePopup::new());
                    }
                    (KeyCode::Char('c'), KeyModifiers::NONE) => {
                        app.filter.clear();
                        app.tag_filter = None;
                        app.rebuild_filter();
                    }
                    (KeyCode::Enter, _) => {
                        app.action_open();
                    }
                    (KeyCode::Char('e'), KeyModifiers::NONE) => {
                        app.action_edit(terminal, tty_ctl)?;
                    }
                    (KeyCode::Char('y'), KeyModifiers::NONE) => {
                        app.action_copy_url();
                    }
                    (KeyCode::Char('Y'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        app.action_copy_markdown();
                    }
                    (KeyCode::Char('a'), KeyModifiers::NONE) => {
                        app.add_input = Some(String::new());
                    }
                    (KeyCode::Char('r'), KeyModifiers::NONE) => {
                        app.action_enrich_selected();
                    }
                    (KeyCode::Char('R'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        app.action_enrich_all();
                    }
                    (KeyCode::Char('d'), KeyModifiers::NONE) => {
                        run_dedup(terminal, app)?;
                    }
                    (KeyCode::Char('D'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        app.action_delete_prompt();
                    }
                    (KeyCode::Char('I'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        app.action_reindex();
                    }
                    (KeyCode::Char('V'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        app.action_validate();
                    }
                    (KeyCode::Char('s'), KeyModifiers::NONE) => {
                        if app.filter.is_empty() {
                            app.sort_mode = app.sort_mode.next();
                            app.rebuild_filter();
                        }
                    }
                    (KeyCode::Char('J'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        app.preview_scroll = app.preview_scroll.saturating_add(3);
                    }
                    (KeyCode::Char('K'), KeyModifiers::SHIFT | KeyModifiers::NONE) => {
                        app.preview_scroll = app.preview_scroll.saturating_sub(3);
                    }
                    (KeyCode::Char('?'), _) => {
                        app.show_help = true;
                    }
                    (KeyCode::Char(' '), _) => {
                        app.action_open_reader();
                    }
                    _ => {}
                },
            }
        }
    }
}

impl App {
    fn new(config: &AppConfig, library: &Path, initial_query: Option<&str>) -> Result<Self> {
        let dirs = storage::list_bookmark_dirs(library)?;
        let entries: Vec<Entry> = dirs.into_iter().filter_map(make_entry).collect();

        let filter = initial_query.unwrap_or("").to_string();
        let filtered_indices: Vec<usize> = (0..entries.len()).collect();

        let theme = theme::load_theme(config.theme.as_deref());

        let mut tag_set = std::collections::BTreeSet::new();
        for e in &entries {
            for tag in &e.bookmark.tags {
                tag_set.insert(tag.clone());
            }
        }
        let all_tags: Vec<String> = tag_set.into_iter().collect();

        let mut app = App {
            entries,
            filtered_indices,
            filter,
            list_state: ListState::default(),
            config: config.clone(),
            theme,
            input_mode: InputMode::Browse,
            should_quit: false,
            tag_filter: None,
            all_tags,
            tag_popup: None,
            theme_popup: None,
            layout: LayoutMode::from_config(config.layout.as_deref()),
            flash: None,
            preview_scroll: 0,
            show_help: false,
            list_height: 20,
            add_input: None,
            enrich_preview: None,
            enrich_rx: None,
            sort_mode: SortMode::Added,
            validate_popup: None,
            preview_image_path: None,
            preview_image_area: None,
            last_blit_key: None,
            reader_view: None,
            delete_confirm: None,
        };

        app.rebuild_filter();
        if !app.filtered_indices.is_empty() {
            app.list_state.select(Some(0));
        }

        Ok(app)
    }

    fn rebuild_filter(&mut self) {
        let tag_filtered: Vec<usize> = if let Some(ref tag) = self.tag_filter {
            self.entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.bookmark.tags.iter().any(|t| t == tag))
                .map(|(i, _)| i)
                .collect()
        } else {
            (0..self.entries.len()).collect()
        };

        if self.filter.is_empty() {
            self.filtered_indices = tag_filtered;
            self.apply_sort();
        } else {
            let pattern = Pattern::parse(&self.filter, CaseMatching::Ignore, Normalization::Smart);
            let mut matcher = Matcher::new(Config::DEFAULT);
            let mut buf = Vec::new();

            let mut scored: Vec<(usize, u32)> = tag_filtered
                .into_iter()
                .filter_map(|i| {
                    let haystack = Utf32Str::new(&self.entries[i].display, &mut buf);
                    pattern.score(haystack, &mut matcher).map(|s| (i, s))
                })
                .collect();
            scored.sort_by_key(|&(_, s)| std::cmp::Reverse(s));
            self.filtered_indices = scored.into_iter().map(|(i, _)| i).collect();
        }

        if self.filtered_indices.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
        self.preview_scroll = 0;
    }

    fn apply_sort(&mut self) {
        let entries = &self.entries;
        self.filtered_indices.sort_by(|&a, &b| {
            let ea = &entries[a];
            let eb = &entries[b];
            match self.sort_mode {
                SortMode::Added => {
                    let aa = ea.bookmark.added.as_deref().unwrap_or("");
                    let ba = eb.bookmark.added.as_deref().unwrap_or("");
                    ba.cmp(aa)
                }
                SortMode::Site => {
                    let aa = ea.bookmark.site.as_deref().unwrap_or("").to_lowercase();
                    let ba = eb.bookmark.site.as_deref().unwrap_or("").to_lowercase();
                    aa.cmp(&ba)
                }
                SortMode::Title => ea
                    .bookmark
                    .title
                    .to_lowercase()
                    .cmp(&eb.bookmark.title.to_lowercase()),
                SortMode::Year => {
                    let ya = ea.bookmark.year.unwrap_or(0);
                    let yb = eb.bookmark.year.unwrap_or(0);
                    yb.cmp(&ya)
                }
            }
        });
    }

    fn selected_entry(&self) -> Option<&Entry> {
        let selected = self.list_state.selected()?;
        let &idx = self.filtered_indices.get(selected)?;
        self.entries.get(idx)
    }

    fn move_up(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(0) | None => 0,
            Some(i) => i - 1,
        };
        self.list_state.select(Some(i));
        self.preview_scroll = 0;
    }

    fn move_down(&mut self) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = match self.list_state.selected() {
            Some(i) if i >= self.filtered_indices.len() - 1 => self.filtered_indices.len() - 1,
            Some(i) => i + 1,
            None => 0,
        };
        self.list_state.select(Some(i));
        self.preview_scroll = 0;
    }

    fn scroll_up(&mut self, lines: usize) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        self.list_state.select(Some(i.saturating_sub(lines)));
        self.preview_scroll = 0;
    }

    fn scroll_down(&mut self, lines: usize) {
        if self.filtered_indices.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        let max = self.filtered_indices.len() - 1;
        self.list_state.select(Some((i + lines).min(max)));
        self.preview_scroll = 0;
    }

    fn half_page_up(&mut self) {
        self.scroll_up(self.list_height / 2);
    }

    fn half_page_down(&mut self) {
        self.scroll_down(self.list_height / 2);
    }

    fn page_up(&mut self) {
        self.scroll_up(self.list_height);
    }

    fn page_down(&mut self) {
        self.scroll_down(self.list_height);
    }

    fn move_to_top(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.list_state.select(Some(0));
            self.preview_scroll = 0;
        }
    }

    fn move_to_bottom(&mut self) {
        if !self.filtered_indices.is_empty() {
            self.list_state
                .select(Some(self.filtered_indices.len() - 1));
            self.preview_scroll = 0;
        }
    }

    fn action_open(&mut self) {
        let entry = match self.selected_entry() {
            Some(e) => e,
            None => return,
        };
        let url = entry.bookmark.url.clone();
        if url.is_empty() {
            self.flash = Some(("No URL".to_string(), std::time::Instant::now()));
            return;
        }
        match std::process::Command::new(self.config.browser())
            .arg(&url)
            .spawn()
        {
            Ok(_) => {
                self.flash = Some(("Opened in browser".to_string(), std::time::Instant::now()))
            }
            Err(e) => self.flash = Some((format!("Error: {}", e), std::time::Instant::now())),
        }
    }

    fn action_open_reader(&mut self) {
        let entry = match self.selected_entry() {
            Some(e) => e,
            None => return,
        };
        let article_path = entry.dir.join("article.txt");
        let title = entry.bookmark.title.clone();
        match std::fs::read_to_string(&article_path) {
            Ok(text) if !text.trim().is_empty() => {
                self.reader_view = Some(ReaderView {
                    title,
                    text,
                    scroll: 0,
                });
            }
            _ => {
                self.flash = Some((
                    "No article text saved".to_string(),
                    std::time::Instant::now(),
                ));
            }
        }
    }

    fn action_delete_prompt(&mut self) {
        let selected = match self.list_state.selected() {
            Some(s) => s,
            None => return,
        };
        let idx = match self.filtered_indices.get(selected) {
            Some(&i) => i,
            None => return,
        };
        let entry = &self.entries[idx];
        let dir_name = entry
            .dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        self.delete_confirm = Some(DeleteConfirm {
            idx,
            title: entry.bookmark.title.clone(),
            dir_name,
        });
    }

    fn confirm_delete(&mut self) {
        let confirm = match self.delete_confirm.take() {
            Some(c) => c,
            None => return,
        };
        let dir = self.entries[confirm.idx].dir.clone();
        match storage::delete_bookmark_dir(&dir) {
            Ok(()) => {
                let library = self.config.library_dir();
                if let Ok(idx) = index::Index::open(&library) {
                    let _ = idx.delete(&confirm.dir_name);
                }
                self.reload_entries();
                let visible = self.filtered_indices.len();
                if visible == 0 {
                    self.list_state.select(None);
                } else {
                    let pos = self.list_state.selected().unwrap_or(0).min(visible - 1);
                    self.list_state.select(Some(pos));
                }
                self.flash = Some((
                    format!("Deleted: {}", confirm.title),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => {
                self.flash = Some((format!("Delete failed: {}", e), std::time::Instant::now()));
            }
        }
    }

    fn action_edit(&mut self, terminal: &mut Term, tty_ctl: &mut File) -> Result<()> {
        let entry = match self.selected_entry() {
            Some(e) => e,
            None => return Ok(()),
        };
        let info_path = entry.dir.join("info.toml");

        terminal::disable_raw_mode()?;
        tty_ctl.execute(LeaveAlternateScreen)?;

        std::process::Command::new(self.config.editor())
            .arg(&info_path)
            .status()?;

        tty_ctl.execute(EnterAlternateScreen)?;
        terminal::enable_raw_mode()?;
        terminal.clear()?;

        let idx = self.filtered_indices[self.list_state.selected().unwrap_or(0)];
        if let Ok(b) = metadata::read_info(&self.entries[idx].dir) {
            self.update_entry_display(idx, b);
        }
        Ok(())
    }

    fn action_copy_url(&mut self) {
        let entry = match self.selected_entry() {
            Some(e) => e,
            None => return,
        };
        let url = entry.bookmark.url.clone();
        if url.is_empty() {
            self.flash = Some(("No URL".to_string(), std::time::Instant::now()));
            return;
        }
        if pipe_to_pbcopy(&url).is_ok() {
            self.flash = Some(("Copied URL".to_string(), std::time::Instant::now()));
        } else {
            self.flash = Some(("Copy failed".to_string(), std::time::Instant::now()));
        }
    }

    fn action_copy_markdown(&mut self) {
        let entry = match self.selected_entry() {
            Some(e) => e,
            None => return,
        };
        let b = &entry.bookmark;
        if b.url.is_empty() {
            self.flash = Some(("No URL".to_string(), std::time::Instant::now()));
            return;
        }
        let title = if b.title.is_empty() {
            b.url.as_str()
        } else {
            b.title.as_str()
        };
        let md = format!("[{}]({})", title, b.url);
        if pipe_to_pbcopy(&md).is_ok() {
            self.flash = Some(("Copied markdown".to_string(), std::time::Instant::now()));
        } else {
            self.flash = Some(("Copy failed".to_string(), std::time::Instant::now()));
        }
    }

    fn flash_message(&self) -> Option<&str> {
        self.flash.as_ref().and_then(|(msg, t)| {
            if t.elapsed().as_secs() < 2 {
                Some(msg.as_str())
            } else {
                None
            }
        })
    }

    fn submit_add(&mut self) {
        let input = match self.add_input.take() {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => {
                self.add_input = None;
                return;
            }
        };

        self.flash = Some(("Adding...".to_string(), std::time::Instant::now()));

        let bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("tome"));
        let output = std::process::Command::new(bin)
            .arg("add")
            .arg(&input)
            .output();

        match output {
            Ok(o) if o.status.success() => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let msg = stdout
                    .lines()
                    .find(|l| l.starts_with("Added:"))
                    .unwrap_or("Added successfully")
                    .to_string();
                self.flash = Some((msg, std::time::Instant::now()));
                self.reload_entries();
            }
            Ok(o) => {
                let stderr = String::from_utf8_lossy(&o.stderr);
                let msg = stderr.lines().last().unwrap_or("Add failed").to_string();
                self.flash = Some((msg, std::time::Instant::now()));
            }
            Err(e) => {
                self.flash = Some((format!("Error: {}", e), std::time::Instant::now()));
            }
        }
    }

    fn action_reindex(&mut self) {
        let library = self.config.library_dir();
        match index::Index::open(&library).and_then(|idx| idx.reindex(&library)) {
            Ok(count) => {
                self.reload_entries();
                self.flash = Some((
                    format!("Reindexed {} bookmarks", count),
                    std::time::Instant::now(),
                ));
            }
            Err(e) => {
                self.flash = Some((format!("Reindex error: {}", e), std::time::Instant::now()));
            }
        }
    }

    fn action_validate(&mut self) {
        let library = self.config.library_dir();
        match validate::validate(&library, true) {
            Ok(result) => {
                self.reload_entries();
                self.validate_popup = Some(ValidatePopup {
                    summary: result.summary(),
                    issues: result.issues,
                    scroll: 0,
                });
            }
            Err(e) => {
                self.flash = Some((format!("Validate error: {}", e), std::time::Instant::now()));
            }
        }
    }

    fn reload_entries(&mut self) {
        let library = self.config.library_dir();
        let dirs = match storage::list_bookmark_dirs(&library) {
            Ok(d) => d,
            Err(_) => return,
        };
        self.entries = dirs.into_iter().filter_map(make_entry).collect();

        let mut tag_set = std::collections::BTreeSet::new();
        for e in &self.entries {
            for tag in &e.bookmark.tags {
                tag_set.insert(tag.clone());
            }
        }
        self.all_tags = tag_set.into_iter().collect();
        self.rebuild_filter();
    }

    fn action_enrich_selected(&mut self) {
        if self.enrich_rx.is_some() {
            return;
        }
        let selected = match self.list_state.selected() {
            Some(s) => s,
            None => return,
        };
        let idx = self.filtered_indices[selected];
        let bookmark = self.entries[idx].bookmark.clone();
        let dir = self.entries[idx].dir.clone();

        self.flash = Some(("Fetching...".to_string(), std::time::Instant::now()));
        let (tx, rx) = mpsc::channel();
        self.enrich_rx = Some(rx);

        std::thread::spawn(move || {
            let result = match enrich_bookmark(&bookmark, &dir) {
                Ok(Some(updated)) => {
                    let diffs = compute_diffs(&bookmark, &updated);
                    if diffs.is_empty() {
                        vec![]
                    } else {
                        vec![(idx, updated, diffs)]
                    }
                }
                _ => vec![],
            };
            let _ = tx.send(result);
        });
    }

    fn action_enrich_all(&mut self) {
        if self.enrich_rx.is_some() {
            return;
        }

        let work: Vec<(usize, Bookmark, std::path::PathBuf)> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| needs_enrich(&e.bookmark))
            .map(|(i, e)| (i, e.bookmark.clone(), e.dir.clone()))
            .collect();

        if work.is_empty() {
            self.flash = Some(("Nothing to enrich".to_string(), std::time::Instant::now()));
            return;
        }

        self.flash = Some((
            format!("Fetching {} entries...", work.len()),
            std::time::Instant::now(),
        ));
        let (tx, rx) = mpsc::channel();
        self.enrich_rx = Some(rx);

        std::thread::spawn(move || {
            let mut items: Vec<EnrichItem> = Vec::new();
            for (idx, bookmark, dir) in work {
                if let Ok(Some(updated)) = enrich_bookmark(&bookmark, &dir) {
                    let diffs = compute_diffs(&bookmark, &updated);
                    if !diffs.is_empty() {
                        items.push((idx, updated, diffs));
                    }
                }
            }
            let _ = tx.send(items);
        });
    }

    fn apply_enrich(&mut self) {
        let ep = match self.enrich_preview.take() {
            Some(ep) => ep,
            None => return,
        };
        let library = self.config.library_dir();
        let _ = metadata::write_info(&self.entries[ep.idx].dir, &ep.updated);
        crate::index_bookmark(&library, &self.entries[ep.idx].dir, &ep.updated);
        self.update_entry_display(ep.idx, ep.updated);
        let applied = ep.applied + 1;
        self.advance_enrich_queue(ep.batch_queue, applied, ep.skipped);
    }

    fn skip_enrich(&mut self) {
        let ep = match self.enrich_preview.take() {
            Some(ep) => ep,
            None => return,
        };
        let skipped = ep.skipped + 1;
        self.advance_enrich_queue(ep.batch_queue, ep.applied, skipped);
    }

    fn finish_enrich(&mut self) {
        let ep = match self.enrich_preview.take() {
            Some(ep) => ep,
            None => return,
        };
        if ep.applied > 0 || ep.skipped > 0 {
            self.flash = Some((
                format!(
                    "Enriched {}, skipped {}",
                    ep.applied,
                    ep.skipped + ep.batch_queue.len() + 1
                ),
                std::time::Instant::now(),
            ));
        }
    }

    fn advance_enrich_queue(&mut self, mut queue: Vec<EnrichItem>, applied: usize, skipped: usize) {
        if queue.is_empty() {
            let msg = format!("Enriched {}, skipped {}", applied, skipped);
            self.flash = Some((msg, std::time::Instant::now()));
            return;
        }
        let (idx, updated, diffs) = queue.remove(0);
        self.jump_to_entry(idx);
        self.enrich_preview = Some(EnrichPreview {
            idx,
            updated,
            diffs,
            scroll: 0,
            batch_queue: queue,
            applied,
            skipped,
        });
    }

    fn jump_to_entry(&mut self, entry_idx: usize) {
        if let Some(pos) = self.filtered_indices.iter().position(|&i| i == entry_idx) {
            self.list_state.select(Some(pos));
            self.preview_scroll = 0;
        }
    }

    fn update_entry_display(&mut self, idx: usize, b: Bookmark) {
        let display = entry_display(&b);
        self.entries[idx].bookmark = b;
        self.entries[idx].display = display;
    }
}

fn make_entry(dir: PathBuf) -> Option<Entry> {
    let dir_name = dir.file_name()?.to_string_lossy().to_string();
    let bookmark = metadata::read_info(&dir).ok()?;
    let display = entry_display(&bookmark);
    Some(Entry {
        dir,
        dir_name,
        bookmark,
        display,
    })
}

fn entry_display(b: &Bookmark) -> String {
    let site = b.site.as_deref().unwrap_or("");
    let parts: Vec<&str> = [site, b.title.as_str(), b.url.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    parts.join("  ")
}

fn compute_diffs(old: &Bookmark, new: &Bookmark) -> Vec<FieldDiff> {
    let mut diffs = Vec::new();

    if old.title != new.title && !new.title.is_empty() {
        diffs.push(("title".into(), old.title.clone(), new.title.clone()));
    }
    if old.description != new.description && new.description.is_some() {
        diffs.push((
            "description".into(),
            old.description.clone().unwrap_or_default(),
            new.description.clone().unwrap_or_default(),
        ));
    }
    if old.authors != new.authors && !new.authors.is_empty() {
        diffs.push((
            "authors".into(),
            old.authors.join(", "),
            new.authors.join(", "),
        ));
    }
    if old.site != new.site && new.site.is_some() {
        diffs.push((
            "site".into(),
            old.site.clone().unwrap_or_default(),
            new.site.clone().unwrap_or_default(),
        ));
    }
    if old.year != new.year && new.year.is_some() {
        diffs.push((
            "year".into(),
            old.year.map(|y| y.to_string()).unwrap_or_default(),
            new.year.map(|y| y.to_string()).unwrap_or_default(),
        ));
    }

    diffs
}

fn needs_enrich(b: &Bookmark) -> bool {
    b.title.is_empty()
        || b.title == b.url
        || b.description.is_none()
        || b.site.is_none()
        || b.year.is_none()
}

fn enrich_bookmark(b: &Bookmark, dir: &Path) -> Result<Option<Bookmark>> {
    use crate::fetch;

    if b.url.is_empty() {
        return Ok(None);
    }

    let fetched = match fetch::fetch_url(&b.url) {
        Ok(f) => f,
        Err(_) => return Ok(None),
    };
    let fetched_bookmark = fetched.bookmark;

    let mut updated = b.clone();

    if updated.title.is_empty() || updated.title == updated.url {
        updated.title = fetched_bookmark.title;
    }
    if updated.description.is_none() {
        updated.description = fetched_bookmark.description;
    }
    if updated.authors.is_empty() {
        updated.authors = fetched_bookmark.authors;
    }
    if updated.site.is_none() {
        updated.site = fetched_bookmark.site;
    }
    if updated.year.is_none() {
        updated.year = fetched_bookmark.year;
    }

    if updated.preview.is_none()
        && let Some(ref img_url) = fetched.image_url
        && let Ok((bytes, ext)) = fetch::download_preview(img_url)
    {
        let filename = format!("preview.{}", ext);
        if std::fs::write(dir.join(&filename), &bytes).is_ok() {
            updated.preview = Some(filename);
        }
    }

    Ok(Some(updated))
}

fn blit_preview_image(app: &mut App, tty_ctl: &mut File) {
    use std::io::Write;
    let suppress = app.reader_view.is_some();
    let key = if suppress {
        None
    } else {
        app.preview_image_path
            .as_ref()
            .zip(app.preview_image_area)
            .map(|(p, r)| (p.clone(), r))
    };

    if key == app.last_blit_key {
        return;
    }

    if app.last_blit_key.is_some() && key.is_none() {
        let _ = tty_ctl.write_all(b"\x1b_Ga=d,d=A\x1b\\");
        let _ = tty_ctl.flush();
    }

    if let Some((ref path, rect)) = key {
        let cfg = viuer::Config {
            x: rect.x,
            y: rect.y as i16,
            width: Some(rect.width as u32),
            height: Some(rect.height as u32),
            absolute_offset: true,
            transparent: true,
            restore_cursor: true,
            ..Default::default()
        };
        let _ = viuer::print_from_file(path, &cfg);
    }

    app.last_blit_key = key;
}

fn pipe_to_pbcopy(s: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut child = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(s.as_bytes())?;
    }
    child.wait()?;
    Ok(())
}

fn is_importable(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.starts_with("http://") || trimmed.starts_with("https://")
}

fn draw(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let s_text = Style::default().fg(t.text);
    let s_dim = Style::default().fg(t.text_dim);
    let s_muted = Style::default().fg(t.text_muted);
    let s_author = Style::default().fg(t.author);
    let s_hl = Style::default().fg(t.highlight);
    let s_link = Style::default().fg(t.link);
    let s_date = Style::default().fg(t.date);

    let area = f.area();
    let resolved = app.layout.resolve(area.width, area.height);

    let border_style = Style::default().fg(t.border);

    let (left_col, preview_area) = match resolved {
        ResolvedLayout::Wide => {
            let chunks =
                Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(area);
            (chunks[0], Some(chunks[1]))
        }
        ResolvedLayout::Tall => {
            let chunks = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);
            (chunks[0], Some(chunks[1]))
        }
    };

    let left_parts = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(left_col);
    let search_area = left_parts[0];
    let list_area = left_parts[1];

    let search_content = if let Some(ref add_text) = app.add_input {
        Line::from(Span::styled(add_text.as_str(), s_text))
    } else {
        let mut spans = Vec::new();
        if let Some(ref tag) = app.tag_filter {
            spans.push(Span::styled(format!("[{}] ", tag), s_hl));
        }
        if !app.filter.is_empty() {
            spans.push(Span::styled(&app.filter, s_text));
        }
        Line::from(spans)
    };

    let search_title = if app.add_input.is_some() {
        Line::from(Span::styled(" Add URL ", s_hl))
    } else {
        Line::from(Span::styled(" Search ", s_hl))
    };

    let search_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(search_title);
    let search_inner = search_block.inner(search_area);
    f.render_widget(search_block, search_area);
    f.render_widget(Paragraph::new(search_content), search_inner);

    if app.add_input.is_some() {
        let add_text = app.add_input.as_ref().unwrap();
        let cursor_x = search_inner.x + add_text.len() as u16;
        f.set_cursor_position((cursor_x, search_inner.y));
    } else if app.input_mode == InputMode::Search {
        let tag_label_len = app.tag_filter.as_ref().map(|t| t.len() + 3).unwrap_or(0);
        let cursor_x = search_inner.x + tag_label_len as u16 + app.filter.len() as u16;
        f.set_cursor_position((cursor_x, search_inner.y));
    }

    let count_str = format!(" {}/{} ", app.filtered_indices.len(), app.entries.len());
    let mode_indicator = match app.input_mode {
        InputMode::Browse => Span::styled(
            " BROWSE ",
            Style::default()
                .fg(t.status_fg)
                .bg(t.normal_bg)
                .add_modifier(Modifier::BOLD),
        ),
        InputMode::Search => Span::styled(
            " SEARCH ",
            Style::default()
                .fg(t.status_fg)
                .bg(t.insert_bg)
                .add_modifier(Modifier::BOLD),
        ),
    };
    let mode_hint = match app.input_mode {
        InputMode::Search => " esc browse ",
        InputMode::Browse => " / search  c clear  q quit ",
    };
    let mut bottom_spans = vec![mode_indicator, Span::styled(count_str, s_muted)];
    if let Some(flash) = app.flash_message() {
        bottom_spans.push(Span::styled(format!(" {} ", flash), s_hl));
    } else {
        bottom_spans.push(Span::styled(mode_hint, s_muted));
    }
    let bottom_left = Line::from(bottom_spans);

    let sort_right = if app.sort_mode != SortMode::Added && app.filter.is_empty() {
        Line::from(Span::styled(
            format!(" sort: {} ", app.sort_mode.label()),
            s_hl,
        ))
    } else {
        Line::default()
    };

    let list_block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Line::from(Span::styled(" Bookmarks ", s_hl)))
        .title_bottom(bottom_left)
        .title_bottom(sort_right.alignment(ratatui::layout::Alignment::Right));

    let list_inner = list_block.inner(list_area);
    f.render_widget(list_block, list_area);

    app.list_height = list_inner.height as usize;
    let list_width = list_inner.width as usize;
    let prefix_width = 3 + 10 + 16; // marker + date + site
    let title_max = list_width.saturating_sub(prefix_width);

    if app.filtered_indices.is_empty() {
        let is_query_importable = is_importable(&app.filter);
        let msg = if is_query_importable {
            vec![
                Line::from(Span::styled("No bookmarks match this query.", s_dim)),
                Line::from(""),
                Line::from(Span::styled(
                    "This query looks like an importable URL!",
                    s_hl.add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Press ", s_dim),
                    Span::styled("Tab", s_hl.add_modifier(Modifier::BOLD)),
                    Span::styled(" or ", s_dim),
                    Span::styled("Ctrl-A", s_hl.add_modifier(Modifier::BOLD)),
                    Span::styled(" to load it into the Add bar.", s_dim),
                ]),
            ]
        } else {
            vec![Line::from(Span::styled(
                "No bookmarks match this query.",
                s_dim,
            ))]
        };

        let num_lines = msg.len();
        let paragraph = Paragraph::new(msg)
            .alignment(ratatui::layout::Alignment::Center)
            .wrap(Wrap { trim: true });

        // Center vertically inside list_inner
        let vertical_margin = (list_inner.height as usize).saturating_sub(num_lines) / 2;
        let hint_area = Layout::vertical([
            Constraint::Length(vertical_margin as u16),
            Constraint::Min(num_lines as u16),
        ])
        .split(list_inner)[1];

        f.render_widget(paragraph, hint_area);
    } else {
        let items: Vec<ListItem> = app
            .filtered_indices
            .iter()
            .map(|&idx| {
                let b = &app.entries[idx].bookmark;

                let date_str = b
                    .added
                    .as_deref()
                    .map(|d| format!(" {} ", truncate_str(d, 10)))
                    .unwrap_or_else(|| "            ".to_string());

                let site_str = b
                    .site
                    .as_deref()
                    .map(|s| format!("{:>14}  ", truncate_str(s, 14)))
                    .unwrap_or_else(|| "                ".to_string());

                let title = truncate_ellipsis(&b.title, title_max);

                ListItem::new(Line::from(vec![
                    Span::styled(date_str, s_date),
                    Span::styled(site_str, s_author),
                    Span::styled(title, s_text),
                ]))
            })
            .collect();

        let list = List::new(items)
            .highlight_style(
                Style::default()
                    .bg(t.selection)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol(" > ");

        f.render_stateful_widget(list, list_inner, &mut app.list_state);
    }

    if let Some(pane_area) = preview_area {
        let preview_title = app
            .selected_entry()
            .map(|e| Line::from(Span::styled(format!(" {} ", e.dir_name), s_hl)))
            .unwrap_or_default();

        let preview_block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(preview_title)
            .padding(ratatui::widgets::Padding::horizontal(1));

        let content_area = preview_block.inner(pane_area);
        f.render_widget(preview_block, pane_area);

        let styles = Styles {
            text: s_text,
            dim: s_dim,
            muted: s_muted,
            author: s_author,
            highlight: s_hl,
            link: s_link,
            date: s_date,
        };
        draw_preview(f, app, content_area, &styles);
    } else {
        app.preview_image_path = None;
        app.preview_image_area = None;
    }

    if let Some(ref mut popup) = app.tag_popup {
        let area = f.area();
        let max_visible = 20.min(popup.filtered_tags.len());
        let height = max_visible as u16 + 4;
        let width = 36.min(area.width.saturating_sub(4));
        let x = area.width.saturating_sub(width) / 2;
        let y = area.height.saturating_sub(height) / 2;
        let popup_area = ratatui::layout::Rect::new(x, y, width, height);

        f.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(t.popup_bg))
            .border_style(Style::default().fg(t.popup_border))
            .title(" Tags ")
            .title_style(s_author.add_modifier(Modifier::BOLD));
        let inner = block.inner(popup_area);
        f.render_widget(block, popup_area);

        let popup_chunks = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);

        let filter_line = if popup.filter.is_empty() {
            Line::from(Span::styled(" type to filter...", s_muted))
        } else {
            Line::from(vec![
                Span::styled(" > ", s_author),
                Span::styled(&popup.filter, s_text),
            ])
        };
        f.render_widget(Paragraph::new(filter_line), popup_chunks[0]);

        popup.clamp_scroll(max_visible);

        let inner_width = popup_chunks[1].width as usize;
        let lines: Vec<Line> = popup
            .filtered_tags
            .iter()
            .enumerate()
            .skip(popup.scroll)
            .take(max_visible)
            .map(|(i, tag)| {
                let is_selected = i == popup.selected;
                let prefix = if is_selected { " > " } else { "   " };
                let style = if is_selected {
                    Style::default()
                        .fg(t.text)
                        .bg(t.selection)
                        .add_modifier(Modifier::BOLD)
                } else {
                    s_dim
                };
                let count = popup.count_for(tag);
                let count_str = format!("{} ", count);
                let label = format!("{}{}", prefix, tag);
                let pad = inner_width.saturating_sub(label.len() + count_str.len());
                let count_style = if is_selected {
                    Style::default().fg(t.text_dim).bg(t.selection)
                } else {
                    s_muted
                };
                Line::from(vec![
                    Span::styled(label, style),
                    Span::styled(" ".repeat(pad), style),
                    Span::styled(count_str, count_style),
                ])
            })
            .collect();
        f.render_widget(Paragraph::new(lines), popup_chunks[1]);

        let hint = Line::from(Span::styled(" enter select  esc cancel", s_muted));
        f.render_widget(Paragraph::new(hint), popup_chunks[2]);
    }

    if let Some(ref popup) = app.theme_popup {
        let area = f.area();
        let max_visible = 12.min(popup.names.len());
        let height = max_visible as u16 + 3;
        let width = 30.min(area.width.saturating_sub(4));
        let x = area.width.saturating_sub(width) / 2;
        let y = area.height.saturating_sub(height) / 2;
        let popup_area = ratatui::layout::Rect::new(x, y, width, height);

        f.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(t.popup_bg))
            .border_style(Style::default().fg(t.popup_border))
            .title(" Theme ")
            .title_style(s_author.add_modifier(Modifier::BOLD));
        let inner = block.inner(popup_area);
        f.render_widget(block, popup_area);

        let popup_chunks =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);

        let scroll = popup.selected.saturating_sub(max_visible - 1);

        let lines: Vec<Line> = popup
            .names
            .iter()
            .enumerate()
            .skip(scroll)
            .take(max_visible)
            .map(|(i, name)| {
                let is_selected = i == popup.selected;
                let prefix = if is_selected { " > " } else { "   " };
                let style = if is_selected {
                    Style::default()
                        .fg(t.text)
                        .bg(t.selection)
                        .add_modifier(Modifier::BOLD)
                } else {
                    s_dim
                };
                Line::from(Span::styled(format!("{}{}", prefix, name), style))
            })
            .collect();
        f.render_widget(Paragraph::new(lines), popup_chunks[0]);

        let hint = Line::from(Span::styled(
            " j/k preview  enter select  esc cancel",
            s_muted,
        ));
        f.render_widget(Paragraph::new(hint), popup_chunks[1]);
    }

    if let Some(ref ep) = app.enrich_preview {
        let title_text = truncate_ellipsis(&app.entries[ep.idx].bookmark.title, 40);
        let batch_info = if !ep.batch_queue.is_empty() || ep.applied > 0 || ep.skipped > 0 {
            let remaining = ep.batch_queue.len() + 1;
            let total = ep.applied + ep.skipped + remaining;
            format!(" [{}/{}] ", ep.applied + ep.skipped + 1, total)
        } else {
            String::new()
        };
        let header = format!(" Enrich{}: {} ", batch_info, title_text);

        let mut lines: Vec<Line> = Vec::new();
        for (field, old_val, new_val) in &ep.diffs {
            lines.push(Line::from(Span::styled(
                format!(" {}:", field),
                s_author.add_modifier(Modifier::BOLD),
            )));
            if !old_val.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("  - ", Style::default().fg(t.highlight)),
                    Span::styled(old_val.as_str(), s_dim),
                ]));
            }
            lines.push(Line::from(vec![
                Span::styled("  + ", Style::default().fg(t.insert_bg)),
                Span::styled(new_val.as_str(), s_text),
            ]));
        }

        let content_height = lines.len() as u16;
        let height = (content_height + 5).min(area.height.saturating_sub(4));
        let width = 70.min(area.width.saturating_sub(4));
        let x = area.width.saturating_sub(width) / 2;
        let y = area.height.saturating_sub(height) / 2;
        let popup_area = ratatui::layout::Rect::new(x, y, width, height);

        f.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(t.popup_bg))
            .border_style(Style::default().fg(t.popup_border))
            .title(header)
            .title_style(s_author.add_modifier(Modifier::BOLD));
        let inner = block.inner(popup_area);
        f.render_widget(block, popup_area);

        let popup_chunks =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);

        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((ep.scroll, 0)),
            popup_chunks[0],
        );

        let hint = Line::from(vec![
            Span::styled(" enter/y", s_author),
            Span::styled("=apply  ", s_dim),
            Span::styled("n/s", s_author),
            Span::styled("=skip  ", s_dim),
            Span::styled("esc", s_author),
            Span::styled("=cancel", s_dim),
        ]);
        f.render_widget(Paragraph::new(hint), popup_chunks[1]);
    }

    if let Some(ref vp) = app.validate_popup {
        let line_count = vp.issues.len() as u16 + 3;
        let height = (line_count + 4).min(area.height.saturating_sub(4));
        let width = 60.min(area.width.saturating_sub(4));
        let x = area.width.saturating_sub(width) / 2;
        let y = area.height.saturating_sub(height) / 2;
        let popup_area = ratatui::layout::Rect::new(x, y, width, height);

        f.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(t.popup_bg))
            .border_style(Style::default().fg(t.popup_border))
            .title(" Validate ")
            .title_style(s_author.add_modifier(Modifier::BOLD));
        let inner = block.inner(popup_area);
        f.render_widget(block, popup_area);

        let mut lines: Vec<Line> = Vec::new();
        lines.push(Line::from(Span::styled(format!(" {}", vp.summary), s_text)));
        lines.push(Line::from(""));

        for issue in &vp.issues {
            lines.push(Line::from(Span::styled(format!(" {}", issue), s_dim)));
        }

        f.render_widget(Paragraph::new(lines).scroll((vp.scroll, 0)), inner);
    }

    if let Some(ref rv) = app.reader_view {
        let area = f.area();
        f.render_widget(Clear, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(t.popup_bg))
            .border_style(Style::default().fg(t.popup_border))
            .title(Line::from(Span::styled(
                format!(" {} ", rv.title),
                s_hl.add_modifier(Modifier::BOLD),
            )))
            .title_bottom(
                Line::from(vec![
                    Span::styled(" j/k", s_author),
                    Span::styled(" scroll  ", s_dim),
                    Span::styled("space", s_author),
                    Span::styled(" page down  ", s_dim),
                    Span::styled("g", s_author),
                    Span::styled(" top  ", s_dim),
                    Span::styled("q/esc", s_author),
                    Span::styled(" close ", s_dim),
                ])
                .alignment(ratatui::layout::Alignment::Right),
            );
        let inner = block.inner(area);
        f.render_widget(block, area);
        f.render_widget(
            Paragraph::new(rv.text.as_str())
                .style(s_text)
                .wrap(Wrap { trim: false })
                .scroll((rv.scroll, 0)),
            inner,
        );
    }

    if app.show_help {
        let help_lines = vec![
            ("", "Browse mode"),
            ("j / k", "Move down / up"),
            ("g / G", "Jump to top / bottom"),
            ("^d / ^u", "Half-page down / up"),
            ("^f / ^b", "Page down / up"),
            ("J / K", "Scroll preview down / up"),
            ("/ or i", "Enter search mode"),
            ("enter", "Open URL in browser"),
            ("e", "Edit info.toml"),
            ("y", "Copy URL"),
            ("Y", "Copy as markdown link"),
            ("space", "Open reader (article text)"),
            ("a", "Add bookmark (URL)"),
            ("r", "Refetch metadata for selected"),
            ("R", "Refetch metadata for all w/ missing fields"),
            ("s", "Cycle sort (added/site/title/year)"),
            ("d", "Deduplicate library"),
            ("D", "Delete selected bookmark"),
            ("I", "Reindex library"),
            ("V", "Validate library (auto-fix)"),
            ("c", "Clear search and tag filter"),
            ("t", "Browse tags"),
            ("T", "Switch theme"),
            ("q / esc", "Quit"),
            ("", ""),
            ("", "Search mode"),
            ("esc", "Return to browse mode"),
            ("^p / ^n", "Move up / down"),
            ("^d / ^u", "Half-page down / up"),
            ("^f / ^b", "Page down / up"),
            ("enter", "Confirm search"),
            ("tab", "Browse tags"),
        ];

        let height = help_lines.len() as u16 + 4;
        let width = 56.min(area.width.saturating_sub(4));
        let x = area.width.saturating_sub(width) / 2;
        let y = area.height.saturating_sub(height) / 2;
        let popup_area = ratatui::layout::Rect::new(x, y, width, height);

        f.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(t.popup_bg))
            .border_style(Style::default().fg(t.popup_border))
            .title(" Help ")
            .title_style(s_author.add_modifier(Modifier::BOLD));
        let inner = block.inner(popup_area);
        f.render_widget(block, popup_area);

        let popup_chunks =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);

        let key_width = 10;
        let lines: Vec<Line> = help_lines
            .iter()
            .map(|(key, desc)| {
                if key.is_empty() {
                    Line::from(Span::styled(
                        format!(" {}", desc),
                        s_author.add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(vec![
                        Span::styled(format!(" {:>width$}  ", key, width = key_width), s_author),
                        Span::styled(*desc, s_dim),
                    ])
                }
            })
            .collect();
        f.render_widget(Paragraph::new(lines), popup_chunks[0]);

        let hint = Line::from(Span::styled(" press any key to close", s_muted));
        f.render_widget(Paragraph::new(hint), popup_chunks[1]);
    }

    if let Some(ref confirm) = app.delete_confirm {
        let title_line = truncate_ellipsis(&confirm.title, 60);
        let width = (title_line.len() as u16 + 6)
            .max(40)
            .min(area.width.saturating_sub(4));
        let height: u16 = 6;
        let x = area.width.saturating_sub(width) / 2;
        let y = area.height.saturating_sub(height) / 2;
        let popup_area = ratatui::layout::Rect::new(x, y, width, height);

        f.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(t.popup_bg))
            .border_style(Style::default().fg(t.popup_border))
            .title(" Delete bookmark? ")
            .title_style(s_author.add_modifier(Modifier::BOLD));
        let inner = block.inner(popup_area);
        f.render_widget(block, popup_area);

        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(format!(" {}", title_line), s_text)),
            Line::from(""),
            Line::from(Span::styled(" [y] delete   [n/esc] cancel", s_muted)),
        ];
        f.render_widget(Paragraph::new(lines), inner);
    }
}

struct Styles {
    text: Style,
    dim: Style,
    muted: Style,
    author: Style,
    highlight: Style,
    link: Style,
    date: Style,
}

fn draw_preview(f: &mut Frame, app: &mut App, area: ratatui::layout::Rect, s: &Styles) {
    let s_text = s.text;
    let s_dim = s.dim;
    let s_muted = s.muted;
    let s_author = s.author;
    let s_hl = s.highlight;
    let s_link = s.link;
    let s_date = s.date;

    app.preview_image_path = None;
    app.preview_image_area = None;

    let text_area = if app.config.images_enabled() && area.height >= 14 {
        if let Some(entry) = app.selected_entry()
            && let Some(ref preview) = entry.bookmark.preview
        {
            let path = entry.dir.join(preview);
            if path.exists() {
                let img_height = (area.height * 4 / 10).clamp(8, 16);
                let img_area = ratatui::layout::Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: img_height,
                };
                let text_area = ratatui::layout::Rect {
                    x: area.x,
                    y: area.y + img_height,
                    width: area.width,
                    height: area.height - img_height,
                };
                app.preview_image_path = Some(path);
                app.preview_image_area = Some(img_area);
                text_area
            } else {
                area
            }
        } else {
            area
        }
    } else {
        area
    };

    if let Some(entry) = app.selected_entry() {
        let b = &entry.bookmark;
        let mut lines: Vec<Line> = Vec::new();

        lines.push(Line::from(""));

        lines.push(Line::from(Span::styled(
            &b.title,
            s_text.add_modifier(Modifier::BOLD),
        )));

        if b.site.is_some() || b.year.is_some() {
            let mut parts: Vec<Span> = Vec::new();
            if let Some(ref site) = b.site {
                parts.push(Span::styled(site.as_str(), s_author));
            }
            if let Some(year) = b.year {
                if b.site.is_some() {
                    parts.push(Span::styled(" · ", s_muted));
                }
                parts.push(Span::styled(year.to_string(), s_date));
            }
            lines.push(Line::from(parts));
        }

        lines.push(Line::from(""));

        lines.push(Line::from(Span::styled(b.url.as_str(), s_link)));

        if !b.authors.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(b.authors.join(" · "), s_author)));
        }

        if let Some(ref added) = b.added {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("added ", s_muted),
                Span::styled(added.as_str(), s_date),
            ]));
        }

        if !b.tags.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("tags  ", s_muted),
                Span::styled(b.tags.join(", "), s_hl),
            ]));
        }

        if let Some(ref desc) = b.description {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(desc.as_str(), s_dim)));
        }

        f.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .scroll((app.preview_scroll, 0)),
            text_area,
        );
    } else {
        f.render_widget(
            Paragraph::new(Span::styled("No selection", s_muted)),
            text_area,
        );
    }
}

fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..s.floor_char_boundary(max)]
    }
}

fn truncate_ellipsis(s: &str, max: usize) -> String {
    if max < 2 || s.len() <= max {
        s.to_string()
    } else {
        let end = s.floor_char_boundary(max - 1);
        format!("{}…", &s[..end])
    }
}

// --- Dedup TUI ---

fn run_dedup(terminal: &mut Term, app: &mut App) -> Result<()> {
    let library = app.config.library_dir();
    let groups = find_duplicate_groups(&library)?;
    if groups.is_empty() {
        app.flash = Some(("No duplicates found".to_string(), std::time::Instant::now()));
        return Ok(());
    }

    let trash_dir = library.join(".trash");
    let mut removed = 0usize;
    let total_groups = groups.len();

    for (group_idx, group) in groups.iter().enumerate() {
        let mut selected: usize = 0;
        let entries: Vec<DedupEntry> = group.iter().map(|p| DedupEntry::from_path(p)).collect();

        if let Some((best, _)) = entries.iter().enumerate().max_by_key(|(_, e)| e.score) {
            selected = best;
        }

        loop {
            let theme = &app.theme;
            terminal.draw(|f| {
                draw_dedup(f, theme, &entries, selected, group_idx, total_groups);
            })?;

            if let Event::Key(key) = event::read()? {
                match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _)
                    | (KeyCode::Esc, _)
                    | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        let msg = if removed > 0 {
                            format!("Dedup: removed {}", removed)
                        } else {
                            "Dedup cancelled".to_string()
                        };
                        app.flash = Some((msg, std::time::Instant::now()));
                        terminal.clear()?;
                        return Ok(());
                    }
                    (KeyCode::Char('s'), _) => break,
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                        selected = selected.saturating_sub(1);
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                        selected = (selected + 1).min(entries.len() - 1);
                    }
                    (KeyCode::Enter, _) => {
                        std::fs::create_dir_all(&trash_dir)?;
                        for (i, entry) in entries.iter().enumerate() {
                            if i != selected {
                                let dest = trash_dir.join(&entry.dir_name);
                                std::fs::rename(&entry.path, &dest)?;
                                removed += 1;
                            }
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    let msg = if removed > 0 {
        format!("Dedup: removed {} (run reindex)", removed)
    } else {
        "Dedup: no changes".to_string()
    };
    app.flash = Some((msg, std::time::Instant::now()));
    app.reload_entries();
    terminal.clear()?;
    Ok(())
}

struct DedupEntry {
    path: PathBuf,
    dir_name: String,
    bookmark: Bookmark,
    score: u32,
}

impl DedupEntry {
    fn from_path(path: &Path) -> Self {
        let dir_name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let bookmark = metadata::read_info(path).unwrap_or_else(|_| Bookmark {
            url: String::new(),
            title: "Unknown".to_string(),
            description: None,
            authors: vec![],
            site: None,
            year: None,
            tags: vec![],
            added: None,
            files: vec![],
            preview: None,
        });
        let score = metadata_score(&bookmark);
        Self {
            path: path.to_path_buf(),
            dir_name,
            bookmark,
            score,
        }
    }
}

fn metadata_score(b: &Bookmark) -> u32 {
    let mut score = 0u32;
    if !b.title.is_empty() {
        score += 1;
    }
    if !b.url.is_empty() {
        score += 1;
    }
    if b.description.is_some() {
        score += 1;
    }
    if !b.authors.is_empty() {
        score += 1;
    }
    if b.site.is_some() {
        score += 1;
    }
    if b.year.is_some() {
        score += 1;
    }
    if !b.tags.is_empty() {
        score += 1;
    }
    if b.added.is_some() {
        score += 1;
    }
    score
}

fn find_duplicate_groups(library: &Path) -> Result<Vec<Vec<PathBuf>>> {
    use std::collections::{HashMap, HashSet};

    let dirs = storage::list_bookmark_dirs(library)?;
    let mut by_url: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let mut by_title: HashMap<String, Vec<PathBuf>> = HashMap::new();

    for dir in &dirs {
        let b = match metadata::read_info(dir) {
            Ok(b) => b,
            Err(_) => continue,
        };

        let canonical_url = canonicalize_url(&b.url);
        if !canonical_url.is_empty() {
            by_url.entry(canonical_url).or_default().push(dir.clone());
        }

        let normalized_title = b.title.trim().to_lowercase();
        if !normalized_title.is_empty() {
            by_title
                .entry(normalized_title)
                .or_default()
                .push(dir.clone());
        }
    }

    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut groups: Vec<Vec<PathBuf>> = Vec::new();

    for paths in by_url.values() {
        if paths.len() > 1 {
            let group: Vec<_> = paths
                .iter()
                .filter(|p| !seen.contains(*p))
                .cloned()
                .collect();
            if group.len() > 1 {
                for p in &group {
                    seen.insert(p.clone());
                }
                groups.push(group);
            }
        }
    }

    for paths in by_title.values() {
        if paths.len() > 1 {
            let group: Vec<_> = paths
                .iter()
                .filter(|p| !seen.contains(*p))
                .cloned()
                .collect();
            if group.len() > 1 {
                for p in &group {
                    seen.insert(p.clone());
                }
                groups.push(group);
            }
        }
    }

    Ok(groups)
}

fn canonicalize_url(url: &str) -> String {
    let s = url.trim().to_lowercase();
    let s = s.trim_end_matches('/');
    let s = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    let s = s.strip_prefix("www.").unwrap_or(s);
    s.to_string()
}

fn draw_dedup(
    f: &mut Frame,
    theme: &Theme,
    entries: &[DedupEntry],
    selected: usize,
    group_idx: usize,
    total_groups: usize,
) {
    let t = theme;
    let s_text = Style::default().fg(t.text);
    let s_dim = Style::default().fg(t.text_dim);
    let s_muted = Style::default().fg(t.text_muted);
    let s_author = Style::default().fg(t.author);
    let s_hl = Style::default().fg(t.highlight);
    let s_link = Style::default().fg(t.link);

    let area = f.area();

    let chunks =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);

    let left = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(chunks[0]);

    let title = &entries[0].bookmark.title;
    let header = Line::from(vec![
        Span::styled(format!(" [{}/{}] ", group_idx + 1, total_groups), s_dim),
        Span::styled(
            truncate_ellipsis(title, left[0].width.saturating_sub(12) as usize),
            s_text,
        ),
    ]);
    f.render_widget(Paragraph::new(header), left[0]);

    let items: Vec<ListItem> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let marker = if i == selected { "> " } else { "  " };
            let label = format!("{}{} ({}/8)", marker, e.dir_name, e.score);
            let style = if i == selected {
                s_author.add_modifier(Modifier::BOLD)
            } else {
                s_dim
            };
            ListItem::new(Span::styled(label, style))
        })
        .collect();

    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::TOP).border_style(s_muted)),
        left[1],
    );

    let footer = Line::from(vec![
        Span::styled(" enter", s_author),
        Span::styled("=keep  ", s_dim),
        Span::styled("s", s_author),
        Span::styled("=skip  ", s_dim),
        Span::styled("q", s_author),
        Span::styled("=quit", s_dim),
    ]);
    f.render_widget(Paragraph::new(footer), left[2]);

    let entry = &entries[selected];
    let b = &entry.bookmark;
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(Span::styled(
        &b.title,
        s_text.add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::default());

    if !b.url.is_empty() {
        lines.push(Line::from(Span::styled(b.url.as_str(), s_link)));
    }
    if let Some(ref site) = b.site {
        lines.push(Line::from(Span::styled(site.as_str(), s_author)));
    }
    if let Some(year) = b.year {
        lines.push(Line::from(Span::styled(format!("{}", year), s_dim)));
    }
    if !b.authors.is_empty() {
        lines.push(Line::from(Span::styled(b.authors.join(" · "), s_author)));
    }
    if let Some(ref added) = b.added {
        lines.push(Line::from(vec![
            Span::styled("added: ", s_muted),
            Span::styled(added.as_str(), s_dim),
        ]));
    }
    if !b.tags.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("tags: ", s_muted),
            Span::styled(b.tags.join(", "), s_hl),
        ]));
    }

    if let Some(ref desc) = b.description {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(desc.as_str(), s_dim)));
    }

    f.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::LEFT)
                    .border_style(s_muted),
            )
            .wrap(Wrap { trim: false }),
        chunks[1],
    );
}
