//! Application state and key handling.
//!
//! Key handling returns a [`Flow`] rather than touching the terminal, so the
//! whole state machine can be driven from a test.

use anyhow::Result;
use chrono::Local;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use super::categories::{self, Categories, CategoryEdit, Pane, TypeEdit};
use super::entry::{Choice, EntryForm, Field, Mode};
use super::history::{EditField, History};
use super::projects::Projects;
use super::stats::Stats;
use crate::backup;
use crate::config::Config;
use crate::db::models::Status;
use crate::db::{Db, DbError, repo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Entry,
    History,
    Projects,
    Categories,
    Stats,
}

impl Screen {
    pub fn title(self) -> &'static str {
        match self {
            Screen::Entry => "Entry",
            Screen::History => "History",
            Screen::Projects => "Projects",
            Screen::Categories => "Categories",
            Screen::Stats => "Stats",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flash {
    pub text: String,
    pub tone: Tone,
}

/// What the event loop should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Quit,
}

pub struct App {
    pub db: Db,
    pub config: Config,
    pub screen: Screen,
    pub form: EntryForm,
    pub history: History,
    pub projects: Projects,
    pub categories: Categories,
    pub stats: Stats,
    /// How many project rows the last frame had room for, so scrolling moves by
    /// what the writer can actually see rather than by a guess.
    pub project_graph_rows: usize,
    pub flash: Option<Flash>,
    pub help_visible: bool,
    /// The leader key has been pressed and the next key completes a sequence.
    pub leader_pending: bool,
    /// Whether to draw the menu of leader continuations.
    ///
    /// Set by the event loop once the writer has paused, rather than the instant
    /// leader is pressed: a menu that appears and vanishes inside a fast
    /// `<leader><CR>` is a flicker, not help.
    pub leader_hint: bool,
    /// `g` has been pressed, waiting for a second one.
    pending_g: bool,
    /// `q` was pressed with unsaved input and is waiting for confirmation.
    pub confirming_quit: bool,
    /// The database's `data_version` as of the last time everything was loaded.
    ///
    /// Checked when it matters rather than continuously: changing screen, and
    /// submitting an entry. Those are the moments the cached figures are about
    /// to be read or written against, and polling in between would buy nothing.
    last_data_version: i64,
}

impl App {
    pub fn new(db: Db, config: Config) -> Result<Self> {
        let form = EntryForm::new(load_choices(&db)?, Local::now().date_naive());
        let history = History::new(
            repo::recent_entries_with_project(db.conn(), None)?,
            load_choices(&db)?,
        );
        let projects = Projects::new(
            repo::list_projects_with_metrics(db.conn())?,
            repo::list_categories_with_usage(db.conn())?,
        );
        let categories = Categories::new(
            repo::list_categories_with_usage(db.conn())?,
            repo::list_types_with_usage(db.conn())?,
        );
        let stats = Stats::load(&db)?;
        let last_data_version = db.data_version()?;
        // Taken before anything in this session can change the log, so the copy
        // on disk is the state to come back to if this session goes wrong.
        let startup = if config.backups_enabled() {
            backup::best_effort(&db, &config).map(Some)
        } else {
            Ok(None)
        };
        let mut app = Self {
            db,
            config,
            screen: Screen::Entry,
            form,
            history,
            projects,
            categories,
            stats,
            project_graph_rows: 1,
            flash: None,
            help_visible: false,
            leader_pending: false,
            leader_hint: false,
            pending_g: false,
            confirming_quit: false,
            last_data_version,
        };
        if let Err(message) = startup {
            app.set_flash(Tone::Warning, format!("backup failed: {message}"));
        }
        Ok(app)
    }

    /// Back up after a change, without letting a backup failure matter.
    ///
    /// A warning replaces the success message rather than being queued behind it:
    /// one line of status bar, and "your entry saved but the backup did not" is
    /// the more important half.
    fn backup_after_change(&mut self) {
        if !self.config.backups_enabled() {
            return;
        }
        if let Err(message) = backup::best_effort(&self.db, &self.config) {
            self.set_flash(
                Tone::Warning,
                format!("saved, but the backup failed: {message}"),
            );
        }
    }

    /// Reload if another session has committed since the last look.
    ///
    /// Returns whether anything was reloaded, so a caller can decide what to do
    /// about it. A second terminal is the ordinary case here, not an edge: the
    /// command line writes entries too.
    fn adopt_external_changes(&mut self) -> Result<bool> {
        let current = self.db.data_version()?;
        if current == self.last_data_version {
            return Ok(false);
        }
        self.last_data_version = current;
        self.reload_everything()?;
        Ok(true)
    }

    fn set_flash(&mut self, tone: Tone, text: impl Into<String>) {
        self.flash = Some(Flash {
            tone,
            text: text.into(),
        });
    }

    /// Typed input that quitting would throw away, on any screen.
    fn has_unsaved_input(&self) -> bool {
        self.form.has_unsaved_input()
            || self.history.has_unsaved_input()
            || self.projects.has_unsaved_input()
            || self.categories.has_unsaved_input()
    }

