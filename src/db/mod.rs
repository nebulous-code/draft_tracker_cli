//! The SQLite data layer.

pub mod migrations;
pub mod models;
pub mod repo;
pub mod stats;

use std::path::{Path, PathBuf};

use rusqlite::Connection;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("could not create {path}: {source}")]
    CreateDir {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error(
        "this database was written by a newer version of draft-tracker \
         (schema {found}, this build supports {supported})"
    )]
    NewerSchema { found: i64, supported: i64 },

    #[error("a session must be at least {minimum} hours; got {got}")]
    SessionTooShort { got: f64, minimum: f64 },

    #[error("no project with id {0}")]
    NoSuchProject(i64),
}

/// An open database, migrated to the version this build expects.
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Open (creating if needed) the database at `path` and migrate it.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|source| DbError::CreateDir {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        Self::from_connection(Connection::open(path)?)
    }

    /// An in-memory database, for tests.
    pub fn open_in_memory() -> Result<Self, DbError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self, DbError> {
        let mut db = Self { conn };
        db.configure()?;
        migrations::migrate(&mut db.conn)?;
        Ok(db)
    }

    /// Pragmas worth setting on every connection.
    ///
    /// Foreign keys are off by default in SQLite and have to be asked for.
    /// WAL survives an abrupt exit better than the rollback journal, which
    /// matters for a tool that lives in a terminal and gets closed abruptly.
    fn configure(&mut self) -> Result<(), DbError> {
        self.conn.pragma_update(None, "foreign_keys", "ON")?;
        self.conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // An in-memory database has no file to journal; asking for WAL fails.
        let _ = self.conn.pragma_update(None, "journal_mode", "WAL");
        Ok(())
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    pub fn schema_version(&self) -> Result<i64, DbError> {
        migrations::current_version(&self.conn)
    }

    /// True when no project has been created yet.
    ///
    /// The importer refuses to run against a non-empty database unless told to
    /// reset, so that it cannot silently double-import.
    pub fn is_empty(&self) -> Result<bool, DbError> {
        let projects: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))?;
        let entries: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM entries", [], |row| row.get(0))?;
        Ok(projects == 0 && entries == 0)
    }

    /// Delete all data, keeping the schema. Used by `import --reset`.
    pub fn reset(&mut self) -> Result<(), DbError> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "DELETE FROM entries;
             DELETE FROM projects;
             DELETE FROM categories;
             DELETE FROM types;",
        )?;
        tx.commit()?;
        Ok(())
    }
}
