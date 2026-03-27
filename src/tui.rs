use crate::config;
use crate::editor;
use crate::notes::{self, Note};
use crate::theme::{self, Theme};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::DefaultTerminal;
use std::collections::HashMap;
use std::io::stdout;
use std::sync::mpsc;
use std::thread;

enum Mode {
    Browse,
    Search,
    Help,
    ThemePicker,
    FolderPicker,
    FolderInput(FolderInputKind),
    MovePicker,
}

enum FolderInputKind {
    New,
    Rename(String),
}


struct App {
    notes: Vec<Note>,
    filtered: Vec<usize>,
    list_state: ListState,
    search_query: String,
    mode: Mode,
    should_quit: bool,
    confirm_delete: bool,
    theme: Theme,
    theme_selected: usize,
    theme_before_preview: Theme,
    folders: Vec<String>,
    folder_selected: usize,
    active_folder: Option<String>,
    move_selected: usize,
    input_buf: String,
    confirm_folder_delete: bool,
    body_cache: HashMap<String, String>,
    body_rx: mpsc::Receiver<Vec<(String, String)>>,
}

impl App {
    fn new(notes: Vec<Note>, theme: Theme, body_rx: mpsc::Receiver<Vec<(String, String)>>) -> Self {
        let filtered: Vec<usize> = (0..notes.len()).collect();
        let mut list_state = ListState::default();
        if !filtered.is_empty() {
            list_state.select(Some(0));
        }
        let theme_selected = theme::ALL_THEMES
            .iter()
            .position(|(_, t)| t.accent == theme.accent && t.border == theme.border)
            .unwrap_or(0);
        Self {
            notes,
            filtered,
            list_state,
            search_query: String::new(),
            mode: Mode::Browse,
            should_quit: false,
            confirm_delete: false,
            theme,
            theme_selected,
            theme_before_preview: theme,
            folders: vec![],
            folder_selected: 0,
            active_folder: None,
            move_selected: 0,
            input_buf: String::new(),
            confirm_folder_delete: false,
            body_cache: HashMap::new(),
            body_rx,
        }
    }

    fn apply_filter(&mut self) {
        if self.search_query.is_empty() {
            self.filtered = self
                .notes
                .iter()
                .enumerate()
                .filter(|(_, n)| {
                    self.active_folder
                        .as_ref()
                        .is_none_or(|f| n.folder == *f)
                })
                .map(|(i, _)| i)
                .collect();
        } else {
            let pattern =
                Pattern::new(&self.search_query, CaseMatching::Ignore, Normalization::Smart, AtomKind::Fuzzy);
            let mut matcher = nucleo_matcher::Matcher::new(nucleo_matcher::Config::DEFAULT);
            let mut buf = Vec::new();

            let mut scored: Vec<(usize, u32)> = self
                .notes
                .iter()
                .enumerate()
                .filter(|(_, n)| {
                    self.active_folder
                        .as_ref()
                        .is_none_or(|f| n.folder == *f)
                })
                .filter_map(|(i, n)| {
                    let body = self.body_cache.get(&n.name).map(|s| s.as_str()).unwrap_or("");
                    let haystack = format!("{}/{} {}", n.folder, n.name, body);
                    let score = pattern.score(
                        nucleo_matcher::Utf32Str::new(&haystack, &mut buf),
                        &mut matcher,
                    )?;
                    Some((i, score))
                })
                .collect();

            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered = scored.into_iter().map(|(i, _)| i).collect();
        }

        if self.filtered.is_empty() {
            self.list_state.select(None);
        } else {
            self.list_state.select(Some(0));
        }
    }

    fn selected_note(&self) -> Option<&Note> {
        self.list_state
            .selected()
            .and_then(|i| self.filtered.get(i))
            .map(|&idx| &self.notes[idx])
    }

    fn move_up(&mut self) {
        if let Some(selected) = self.list_state.selected() {
            if selected > 0 {
                self.list_state.select(Some(selected - 1));
            }
        }
    }

