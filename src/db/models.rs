//! Domain types stored in, and read back from, the database.

use std::fmt;
use std::str::FromStr;

use chrono::NaiveDate;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

/// Where a project stands.
///
/// `Abandoned` and `Completed` both mean the writer has stopped, and every
/// derived calculation treats them the same, including giving both an end date.
/// They are distinct for one reason: work abandoned without being shipped must
/// not be counted as finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Status {
    Open,
    Completed,
    Abandoned,
}

impl Status {
    pub const ALL: [Status; 3] = [Status::Open, Status::Completed, Status::Abandoned];

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::Completed => "completed",
            Status::Abandoned => "abandoned",
        }
    }

    /// True when the writer has stopped work, whether finished or not.
    ///
    /// This is the predicate behind the end date and behind a project being
    /// left out of the entry form's default list.
    pub fn is_closed(self) -> bool {
        !matches!(self, Status::Open)
    }

    /// The next status in the cycle the projects list walks through.
    pub fn next(self) -> Status {
        match self {
            Status::Open => Status::Completed,
            Status::Completed => Status::Abandoned,
            Status::Abandoned => Status::Open,
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parses case-insensitively after trimming, because these values are typed by
/// hand into a CSV where `Abandoned ` means what it says.
impl FromStr for Status {
    type Err = ParseStatusError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "open" => Ok(Status::Open),
            "completed" => Ok(Status::Completed),
            "abandoned" => Ok(Status::Abandoned),
            _ => Err(ParseStatusError(s.to_string())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unrecognised status {0:?}; expected open, completed or abandoned")]
pub struct ParseStatusError(pub String);

impl ToSql for Status {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.as_str()))
    }
}

impl FromSql for Status {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let text = value.as_str()?;
        text.parse().map_err(|e| FromSqlError::Other(Box::new(e)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Type {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub type_id: i64,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub id: i64,
    pub title: String,
    pub category_id: Option<i64>,
    pub status: Status,
    pub description: Option<String>,
}

/// A single writing session.
///
/// `session_word_count` may be negative: a revision day that cuts words is a
/// real working day, and three such days exist in the imported history.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: i64,
    pub entry_date: NaiveDate,
    pub project_id: i64,
    pub session_word_count: i64,
    pub hours: f64,
    pub notes: Option<String>,
}

/// A new session, before it has an id.
#[derive(Debug, Clone, PartialEq)]
pub struct NewEntry {
    pub entry_date: NaiveDate,
    pub project_id: i64,
    pub session_word_count: i64,
    pub hours: f64,
    pub notes: Option<String>,
}

/// Everything the project screen shows, derived from a project's entries.
///
/// Rates are `None` rather than zero when their denominator is zero, so that
/// "no basis to calculate" is never displayed as a real figure of nothing.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectMetrics {
    pub project_id: i64,
    pub start_date: Option<NaiveDate>,
    pub end_date: Option<NaiveDate>,
    pub total_word_count: i64,
    pub days_written: i64,
    pub hours_written: f64,
    pub entry_count: i64,
}

impl ProjectMetrics {
    pub fn hours_per_day(&self) -> Option<f64> {
        (self.days_written > 0).then(|| self.hours_written / self.days_written as f64)
    }

    pub fn words_per_hour(&self) -> Option<f64> {
        (self.hours_written > 0.0).then(|| self.total_word_count as f64 / self.hours_written)
    }
}
