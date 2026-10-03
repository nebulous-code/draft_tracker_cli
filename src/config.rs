//! Where the database lives.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// Environment variable that overrides the default database location.
pub const DB_PATH_ENV: &str = "DRAFT_TRACKER_DB";

/// Resolve the database path.
///
/// An explicit path wins, then [`DB_PATH_ENV`], then the platform data
/// directory. The platform directory is used rather than the working directory
/// so that the log is tied to the writer rather than to wherever a terminal
/// happened to be open.
pub fn resolve_db_path(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    if let Some(from_env) = std::env::var_os(DB_PATH_ENV)
        && !from_env.is_empty()
    {
        return Ok(PathBuf::from(from_env));
    }
    let data_dir = dirs::data_dir().context(
        "could not determine a platform data directory; pass --db or set DRAFT_TRACKER_DB",
    )?;
    Ok(data_dir.join("draft_tracker").join("draft_tracker.db"))
}