    fn move_down(&mut self) {
        if let Some(selected) = self.list_state.selected() {
            if selected + 1 < self.filtered.len() {
                self.list_state.select(Some(selected + 1));
            }
        }
    }

    fn move_top(&mut self) {
        if !self.filtered.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    fn move_bottom(&mut self) {
        if !self.filtered.is_empty() {
            self.list_state.select(Some(self.filtered.len() - 1));
        }
    }

    fn move_page_up(&mut self, page_size: usize) {
        if let Some(selected) = self.list_state.selected() {
            self.list_state
                .select(Some(selected.saturating_sub(page_size)));
        }
    }

    fn move_page_down(&mut self, page_size: usize) {
        if let Some(selected) = self.list_state.selected() {
            let last = self.filtered.len().saturating_sub(1);
            self.list_state
                .select(Some((selected + page_size).min(last)));
        }
    }

    fn drain_bodies(&mut self) {
        if let Ok(bodies) = self.body_rx.try_recv() {
            for (name, body) in bodies {
                self.body_cache.insert(name, body);
            }
        }
    }

    fn refresh(&mut self) {
        if let Ok(refreshed) = notes::list_notes(None) {
            self.notes = refreshed;
            self.body_cache.clear();
            self.apply_filter();
        }
        // Re-spawn background body loader
        let (tx, rx) = mpsc::channel();
        self.body_rx = rx;
        thread::spawn(move || {
            if let Ok(bodies) = notes::fetch_all_bodies() {
                let _ = tx.send(bodies);
            }
        });
    }

    fn load_folders(&mut self) {
        if let Ok(folders) = notes::list_folders() {
            self.folders = folders;
        }
    }
}

pub fn run(theme: Theme) -> Result<()> {
    let notes = notes::list_notes(None)?;
    if notes.is_empty() {
        println!("No notes found.");
        return Ok(());
    }

    // Fetch bodies in background
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Ok(bodies) = notes::fetch_all_bodies() {
            let _ = tx.send(bodies);
        }
    });

    let mut app = App::new(notes, theme, rx);

    terminal::enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = ratatui::init();

    let result = run_loop(&mut terminal, &mut app);

    ratatui::restore();
    execute!(stdout(), LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;

    result
}

