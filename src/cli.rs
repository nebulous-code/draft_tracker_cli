//! The command line path for the thing done most often: logging one session.
//!
//! The terminal interface is for reviewing, fixing and planning. Logging a day's
//! work is a single short command, so it does not need a screen, and a writer
//! with this binary on their path should not have to open one.
//!
//! Resolving and summarising are separate from committing, so the whole flow can
//! be tested without a terminal: `plan` works out what would be written, the
//! caller shows it and asks, and only then does `commit` run.

use std::fmt;
use std::io::Write;

use chrono::{Local, NaiveDate};

use crate::MIN_SESSION_HOURS;
use crate::db::models::{NewEntry, Status};
use crate::db::{Db, repo};
use crate::import::source::parse_date_text;

/// What the writer asked for on the command line.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// All or part of a project's title, matched without regard to case.
    pub project: String,
    /// The project's new total word count, or the session's words on a first entry.
    pub words: i64,
    pub hours: f64,
    pub date: Option<NaiveDate>,
    pub notes: Option<String>,
    /// Allow a completed or abandoned project to be matched.
    pub include_closed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("no open project matches {query:?}")]
    NoMatch { query: String, open: Vec<String> },

    #[error("{query:?} matches {} projects", matches.len())]
    Ambiguous { query: String, matches: Vec<String> },

    #[error(
        "{title:?} is {status}; reopen it with the projects screen, or pass --any to log against it"
    )]
    Closed { title: String, status: Status },

    #[error("there are no projects yet; create one in the interface first")]
    NoProjects,

    #[error("a session must be at least {MIN_SESSION_HOURS} hours; got {0}")]
    HoursTooShort(f64),

    #[error(transparent)]
    Db(#[from] crate::db::DbError),
}

/// A resolved entry, ready to be shown and then written.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned {
    pub project_id: i64,
    pub project_title: String,
    pub date: NaiveDate,
    pub hours: f64,
    pub notes: Option<String>,
    /// What the writer typed: a running total, or the session itself if first.
    pub typed_words: i64,
    pub session_words: i64,
    /// Words already logged against the project.
    pub logged_before: i64,
    /// True when the typed number was taken as the session rather than a total.
    pub first_entry: bool,
}

impl Planned {
    pub fn words_per_hour(&self) -> Option<f64> {
        (self.hours > 0.0).then(|| self.session_words as f64 / self.hours)
    }
}

/// Work out what would be written, without writing it.
pub fn plan(db: &Db, request: &Request) -> Result<Planned, LogError> {
    if request.hours.is_nan() || request.hours < MIN_SESSION_HOURS {
        return Err(LogError::HoursTooShort(request.hours));
    }
    let conn = db.conn();
    let project = resolve_project(db, &request.project, request.include_closed)?;

    let logged_before = repo::project_logged_total(conn, project.id)?;
    let first_entry = repo::project_entry_count(conn, project.id)? == 0;
    // The same rule the entry form uses: with no prior total to subtract from,
    // the number given is the session itself.
    let session_words = if first_entry {
        request.words
    } else {
        request.words - logged_before
    };

    Ok(Planned {
        project_id: project.id,
        project_title: project.title,
        date: request.date.unwrap_or_else(|| Local::now().date_naive()),
        hours: request.hours,
        notes: request.notes.clone(),
        typed_words: request.words,
        session_words,
        logged_before,
        first_entry,
    })
}

/// Write a planned entry. Returns its id.
pub fn commit(db: &Db, planned: &Planned) -> Result<i64, LogError> {
    Ok(repo::insert_entry(
        db.conn(),
        &NewEntry {
            entry_date: planned.date,
            project_id: planned.project_id,
            session_word_count: planned.session_words,
            hours: planned.hours,
            notes: planned.notes.clone(),
        },
    )?)
}

struct Matched {
    id: i64,
    title: String,
}

