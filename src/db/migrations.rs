//! Schema migrations, applied in order and tracked by SQLite's `user_version`.
//!
//! A framework would be more machinery than a four-table schema earns. Each
//! entry in [`MIGRATIONS`] is applied once, in order, inside a transaction, and
//! `user_version` is set to the number applied. Appending to the slice is the
//! only supported way to change the schema; editing an existing entry would
//! leave already-migrated databases behind.

use rusqlite::{Connection, Transaction};

use super::DbError;

/// Ordered schema migrations. Index 0 is migration 1.
pub const MIGRATIONS: &[&str] = &[
    // 1: initial schema
    r#"
    CREATE TABLE types (
      id          INTEGER PRIMARY KEY,
      name        TEXT NOT NULL UNIQUE,
      description TEXT
    );

    CREATE TABLE categories (
      id          INTEGER PRIMARY KEY,
      name        TEXT NOT NULL UNIQUE,
      type_id     INTEGER NOT NULL REFERENCES types(id),
      description TEXT
    );

    CREATE TABLE projects (
      id          INTEGER PRIMARY KEY,
      title       TEXT NOT NULL UNIQUE,
      category_id INTEGER REFERENCES categories(id),
      status      TEXT NOT NULL DEFAULT 'open'
                    CHECK (status IN ('open', 'completed', 'abandoned')),
      description TEXT,
      created_at  TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE TABLE entries (
      id                 INTEGER PRIMARY KEY,
      entry_date         TEXT    NOT NULL,
      project_id         INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
      session_word_count INTEGER NOT NULL,
      hours              REAL    NOT NULL CHECK (hours >= 0.25),
      notes              TEXT,
      created_at         TEXT    NOT NULL DEFAULT (datetime('now'))
    );

    CREATE INDEX idx_entries_project ON entries(project_id);
    CREATE INDEX idx_entries_date    ON entries(entry_date);
    CREATE INDEX idx_projects_status ON projects(status);
    "#,
];

/// The schema version this build expects.
pub fn target_version() -> i64 {
    MIGRATIONS.len() as i64
}

/// Read the version recorded in the database file.
pub fn current_version(conn: &Connection) -> Result<i64, DbError> {
    Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

/// Apply every migration the database has not seen yet.
pub fn migrate(conn: &mut Connection) -> Result<(), DbError> {
    let from = current_version(conn)?;
    let to = target_version();
    if from > to {
        return Err(DbError::NewerSchema {
            found: from,
            supported: to,
        });
    }
    for (index, sql) in MIGRATIONS.iter().enumerate() {
        let version = index as i64 + 1;
        if version <= from {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        set_version(&tx, version)?;
        tx.commit()?;
    }
    Ok(())
}

/// `PRAGMA user_version` takes no bind parameters, so the value is formatted in.
/// It is an `i64` from our own slice index, never user input.
fn set_version(tx: &Transaction<'_>, version: i64) -> Result<(), DbError> {
    tx.pragma_update(None, "user_version", version)?;
    Ok(())
}
