//! Backups: a consistent copy of the log, written somewhere else.
//!
//! `VACUUM INTO` is the mechanism. It is one statement, it reads the logical
//! database rather than the file so WAL is not a consideration, it compacts as it
//! goes, and it refuses to write over anything that already exists.
//!
//! Backups are skipped when nothing has changed. A fresh copy is written to a
//! temporary file and compared against the most recent backup byte for byte; if
//! they match, the temporary file is discarded. This is exact rather than a
//! guess, because `VACUUM INTO` of unchanged content is byte-identical, which was
//! measured rather than assumed. The point is that "keep twenty" then means the
//! last twenty states the log was actually in, not the last twenty times the
//! program happened to start. Opening it to read does not cost a generation.

use std::io::{self, Read};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};

use crate::config::Config;
use crate::db::{Db, DbError};

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("could not create the backup directory {}: {source}", path.display())]
    CreateDir { path: PathBuf, source: io::Error },

    #[error("could not write a backup to {}: {message}", path.display())]
    Write { path: PathBuf, message: String },

    #[error("could not read {}: {source}", path.display())]
    Read { path: PathBuf, source: io::Error },

    #[error(transparent)]
    Db(#[from] DbError),
}

/// What a backup attempt did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A new backup was written, and this many older ones were pruned.
    Written { path: PathBuf, pruned: Vec<PathBuf> },
    /// Nothing had changed since the most recent backup.
    Unchanged { matched: PathBuf },
}

impl Outcome {
    pub fn written_path(&self) -> Option<&Path> {
        match self {
            Outcome::Written { path, .. } => Some(path),
            Outcome::Unchanged { .. } => None,
        }
    }
}

/// Take a backup of `db` according to `config`.
pub fn run(db: &Db, config: &Config) -> Result<Outcome, BackupError> {
    run_at(db, config, Local::now())
}

/// As [`run`], with the timestamp supplied, so tests are not at the mercy of the
/// clock.
pub fn run_at(db: &Db, config: &Config, now: DateTime<Local>) -> Result<Outcome, BackupError> {
    let directory = &config.backup_directory.value;
    std::fs::create_dir_all(directory).map_err(|source| BackupError::CreateDir {
        path: directory.clone(),
        source,
    })?;

    let name = &config.name.value;
    let temporary = directory.join(format!(".{name}-tmp-{}.db", std::process::id()));
    // VACUUM INTO refuses an existing destination, so clear any leftover from a
    // previous run that was interrupted.
    let _ = std::fs::remove_file(&temporary);

    vacuum_into(db, &temporary)?;

    // Compare against the newest existing backup rather than hashing the live
    // database: the live file can be unchanged while the write-ahead log holds
    // commits, so only the logical content is a sound basis for comparison.
    let existing = list(directory, name)?;
    if let Some(newest) = existing.last()
        && identical(&temporary, newest)?
    {
        let _ = std::fs::remove_file(&temporary);
        return Ok(Outcome::Unchanged {
            matched: newest.clone(),
        });
    }

    let destination = unique_destination(directory, name, now);
    std::fs::rename(&temporary, &destination).map_err(|source| BackupError::Read {
        path: destination.clone(),
        source,
    })?;

    let pruned = prune(directory, name, config.backups_to_keep.value)?;
    Ok(Outcome::Written {
        path: destination,
        pruned,
    })
}

fn vacuum_into(db: &Db, destination: &Path) -> Result<(), BackupError> {
    let target = destination.to_string_lossy().into_owned();
    db.conn()
        .execute("VACUUM INTO ?1", [&target])
        .map(|_| ())
        .map_err(|error| BackupError::Write {
            path: destination.to_path_buf(),
            message: error.to_string(),
        })
}

/// Write a backup to exactly this path, whatever the configuration says.
///
/// For somebody who asked for a copy at a named place: no skipping, no pruning.
/// `VACUUM INTO` refuses an existing destination, which is the behaviour wanted
/// here, so a typo cannot quietly replace last month's archive.
pub fn write_to(db: &Db, destination: &Path) -> Result<(), BackupError> {
    if let Some(parent) = destination.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| BackupError::CreateDir {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    vacuum_into(db, destination)
}

/// Existing backups for this name, oldest first.
///
/// Ordered by modification time, with the filename breaking ties. Relying on the
/// names alone was a mistake: pruning frees a name, the next backup can take it
/// again, and a reused name makes the newest backup look like the oldest, at
/// which point pruning deletes the wrong one. Time is what is actually meant, so
/// time is what is compared.
pub fn list(directory: &Path, name: &str) -> Result<Vec<PathBuf>, BackupError> {
    let prefix = format!("{name}-");
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        // No directory yet means no backups, which is not an error.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(BackupError::Read {
                path: directory.to_path_buf(),
                source,
            });
        }
    };
    let mut found: Vec<(Option<std::time::SystemTime>, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(".db"))
        })
        .map(|path| {
            let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            (modified, path)
        })
        .collect();
    found.sort();
    Ok(found.into_iter().map(|(_, path)| path).collect())
}