fn run_loop(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    loop {
        // Check for bodies from background loader
        app.drain_bodies();

        terminal.draw(|frame| draw(frame, app))?;

        // Poll with timeout so we re-draw when background bodies arrive
        if !event::poll(std::time::Duration::from_millis(100))? {
            continue;
        }
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match app.mode {
                Mode::Browse => match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                    KeyCode::Char('j') | KeyCode::Down => app.move_down(),
                    KeyCode::Char('k') | KeyCode::Up => app.move_up(),
                    KeyCode::Char('G') => app.move_bottom(),
                    KeyCode::Home => app.move_top(),
                    KeyCode::End => app.move_bottom(),
                    KeyCode::PageUp => app.move_page_up(10),
                    KeyCode::PageDown => app.move_page_down(10),
                    KeyCode::Char('/') => {
                        app.mode = Mode::Search;
                    }
                    KeyCode::Char('?') => {
                        app.mode = Mode::Help;
                    }
                    KeyCode::Char('t') => {
                        app.theme_before_preview = app.theme;
                        app.mode = Mode::ThemePicker;
                    }
                    KeyCode::Char('m') => {
                        if app.selected_note().is_some() {
                            app.load_folders();
                            app.move_selected = 0;
                            app.mode = Mode::MovePicker;
                        }
                    }
                    KeyCode::Char('g') => app.move_top(),
                    KeyCode::Char('f') => {
                        app.load_folders();
                        app.folder_selected = match &app.active_folder {
                            Some(name) => app
                                .folders
                                .iter()
                                .position(|f| f == name)
                                .map(|i| i + 1)
                                .unwrap_or(0),
                            None => 0,
                        };
                        app.mode = Mode::FolderPicker;
                    }
                    KeyCode::Char('r') => {
                        app.refresh();
                    }
                    KeyCode::Char('d') => {
                        if app.selected_note().is_some() {
                            app.confirm_delete = true;
                        }
                    }
                    KeyCode::Char('y') if app.confirm_delete => {
                        if let Some(note) = app.selected_note() {
                            let name = note.name.clone();
                            let _ = notes::delete_note(&name);
                            app.refresh();
                        }
                        app.confirm_delete = false;
                    }
                    KeyCode::Char('n') if app.confirm_delete => {
                        app.confirm_delete = false;
                    }
                    KeyCode::Char('n') => {
                        ratatui::restore();
                        execute!(stdout(), LeaveAlternateScreen)?;
                        terminal::disable_raw_mode()?;

                        if let Ok(edited) = editor::edit("", "tome_new.md") {
                            let edited = edited.trim().to_string();
                            if !edited.is_empty() {
                                let (title, body) = edited
                                    .split_once('\n')
                                    .map(|(t, b)| (t.trim().to_string(), b.trim_start().to_string()))
                                    .unwrap_or((edited, String::new()));
                                notes::create_note(&title, &body, app.active_folder.as_deref())?;
                            }
                        }

                        terminal::enable_raw_mode()?;
                        execute!(stdout(), EnterAlternateScreen)?;
                        *terminal = ratatui::init();
                        app.refresh();
                    }
                    KeyCode::Enter => {
                        if let Some(note) = app.selected_note() {
                            let name = note.name.clone();
                            ratatui::restore();
                            execute!(stdout(), LeaveAlternateScreen)?;
                            terminal::disable_raw_mode()?;

                            if let Ok(full_note) = notes::get_note(&name) {
                                let content = format!("{}\n\n{}", full_note.name, full_note.body);
                                if let Ok(edited) =
                                    editor::edit(&content, &format!("{}.md", name))
                                {
                                    if edited != content {
                                        let (new_title, new_body) = edited.split_once('\n')
                                            .map(|(t, b)| (t.trim().to_string(), b.trim_start().to_string()))
                                            .unwrap_or((edited, String::new()));
                                        notes::update_note(&full_note.name, &new_title, &new_body)?;
                                    }
                                }
                            }

                            terminal::enable_raw_mode()?;
                            execute!(stdout(), EnterAlternateScreen)?;
                            *terminal = ratatui::init();
                            app.refresh();
                        }
                    }
                    _ => {
                        if app.confirm_delete {
                            app.confirm_delete = false;
                        }
                    }
                },
                Mode::Search => match key.code {
                    KeyCode::Esc => {
                        app.mode = Mode::Browse;
                        app.search_query.clear();
                        app.apply_filter();
                    }
                    KeyCode::Enter => {
                        app.mode = Mode::Browse;
                    }
                    KeyCode::Backspace => {
                        app.search_query.pop();
                        app.apply_filter();
                    }
                    KeyCode::Char(c) => {
                        app.search_query.push(c);
                        app.apply_filter();
                    }
                    _ => {}
                },
                Mode::Help => match key.code {
                    KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q') => {
                        app.mode = Mode::Browse;
                    }
                    _ => {}
                },
                Mode::ThemePicker => match key.code {
                    KeyCode::Esc | KeyCode::Char('t') => {
                        app.theme = app.theme_before_preview;
                        app.mode = Mode::Browse;
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        if app.theme_selected + 1 < theme::ALL_THEMES.len() {
                            app.theme_selected += 1;
                            app.theme = theme::ALL_THEMES[app.theme_selected].1;
                        }
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        if app.theme_selected > 0 {
                            app.theme_selected -= 1;
                            app.theme = theme::ALL_THEMES[app.theme_selected].1;
                        }
                    }
                    KeyCode::Home => {
                        app.theme_selected = 0;
                        app.theme = theme::ALL_THEMES[0].1;
                    }
                    KeyCode::End | KeyCode::Char('G') => {
                        app.theme_selected = theme::ALL_THEMES.len() - 1;
                        app.theme = theme::ALL_THEMES[app.theme_selected].1;
                    }
                    KeyCode::PageUp => {
                        app.theme_selected = app.theme_selected.saturating_sub(10);
                        app.theme = theme::ALL_THEMES[app.theme_selected].1;
                    }
                    KeyCode::PageDown => {
                        app.theme_selected = (app.theme_selected + 10).min(theme::ALL_THEMES.len() - 1);
                        app.theme = theme::ALL_THEMES[app.theme_selected].1;
                    }
                    KeyCode::Enter => {
                        let (name, selected_theme) = theme::ALL_THEMES[app.theme_selected];
                        app.theme = selected_theme;
                        app.mode = Mode::Browse;
                        let mut cfg = config::load();
                        cfg.theme = Some(name.to_string());
                        let _ = config::save(&cfg);
                    }
                    _ => {}
                },
                Mode::FolderPicker => match key.code {
                    KeyCode::Esc | KeyCode::Char('f') => {
                        app.confirm_folder_delete = false;
                        app.mode = Mode::Browse;
                    }
                    KeyCode::Char('j') | KeyCode::Down if !app.confirm_folder_delete => {
                        let total = app.folders.len() + 1;
                        if app.folder_selected + 1 < total {
                            app.folder_selected += 1;
                        }
                    }
                    KeyCode::Char('k') | KeyCode::Up if !app.confirm_folder_delete => {
                        if app.folder_selected > 0 {
                            app.folder_selected -= 1;
                        }
                    }
                    KeyCode::Home if !app.confirm_folder_delete => {
                        app.folder_selected = 0;
                    }
                    KeyCode::End | KeyCode::Char('G') if !app.confirm_folder_delete => {
                        app.folder_selected = app.folders.len(); // last item (folders.len() = "All" + folders - 1)
                    }
                    KeyCode::PageUp if !app.confirm_folder_delete => {
                        app.folder_selected = app.folder_selected.saturating_sub(10);
                    }
                    KeyCode::PageDown if !app.confirm_folder_delete => {
                        let total = app.folders.len() + 1;
                        app.folder_selected = (app.folder_selected + 10).min(total - 1);
                    }
                    KeyCode::Enter if !app.confirm_folder_delete => {
                        if app.folder_selected == 0 {
                            app.active_folder = None;
                        } else {
                            app.active_folder =
                                Some(app.folders[app.folder_selected - 1].clone());
                        }
                        app.apply_filter();
                        app.mode = Mode::Browse;
                    }
                    KeyCode::Char('n') if !app.confirm_folder_delete => {
                        app.input_buf.clear();
                        app.mode = Mode::FolderInput(FolderInputKind::New);
                    }
                    KeyCode::Char('r') if !app.confirm_folder_delete => {
                        if app.folder_selected > 0 {
                            let old_name = app.folders[app.folder_selected - 1].clone();
                            app.input_buf = old_name.clone();
                            app.mode = Mode::FolderInput(FolderInputKind::Rename(old_name));
                        }
                    }
                    KeyCode::Char('d') if !app.confirm_folder_delete => {
                        if app.folder_selected > 0 {
                            app.confirm_folder_delete = true;
                        }
                    }
                    KeyCode::Char('y') if app.confirm_folder_delete => {
                        if app.folder_selected > 0 {
                            let name = app.folders[app.folder_selected - 1].clone();
                            let _ = notes::delete_folder(&name);
                            if app.active_folder.as_deref() == Some(&name) {
                                app.active_folder = None;
                            }
                            app.load_folders();
                            app.refresh();
                            app.folder_selected = 0;
                        }
                        app.confirm_folder_delete = false;
                    }
                    _ => {
                        app.confirm_folder_delete = false;
                    }
                },
                Mode::FolderInput(ref kind) => match key.code {
                    KeyCode::Esc => {
                        app.mode = Mode::FolderPicker;
                    }
                    KeyCode::Enter => {
                        let name = app.input_buf.trim().to_string();
                        if !name.is_empty() {
                            match kind {
                                FolderInputKind::New => {
                                    let _ = notes::create_folder(&name);
                                }
                                FolderInputKind::Rename(old) => {
                                    let _ = notes::rename_folder(old, &name);
                                    if app.active_folder.as_deref() == Some(old.as_str()) {
                                        app.active_folder = Some(name.clone());
                                    }
                                }
                            }
                            app.load_folders();
                            app.refresh();
                        }
                        app.folder_selected = 0;
                        app.mode = Mode::FolderPicker;
                    }
                    KeyCode::Backspace => {
                        app.input_buf.pop();
                    }
                    KeyCode::Char(c) => {
                        app.input_buf.push(c);
                    }
                    _ => {}
                },
                Mode::MovePicker => match key.code {
                    KeyCode::Esc | KeyCode::Char('m') => {
                        app.mode = Mode::Browse;
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        if app.move_selected + 1 < app.folders.len() {
                            app.move_selected += 1;
                        }
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        if app.move_selected > 0 {
                            app.move_selected -= 1;
                        }
                    }
                    KeyCode::Home => {
                        app.move_selected = 0;
                    }
                    KeyCode::End | KeyCode::Char('G') => {
                        if !app.folders.is_empty() {
                            app.move_selected = app.folders.len() - 1;
                        }
                    }
                    KeyCode::PageUp => {
                        app.move_selected = app.move_selected.saturating_sub(10);
                    }
                    KeyCode::PageDown => {
                        if !app.folders.is_empty() {
                            app.move_selected = (app.move_selected + 10).min(app.folders.len() - 1);
                        }
                    }
                    KeyCode::Enter => {
                        if let Some(note) = app.selected_note() {
                            let name = note.name.clone();
                            let target = app.folders[app.move_selected].clone();
                            let _ = notes::move_note(&name, &target);
                            app.refresh();
                        }
                        app.mode = Mode::Browse;
                    }
                    _ => {}
                },
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn draw(frame: &mut ratatui::Frame, app: &mut App) {
    let t = &app.theme;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    // Search bar
    let search_text = if app.search_query.is_empty() {
        match app.mode {
            Mode::Search => String::from("_"),
            _ => String::from("Press / to search"),
        }
    } else {
        match app.mode {
            Mode::Search => format!("{}_", app.search_query),
            _ => app.search_query.clone(),
        }
    };

    let search_style = match app.mode {
        Mode::Search => Style::default().fg(t.accent),
        _ => Style::default().fg(t.text_muted),
    };

    let search = Paragraph::new(search_text).style(search_style).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border))
            .title(" Search ")
            .title_style(Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
    );
    frame.render_widget(search, chunks[0]);

    // Split main area into list + preview
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(chunks[1]);

    // Note list
    let items: Vec<ListItem> = app
        .filtered
        .iter()
        .map(|&idx| {
            let note = &app.notes[idx];
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{}/", note.folder),
                    Style::default().fg(t.text_muted),
                ),
                Span::styled(&note.name, Style::default().fg(t.text)),
            ]))
        })
        .collect();

    let title = match &app.active_folder {
        Some(name) => format!(" Notes - {} ", name),
        None => " Notes ".to_string(),
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.border))
                .title(title)
                .title_style(Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
        )
        .highlight_style(
            Style::default()
                .fg(t.text_bright)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, main_chunks[0], &mut app.list_state);

    // Preview pane
    let (preview_title, preview_body) = match app.selected_note() {
        Some(note) => (
            format!(" {} ", note.name),
            app.body_cache.get(&note.name).cloned().unwrap_or_default(),
        ),
        None => (" Preview ".to_string(), String::new()),
    };

    let preview = Paragraph::new(preview_body)
        .style(Style::default().fg(t.text))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(t.border))
                .title(preview_title)
                .title_style(Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
        );

    frame.render_widget(preview, main_chunks[1]);

    // Status bar
    let status = if app.confirm_delete {
        Line::from(vec![
            Span::styled(" Delete note? ", Style::default().fg(t.error)),
            Span::styled("y", Style::default().fg(t.accent)),
            Span::styled(" yes  ", Style::default().fg(t.text)),
            Span::styled("n", Style::default().fg(t.accent)),
            Span::styled(" no", Style::default().fg(t.text)),
        ])
    } else {
        Line::from(vec![
            Span::styled(" ?", Style::default().fg(t.accent)),
            Span::styled(" help  ", Style::default().fg(t.text_dim)),
            Span::styled("q", Style::default().fg(t.accent)),
            Span::styled(" quit", Style::default().fg(t.text_dim)),
        ])
    };
    frame.render_widget(Paragraph::new(status), chunks[2]);

    // Overlays
    match app.mode {
        Mode::Help => draw_help(frame, t),
        Mode::ThemePicker => draw_theme_picker(frame, app),
        Mode::FolderPicker | Mode::FolderInput(_) => draw_folder_picker(frame, app),
        Mode::MovePicker => draw_move_picker(frame, app),
        _ => {}
    }
}

