//! Application state and key handling.
//!
//! Key handling returns a [`Flow`] rather than touching the terminal, so the
//! whole state machine can be driven from a test.

use anyhow::Result;
use chrono::Local;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use super::categories::{self, Categories, CategoryEdit, Pane, TypeEdit};
use super::entry::{Choice, EntryForm, Mode};
use super::history::History;
use super::projects::Projects;
use super::stats::Stats;
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
    pub screen: Screen,
    pub form: EntryForm,
    pub history: History,
    pub projects: Projects,
    pub categories: Categories,
    pub stats: Stats,
    pub flash: Option<Flash>,
    pub help_visible: bool,
    /// The leader key has been pressed and the next key completes a sequence.
    pub leader_pending: bool,
    /// `g` has been pressed, waiting for a second one.
    pending_g: bool,
    /// `q` was pressed with unsaved input and is waiting for confirmation.
    pub confirming_quit: bool,
}

impl App {
    pub fn new(db: Db) -> Result<Self> {
        let form = EntryForm::new(load_choices(&db)?, Local::now().date_naive());
        let history = History::new(repo::recent_entries_with_project(db.conn(), None)?);
        let projects = Projects::new(
            repo::list_projects_with_metrics(db.conn())?,
            repo::list_categories_with_usage(db.conn())?,
        );
        let categories = Categories::new(
            repo::list_categories_with_usage(db.conn())?,
            repo::list_types_with_usage(db.conn())?,
        );
        let stats = Stats::load(&db)?;
        Ok(Self {
            db,
            screen: Screen::Entry,
            form,
            history,
            projects,
            categories,
            stats,
            flash: None,
            help_visible: false,
            leader_pending: false,
            pending_g: false,
            confirming_quit: false,
        })
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
        self.history.refresh(rows);
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
                if self.take_pending_g(key) {
                    self.form.select_first();
                    return Ok(Flow::Continue);
                }
                match key.code {
                    KeyCode::Char(' ') => self.leader_pending = true,
                    KeyCode::Char('?') => self.help_visible = true,
                    KeyCode::Char('q') => return Ok(self.request_quit()),
                    KeyCode::Tab => self.form.next_field(),
                    KeyCode::BackTab => self.form.previous_field(),
                    KeyCode::Char('j') | KeyCode::Down => self.form.select_next(),
                    KeyCode::Char('k') | KeyCode::Up => self.form.select_previous(),
                    KeyCode::Char('g') => self.pending_g = true,
                    KeyCode::Char('G') => self.form.select_last(),
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

    /// Write the entry, or report everything stopping it.
    fn submit_entry(&mut self) -> Result<()> {
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
                Ok(Flow::Continue)
            }
            Mode::Normal => {
                match key.code {
                    KeyCode::Char(' ') => self.leader_pending = true,
                    KeyCode::Char('?') => self.help_visible = true,
                    KeyCode::Tab => edit.field = edit.field.next(),
                    KeyCode::BackTab => edit.field = edit.field.previous(),
                    KeyCode::Char('i') | KeyCode::Char('a') => edit.mode = Mode::Insert,
                    KeyCode::Esc => self.discard_edit(),
                    KeyCode::Char('q') => return Ok(self.request_quit()),
                    _ => {}
                }
                Ok(Flow::Continue)
            }
        }
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
            KeyCode::Tab => edit.field = edit.field.next(),
            KeyCode::BackTab => edit.field = edit.field.previous(),
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
            KeyCode::Tab => edit.next_field(),
            KeyCode::BackTab => edit.previous_field(),
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
        match key.code {
            KeyCode::Char(' ') => self.leader_pending = true,
            KeyCode::Char('?') => self.help_visible = true,
            KeyCode::Char('q') => return Ok(self.request_quit()),
            KeyCode::Char('[') | KeyCode::Left | KeyCode::Char('h') => self.step_year(-1)?,
            KeyCode::Char(']') | KeyCode::Right | KeyCode::Char('l') => self.step_year(1)?,
            KeyCode::Char('0') => self.stats.go_to_current_year(&self.db)?,
            _ => {}
        }
        Ok(Flow::Continue)
    }

    /// Step the dashboard a year back or forward, saying so at either end.
    fn step_year(&mut self, direction: i64) -> Result<()> {
        let moved = if direction < 0 {
            self.stats.previous_year(&self.db)?
        } else {
            self.stats.next_year(&self.db)?
        };
        if !moved {
            let edge = if direction < 0 { "earliest" } else { "latest" };
            self.set_flash(
                Tone::Info,
                format!("{} is the {edge} year with entries", self.stats.year),
            );
        }
        Ok(())
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
