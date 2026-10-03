//! Reads and writes for the four tables, plus the arithmetic the entry form needs.

use chrono::NaiveDate;
use rusqlite::{Connection, OptionalExtension, params};

use super::DbError;
use super::models::{Category, Entry, NewEntry, Project, ProjectMetrics, Status, Type};
use crate::MIN_SESSION_HOURS;

// ---------------------------------------------------------------- types

pub fn insert_type(
    conn: &Connection,
    name: &str,
    description: Option<&str>,
) -> Result<i64, DbError> {
    conn.execute(
        "INSERT INTO types (name, description) VALUES (?1, ?2)",
        params![name, description],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Insert if absent, returning the id either way.
pub fn ensure_type(conn: &Connection, name: &str) -> Result<i64, DbError> {
    if let Some(id) = type_id_by_name(conn, name)? {
        return Ok(id);
    }
    insert_type(conn, name, None)
}

pub fn type_id_by_name(conn: &Connection, name: &str) -> Result<Option<i64>, DbError> {
    Ok(conn
        .query_row("SELECT id FROM types WHERE name = ?1", params![name], |r| {
            r.get(0)
        })
        .optional()?)
}

pub fn list_types(conn: &Connection) -> Result<Vec<Type>, DbError> {
    let mut stmt = conn.prepare("SELECT id, name, description FROM types ORDER BY name")?;
    let rows = stmt.query_map([], |r| {
        Ok(Type {
            id: r.get(0)?,
            name: r.get(1)?,
            description: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// ---------------------------------------------------------------- categories

pub fn insert_category(
    conn: &Connection,
    name: &str,
    type_id: i64,
    description: Option<&str>,
) -> Result<i64, DbError> {
    conn.execute(
        "INSERT INTO categories (name, type_id, description) VALUES (?1, ?2, ?3)",
        params![name, type_id, description],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn ensure_category(conn: &Connection, name: &str, type_id: i64) -> Result<i64, DbError> {
    if let Some(id) = category_id_by_name(conn, name)? {
        return Ok(id);
    }
    insert_category(conn, name, type_id, None)
}

pub fn category_id_by_name(conn: &Connection, name: &str) -> Result<Option<i64>, DbError> {
    Ok(conn
        .query_row(
            "SELECT id FROM categories WHERE name = ?1",
            params![name],
            |r| r.get(0),
        )
        .optional()?)
}

pub fn list_categories(conn: &Connection) -> Result<Vec<Category>, DbError> {
    let mut stmt =
        conn.prepare("SELECT id, name, type_id, description FROM categories ORDER BY name")?;
    let rows = stmt.query_map([], |r| {
        Ok(Category {
            id: r.get(0)?,
            name: r.get(1)?,
            type_id: r.get(2)?,
            description: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn update_category(
    conn: &Connection,
    id: i64,
    name: &str,
    type_id: i64,
    description: Option<&str>,
) -> Result<(), DbError> {
    conn.execute(
        "UPDATE categories SET name = ?2, type_id = ?3, description = ?4 WHERE id = ?1",
        params![id, name, type_id, description],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- projects

pub fn insert_project(
    conn: &Connection,
    title: &str,
    category_id: Option<i64>,
    status: Status,
    description: Option<&str>,
) -> Result<i64, DbError> {
    conn.execute(
        "INSERT INTO projects (title, category_id, status, description)
         VALUES (?1, ?2, ?3, ?4)",
        params![title, category_id, status, description],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn project_id_by_title(conn: &Connection, title: &str) -> Result<Option<i64>, DbError> {
    Ok(conn
        .query_row(
            "SELECT id FROM projects WHERE title = ?1",
            params![title],
            |r| r.get(0),
        )
        .optional()?)
}

const PROJECT_COLUMNS: &str = "id, title, category_id, status, description";

fn project_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        title: r.get(1)?,
        category_id: r.get(2)?,
        status: r.get(3)?,
        description: r.get(4)?,
    })
}

pub fn get_project(conn: &Connection, id: i64) -> Result<Project, DbError> {
    conn.query_row(
        &format!("SELECT {PROJECT_COLUMNS} FROM projects WHERE id = ?1"),
        params![id],
        project_from_row,
    )
    .optional()?
    .ok_or(DbError::NoSuchProject(id))
}

pub fn list_projects(conn: &Connection) -> Result<Vec<Project>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {PROJECT_COLUMNS} FROM projects ORDER BY title"
    ))?;
    let rows = stmt.query_map([], project_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Projects offered by the entry form by default.
///
/// Only open ones, which is what takes the picker from every project ever
/// started down to the handful actually in progress.
pub fn list_open_projects(conn: &Connection) -> Result<Vec<Project>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {PROJECT_COLUMNS} FROM projects WHERE status = 'open' ORDER BY title"
    ))?;
    let rows = stmt.query_map([], project_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn update_project(
    conn: &Connection,
    id: i64,
    title: &str,
    category_id: Option<i64>,
    status: Status,
    description: Option<&str>,
) -> Result<(), DbError> {
    let changed = conn.execute(
        "UPDATE projects
            SET title = ?2, category_id = ?3, status = ?4, description = ?5
          WHERE id = ?1",
        params![id, title, category_id, status, description],
    )?;
    if changed == 0 {
        return Err(DbError::NoSuchProject(id));
    }
    Ok(())
}

pub fn set_project_status(conn: &Connection, id: i64, status: Status) -> Result<(), DbError> {
    let changed = conn.execute(
        "UPDATE projects SET status = ?2 WHERE id = ?1",
        params![id, status],
    )?;
    if changed == 0 {
        return Err(DbError::NoSuchProject(id));
    }
    Ok(())
}

// ---------------------------------------------------------------- entries

/// Reject a session shorter than the minimum before SQLite has to.
///
/// The `CHECK` constraint is the backstop; this exists so the caller gets an
/// error naming the rule rather than a constraint violation. `NaN` is refused
/// explicitly, because it compares false against every bound and would otherwise
/// slip past a plain `<` check and land in the database.
pub fn validate_hours(hours: f64) -> Result<(), DbError> {
    if hours.is_nan() || hours < MIN_SESSION_HOURS {
        return Err(DbError::SessionTooShort {
            got: hours,
            minimum: MIN_SESSION_HOURS,
        });
    }
    Ok(())
}

pub fn insert_entry(conn: &Connection, entry: &NewEntry) -> Result<i64, DbError> {
    validate_hours(entry.hours)?;
    conn.execute(
        "INSERT INTO entries (entry_date, project_id, session_word_count, hours, notes)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            entry.entry_date,
            entry.project_id,
            entry.session_word_count,
            entry.hours,
            entry.notes
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn update_entry(conn: &Connection, id: i64, entry: &NewEntry) -> Result<(), DbError> {
    validate_hours(entry.hours)?;
    conn.execute(
        "UPDATE entries
            SET entry_date = ?2, project_id = ?3, session_word_count = ?4,
                hours = ?5, notes = ?6
          WHERE id = ?1",
        params![
            id,
            entry.entry_date,
            entry.project_id,
            entry.session_word_count,
            entry.hours,
            entry.notes
        ],
    )?;
    Ok(())
}

pub fn delete_entry(conn: &Connection, id: i64) -> Result<(), DbError> {
    conn.execute("DELETE FROM entries WHERE id = ?1", params![id])?;
    Ok(())
}

const ENTRY_COLUMNS: &str = "id, entry_date, project_id, session_word_count, hours, notes";

fn entry_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: r.get(0)?,
        entry_date: r.get(1)?,
        project_id: r.get(2)?,
        session_word_count: r.get(3)?,
        hours: r.get(4)?,
        notes: r.get(5)?,
    })
}

/// Most recent entries first, for the history screen.
pub fn recent_entries(conn: &Connection, limit: i64) -> Result<Vec<Entry>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {ENTRY_COLUMNS} FROM entries ORDER BY entry_date DESC, id DESC LIMIT ?1"
    ))?;
    let rows = stmt.query_map(params![limit], entry_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn entries_for_project(conn: &Connection, project_id: i64) -> Result<Vec<Entry>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {ENTRY_COLUMNS} FROM entries WHERE project_id = ?1 ORDER BY entry_date, id"
    ))?;
    let rows = stmt.query_map(params![project_id], entry_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// ---------------------------------------------------------------- derived

pub fn project_metrics(conn: &Connection, project_id: i64) -> Result<ProjectMetrics, DbError> {
    let status = get_project(conn, project_id)?.status;
    let (start, last, total, days, hours, count): (
        Option<NaiveDate>,
        Option<NaiveDate>,
        i64,
        i64,
        f64,
        i64,
    ) = conn.query_row(
        "SELECT MIN(entry_date), MAX(entry_date),
                COALESCE(SUM(session_word_count), 0),
                COUNT(DISTINCT entry_date),
                COALESCE(SUM(hours), 0.0),
                COUNT(*)
           FROM entries WHERE project_id = ?1",
        params![project_id],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )?;
    Ok(ProjectMetrics {
        project_id,
        start_date: start,
        // Both closed statuses get an end date: the writer stopped on a known day.
        end_date: if status.is_closed() { last } else { None },
        total_word_count: total,
        days_written: days,
        hours_written: hours,
        entry_count: count,
    })
}

/// Words already logged against a project.
///
/// This is the reference the entry form subtracts from. It is deliberately not
/// date-ordered: the number the writer types is the manuscript's count right
/// now, so it is measured against everything logged, which is also why
/// back-dating an entry needs no recalculation.
pub fn project_logged_total(conn: &Connection, project_id: i64) -> Result<i64, DbError> {
    Ok(conn.query_row(
        "SELECT COALESCE(SUM(session_word_count), 0) FROM entries WHERE project_id = ?1",
        params![project_id],
        |r| r.get(0),
    )?)
}

pub fn project_entry_count(conn: &Connection, project_id: i64) -> Result<i64, DbError> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM entries WHERE project_id = ?1",
        params![project_id],
        |r| r.get(0),
    )?)
}

/// What the entry form shows while the writer types.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionPreview {
    pub session_word_count: i64,
    /// True when this is the project's first entry, so the typed number was
    /// taken as the session count rather than as a manuscript total.
    pub first_entry: bool,
    pub words_per_hour: Option<f64>,
}

/// Work out the session count from what the writer typed.
///
/// On a project's first entry there is no prior total to subtract from, so the
/// typed number *is* the session. That is how a manuscript begun before tracking
/// gets onto the books now that no baseline is stored.
pub fn preview_session(
    conn: &Connection,
    project_id: i64,
    typed_word_count: i64,
    hours: f64,
) -> Result<SessionPreview, DbError> {
    let first_entry = project_entry_count(conn, project_id)? == 0;
    let session = if first_entry {
        typed_word_count
    } else {
        typed_word_count - project_logged_total(conn, project_id)?
    };
    Ok(SessionPreview {
        session_word_count: session,
        first_entry,
        // Negative is meaningful here: a revision day that cuts words is real
        // work. Undefined is reserved for a zero denominator.
        words_per_hour: (hours > 0.0).then(|| session as f64 / hours),
    })
}
