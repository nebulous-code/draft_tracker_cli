//! The projects screen: the derived figures, and the one-key status pass.

use super::entry::Mode;
use crate::db::models::Status;
use crate::db::repo::{CategoryRow, ProjectRow};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Title,
    Category,
    Status,
    Description,
}

impl Field {
    pub const ORDER: [Field; 4] = [
        Field::Title,
        Field::Category,
        Field::Status,
        Field::Description,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Field::Title => "Title",
            Field::Category => "Category",
            Field::Status => "Status",
            Field::Description => "Description",
        }
    }

    fn index(self) -> usize {
        Self::ORDER
            .iter()
            .position(|f| *f == self)
            .expect("field is in ORDER")
    }

    pub fn next(self) -> Self {
        Self::ORDER[(self.index() + 1) % Self::ORDER.len()]
    }

    pub fn previous(self) -> Self {
        let i = self.index();
        Self::ORDER[(i + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }

    /// Fields whose value is chosen from a list rather than typed.
    pub fn is_choice(self) -> bool {
        matches!(self, Field::Category | Field::Status)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    EmptyTitle,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::EmptyTitle => write!(f, "a project needs a title"),
        }
    }
}

/// The edit panel, over an existing project or a new one.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    /// `None` while creating.
    pub id: Option<i64>,
    pub title: String,
    pub category_id: Option<i64>,
    pub status: Status,
    pub description: String,
    pub field: Field,
    pub mode: Mode,
    original: (String, Option<i64>, Status, String),
}

impl Edit {
    pub fn existing(row: &ProjectRow) -> Self {
        let title = row.project.title.clone();
        let description = row.project.description.clone().unwrap_or_default();
        Self {
            id: Some(row.project.id),
            title: title.clone(),
            category_id: row.project.category_id,
            status: row.project.status,
            description: description.clone(),
            field: Field::Title,
            mode: Mode::Normal,
            original: (
                title,
                row.project.category_id,
                row.project.status,
                description,
            ),
        }
    }

    pub fn creating(category_id: Option<i64>) -> Self {
        Self {
            id: None,
            title: String::new(),
            category_id,
            status: Status::Open,
            description: String::new(),
            field: Field::Title,
            // A new project starts with the cursor in its title, ready to type:
            // there is nothing to read first.
            mode: Mode::Insert,
            original: (String::new(), category_id, Status::Open, String::new()),
        }
    }

    pub fn is_creating(&self) -> bool {
        self.id.is_none()
    }

    pub fn is_changed(&self) -> bool {
        self.original
            != (
                self.title.clone(),
                self.category_id,
                self.status,
                self.description.clone(),
            )
    }

    pub fn text_of(&self, field: Field) -> &str {
        match field {
            Field::Title => &self.title,
            Field::Description => &self.description,
            _ => "",
        }
    }

    fn text_mut(&mut self) -> Option<&mut String> {
        match self.field {
            Field::Title => Some(&mut self.title),
            Field::Description => Some(&mut self.description),
            _ => None,
        }
    }

    pub fn push_char(&mut self, c: char) {
        if self.mode == Mode::Insert
            && let Some(text) = self.text_mut()
        {
            text.push(c);
        }
    }

    pub fn backspace(&mut self) {
        if self.mode == Mode::Insert
            && let Some(text) = self.text_mut()
        {
            text.pop();
        }
    }

    pub fn problems(&self) -> Vec<Problem> {
        if self.title.trim().is_empty() {
            vec![Problem::EmptyTitle]
        } else {
            Vec::new()
        }
    }

    pub fn description_option(&self) -> Option<String> {
        let trimmed = self.description.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    }
}

pub struct Projects {
    rows: Vec<ProjectRow>,
    categories: Vec<CategoryRow>,
    pub filter: String,
    pub mode: Mode,
    pub edit: Option<Edit>,
    selected: usize,
    pub confirming_delete: bool,
    pending_d: bool,
    /// Show completed and abandoned projects as well as open ones.
    ///
    /// Off by default: most of a writing life accumulates as finished work, and
    /// the handful still open is what a writer is usually looking at.
    show_all: bool,
    /// A project held in view even though the current filter excludes it.
    ///
    /// Marking a project completed in the open-only view would otherwise make it
    /// vanish under the cursor: the writer could not see what the keystroke did,
    /// and could not press the key again to carry on to abandoned. It is held
    /// until the cursor moves off it.
    pinned: Option<i64>,
}

impl Projects {
    pub fn new(rows: Vec<ProjectRow>, categories: Vec<CategoryRow>) -> Self {
        Self {
            rows,
            categories,
            filter: String::new(),
            mode: Mode::Normal,
            edit: None,
            selected: 0,
            confirming_delete: false,
            pending_d: false,
            show_all: false,
            pinned: None,
        }
    }

    /// Keep a project in view that the filter would otherwise drop.
    pub fn pin(&mut self, id: i64) {
        self.pinned = Some(id);
    }

    pub fn pinned(&self) -> Option<i64> {
        self.pinned
    }

    /// Drop the pin, holding the cursor on whichever project it names.
    ///
    /// Taken before a move so the move lands where the writer expects: unpinning
    /// shortens the list, which would otherwise shift every index after it.
    fn release_pin(&mut self) {
        if self.pinned.is_none() {
            return;
        }
        let staying = self.selected_row().map(|r| r.project.id);
        self.pinned = None;
        self.selected = staying
            .and_then(|id| self.visible().iter().position(|r| r.project.id == id))
            .unwrap_or(self.selected);
        self.clamp();
    }

    pub fn show_all(&self) -> bool {
        self.show_all
    }