    /// Whichever screen is waiting on a delete confirmation.
    fn awaiting_delete(&self) -> bool {
        match self.screen {
            Screen::History => self.history.confirming_delete,
            Screen::Projects => self.projects.confirming_delete,
            Screen::Categories => self.categories.confirming_delete,
            _ => false,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Result<Flow> {
        // Windows reports both press and release; acting on both doubles input.
        if key.kind != KeyEventKind::Press {
            return Ok(Flow::Continue);
        }

        if self.help_visible {
            self.help_visible = false;
            return Ok(Flow::Continue);
        }

        if self.confirming_quit {
            self.confirming_quit = false;
            if matches!(key.code, KeyCode::Char('q')) {
                return Ok(Flow::Quit);
            }
            self.set_flash(Tone::Info, "still here");
            return Ok(Flow::Continue);
        }

        if self.awaiting_delete() {
            return self.resolve_delete(key);
        }

        // A fresh keystroke supersedes the last message.
        self.flash = None;

        if self.leader_pending {
            self.leader_pending = false;
            self.leader_hint = false;
            return self.on_leader(key);
        }

        match self.screen {
            Screen::Entry => self.on_key_entry(key),
            Screen::History => self.on_key_history(key),
            Screen::Projects => self.on_key_projects(key),
            Screen::Categories => self.on_key_categories(key),
            Screen::Stats => self.on_key_stats(key),
        }
    }

    fn on_leader(&mut self, key: KeyEvent) -> Result<Flow> {
        match key.code {
            KeyCode::Enter => {
                match self.screen {
                    Screen::Entry => self.submit_entry()?,
                    Screen::History => self.save_edit()?,
                    Screen::Projects => self.save_project()?,
                    Screen::Categories => self.save_category_or_type()?,
                    Screen::Stats => {}
                }
                Ok(Flow::Continue)
            }
            KeyCode::Char('e') => self.go(Screen::Entry),
            KeyCode::Char('h') => self.go(Screen::History),
            KeyCode::Char('p') => self.go(Screen::Projects),
            KeyCode::Char('c') => self.go(Screen::Categories),
            KeyCode::Char('s') => self.go(Screen::Stats),
            // Asking for help from the leader menu opens the full reference
            // rather than reporting an unbound key.
            KeyCode::Char('?') => {
                self.help_visible = true;
                Ok(Flow::Continue)
            }
            KeyCode::Esc => Ok(Flow::Continue),
            other => {
                self.set_flash(
                    Tone::Info,
                    format!("<leader>{} is not bound", describe(other)),
                );
                Ok(Flow::Continue)
            }
        }
    }

    fn go(&mut self, screen: Screen) -> Result<Flow> {
        // Leaving a half-finished edit behind would be a trap: it would still be
        // open, holding changes, the next time this screen came back.
        if screen != self.screen {
            let discarded = match self.screen {
                Screen::History => self.history.cancel_edit(),
                Screen::Projects => self.projects.cancel_edit(),
                Screen::Categories => self.categories.cancel_edit(),
                _ => false,
            };
            if discarded {
                self.set_flash(Tone::Warning, "edit discarded");
            }
        }
        // Another session may have changed things since this one last looked.
        if self.adopt_external_changes()? {
            self.set_flash(Tone::Info, "another session changed the log; reloaded");
        }
        // Reload whatever the screen shows: another screen may have changed it.
        match screen {
            Screen::History => self.reload_history()?,
            Screen::Entry => self.form.refresh_choices(load_choices(&self.db)?),
            Screen::Projects => self.reload_projects()?,
            Screen::Categories => self.reload_categories()?,
            Screen::Stats => self.stats.reload(&self.db)?,
        }
        self.screen = screen;
        Ok(Flow::Continue)
    }

    fn reload_history(&mut self) -> Result<()> {
        let rows = repo::recent_entries_with_project(self.db.conn(), None)?;
        self.history.refresh(rows, load_choices(&self.db)?);
        Ok(())
    }

    fn reload_projects(&mut self) -> Result<()> {
        let rows = repo::list_projects_with_metrics(self.db.conn())?;
        let categories = repo::list_categories_with_usage(self.db.conn())?;
        self.projects.refresh(rows, categories);
        Ok(())
    }

    fn reload_categories(&mut self) -> Result<()> {
        let categories = repo::list_categories_with_usage(self.db.conn())?;
        let types = repo::list_types_with_usage(self.db.conn())?;
        self.categories.refresh(categories, types);
        Ok(())
    }

    /// After anything that changes projects or categories, every screen that
    /// shows them has to be told, or two screens will disagree.
    fn reload_everything(&mut self) -> Result<()> {
        self.form.refresh_choices(load_choices(&self.db)?);
        self.reload_history()?;
        self.reload_projects()?;
        self.reload_categories()?;
        self.stats.reload(&self.db)?;
        self.last_data_version = self.db.data_version()?;
        Ok(())
    }

    fn request_quit(&mut self) -> Flow {
        if self.has_unsaved_input() {
            self.confirming_quit = true;
            self.set_flash(
                Tone::Warning,
                "unsaved input: press q again to quit, anything else to stay",
            );
            Flow::Continue
        } else {
            Flow::Quit
        }
    }

    // ---------------------------------------------------------------- entry form

    fn on_key_entry(&mut self, key: KeyEvent) -> Result<Flow> {
        match self.form.mode {
            Mode::Insert | Mode::Filter => {
                match key.code {
                    KeyCode::Esc | KeyCode::Enter => self.form.leave_insert(),
                    KeyCode::Backspace => self.form.backspace(),
                    KeyCode::Tab => self.form.next_field(),
                    KeyCode::BackTab => self.form.previous_field(),
                    KeyCode::Down if self.form.mode == Mode::Filter => self.form.select_next(),
                    KeyCode::Up if self.form.mode == Mode::Filter => self.form.select_previous(),
                    KeyCode::Char(c) => self.form.push_char(c),
                    _ => {}
                }
                Ok(Flow::Continue)
            }
            Mode::Normal => {
                // `gg` and `G` only mean anything to the project list.
                if self.take_pending_g(key) {
                    if self.form.field == Field::Project {
                        self.form.select_first();
                    }
                    return Ok(Flow::Continue);
                }
                match key.code {
                    KeyCode::Char(' ') => self.leader_pending = true,
                    KeyCode::Char('?') => self.help_visible = true,
                    KeyCode::Char('q') => return Ok(self.request_quit()),
                    // j and k walk the form, the same as Tab, so they never
                    // reach past the focused field to move something else.
                    KeyCode::Tab | KeyCode::Char('j') | KeyCode::Down => self.form.next_field(),
                    KeyCode::BackTab | KeyCode::Char('k') | KeyCode::Up => {
                        self.form.previous_field()
                    }
                    // A choice field steps with h and l, as on the projects screen.
                    KeyCode::Char('l') | KeyCode::Right => self.step_project(1),
                    KeyCode::Char('h') | KeyCode::Left => self.step_project(-1),
                    KeyCode::Char('g') if self.form.field == Field::Project => {
                        self.pending_g = true
                    }
                    KeyCode::Char('G') if self.form.field == Field::Project => {
                        self.form.select_last()
                    }
                    KeyCode::Char('i') | KeyCode::Char('a') => self.form.begin_insert(),
                    KeyCode::Char('/') => self.form.begin_filter(),
                    KeyCode::Char('A') => {
                        self.form.toggle_include_closed();
                        let scope = if self.form.include_closed {
                            "showing all projects"
                        } else {
                            "showing open projects"
                        };
                        self.set_flash(Tone::Info, scope);
                    }
                    KeyCode::Esc if !self.form.filter.is_empty() => self.form.clear_filter(),
                    KeyCode::Enter => self.form.next_field(),
                    _ => {}
                }
                Ok(Flow::Continue)
            }
        }
    }

    /// Step the project choice, saying so if the cursor is somewhere else.
    fn step_project(&mut self, step: i64) {
        if self.form.field != Field::Project {
            self.set_flash(
                Tone::Info,
                format!(
                    "h and l step the project; {} is typed",
                    self.form.field.label()
                ),
            );
            return;
        }
        if step >= 0 {
            self.form.select_next();
        } else {
            self.form.select_previous();
        }
    }

    /// Write the entry, or report everything stopping it.
    ///
    /// A session count is the difference between what the writer typed and what
    /// the project already holds, so a stale total silently inflates it. If
    /// another session has written since this one loaded, the figures are
    /// refreshed and the submit is refused once: the recalculated session is put
    /// on screen so that the number written is a number the writer has seen.
    fn submit_entry(&mut self) -> Result<()> {
        if self.adopt_external_changes()? {
            self.set_flash(
                Tone::Warning,
                "another session added work; the figures have been recalculated, \
                 check them and submit again",
            );
            return Ok(());
        }
        match self.form.to_new_entry() {
            Err(problems) => self.flash_problems(problems.iter().map(|p| p.to_string())),
            Ok(entry) => {
                let title = self
                    .form
                    .selected_project()
                    .map(|c| c.title.clone())
                    .unwrap_or_default();
                let words = entry.session_word_count;
                repo::insert_entry(self.db.conn(), &entry)?;
                self.form.refresh_choices(load_choices(&self.db)?);
                self.form.clear_after_submit();
                self.reload_history()?;
                let summary = if words < 0 {
                    format!("saved: cut {} words from {title}", words.abs())
                } else {
                    format!("saved: {words} words on {title}")
                };
                self.set_flash(Tone::Success, summary);
                self.backup_after_change();
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------- history

    fn on_key_history(&mut self, key: KeyEvent) -> Result<Flow> {
        // The edit panel owns the keyboard while it is open.
        if self.history.edit.is_some() {
            return self.on_key_editing(key);
        }

        if self.history.mode == Mode::Filter {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => self.history.leave_filter(),
                KeyCode::Backspace => self.history.backspace_filter(),
                KeyCode::Down => self.history.select_next(),
                KeyCode::Up => self.history.select_previous(),
                KeyCode::Char(c) => self.history.push_filter_char(c),
                _ => {}
            }
            return Ok(Flow::Continue);
        }

        // `d` alone is a prefix; anything other than a second `d` abandons it.
        if self.history.pending_d() && !matches!(key.code, KeyCode::Char('d')) {
            self.history.clear_pending_d();
        }
        if self.take_pending_g(key) {
            self.history.select_first();
            return Ok(Flow::Continue);
        }

        match key.code {
            KeyCode::Char(' ') => self.leader_pending = true,
            KeyCode::Char('?') => self.help_visible = true,
            KeyCode::Char('q') => return Ok(self.request_quit()),
            KeyCode::Char('j') | KeyCode::Down => self.history.select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.history.select_previous(),
            KeyCode::Char('g') => self.pending_g = true,
            KeyCode::Char('G') => self.history.select_last(),
            KeyCode::Char('/') => self.history.begin_filter(),
            KeyCode::Esc if !self.history.filter.is_empty() => self.history.clear_filter(),
            KeyCode::Char('i') | KeyCode::Char('a') | KeyCode::Enter => self.start_edit(),
            KeyCode::Char('d') => self.press_d(),
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn on_key_editing(&mut self, key: KeyEvent) -> Result<Flow> {
        let edit = self.history.edit.as_mut().expect("checked by caller");
        match edit.mode {
            Mode::Insert | Mode::Filter => {
                match key.code {
                    KeyCode::Esc | KeyCode::Enter => edit.mode = Mode::Normal,
                    KeyCode::Backspace => edit.backspace(),
                    KeyCode::Tab => {
                        let next = edit.field.next();
                        edit.set_field(next);
                    }
                    KeyCode::BackTab => {
                        let previous = edit.field.previous();
                        edit.set_field(previous);
                    }
                    // While narrowing the picker the list keeps moving under it.
                    KeyCode::Down if edit.mode == Mode::Filter => self.history.step_project(1),
                    KeyCode::Up if edit.mode == Mode::Filter => self.history.step_project(-1),
                    KeyCode::Char(c) => edit.push_char(c),
                    _ => {}
                }
                // Typing to find a project should land on it.
                self.history.sync_project_to_filter();
                Ok(Flow::Continue)
            }
            Mode::Normal => {
                match key.code {
                    KeyCode::Char(' ') => self.leader_pending = true,
                    KeyCode::Char('?') => self.help_visible = true,
                    KeyCode::Tab | KeyCode::Char('j') | KeyCode::Down => {
                        let next = edit.field.next();
                        edit.set_field(next);
                    }
                    KeyCode::BackTab | KeyCode::Char('k') | KeyCode::Up => {
                        let previous = edit.field.previous();
                        edit.set_field(previous);
                    }
                    KeyCode::Char('l') | KeyCode::Right => self.step_edit_project(1),
                    KeyCode::Char('h') | KeyCode::Left => self.step_edit_project(-1),
                    KeyCode::Char('i') | KeyCode::Char('a') => edit.begin_insert(),
                    KeyCode::Char('/') => {
                        edit.set_field(EditField::Project);
                        edit.begin_insert();
                    }
                    KeyCode::Esc => self.discard_edit(),
                    KeyCode::Char('q') => return Ok(self.request_quit()),
                    _ => {}
                }
                Ok(Flow::Continue)
            }
        }
    }

    /// Step the entry's project, saying where the keys apply if elsewhere.
    fn step_edit_project(&mut self, step: i64) {
        let on_project = self
            .history
            .edit
            .as_ref()
            .is_some_and(|e| e.field == EditField::Project);
        if !on_project {
            let label = self
                .history
                .edit
                .as_ref()
                .map(|e| e.field.label())
                .unwrap_or("this field");
            self.set_flash(
                Tone::Info,
                format!("h and l step the project; {label} is typed"),
            );
            return;
        }
        self.history.step_project(step);
    }

    fn start_edit(&mut self) {
        if !self.history.begin_edit() {
            self.set_flash(Tone::Info, "nothing to edit");
        }
    }

    /// `dd` deletes; a single `d` only arms it.
    fn press_d(&mut self) {
        if !self.history.press_d() {
            return;
        }
        if self.history.selected_row().is_some() {
            self.history.confirming_delete = true;
        } else {
            self.set_flash(Tone::Info, "nothing to delete");
        }
    }

    /// Close the edit panel, saying so when changes were thrown away.
    fn discard_edit(&mut self) {
        if self.history.cancel_edit() {
            self.set_flash(Tone::Warning, "edit discarded");
        }
    }

    fn save_edit(&mut self) -> Result<()> {
        let Some(edit) = self.history.edit.as_ref() else {
            self.set_flash(Tone::Info, "no entry is open for editing");
            return Ok(());
        };
        match edit.to_new_entry() {
            Err(problems) => self.flash_problems(problems.iter().map(|p| p.to_string())),
            Ok(updated) => {
                let id = edit.entry_id;
                let title = edit.project_title.clone();
                let words = updated.session_word_count;
                repo::update_entry(self.db.conn(), id, &updated)?;
                self.history.edit = None;
                self.reload_history()?;
                self.form.refresh_choices(load_choices(&self.db)?);
                self.set_flash(Tone::Success, format!("updated: {words} words on {title}"));
            }
        }
        Ok(())
    }

    /// Resolve the delete prompt. Anything but `y` leaves the entry alone.
    fn resolve_delete(&mut self, key: KeyEvent) -> Result<Flow> {
        // Every screen's prompt is cleared, so a stray flag cannot leave one
        // armed and have the next keystroke delete something unrelated.
        self.history.confirming_delete = false;
        self.projects.confirming_delete = false;
        self.categories.confirming_delete = false;
        if !matches!(key.code, KeyCode::Char('y')) {
            self.set_flash(Tone::Info, "kept");
            return Ok(Flow::Continue);
        }
        match self.screen {
            Screen::History => self.delete_selected_entry(),
            Screen::Projects => self.delete_selected_project(),
            Screen::Categories => self.delete_selected_category_or_type(),
            _ => Ok(Flow::Continue),
        }
    }

    fn delete_selected_entry(&mut self) -> Result<Flow> {
        let Some(row) = self.history.selected_row() else {
            return Ok(Flow::Continue);
        };
        let (id, words, title) = (
            row.entry.id,
            row.entry.session_word_count,
            row.project_title.clone(),
        );
        repo::delete_entry(self.db.conn(), id)?;
        self.reload_everything()?;
        self.set_flash(Tone::Success, format!("deleted: {words} words on {title}"));
        Ok(Flow::Continue)
    }

    fn delete_selected_project(&mut self) -> Result<Flow> {
        let Some(row) = self.projects.selected_row() else {
            return Ok(Flow::Continue);
        };
        let id = row.project.id;
        // The repository refuses a project that has entries whatever this screen
        // believes, so a stale list cannot destroy the record of work done.
        match repo::delete_project(self.db.conn(), id) {
            Err(DbError::ProjectHasEntries { title, entries }) => {
                self.set_flash(
                    Tone::Error,
                    format!("{title} still has {entries} entries; nothing deleted"),
                );
            }
            Err(other) => return Err(other.into()),
            Ok(()) => {
                let title = row.project.title.clone();
                self.reload_everything()?;
                self.set_flash(Tone::Success, format!("deleted: {title}"));
            }
        }
        Ok(Flow::Continue)
    }

    fn delete_selected_category_or_type(&mut self) -> Result<Flow> {
        let (result, name) = match self.categories.pane {
            Pane::Categories => {
                let Some(row) = self.categories.selected_category() else {
                    return Ok(Flow::Continue);
                };
                let name = row.category.name.clone();
                (repo::delete_category(self.db.conn(), row.category.id), name)
            }
            Pane::Types => {
                let Some(row) = self.categories.selected_type() else {
                    return Ok(Flow::Continue);
                };
                let name = row.type_.name.clone();
                (repo::delete_type(self.db.conn(), row.type_.id), name)
            }
        };
        match result {
            Err(error @ (DbError::CategoryInUse { .. } | DbError::TypeInUse { .. })) => {
                self.set_flash(Tone::Error, error.to_string());
            }
            Err(other) => return Err(other.into()),
            Ok(()) => {
                self.reload_everything()?;
                self.set_flash(Tone::Success, format!("deleted: {name}"));
            }
        }
        Ok(Flow::Continue)
    }

    // ---------------------------------------------------------------- projects

    fn on_key_projects(&mut self, key: KeyEvent) -> Result<Flow> {
        if self.projects.edit.is_some() {
            return self.on_key_project_edit(key);
        }
        if self.projects.mode == Mode::Filter {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => self.projects.leave_filter(),
                KeyCode::Backspace => self.projects.backspace_filter(),
                KeyCode::Down => self.projects.select_next(),
                KeyCode::Up => self.projects.select_previous(),
                KeyCode::Char(c) => self.projects.push_filter_char(c),
                _ => {}
            }
            return Ok(Flow::Continue);
        }
        if self.projects.pending_d() && !matches!(key.code, KeyCode::Char('d')) {
            self.projects.clear_pending_d();
        }
        if self.take_pending_g(key) {
            self.projects.select_first();
            return Ok(Flow::Continue);
        }

        match key.code {
            KeyCode::Char(' ') => self.leader_pending = true,
            KeyCode::Char('?') => self.help_visible = true,
            KeyCode::Char('q') => return Ok(self.request_quit()),
            KeyCode::Char('j') | KeyCode::Down => self.projects.select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.projects.select_previous(),
            KeyCode::Char('g') => self.pending_g = true,
            KeyCode::Char('G') => self.projects.select_last(),
            KeyCode::Char('/') => self.projects.begin_filter(),
            KeyCode::Esc if !self.projects.filter.is_empty() => self.projects.clear_filter(),
            KeyCode::Char('i') | KeyCode::Char('a') | KeyCode::Enter => self.start_project_edit(),
            KeyCode::Char('o') => self.projects.begin_create(),
            KeyCode::Char('x') => self.cycle_selected_status()?,
            KeyCode::Char('A') => {
                self.projects.toggle_show_all();
                let scope = if self.projects.show_all() {
                    "showing all projects"
                } else {
                    "showing open projects"
                };
                self.set_flash(Tone::Info, scope);
            }
            KeyCode::Char('d') => self.press_d_projects(),
            _ => {}
        }
        Ok(Flow::Continue)
    }

    /// The one-key status pass over the project list.
    fn cycle_selected_status(&mut self) -> Result<()> {
        let Some((id, title, status)) = self.projects.next_status_of_selected() else {
            self.set_flash(Tone::Info, "no project selected");
            return Ok(());
        };
        repo::set_project_status(self.db.conn(), id, status)?;
        // Held in view so the change is visible and the key can be pressed
        // again to carry on round the cycle.
        self.projects.pin(id);
        self.reload_everything()?;
        self.set_flash(Tone::Success, format!("{title} is now {status}"));
        Ok(())
    }

    fn press_d_projects(&mut self) {
        if !self.projects.press_d() {
            return;
        }
        match self.projects.selected_row() {
            Some(row) if row.metrics.entry_count > 0 => {
                // Deleting would cascade to the entries, so this is refused
                // outright rather than confirmed.
                self.set_flash(
                    Tone::Error,
                    format!(
                        "{} has {} entries; mark it abandoned with x instead",
                        row.project.title, row.metrics.entry_count
                    ),
                );
            }
            Some(_) => self.projects.confirming_delete = true,
            None => self.set_flash(Tone::Info, "nothing to delete"),
        }
    }

    fn on_key_project_edit(&mut self, key: KeyEvent) -> Result<Flow> {
        let edit = self.projects.edit.as_mut().expect("checked by caller");
        if edit.mode == Mode::Insert {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => edit.mode = Mode::Normal,
                KeyCode::Backspace => edit.backspace(),
                KeyCode::Tab => {
                    edit.field = edit.field.next();
                    edit.mode = Mode::Normal;
                }
                KeyCode::BackTab => {
                    edit.field = edit.field.previous();
                    edit.mode = Mode::Normal;
                }
                KeyCode::Char(c) => edit.push_char(c),
                _ => {}
            }
            return Ok(Flow::Continue);
        }
        match key.code {
            KeyCode::Char(' ') => self.leader_pending = true,
            KeyCode::Char('?') => self.help_visible = true,
            KeyCode::Char('q') => return Ok(self.request_quit()),
            // Tab and j/k both walk the fields: Tab is the habit from forms,
            // j and k the habit from vim, and neither costs the other anything.
            KeyCode::Tab | KeyCode::Char('j') | KeyCode::Down => edit.field = edit.field.next(),
            KeyCode::BackTab | KeyCode::Char('k') | KeyCode::Up => {
                edit.field = edit.field.previous()
            }
            // A choice field steps with h and l; a text field is typed into.
            KeyCode::Char('l') | KeyCode::Right => self.projects.cycle_choice(1),
            KeyCode::Char('h') | KeyCode::Left => self.projects.cycle_choice(-1),
            KeyCode::Char('i') | KeyCode::Char('a') => {
                if edit.field.is_choice() {
                    self.set_flash(Tone::Info, "h and l step this field");
                } else {
                    edit.mode = Mode::Insert;
                }
            }
            KeyCode::Esc => self.discard_project_edit(),
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn start_project_edit(&mut self) {
        if !self.projects.begin_edit() {
            self.set_flash(Tone::Info, "nothing to edit");
        }
    }

    fn discard_project_edit(&mut self) {
        if self.projects.cancel_edit() {
            self.set_flash(Tone::Warning, "edit discarded");
        }
    }

    fn save_project(&mut self) -> Result<()> {
        let Some(edit) = self.projects.edit.as_ref() else {
            self.set_flash(Tone::Info, "no project is open for editing");
            return Ok(());
        };
        let problems = edit.problems();
        if !problems.is_empty() {
            self.flash_problems(problems.iter().map(|p| p.to_string()));
            return Ok(());
        }
        let title = edit.title.trim().to_string();
        let description = edit.description_option();
        let (category_id, status, id) = (edit.category_id, edit.status, edit.id);

        let outcome = match id {
            Some(id) => repo::update_project(
                self.db.conn(),
                id,
                &title,
                category_id,
                status,
                description.as_deref(),
            )
            .map(|()| "updated"),
            None => repo::insert_project(
                self.db.conn(),
                &title,
                category_id,
                status,
                description.as_deref(),
            )
            .map(|_| "created"),
        };
        match outcome.map_err(|e| repo::name_taken(e, &title)) {
            Err(DbError::NameTaken(name)) => {
                self.set_flash(
                    Tone::Error,
                    format!("a project called {name:?} already exists"),
                );
            }
            Err(other) => return Err(other.into()),
            Ok(verb) => {
                self.projects.edit = None;
                self.reload_everything()?;
                self.set_flash(Tone::Success, format!("{verb}: {title}"));
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------- categories

    fn on_key_categories(&mut self, key: KeyEvent) -> Result<Flow> {
        if self.categories.edit.is_some() {
            return self.on_key_category_edit(key);
        }
        if self.categories.pending_d() && !matches!(key.code, KeyCode::Char('d')) {
            self.categories.clear_pending_d();
        }
        if self.take_pending_g(key) {
            self.categories.select_first();
            return Ok(Flow::Continue);
        }
        match key.code {
            KeyCode::Char(' ') => self.leader_pending = true,
            KeyCode::Char('?') => self.help_visible = true,
            KeyCode::Char('q') => return Ok(self.request_quit()),
            KeyCode::Char('j') | KeyCode::Down => self.categories.select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.categories.select_previous(),
            KeyCode::Char('g') => self.pending_g = true,
            KeyCode::Char('G') => self.categories.select_last(),
            KeyCode::Tab | KeyCode::Char('l') | KeyCode::Char('h') => {
                self.categories.switch_pane();
            }
            KeyCode::Char('i') | KeyCode::Char('a') | KeyCode::Enter => self.start_category_edit(),
            KeyCode::Char('o') => self.categories.begin_create(),
            KeyCode::Char('d') => self.press_d_categories(),
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn press_d_categories(&mut self) {
        if !self.categories.press_d() {
            return;
        }
        let blocked = match self.categories.pane {
            Pane::Categories => self.categories.selected_category().map(|row| {
                (
                    row.project_count,
                    format!("{} projects use", row.project_count),
                )
            }),
            Pane::Types => self.categories.selected_type().map(|row| {
                (
                    row.category_count,
                    format!("{} categories use", row.category_count),
                )
            }),
        };
        match blocked {
            Some((0, _)) => self.categories.confirming_delete = true,
            Some((_, reason)) => {
                self.set_flash(Tone::Error, format!("{reason} this; move them first"));
            }
            None => self.set_flash(Tone::Info, "nothing to delete"),
        }
    }

    fn on_key_category_edit(&mut self, key: KeyEvent) -> Result<Flow> {
        let edit = self.categories.edit.as_mut().expect("checked by caller");
        if edit.mode() == Mode::Insert {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => edit.set_mode(Mode::Normal),
                KeyCode::Backspace => edit.backspace(),
                KeyCode::Tab => {
                    edit.next_field();
                    edit.set_mode(Mode::Normal);
                }
                KeyCode::BackTab => {
                    edit.previous_field();
                    edit.set_mode(Mode::Normal);
                }
                KeyCode::Char(c) => edit.push_char(c),
                _ => {}
            }
            return Ok(Flow::Continue);
        }
        match key.code {
            KeyCode::Char(' ') => self.leader_pending = true,
            KeyCode::Char('?') => self.help_visible = true,
            KeyCode::Char('q') => return Ok(self.request_quit()),
            KeyCode::Tab | KeyCode::Char('j') | KeyCode::Down => edit.next_field(),
            KeyCode::BackTab | KeyCode::Char('k') | KeyCode::Up => edit.previous_field(),
            KeyCode::Char('l') | KeyCode::Right => self.categories.cycle_choice(1),
            KeyCode::Char('h') | KeyCode::Left => self.categories.cycle_choice(-1),
            KeyCode::Char('i') | KeyCode::Char('a') => edit.set_mode(Mode::Insert),
            KeyCode::Esc => self.discard_category_edit(),
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn start_category_edit(&mut self) {
        if !self.categories.begin_edit() {
            self.set_flash(Tone::Info, "nothing to edit");
        }
    }

    fn discard_category_edit(&mut self) {
        if self.categories.cancel_edit() {
            self.set_flash(Tone::Warning, "edit discarded");
        }
    }

    fn save_category_or_type(&mut self) -> Result<()> {
        let Some(edit) = self.categories.edit.as_ref() else {
            self.set_flash(Tone::Info, "nothing is open for editing");
            return Ok(());
        };
        let problems = edit.problems();
        if !problems.is_empty() {
            self.flash_problems(problems.iter().map(|p| p.to_string()));
            return Ok(());
        }
        let (name, outcome) = match edit.clone() {
            categories::Edit::Category(CategoryEdit {
                id, name, type_id, ..
            }) => {
                let name = name.trim().to_string();
                let description = match &self.categories.edit {
                    Some(categories::Edit::Category(e)) => e.description_option(),
                    _ => None,
                };
                let type_id = type_id.expect("validated");
                let outcome = match id {
                    Some(id) => repo::update_category(
                        self.db.conn(),
                        id,
                        &name,
                        type_id,
                        description.as_deref(),
                    )
                    .map(|()| "updated"),
                    None => repo::insert_category(
                        self.db.conn(),
                        &name,
                        type_id,
                        description.as_deref(),
                    )
                    .map(|_| "created"),
                };
                (name, outcome)
            }
            categories::Edit::Type(TypeEdit { id, name, .. }) => {
                let name = name.trim().to_string();
                let description = match &self.categories.edit {
                    Some(categories::Edit::Type(e)) => e.description_option(),
                    _ => None,
                };
                let outcome = match id {
                    Some(id) => {
                        repo::update_type(self.db.conn(), id, &name, description.as_deref())
                            .map(|()| "updated")
                    }
                    None => repo::insert_type(self.db.conn(), &name, description.as_deref())
                        .map(|_| "created"),
                };
                (name, outcome)
            }
        };
        match outcome.map_err(|e| repo::name_taken(e, &name)) {
            Err(DbError::NameTaken(name)) => {
                self.set_flash(Tone::Error, format!("{name:?} already exists"));
            }
            Err(other) => return Err(other.into()),
            Ok(verb) => {
                self.categories.edit = None;
                self.reload_everything()?;
                self.set_flash(Tone::Success, format!("{verb}: {name}"));
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------------- stats

    fn on_key_stats(&mut self, key: KeyEvent) -> Result<Flow> {
        if self.take_pending_g(key) {
            let visible = self.project_graph_rows;
            self.stats.scroll_projects(i64::MIN / 4, visible);
            return Ok(Flow::Continue);
        }
        match key.code {
            KeyCode::Char(' ') => self.leader_pending = true,
            KeyCode::Char('?') => self.help_visible = true,
            KeyCode::Char('q') => return Ok(self.request_quit()),
            // The years are stacked vertically in the graph, so k is older and
            // j is newer, with all time sitting past the newest.
            KeyCode::Char('k') | KeyCode::Up | KeyCode::Char('[') => self.step_scope(-1)?,
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Char(']') => self.step_scope(1)?,
            // h and l are free here, so they walk the project graph, which in the
            // all-time view holds every project ever started.
            KeyCode::Char('l') | KeyCode::Right => self.scroll_projects(1),
            KeyCode::Char('h') | KeyCode::Left => self.scroll_projects(-1),
            KeyCode::Char('0') => self.stats.go_to_current_year(&self.db)?,
            KeyCode::Char('$') => self.stats.go_to_lifetime(&self.db)?,
            KeyCode::Char('g') => self.pending_g = true,
            KeyCode::Char('G') => {
                let visible = self.project_graph_rows;
                self.stats.scroll_projects(i64::MAX / 4, visible);
            }
            _ => {}
        }
        Ok(Flow::Continue)
    }

    /// Step the dashboard's scope, saying so at either end of the sequence.
    fn step_scope(&mut self, direction: i64) -> Result<()> {
        if !self.stats.step(direction, &self.db)? {
            let edge = if direction < 0 {
                "earliest year"
            } else {
                "end"
            };
            self.set_flash(
                Tone::Info,
                format!("{} is the {edge}", self.stats.scope.label()),
            );
        }
        Ok(())
    }

    fn scroll_projects(&mut self, step: i64) {
        let visible = self.project_graph_rows;
        if !self.stats.scroll_projects(step, visible) {
            let total = self.stats.selected_projects.len();
            if total > visible {
                let edge = if step < 0 { "top" } else { "end" };
                self.set_flash(Tone::Info, format!("already at the {edge} of the list"));
            }
        }
    }

    // ---------------------------------------------------------------- shared

    /// True when this key completes a `gg`.
    fn take_pending_g(&mut self, key: KeyEvent) -> bool {
        if !self.pending_g {
            return false;
        }
        self.pending_g = false;
        matches!(key.code, KeyCode::Char('g'))
    }

    fn flash_problems(&mut self, problems: impl Iterator<Item = String>) {
        let problems: Vec<String> = problems.collect();
        let first = problems
            .first()
            .cloned()
            .unwrap_or_else(|| "cannot save".to_string());
        let extra = problems.len().saturating_sub(1);
        let text = if extra > 0 {
            format!("{first} (and {extra} more)")
        } else {
            first
        };
        self.set_flash(Tone::Error, text);
    }
}

/// Every project, with the figures the picker and the preview need.
pub fn load_choices(db: &Db) -> Result<Vec<Choice>> {
    let conn = db.conn();
    let mut choices = Vec::new();
    for project in repo::list_projects(conn)? {
        choices.push(Choice {
            id: project.id,
            title: project.title,
            status: project.status,
            logged_words: repo::project_logged_total(conn, project.id)?,
            entry_count: repo::project_entry_count(conn, project.id)?,
        });
    }
    // Open projects first, then by title, so the daily path needs no scrolling.
    choices.sort_by(|a, b| {
        let rank = |s: Status| u8::from(s != Status::Open);
        rank(a.status)
            .cmp(&rank(b.status))
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(choices)
}

fn describe(code: KeyCode) -> String {
    match code {
        KeyCode::Char(c) => c.to_string(),
        other => format!("{other:?}").to_lowercase(),
    }
}

/// The mode indicator for the status bar.
pub fn mode_label(app: &App) -> &'static str {
    if app.leader_pending {
        return "LEADER";
    }
    let mode = match app.screen {
        Screen::History => app
            .history
            .edit
            .as_ref()
            .map(|e| e.mode)
            .unwrap_or(app.history.mode),
        Screen::Projects => app
            .projects
            .edit
            .as_ref()
            .map(|e| e.mode)
            .unwrap_or(app.projects.mode),
        Screen::Categories => app
            .categories
            .edit
            .as_ref()
            .map(|e| e.mode())
            .unwrap_or(app.categories.mode),
        // The dashboard has no modes: it is read-only.
        Screen::Stats => Mode::Normal,
        _ => app.form.mode,
    };
    match mode {
        Mode::Normal => "NORMAL",
        Mode::Insert => "INSERT",
        Mode::Filter => "FILTER",
    }
}
