//! Settings: where the log lives, where backups go, and how many to keep.
//!
//! Every setting records where it came from, so `draft-tracker config` can show
//! not just the resolved values but which of the four sources won. With a live
//! database and a scratch one in play, "which database am I about to write to"
//! is a question worth being able to answer without guessing.

use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// Environment variable that overrides the database location.
pub const DB_PATH_ENV: &str = "DRAFT_TRACKER_DB";
/// Environment variable that overrides where backups are written.
pub const BACKUP_DIR_ENV: &str = "DRAFT_TRACKER_BACKUP_DIR";

/// Backups kept when the config file says nothing.
///
/// Only changes count: an unchanged database is not backed up again, so this is
/// twenty distinct states rather than twenty launches. For someone writing daily
/// that is about three weeks of history, which is a reasonable default for a
/// person with no other protection.
pub const DEFAULT_BACKUPS_TO_KEEP: usize = 20;

/// Where a setting's value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A command line option.
    Flag,
    /// An environment variable.
    Environment(&'static str),
    /// The configuration file.
    File,
    /// Nothing said otherwise.
    Default,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Flag => write!(f, "command line"),
            Source::Environment(name) => write!(f, "${name}"),
            Source::File => write!(f, "config file"),
            Source::Default => write!(f, "default"),
        }
    }
}

/// A resolved value and the reason it is that value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting<T> {
    pub value: T,
    pub source: Source,
}

impl<T> Setting<T> {
    fn new(value: T, source: Source) -> Self {
        Self { value, source }
    }
}

/// The file's shape. Every key optional, so a partial file is valid.
///
/// Unknown keys are refused rather than ignored: this file is edited by hand, and
/// a silently discarded `backups_to_keeep` is worse than an error naming it.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    database: Option<PathBuf>,
    backup_directory: Option<PathBuf>,
    backups_to_keep: Option<usize>,
    backups: Option<bool>,
    name: Option<String>,
}

/// Everything resolved, with provenance.
#[derive(Debug, Clone)]
pub struct Config {
    pub database: Setting<PathBuf>,
    pub backup_directory: Setting<PathBuf>,
    pub backups_to_keep: Setting<usize>,
    /// Whether backups are taken automatically.
    ///
    /// Only the automatic ones: asking for a backup outright always takes one,
    /// because somebody who typed the command meant it. Someone whose filesystem
    /// already snapshots the live database may reasonably want this off.
    pub backups: Setting<bool>,
    /// Prefix for backup filenames, so one directory can hold several logs.
    pub name: Setting<String>,
    /// Where the configuration file would be, whether or not it exists.
    pub config_path: PathBuf,
    pub config_exists: bool,
}

/// What the command line supplied, before anything else is consulted.
#[derive(Debug, Default, Clone)]
pub struct Overrides {
    pub database: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    /// Set by `--no-backup`, turning the automatic ones off for this run.
    pub no_backup: bool,
}

impl Config {
    /// Resolve every setting: flag, then environment, then file, then default.
    pub fn load(overrides: &Overrides) -> Result<Self> {
        let config_path = match &overrides.config_path {
            Some(path) => path.clone(),
            None => default_config_path()?,
        };
        let config_exists = config_path.exists();
        let file = if config_exists {
            let text = std::fs::read_to_string(&config_path)
                .with_context(|| format!("reading {}", config_path.display()))?;
            toml::from_str::<FileConfig>(&text)
                .with_context(|| format!("reading {}", config_path.display()))?
        } else {
            FileConfig::default()
        };

        let data_dir = dirs::data_dir()
            .map(|dir| dir.join("draft_tracker"))
            .context("no platform data directory; set `database` in the config file")?;

        let database = if let Some(path) = overrides.database.clone() {
            Setting::new(path, Source::Flag)
        } else if let Some(path) = non_empty_env(DB_PATH_ENV) {
            Setting::new(path, Source::Environment(DB_PATH_ENV))
        } else if let Some(path) = file.database.clone() {
            Setting::new(path, Source::File)
        } else {
            Setting::new(data_dir.join("draft_tracker.db"), Source::Default)
        };

        let backup_directory = if let Some(path) = non_empty_env(BACKUP_DIR_ENV) {
            Setting::new(path, Source::Environment(BACKUP_DIR_ENV))
        } else if let Some(path) = file.backup_directory.clone() {
            Setting::new(path, Source::File)
        } else {
            // Beside the database rather than beside the binary: backups belong
            // with the data they protect.
            let beside = database
                .value
                .parent()
                .map(|parent| parent.join("backups"))
                .unwrap_or_else(|| data_dir.join("backups"));
            Setting::new(beside, Source::Default)
        };

        let backups_to_keep = match file.backups_to_keep {
            Some(count) => Setting::new(count, Source::File),
            None => Setting::new(DEFAULT_BACKUPS_TO_KEEP, Source::Default),
        };

        let backups = if overrides.no_backup {
            Setting::new(false, Source::Flag)
        } else {
            match file.backups {
                Some(enabled) => Setting::new(enabled, Source::File),
                None => Setting::new(true, Source::Default),
            }
        };

        let name = match file.name.clone() {
            Some(name) => Setting::new(name, Source::File),
            // Taken from the database's filename, so a scratch database's
            // backups are named apart from the real one's and cannot evict them.
            None => Setting::new(database_stem(&database.value), Source::Default),
        };

        Ok(Self {
            database,
            backup_directory,
            backups_to_keep,
            backups,
            name,
            config_path,
            config_exists,
        })
    }

