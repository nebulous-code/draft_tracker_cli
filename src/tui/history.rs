//! Entry history: browse what was logged, and fix or remove a row.
//!
//! This is what makes a mistyped word count recoverable. Note that the number
//! edited here is the session count itself, not a manuscript total: the entry
//! form derives a session from a running total, but once stored there is no
//! unambiguous total to re-derive it from, so the honest thing to edit is the
//! figure actually held in the row.

use chrono::NaiveDate;

use super::entry::Mode;
use crate::MIN_SESSION_HOURS;
use crate::db::models::{NewEntry, Status};
use crate::db::repo::EntryWithProject;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    Date,
    SessionWords,
    Hours,
    Notes,
}

impl EditField {
    pub const ORDER: [EditField; 4] = [
        EditField::Date,
        EditField::SessionWords,
        EditField::Hours,
        EditField::Notes,
    ];

    pub fn label(self) -> &'static str {
        match self {
            EditField::Date => "Date",
            EditField::SessionWords => "Session WC",
            EditField::Hours => "Hours",
            EditField::Notes => "Notes",
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
}

/// Why an edit will not save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditProblem {
    BadDate(String),
    BadWords(String),
    BadHours(String),
    HoursTooShort(String),
}

impl std::fmt::Display for EditProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditProblem::BadDate(v) => write!(f, "date {v:?} is not a date; use YYYY-MM-DD"),
            EditProblem::BadWords(v) => write!(f, "word count {v:?} is not a whole number"),
            EditProblem::BadHours(v) => write!(f, "hours {v:?} is not a number"),
            EditProblem::HoursTooShort(v) => write!(
                f,
                "hours {v:?} is below the {MIN_SESSION_HOURS} minimum; a minute counts as fifteen"
            ),
        }
    }
}

/// The edit panel, open over one entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub entry_id: i64,
    pub project_id: i64,
    pub project_title: String,
    pub date: String,
    pub session_words: String,
    pub hours: String,
    pub notes: String,
    pub field: EditField,
    pub mode: Mode,
    original: (String, String, String, String),
}

impl Edit {
    fn new(row: &EntryWithProject) -> Self {
        let date = row.entry.entry_date.to_string();
        let words = row.entry.session_word_count.to_string();
        let hours = format_hours(row.entry.hours);
        let notes = row.entry.notes.clone().unwrap_or_default();
        Self {
            entry_id: row.entry.id,
            project_id: row.entry.project_id,
            project_title: row.project_title.clone(),
            date: date.clone(),
            session_words: words.clone(),
            hours: hours.clone(),
            notes: notes.clone(),
            field: EditField::SessionWords,
            mode: Mode::Normal,
            original: (date, words, hours, notes),
        }
    }

    pub fn text_of(&self, field: EditField) -> &str {
        match field {
            EditField::Date => &self.date,
            EditField::SessionWords => &self.session_words,
            EditField::Hours => &self.hours,
            EditField::Notes => &self.notes,
        }
    }

    fn text_mut(&mut self) -> &mut String {
        match self.field {
            EditField::Date => &mut self.date,
            EditField::SessionWords => &mut self.session_words,
            EditField::Hours => &mut self.hours,
            EditField::Notes => &mut self.notes,
        }
    }

    pub fn is_changed(&self) -> bool {
        self.original
            != (
                self.date.clone(),
                self.session_words.clone(),
                self.hours.clone(),
                self.notes.clone(),
            )
    }

    pub fn push_char(&mut self, c: char) {
        if self.mode == Mode::Insert {
            self.text_mut().push(c);
        }
    }

    pub fn backspace(&mut self) {
        if self.mode == Mode::Insert {
            self.text_mut().pop();
        }
    }