/// Find the project a partial title refers to.
///
/// An exact title wins outright, so a project whose name is contained in another
/// is still reachable. Otherwise a single substring match is taken and anything
/// else is reported with the candidates, because guessing which project a day's
/// work belongs to is not a thing to do quietly.
fn resolve_project(db: &Db, query: &str, include_closed: bool) -> Result<Matched, LogError> {
    let conn = db.conn();
    let all = repo::list_projects(conn)?;
    if all.is_empty() {
        return Err(LogError::NoProjects);
    }
    let open_titles: Vec<String> = all
        .iter()
        .filter(|p| p.status == Status::Open)
        .map(|p| p.title.clone())
        .collect();

    let needle = query.trim().to_lowercase();
    let eligible: Vec<_> = all
        .iter()
        .filter(|p| include_closed || p.status == Status::Open)
        .collect();

    if let Some(exact) = eligible.iter().find(|p| p.title.to_lowercase() == needle) {
        return Ok(Matched {
            id: exact.id,
            title: exact.title.clone(),
        });
    }
    let partial: Vec<_> = eligible
        .iter()
        .filter(|p| p.title.to_lowercase().contains(&needle))
        .collect();
    match partial.as_slice() {
        [one] => {
            return Ok(Matched {
                id: one.id,
                title: one.title.clone(),
            });
        }
        [] => {}
        many => {
            return Err(LogError::Ambiguous {
                query: query.to_string(),
                matches: many.iter().map(|p| p.title.clone()).collect(),
            });
        }
    }

    // Nothing open matched. If a closed project does, say so rather than
    // claiming the project does not exist.
    if !include_closed
        && let Some(closed) = all
            .iter()
            .find(|p| p.status != Status::Open && p.title.to_lowercase().contains(&needle))
    {
        return Err(LogError::Closed {
            title: closed.title.clone(),
            status: closed.status,
        });
    }
    Err(LogError::NoMatch {
        query: query.to_string(),
        open: open_titles,
    })
}

/// 17228 reads as 17,228. Word counts are compared at a glance all day.
pub fn thousands(n: i64) -> String {
    let negative = n < 0;
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if negative { format!("-{out}") } else { out }
}

/// The summary shown before asking, so what is about to be written is visible.
pub struct Summary<'a>(pub &'a Planned);

impl fmt::Display for Summary<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let p = self.0;
        writeln!(f, "  Project      {}", p.project_title)?;
        writeln!(f, "  Date         {}", p.date)?;
        writeln!(f, "  Hours        {:.2}", p.hours)?;
        if p.first_entry {
            writeln!(f, "  Words        {}", thousands(p.typed_words))?;
            writeln!(
                f,
                "               first entry: taken as this session, not a total"
            )?;
        } else {
            writeln!(f, "  Project WC   {}", thousands(p.typed_words))?;
            writeln!(
                f,
                "               logged so far {}",
                thousands(p.logged_before)
            )?;
        }
        writeln!(f, "  Session WC   {}", thousands(p.session_words))?;
        match p.words_per_hour() {
            Some(rate) => writeln!(f, "  Words/Hour   {rate:.0}")?,
            None => writeln!(f, "  Words/Hour   -")?,
        }
        if let Some(notes) = &p.notes {
            writeln!(f, "  Notes        {notes}")?;
        }
        if p.session_words < 0 {
            writeln!(
                f,
                "\n  This cuts {} words. A revision day is a real day, so this is allowed.",
                thousands(-p.session_words)
            )?;
        }
        Ok(())
    }
}

/// Print what would be written. Separate from asking, so a caller can decide how.
pub fn write_summary(out: &mut impl Write, planned: &Planned) -> std::io::Result<()> {
    write!(out, "{}", Summary(planned))
}

/// Advice to print when a project cannot be resolved.
pub fn write_error_help(out: &mut impl Write, error: &LogError) -> std::io::Result<()> {
    match error {
        LogError::NoMatch { open, .. } if !open.is_empty() => {
            writeln!(out, "Open projects:")?;
            for title in open {
                writeln!(out, "  {title}")?;
            }
        }
        LogError::Ambiguous { matches, .. } => {
            writeln!(out, "Matches:")?;
            for title in matches {
                writeln!(out, "  {title}")?;
            }
            writeln!(out, "Give enough of the title to pick one.")?;
        }
        _ => {}
    }
    Ok(())
}

/// Parse a date argument: ISO-8601, the month-first forms, or `today`/`yesterday`.
pub fn parse_date_argument(raw: &str) -> Option<NaiveDate> {
    let trimmed = raw.trim();
    match trimmed.to_lowercase().as_str() {
        "today" => return Some(Local::now().date_naive()),
        "yesterday" => {
            return Local::now().date_naive().pred_opt();
        }
        _ => {}
    }
    parse_date_text(trimmed)
}

/// Parse a word count, tolerating the thousands separators a writer will type.
pub fn parse_words(raw: &str) -> Option<i64> {
    let cleaned: String = raw.chars().filter(|c| *c != ',' && *c != '_').collect();
    cleaned.trim().parse().ok()
}