    /// Convenience for the common case of only needing the database path.
    pub fn database(&self) -> &Path {
        &self.database.value
    }

    /// A configuration with every value stated outright.
    ///
    /// Nothing is read from the environment, the config file or the platform
    /// directories. Tests use this so that running them can never write a backup
    /// into somebody's real backup directory.
    pub fn explicit(
        database: impl Into<PathBuf>,
        backup_directory: impl Into<PathBuf>,
        backups_to_keep: usize,
        name: impl Into<String>,
    ) -> Self {
        Self {
            database: Setting::new(database.into(), Source::Flag),
            backup_directory: Setting::new(backup_directory.into(), Source::Flag),
            backups_to_keep: Setting::new(backups_to_keep, Source::Flag),
            backups: Setting::new(true, Source::Default),
            name: Setting::new(name.into(), Source::Flag),
            config_path: PathBuf::new(),
            config_exists: false,
        }
    }

    /// Turn automatic backups on or off, for tests and for `--no-backup`.
    pub fn with_backups(mut self, enabled: bool) -> Self {
        self.backups = Setting::new(enabled, Source::Flag);
        self
    }

    /// Whether an automatic backup should be taken.
    pub fn backups_enabled(&self) -> bool {
        self.backups.value
    }
}

fn non_empty_env(name: &str) -> Option<PathBuf> {
    let value = std::env::var_os(name)?;
    (!value.is_empty()).then(|| PathBuf::from(value))
}

/// `draft_tracker.db` becomes `draft_tracker`; anything unreadable becomes that
/// too, so a backup is never named after nothing.
fn database_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("draft_tracker")
        .to_string()
}

pub fn default_config_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("no platform configuration directory; pass --config")?
        .join("draft_tracker")
        .join("config.toml"))
}

/// A commented file showing every setting at its default, for `config --init`.
pub fn example_file(config: &Config) -> String {
    format!(
        "# draft-tracker configuration.\n\
         #\n\
         # Every setting is optional. A command line option beats an environment\n\
         # variable, which beats this file, which beats the built-in default.\n\
         # `draft-tracker config` shows what is in force and where it came from.\n\
         \n\
         # Where the writing log lives. One SQLite file.\n\
         database = {database}\n\
         \n\
         # Where backups are written. Created if it does not exist.\n\
         backup_directory = {backup_directory}\n\
         \n\
         # How many backups to keep; older ones are pruned. Only changes count,\n\
         # so opening the program without editing anything does not consume one.\n\
         backups_to_keep = {backups_to_keep}\n\
         \n\
         # Whether to take backups automatically, when the interface opens and\n\
         # when an entry is saved. Asking for one outright always works.\n\
         backups = {backups}\n\
         \n\
         # Prefix for backup filenames. Defaults to the database's filename, so\n\
         # one directory can hold backups of more than one log.\n\
         name = {name}\n",
        database = toml_string(&config.database.value.display().to_string()),
        backup_directory = toml_string(&config.backup_directory.value.display().to_string()),
        backups_to_keep = config.backups_to_keep.value,
        backups = config.backups.value,
        name = toml_string(&config.name.value),
    )
}

/// Quote a value for TOML. Paths can contain backslashes on Windows.
fn toml_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
