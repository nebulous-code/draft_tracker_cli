//! The entry form: the screen used every day.
//!
//! All state transitions are plain methods on [`EntryForm`] so the behaviour can
//! be tested without a terminal. Drawing is a separate pass that only reads.

use chrono::{Local, NaiveDate};

use crate::MIN_SESSION_HOURS;
use crate::db::models::{NewEntry, Status};

/// A project as the picker sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    pub id: i64,
    pub title: String,
    pub status: Status,
    /// Words already logged, the figure a typed total is measured against.
    pub logged_words: i64,
    pub entry_count: i64,
}

impl Choice {
    /// No prior total means the typed number is the session itself.
    pub fn is_first_entry(&self) -> bool {
        self.entry_count == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Date,
    Project,
    Hours,
    ProjectWords,
    Notes,
}

impl Field {
    pub const ORDER: [Field; 5] = [
        Field::Date,
        Field::Project,
        Field::Hours,
        Field::ProjectWords,
        Field::Notes,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Field::Date => "Date",
            Field::Project => "Project",
            Field::Hours => "Hours",
            Field::ProjectWords => "Project WC",
            Field::Notes => "Notes",
        }
    }

    fn index(self) -> usize {
        Self::ORDER
            .iter()
            .position(|f| *f == self)
            .expect("field is in ORDER")
    }

    pub fn next(self) -> Field {
        Self::ORDER[(self.index() + 1) % Self::ORDER.len()]
    }

    pub fn previous(self) -> Field {
        let i = self.index();
        Self::ORDER[(i + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }

    /// Whether typing edits text, as opposed to moving through a list.
    pub fn is_text(self) -> bool {
        !matches!(self, Field::Project)
    }
}

/// What the form worked out from what has been typed so far.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Preview {
    pub session_words: Option<i64>,
    pub words_per_hour: Option<f64>,
    /// The typed number was taken as the session, not as a manuscript total.
    pub first_entry: bool,
}

/// Why the form will not submit yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    NoProjects,
    NoProjectSelected,
    BadDate(String),
    BadHours(String),
    HoursTooShort(String),
    BadProjectWords(String),
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::NoProjects => write!(
                f,
                "no open projects; add one on the projects screen, or widen the list with A"
            ),
            Problem::NoProjectSelected => write!(f, "choose a project"),
            Problem::BadDate(v) => write!(f, "date {v:?} is not a date; use YYYY-MM-DD"),
            Problem::BadHours(v) => write!(f, "hours {v:?} is not a number"),
            Problem::HoursTooShort(v) => write!(
                f,
                "hours {v:?} is below the {MIN_SESSION_HOURS} minimum; a minute counts as fifteen"
            ),
            Problem::BadProjectWords(v) => write!(f, "word count {v:?} is not a whole number"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Editing a text field.
    Insert,
    /// Typing into the project filter.
    Filter,
}

pub struct EntryForm {
    pub date: String,
    pub hours: String,
    pub project_words: String,
    pub notes: String,
    pub field: Field,
    pub mode: Mode,
    pub filter: String,
    /// Every project, in the order the picker offers them.
    choices: Vec<Choice>,
    /// Show completed and abandoned projects as well as open ones.
    pub include_closed: bool,
    selected: usize,
}

impl EntryForm {
    pub fn new(choices: Vec<Choice>, today: NaiveDate) -> Self {
        Self {
            date: today.to_string(),
            hours: String::new(),
            project_words: String::new(),
            notes: String::new(),
            field: Field::Project,
            mode: Mode::Normal,
            filter: String::new(),
            choices,
            include_closed: false,
            selected: 0,
        }
    }

    pub fn today(choices: Vec<Choice>) -> Self {
        Self::new(choices, Local::now().date_naive())
    }

    /// Replace the project list, keeping the selection on the same project when
    /// it is still there. Called after a submit changes a project's total.
    pub fn refresh_choices(&mut self, choices: Vec<Choice>) {
        let previous = self.selected_project().map(|c| c.id);
        self.choices = choices;
        self.selected = previous
            .and_then(|id| self.visible().iter().position(|c| c.id == id))
            .unwrap_or(0);
    }

