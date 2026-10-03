//! Dashboard queries.
//!
//! Every one takes an optional year and resolves lifetime when it is absent, so
//! a single query serves both sections of the stats screen and the year is never
//! hardcoded. That is what lets the screen step back through previous years.

use rusqlite::{Connection, params};

use super::DbError;
use super::models::Status;

/// SQLite compares `strftime('%Y', ...)` output as text, so the year is bound
/// as a string. `NULL` means every year.
fn year_param(year: Option<i32>) -> Option<String> {
    year.map(|y| format!("{y:04}"))
}

/// Totals for a year, or for all time when `year` is `None`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Totals {
    pub words: i64,
    pub hours: f64,
}

impl Totals {
    pub fn words_per_hour(&self) -> Option<f64> {
        (self.hours > 0.0).then(|| self.words as f64 / self.hours)
    }
}

pub fn totals(conn: &Connection, year: Option<i32>) -> Result<Totals, DbError> {
    let y = year_param(year);
    let (words, hours) = conn.query_row(
        "SELECT COALESCE(SUM(session_word_count), 0), COALESCE(SUM(hours), 0.0)
           FROM entries
          WHERE ?1 IS NULL OR strftime('%Y', entry_date) = ?1",
        params![y],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(Totals { words, hours })
}

/// Count of completed projects, lifetime or ending within a year.
///
/// Abandoned projects are excluded. They have an end date like any closed
/// project, but counting work that was given up on as finished is exactly the
/// lie the old boolean told.
pub fn completed_project_count(conn: &Connection, year: Option<i32>) -> Result<i64, DbError> {
    let y = year_param(year);
    Ok(conn.query_row(
        "SELECT COUNT(*)
           FROM projects p
          WHERE p.status = 'completed'
            AND (?1 IS NULL
                 OR strftime('%Y', (SELECT MAX(e.entry_date) FROM entries e
                                     WHERE e.project_id = p.id)) = ?1)",
        params![y],
        |r| r.get(0),
    )?)
}

/// One bar of the projects graph.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectWords {
    pub project_id: i64,
    pub title: String,
    pub status: Status,
    pub words: i64,
}

/// Every project with an entry in the year, largest first.
pub fn project_words(conn: &Connection, year: Option<i32>) -> Result<Vec<ProjectWords>, DbError> {
    let y = year_param(year);
    let mut stmt = conn.prepare(
        "SELECT p.id, p.title, p.status, SUM(e.session_word_count) AS words
           FROM entries e
           JOIN projects p ON p.id = e.project_id
          WHERE ?1 IS NULL OR strftime('%Y', e.entry_date) = ?1
          GROUP BY p.id, p.title, p.status
          ORDER BY words DESC, p.title",
    )?;
    let rows = stmt.query_map(params![y], |r| {
        Ok(ProjectWords {
            project_id: r.get(0)?,
            title: r.get(1)?,
            status: r.get(2)?,
            words: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Words per year, oldest first, for the year-over-year graph.
pub fn words_by_year(conn: &Connection) -> Result<Vec<(i32, i64)>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT CAST(strftime('%Y', entry_date) AS INTEGER) AS y,
                SUM(session_word_count)
           FROM entries
          GROUP BY y
          ORDER BY y",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// One segment of the category distribution bar.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryWords {
    pub category: String,
    pub words: i64,
}

/// Words per category for a year.
///
/// A project with no category is bucketed as `Uncategorized` rather than
/// dropped, so the bar always accounts for the whole year.
pub fn category_words(conn: &Connection, year: Option<i32>) -> Result<Vec<CategoryWords>, DbError> {
    let y = year_param(year);
    let mut stmt = conn.prepare(
        "SELECT COALESCE(c.name, 'Uncategorized') AS category,
                SUM(e.session_word_count) AS words
           FROM entries e
           JOIN projects p ON p.id = e.project_id
           LEFT JOIN categories c ON c.id = p.category_id
          WHERE ?1 IS NULL OR strftime('%Y', e.entry_date) = ?1
          GROUP BY category
          ORDER BY words DESC, category",
    )?;
    let rows = stmt.query_map(params![y], |r| {
        Ok(CategoryWords {
            category: r.get(0)?,
            words: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Years that have any entry, oldest first. Drives the year selector.
pub fn years_with_entries(conn: &Connection) -> Result<Vec<i32>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT CAST(strftime('%Y', entry_date) AS INTEGER) AS y
           FROM entries ORDER BY y",
    )?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}