/// A filename nothing else is using. Seconds make a collision unlikely; the
/// counter makes it impossible.
fn unique_destination(directory: &Path, name: &str, now: DateTime<Local>) -> PathBuf {
    // Milliseconds, so every name is unique and every name has the same shape.
    // Uniformity is the point: a scheme where some names carry a suffix and
    // others do not cannot be ordered by comparing them.
    let stamp = now.format("%Y-%m-%dT%H%M%S-%3f");
    let first = directory.join(format!("{name}-{stamp}.db"));
    if !first.exists() {
        return first;
    }
    // `_NN` and not `-NN`: ordering here is lexical, `-` sorts before `.` while
    // `_` sorts after it, so a hyphen would put `...171437-2.db` *before*
    // `...171437.db` and make the newest backup look like the oldest. Pruning
    // trusts this order, so getting it wrong would delete the wrong files.
    for suffix in 2..100 {
        let candidate = directory.join(format!("{name}-{stamp}_{suffix:02}.db"));
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}

/// Delete all but the newest `keep` backups, returning what went.
///
/// `keep` is treated as at least one: a retention setting of zero would delete
/// the backup just written, which cannot be what anybody means.
fn prune(directory: &Path, name: &str, keep: usize) -> Result<Vec<PathBuf>, BackupError> {
    let keep = keep.max(1);
    let existing = list(directory, name)?;
    if existing.len() <= keep {
        return Ok(Vec::new());
    }
    let mut pruned = Vec::new();
    for path in &existing[..existing.len() - keep] {
        if std::fs::remove_file(path).is_ok() {
            pruned.push(path.clone());
        }
    }
    Ok(pruned)
}

/// Compare two files exactly.
///
/// Byte comparison rather than a hash: both files are already on disk, the
/// comparison is exact with no collision to reason about, and it saves a
/// dependency. Length is checked first so differing sizes cost nothing.
fn identical(a: &Path, b: &Path) -> Result<bool, BackupError> {
    let read = |path: &Path| -> Result<std::fs::File, BackupError> {
        std::fs::File::open(path).map_err(|source| BackupError::Read {
            path: path.to_path_buf(),
            source,
        })
    };
    let (mut left, mut right) = (read(a)?, read(b)?);
    let size = |file: &std::fs::File, path: &Path| -> Result<u64, BackupError> {
        file.metadata()
            .map(|m| m.len())
            .map_err(|source| BackupError::Read {
                path: path.to_path_buf(),
                source,
            })
    };
    if size(&left, a)? != size(&right, b)? {
        return Ok(false);
    }

    let mut left_buffer = [0u8; 16 * 1024];
    let mut right_buffer = [0u8; 16 * 1024];
    loop {
        let read_left = fill(&mut left, &mut left_buffer, a)?;
        let read_right = fill(&mut right, &mut right_buffer, b)?;
        if read_left != read_right {
            return Ok(false);
        }
        if read_left == 0 {
            return Ok(true);
        }
        if left_buffer[..read_left] != right_buffer[..read_right] {
            return Ok(false);
        }
    }
}

/// Read until the buffer is full or the file ends, so the two sides stay aligned.
fn fill(file: &mut std::fs::File, buffer: &mut [u8], path: &Path) -> Result<usize, BackupError> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(source) => {
                return Err(BackupError::Read {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }
    }
    Ok(filled)
}

/// What a backup contains, for reporting after an explicit one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub integrity: String,
    pub entries: i64,
    pub words: i64,
    pub projects: i64,
    pub bytes: u64,
}

impl Verified {
    pub fn is_sound(&self) -> bool {
        self.integrity == "ok"
    }
}

/// Open a backup and confirm it holds a sound database with the expected shape.
///
/// A backup nobody has opened is a guess. Reporting the counts turns "a file was
/// written" into "a file containing this much of your writing was written".
pub fn verify(path: &Path) -> Result<Verified, BackupError> {
    let bytes = std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|source| BackupError::Read {
            path: path.to_path_buf(),
            source,
        })?;
    // Read-only, and no migrations. Opening a backup the ordinary way sets its
    // journal mode and so rewrites the very file being checked, which also made
    // it stop matching a fresh copy and defeated the unchanged check.
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(DbError::from)?;
    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(DbError::from)?;
    let (entries, words): (i64, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(session_word_count), 0) FROM entries",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(DbError::from)?;
    let projects: i64 = conn
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .map_err(DbError::from)?;
    Ok(Verified {
        integrity,
        entries,
        words,
        projects,
        bytes,
    })
}

/// Take a backup, turning any failure into a message rather than an error.
///
/// Every automatic backup goes through here. Recording a day's writing must never
/// fail because a backup could not be written: the pool might not be mounted, the
/// disk might be full, and neither is a reason to lose the words. The caller shows
/// the message and carries on.
pub fn best_effort(db: &Db, config: &Config) -> Result<Outcome, String> {
    run(db, config).map_err(|error| error.to_string())
}