fn draw_help(frame: &mut ratatui::Frame, t: &Theme) {
    let area = frame.area();
    let width = 40u16.min(area.width.saturating_sub(4));
    let height = 22u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);

    let help_lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  ↑↓ / j k  ", Style::default().fg(t.accent)),
            Span::styled("Navigate notes", Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  g / Home   ", Style::default().fg(t.accent)),
            Span::styled("Jump to top", Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  G / End    ", Style::default().fg(t.accent)),
            Span::styled("Jump to bottom", Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  PgUp/PgDn  ", Style::default().fg(t.accent)),
            Span::styled("Scroll by page", Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  ⏎ Enter   ", Style::default().fg(t.accent)),
            Span::styled("Edit selected note", Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  n         ", Style::default().fg(t.accent)),
            Span::styled("New note", Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  r         ", Style::default().fg(t.accent)),
            Span::styled("Refresh from Notes.app", Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  d         ", Style::default().fg(t.accent)),
            Span::styled("Delete selected note", Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  m         ", Style::default().fg(t.accent)),
            Span::styled("Move to folder", Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  f         ", Style::default().fg(t.accent)),
            Span::styled("Folders (n/r/d to manage)", Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  t         ", Style::default().fg(t.accent)),
            Span::styled("Change theme", Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  /         ", Style::default().fg(t.accent)),
            Span::styled("Search notes", Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  Esc       ", Style::default().fg(t.accent)),
            Span::styled("Cancel / back", Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  ?         ", Style::default().fg(t.accent)),
            Span::styled("Toggle this help", Style::default().fg(t.text)),
        ]),
        Line::from(vec![
            Span::styled("  q         ", Style::default().fg(t.accent)),
            Span::styled("Quit", Style::default().fg(t.text)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "    Press ? or Esc to close",
            Style::default().fg(t.text_muted),
        )),
    ];

    let help = Paragraph::new(help_lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Help ")
            .title_style(Style::default().fg(t.accent).add_modifier(Modifier::BOLD))
            .border_style(Style::default().fg(t.accent)),
    );
    frame.render_widget(help, popup);
}

fn draw_theme_picker(frame: &mut ratatui::Frame, app: &App) {
    let t = &app.theme;
    let area = frame.area();
    let height = (theme::ALL_THEMES.len() as u16 + 4).min(area.height.saturating_sub(4));
    let width = 30u16.min(area.width.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);

    let items: Vec<ListItem> = theme::ALL_THEMES
        .iter()
        .enumerate()
        .map(|(i, (name, _))| {
            let style = if i == app.theme_selected {
                Style::default()
                    .fg(t.text_bright)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.text)
            };
            ListItem::new(Span::styled(format!("  {name}"), style))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.theme_selected));

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Theme ")
                .title_style(Style::default().fg(t.accent).add_modifier(Modifier::BOLD))
                .border_style(Style::default().fg(t.accent)),
        )
        .highlight_style(
            Style::default()
                .fg(t.text_bright)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, popup, &mut state);
}

fn draw_folder_picker(frame: &mut ratatui::Frame, app: &App) {
    let t = &app.theme;
    let area = frame.area();
    let total = app.folders.len() + 1;
    // Extra lines for hints or input
    let extra = match app.mode {
        Mode::FolderInput(_) => 3,
        _ => 2,
    };
    let height = (total as u16 + extra + 3).min(area.height.saturating_sub(4));
    let width = 40u16.min(area.width.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);

    let mut entries: Vec<ListItem> = vec![ListItem::new(Span::styled(
        "  All",
        Style::default().fg(t.text),
    ))];

    entries.extend(app.folders.iter().enumerate().map(|(i, name)| {
        let style = if app.confirm_folder_delete && app.folder_selected == i + 1 {
            Style::default().fg(t.error)
        } else {
            Style::default().fg(t.text)
        };
        ListItem::new(Span::styled(format!("  {name}"), style))
    }));

    // Add hint/input line
    match &app.mode {
        Mode::FolderInput(kind) => {
            let label = match kind {
                FolderInputKind::New => "New: ",
                FolderInputKind::Rename(_) => "Rename: ",
            };
            entries.push(ListItem::new(Line::from("")));
            entries.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  {label}"), Style::default().fg(t.accent)),
                Span::styled(format!("{}_", app.input_buf), Style::default().fg(t.text_bright)),
            ])));
        }
        _ => {
            let hints = if app.confirm_folder_delete {
                vec![
                    Span::styled(" Delete? ", Style::default().fg(t.error)),
                    Span::styled("y", Style::default().fg(t.accent)),
                    Span::styled("/", Style::default().fg(t.text_dim)),
                    Span::styled("n", Style::default().fg(t.accent)),
                ]
            } else {
                vec![
                    Span::styled(" n", Style::default().fg(t.accent)),
                    Span::styled("ew ", Style::default().fg(t.text_dim)),
                    Span::styled("r", Style::default().fg(t.accent)),
                    Span::styled("ename ", Style::default().fg(t.text_dim)),
                    Span::styled("d", Style::default().fg(t.accent)),
                    Span::styled("elete", Style::default().fg(t.text_dim)),
                ]
            };
            entries.push(ListItem::new(Line::from("")));
            entries.push(ListItem::new(Line::from(hints)));
        }
    }

    let mut state = ListState::default();
    state.select(Some(app.folder_selected));

    let list = List::new(entries)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Folder ")
                .title_style(Style::default().fg(t.accent).add_modifier(Modifier::BOLD))
                .border_style(Style::default().fg(t.accent)),
        )
        .highlight_style(
            Style::default()
                .fg(t.text_bright)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, popup, &mut state);
}

fn draw_move_picker(frame: &mut ratatui::Frame, app: &App) {
    let t = &app.theme;
    let area = frame.area();
    let height = (app.folders.len() as u16 + 4).min(area.height.saturating_sub(4));
    let width = 30u16.min(area.width.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup);

    let items: Vec<ListItem> = app
        .folders
        .iter()
        .map(|name| {
            ListItem::new(Span::styled(
                format!("  {name}"),
                Style::default().fg(t.text),
            ))
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.move_selected));

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Move to ")
                .title_style(Style::default().fg(t.accent).add_modifier(Modifier::BOLD))
                .border_style(Style::default().fg(t.accent)),
        )
        .highlight_style(
            Style::default()
                .fg(t.text_bright)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    frame.render_stateful_widget(list, popup, &mut state);
}