    pub fn parsed_date(&self) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(self.date.trim(), "%Y-%m-%d").ok()
    }

    pub fn parsed_words(&self) -> Option<i64> {
        let raw: String = self.session_words.chars().filter(|c| *c != ',').collect();
        let raw = raw.trim();
        (!raw.is_empty()).then(|| raw.parse().ok()).flatten()
    }

    pub fn parsed_hours(&self) -> Option<f64> {
        let raw = self.hours.trim();
        if raw.is_empty() {
            return None;
        }
        raw.parse::<f64>().ok().filter(|h| !h.is_nan())
    }

    pub fn problems(&self) -> Vec<EditProblem> {
        let mut problems = Vec::new();
        if self.parsed_date().is_none() {
            problems.push(EditProblem::BadDate(self.date.trim().to_string()));
        }
        if self.parsed_words().is_none() {
            problems.push(EditProblem::BadWords(self.session_words.trim().to_string()));
        }
        match self.parsed_hours() {
            None => problems.push(EditProblem::BadHours(self.hours.trim().to_string())),
            Some(h) if h < MIN_SESSION_HOURS => {
                problems.push(EditProblem::HoursTooShort(self.hours.trim().to_string()));
            }
            Some(_) => {}
        }
        problems
    }

    pub fn to_new_entry(&self) -> Result<NewEntry, Vec<EditProblem>> {
        let problems = self.problems();
        if !problems.is_empty() {
            return Err(problems);
        }
        Ok(NewEntry {
            entry_date: self.parsed_date().expect("validated"),
            project_id: self.project_id,
            session_word_count: self.parsed_words().expect("validated"),
            hours: self.parsed_hours().expect("validated"),
            notes: {
                let trimmed = self.notes.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            },
        })
    }
}

/// `1` rather than `1.0`, but `1.25` kept, so the field reads as it was typed.
pub fn format_hours(hours: f64) -> String {
    if (hours.fract()).abs() < f64::EPSILON {
        format!("{hours:.0}")
    } else {
        let text = format!("{hours}");
        text
    }
}

pub struct History {
    rows: Vec<EntryWithProject>,
    pub filter: String,
    pub mode: Mode,
    pub edit: Option<Edit>,
    selected: usize,
    pub confirming_delete: bool,
    pending_d: bool,
}

impl History {
    pub fn new(rows: Vec<EntryWithProject>) -> Self {
        Self {
            rows,
            filter: String::new(),
            mode: Mode::Normal,
            edit: None,
            selected: 0,
            confirming_delete: false,
            pending_d: false,
        }
    }

    /// Reload after a change, keeping the cursor on the same entry when it is
    /// still there. A delete leaves it on the row that took its place.
    pub fn refresh(&mut self, rows: Vec<EntryWithProject>) {
        let previous = self.selected_row().map(|r| r.entry.id);
        self.rows = rows;
        if let Some(id) = previous
            && let Some(index) = self.visible().iter().position(|r| r.entry.id == id)
        {
            self.selected = index;
        }
        self.clamp();
    }

    pub fn visible(&self) -> Vec<&EntryWithProject> {
        let needle = self.filter.to_lowercase();
        self.rows
            .iter()
            .filter(|r| needle.is_empty() || r.project_title.to_lowercase().contains(&needle))
            .collect()
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected_row(&self) -> Option<&EntryWithProject> {
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
        if len > 0 {
            self.selected = (self.selected + 1) % len;
        }
    }

    pub fn select_previous(&mut self) {
        let len = self.visible().len();
        if len > 0 {
            self.selected = (self.selected + len - 1) % len;
        }
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
    }

    pub fn select_last(&mut self) {
        self.selected = self.visible().len().saturating_sub(1);
    }

    pub fn begin_filter(&mut self) {
        self.mode = Mode::Filter;
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

    pub fn leave_filter(&mut self) {
        self.mode = Mode::Normal;
    }

    /// Open the edit panel over the selected entry.
    pub fn begin_edit(&mut self) -> bool {
        match self.selected_row() {
            Some(row) => {
                self.edit = Some(Edit::new(row));
                true
            }
            None => false,
        }
    }

    /// Returns whether anything typed was thrown away, so the caller can say so.
    pub fn cancel_edit(&mut self) -> bool {
        match self.edit.take() {
            Some(edit) => edit.is_changed(),
            None => false,
        }
    }

    /// `dd` needs two keystrokes; this records the first.
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

    /// True when the edit panel holds changes that quitting would lose.
    pub fn has_unsaved_input(&self) -> bool {
        self.edit.as_ref().is_some_and(|e| e.is_changed())
    }

    /// One row of the list, already formatted.
    pub fn row_cells(
        row: &EntryWithProject,
        project_width: usize,
    ) -> (String, String, String, String, String) {
        (
            row.entry.entry_date.to_string(),
            truncate(&row.project_title, project_width),
            super::draw::thousands(row.entry.session_word_count),
            format!("{:.2}", row.entry.hours),
            row.entry.notes.clone().unwrap_or_default(),
        )
    }

    pub fn row_status(row: &EntryWithProject) -> Status {
        row.project_status
    }
}

/// Cut to width, marking that something was cut.
pub fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let keep = width.saturating_sub(1);
    let mut out: String = text.chars().take(keep).collect();
    out.push('…');
    out
}
