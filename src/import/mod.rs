//! The one-time import of existing writing journals.
//!
//! The importer carries no knowledge of any particular writer's projects,
//! categories or titles. Everything it does is a rule that applies to any
//! journal in the shape the input contract describes, which is what makes it
//! useful to someone who clones this repository.

pub mod report;
pub mod resolve;
pub mod source;

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::db::models::{NewEntry, Status};
use crate::db::{Db, DbError, repo};

pub use report::ImportReport;
pub use resolve::Resolved;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("could not read directory {path}: {source}")]
    ReadDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("could not read {file}: {source}")]
    ReadFile {
        file: String,
        source: std::io::Error,
    },

    #[error("could not open {file} as a spreadsheet: {message}")]
    OpenWorkbook { file: String, message: String },

    #[error("{file} has no {sheet:?} sheet: {message}")]
    MissingSheet {
        file: String,
        sheet: String,
        message: String,
    },

    #[error("{file} has no {header:?} column")]
    MissingHeader { file: String, header: String },

    #[error("{file} needs a \"Day WC\" or \"Project WC\" column; it has neither")]
    MissingWordCount { file: String },

    #[error("{file}: {message}")]
    Csv { file: String, message: String },

    #[error(
        "{file} row {row}: {value:?} is not a status for {project:?} (open, completed or abandoned)"
    )]
    BadStatus {
        file: String,
        row: usize,
        project: String,
        value: String,
    },

    #[error("{project:?} has two status rows ({first} and {second}); the second is at {at}")]
    DuplicateStatusRow {
        project: String,
        first: Status,
        second: Status,
        at: String,
    },

    #[error("nothing states the type of category {category:?}; add a Type to one of its rows")]
    UntypedCategory { category: String },

    #[error(
        "the database already holds data; pass --reset to replace it, \
         or --db to import somewhere else"
    )]
    NotEmpty,

    #[error(transparent)]
    Db(#[from] DbError),

    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

/// How an import should treat an existing database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Existing {
    /// Refuse to touch a database that already holds data.
    Refuse,
    /// Delete what is there first.
    Reset,
}

/// Read and resolve a directory without writing anything.
///
/// This is what `--dry-run` reports, and what the tests assert against.
pub fn plan(dir: &Path) -> Result<Resolved, ImportError> {
    let sources = source::read_dir(dir)?;
    resolve::resolve(sources)
}

/// Import a directory of journals into `db`.
pub fn run(db: &mut Db, dir: &Path, existing: Existing) -> Result<ImportReport, ImportError> {
    if !db.is_empty()? {
        match existing {
            Existing::Refuse => return Err(ImportError::NotEmpty),
            Existing::Reset => db.reset()?,
        }
    }
    let resolved = plan(dir)?;
    write(db.conn_mut(), &resolved)?;
    Ok(resolved.report)
}

/// Write a resolved import in one transaction, so a failure leaves nothing behind.
fn write(conn: &mut Connection, resolved: &Resolved) -> Result<(), ImportError> {
    let tx = conn.transaction()?;

    for name in &resolved.types {
        repo::ensure_type(&tx, name)?;
    }
    for (category, type_name) in &resolved.categories {
        let type_id = repo::type_id_by_name(&tx, type_name)?.expect("types inserted first");
        repo::ensure_category(&tx, category, type_id)?;
    }
    for project in &resolved.projects {
        let category_id = match &project.category {
            Some(name) => repo::category_id_by_name(&tx, name)?,
            None => None,
        };
        repo::insert_project(
            &tx,
            &project.title,
            category_id,
            project.status,
            project.description.as_deref(),
        )?;
    }
    for entry in &resolved.entries {
        let project_id =
            repo::project_id_by_title(&tx, &entry.project)?.expect("projects inserted first");
        repo::insert_entry(
            &tx,
            &NewEntry {
                entry_date: entry.date,
                project_id,
                session_word_count: entry.words,
                hours: entry.hours,
                notes: entry.notes.clone(),
            },
        )?;
    }

    tx.commit()?;
    Ok(())
}