    /// The projects the picker is currently offering.
    pub fn visible(&self) -> Vec<&Choice> {
        let needle = self.filter.to_lowercase();
        self.choices
            .iter()
            .filter(|c| self.include_closed || c.status == Status::Open)
            .filter(|c| needle.is_empty() || c.title.to_lowercase().contains(&needle))
            .collect()
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected_project(&self) -> Option<&Choice> {
        self.visible().get(self.selected).copied()
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

    pub fn toggle_include_closed(&mut self) {
        let previous = self.selected_project().map(|c| c.id);
        self.include_closed = !self.include_closed;
        // Keep the cursor on the same project where possible; a narrowing list
        // would otherwise leave the selection pointing at something else.
        self.selected = previous
            .and_then(|id| self.visible().iter().position(|c| c.id == id))
            .unwrap_or(0);
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        let len = self.visible().len();
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
    }

    pub fn next_field(&mut self) {
        self.field = self.field.next();
        self.mode = Mode::Normal;
    }

    pub fn previous_field(&mut self) {
        self.field = self.field.previous();
        self.mode = Mode::Normal;
    }

    fn text_mut(&mut self) -> Option<&mut String> {
        match self.field {
            Field::Date => Some(&mut self.date),
            Field::Hours => Some(&mut self.hours),
            Field::ProjectWords => Some(&mut self.project_words),
            Field::Notes => Some(&mut self.notes),
            Field::Project => None,
        }
    }

    pub fn text_of(&self, field: Field) -> &str {
        match field {
            Field::Date => &self.date,
            Field::Hours => &self.hours,
            Field::ProjectWords => &self.project_words,
            Field::Notes => &self.notes,
            Field::Project => "",
        }
    }

    pub fn push_char(&mut self, c: char) {
        match self.mode {
            Mode::Filter => {
                self.filter.push(c);
                self.selected = 0;
            }
            Mode::Insert => {
                if let Some(text) = self.text_mut() {
                    text.push(c);
                }
            }
            Mode::Normal => {}
        }
    }

    pub fn backspace(&mut self) {
        match self.mode {
            Mode::Filter => {
                self.filter.pop();
                self.selected = 0;
            }
            Mode::Insert => {
                if let Some(text) = self.text_mut() {
                    text.pop();
                }
            }
            Mode::Normal => {}
        }
    }

    pub fn begin_insert(&mut self) {
        self.mode = if self.field.is_text() {
            Mode::Insert
        } else {
            Mode::Filter
        };
    }

    pub fn begin_filter(&mut self) {
        self.field = Field::Project;
        self.mode = Mode::Filter;
    }

    /// Leave insert or filter mode. Clearing the filter here would lose the
    /// narrowing the writer just did, so the filter stays until cleared.
    pub fn leave_insert(&mut self) {
        self.mode = Mode::Normal;
    }

    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.selected = 0;
    }

    /// True when something has been typed that would be lost by quitting.
    ///
    /// The date is excluded: it is filled in by default, so it is not input the
    /// writer would mind losing.
    pub fn has_unsaved_input(&self) -> bool {
        !self.hours.is_empty() || !self.project_words.is_empty() || !self.notes.is_empty()
    }

    pub fn parsed_date(&self) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(self.date.trim(), "%Y-%m-%d").ok()
    }

    pub fn parsed_hours(&self) -> Option<f64> {
        let raw = self.hours.trim();
        if raw.is_empty() {
            return None;
        }
        raw.parse::<f64>().ok().filter(|h| !h.is_nan())
    }

    pub fn parsed_project_words(&self) -> Option<i64> {
        let raw: String = self.project_words.chars().filter(|c| *c != ',').collect();
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        raw.parse::<i64>().ok()
    }

    /// The read-only figures, recalculated as the writer types.
    pub fn preview(&self) -> Preview {
        let Some(choice) = self.selected_project() else {
            return Preview {
                session_words: None,
                words_per_hour: None,
                first_entry: false,
            };
        };
        let first_entry = choice.is_first_entry();
        let session = self.parsed_project_words().map(|typed| {
            if first_entry {
                // No prior total to subtract from, so the typed number is the
                // session. This is how a manuscript begun before tracking gets
                // onto the books.
                typed
            } else {
                typed - choice.logged_words
            }
        });
        // A negative rate is meaningful: a revision day that cuts words is real
        // work. Undefined is reserved for a zero denominator.
        let rate = match (session, self.parsed_hours()) {
            (Some(words), Some(hours)) if hours > 0.0 => Some(words as f64 / hours),
            _ => None,
        };
        Preview {
            session_words: session,
            words_per_hour: rate,
            first_entry,
        }
    }

    /// Everything stopping a submit, in the order the fields appear.
    pub fn problems(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        if self.visible().is_empty() {
            problems.push(if self.choices.is_empty() {
                Problem::NoProjects
            } else {
                Problem::NoProjectSelected
            });
        } else if self.selected_project().is_none() {
            problems.push(Problem::NoProjectSelected);
        }
        if self.parsed_date().is_none() {
            problems.push(Problem::BadDate(self.date.trim().to_string()));
        }
        match self.parsed_hours() {
            None => problems.push(Problem::BadHours(self.hours.trim().to_string())),
            Some(h) if h < MIN_SESSION_HOURS => {
                problems.push(Problem::HoursTooShort(self.hours.trim().to_string()));
            }
            Some(_) => {}
        }
        if self.parsed_project_words().is_none() {
            problems.push(Problem::BadProjectWords(
                self.project_words.trim().to_string(),
            ));
        }
        problems
    }

    /// The entry to write, or everything preventing it.
    pub fn to_new_entry(&self) -> Result<NewEntry, Vec<Problem>> {
        let problems = self.problems();
        if !problems.is_empty() {
            return Err(problems);
        }
        let choice = self.selected_project().expect("validated");
        let session = self.preview().session_words.expect("validated");
        Ok(NewEntry {
            entry_date: self.parsed_date().expect("validated"),
            project_id: choice.id,
            session_word_count: session,
            hours: self.parsed_hours().expect("validated"),
            notes: {
                let trimmed = self.notes.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            },
        })
    }

    /// Clear the per-session fields after a successful submit.
    ///
    /// The date and the project stay: another entry the same day, for this or a
    /// neighbouring project, is the common next action.
    pub fn clear_after_submit(&mut self) {
        self.hours.clear();
        self.project_words.clear();
        self.notes.clear();
        self.mode = Mode::Normal;
        self.field = Field::Project;
    }
}