    /// How many projects exist, whatever the current view shows.
    pub fn total(&self) -> usize {
        self.rows.len()
    }

    /// Flip between every project and the open ones, keeping the cursor on the
    /// same project where it survives the change.
    pub fn toggle_show_all(&mut self) {
        let previous = self.selected_row().map(|r| r.project.id);
        self.show_all = !self.show_all;
        self.pinned = None;
        self.selected = previous
            .and_then(|id| self.visible().iter().position(|r| r.project.id == id))
            .unwrap_or(0);
        self.clamp();
    }

    pub fn refresh(&mut self, rows: Vec<ProjectRow>, categories: Vec<CategoryRow>) {
        let previous = self.selected_row().map(|r| r.project.id);
        self.rows = rows;
        self.categories = categories;
        if let Some(id) = previous
            && let Some(index) = self.visible().iter().position(|r| r.project.id == id)
        {
            self.selected = index;
        }
        self.clamp();
    }

    pub fn categories(&self) -> &[CategoryRow] {
        &self.categories
    }

    pub fn category_name(&self, id: Option<i64>) -> String {
        match id {
            None => "Uncategorized".to_string(),
            Some(id) => self
                .categories
                .iter()
                .find(|c| c.category.id == id)
                .map(|c| c.category.name.clone())
                .unwrap_or_else(|| "Uncategorized".to_string()),
        }
    }

    pub fn visible(&self) -> Vec<&ProjectRow> {
        let needle = self.filter.to_lowercase();
        self.rows
            .iter()
            .filter(|r| {
                self.show_all
                    || r.project.status == Status::Open
                    || self.pinned == Some(r.project.id)
            })
            .filter(|r| {
                needle.is_empty()
                    || r.project.title.to_lowercase().contains(&needle)
                    || r.category_name
                        .as_deref()
                        .is_some_and(|c| c.to_lowercase().contains(&needle))
            })
            .collect()
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected_row(&self) -> Option<&ProjectRow> {
        self.visible().get(self.selected).copied()
    }

    fn clamp(&mut self) {
        let len = self.visible().len();
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
    }

    pub fn select_next(&mut self) {
        let len = self.visible().len();
        if len == 0 {
            return;
        }
        let target = self
            .visible()
            .get((self.selected + 1) % len)
            .map(|r| r.project.id);
        self.move_to(target);
    }

    pub fn select_previous(&mut self) {
        let len = self.visible().len();
        if len == 0 {
            return;
        }
        let target = self
            .visible()
            .get((self.selected + len - 1) % len)
            .map(|r| r.project.id);
        self.move_to(target);
    }

    /// Land on `target`, then release any pin, so the row the cursor left can
    /// fall out of view without taking the cursor with it.
    fn move_to(&mut self, target: Option<i64>) {
        if let Some(id) = target
            && let Some(index) = self.visible().iter().position(|r| r.project.id == id)
        {
            self.selected = index;
        }
        self.release_pin();
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
        self.release_pin();
    }

    pub fn select_last(&mut self) {
        self.release_pin();
        self.selected = self.visible().len().saturating_sub(1);
    }

    pub fn begin_filter(&mut self) {
        self.mode = Mode::Filter;
    }

    pub fn leave_filter(&mut self) {
        self.mode = Mode::Normal;
    }

    pub fn push_filter_char(&mut self, c: char) {
        self.filter.push(c);
        self.selected = 0;
    }

    pub fn backspace_filter(&mut self) {
        self.filter.pop();
        self.selected = 0;
    }

    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.selected = 0;
    }

    pub fn begin_edit(&mut self) -> bool {
        match self.selected_row() {
            Some(row) => {
                self.edit = Some(Edit::existing(row));
                true
            }
            None => false,
        }
    }

    /// Start a new project, pre-set to the category being looked at.
    pub fn begin_create(&mut self) {
        let category = self.selected_row().and_then(|r| r.project.category_id);
        self.edit = Some(Edit::creating(category));
    }

    pub fn cancel_edit(&mut self) -> bool {
        match self.edit.take() {
            // An untouched new project is not a loss worth reporting.
            Some(edit) => edit.is_changed(),
            None => false,
        }
    }

    pub fn has_unsaved_input(&self) -> bool {
        self.edit.as_ref().is_some_and(|e| e.is_changed())
    }

    /// Step a choice field. `step` is +1 or -1.
    pub fn cycle_choice(&mut self, step: i64) {
        let options: Vec<Option<i64>> = std::iter::once(None)
            .chain(self.categories.iter().map(|c| Some(c.category.id)))
            .collect();
        let Some(edit) = self.edit.as_mut() else {
            return;
        };
        match edit.field {
            Field::Status => {
                edit.status = if step >= 0 {
                    edit.status.next()
                } else {
                    edit.status.next().next()
                };
            }
            Field::Category => {
                let current = options
                    .iter()
                    .position(|o| *o == edit.category_id)
                    .unwrap_or(0) as i64;
                let len = options.len() as i64;
                let next = ((current + step) % len + len) % len;
                edit.category_id = options[next as usize];
            }
            _ => {}
        }
    }

    /// `x` on the list: step the selected project's status without opening a panel.
    pub fn next_status_of_selected(&self) -> Option<(i64, String, Status)> {
        self.selected_row().map(|r| {
            (
                r.project.id,
                r.project.title.clone(),
                r.project.status.next(),
            )
        })
    }

    pub fn press_d(&mut self) -> bool {
        if self.pending_d {
            self.pending_d = false;
            true
        } else {
            self.pending_d = true;
            false
        }
    }

    pub fn clear_pending_d(&mut self) {
        self.pending_d = false;
    }

    pub fn pending_d(&self) -> bool {
        self.pending_d
    }
}
