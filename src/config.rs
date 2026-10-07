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

/// Default configuration filename, which differs by build profile.
///
/// A debug build and an installed release build must not share a configuration
/// file. They did, which meant `cargo run` inside the repository resolved to
/// whatever database the real file named, so routine development pointed at the
/// real log by default and backed it up as if it were production. Separating the
/// filenames makes the dev/prod split structural rather than a flag the writer has
/// to remember.
#[cfg(debug_assertions)]
pub const DEFAULT_CONFIG_FILENAME: &str = "config.debug.toml";
#[cfg(not(debug_assertions))]
pub const DEFAULT_CONFIG_FILENAME: &str = "config.toml";

/// Default database filename, likewise split by profile.
///
/// Without this a writer who has no configuration file at all would still have
/// both builds resolve to the same default database.
#[cfg(debug_assertions)]
const DEFAULT_DATABASE_FILENAME: &str = "draft_tracker.debug.db";
#[cfg(not(debug_assertions))]
const DEFAULT_DATABASE_FILENAME: &str = "draft_tracker.db";

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
    /// Worked out from the database path, because the database was overridden and
    /// the file's value described a different log.
    Derived,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Flag => write!(f, "command line"),
            Source::Environment(name) => write!(f, "${name}"),
            Source::File => write!(f, "config file"),
            Source::Default => write!(f, "default"),
            Source::Derived => write!(f, "follows the database"),
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
    ///
    /// This reads the world; [`resolve`] applies the rules.
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

        let outside = Outside {
            data_dir: dirs::data_dir()
                .map(|dir| dir.join("draft_tracker"))
                .context("no platform data directory; set `database` in the config file")?,
            database_env: non_empty_env(DB_PATH_ENV),
            backup_directory_env: non_empty_env(BACKUP_DIR_ENV),
        };

        Ok(resolve(
            overrides,
            &file,
            &outside,
            config_path,
            config_exists,
        ))
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

/// What resolution needs from outside the process, gathered in one place so that
/// [`resolve`] is pure.
///
/// Resolution decides which database gets written to. That is worth being able to
/// test without a real environment, and not testing it is how the backup naming
/// bug below survived being written down as safe.
struct Outside {
    data_dir: PathBuf,
    database_env: Option<PathBuf>,
    backup_directory_env: Option<PathBuf>,
}

/// Apply the precedence rules to already-gathered inputs.
fn resolve(
    overrides: &Overrides,
    file: &FileConfig,
    outside: &Outside,
    config_path: PathBuf,
    config_exists: bool,
) -> Config {
    let database = if let Some(path) = overrides.database.clone() {
        Setting::new(path, Source::Flag)
    } else if let Some(path) = outside.database_env.clone() {
        Setting::new(path, Source::Environment(DB_PATH_ENV))
    } else if let Some(path) = file.database.clone() {
        Setting::new(path, Source::File)
    } else {
        Setting::new(
            outside.data_dir.join(DEFAULT_DATABASE_FILENAME),
            Source::Default,
        )
    };

    // `backup_directory` and `name` in the file describe the database the file
    // names. Once the database has been overridden they describe a different log,
    // so they are dropped rather than carried across. Without this, pointing the
    // program at a scratch database wrote that database's backups into the real
    // backup directory under the real prefix, where pruning counted them and
    // could evict real ones.
    //
    // Policy settings are not dropped: `backups` and `backups_to_keep` say how the
    // writer wants backups taken, which is true of whatever database is in use.
    //
    // An environment variable naming the backup directory still wins, because it
    // was stated for this run rather than inherited from a file describing
    // something else.
    let redirected = matches!(database.source, Source::Flag | Source::Environment(_));

    let backup_directory = if let Some(path) = outside.backup_directory_env.clone() {
        Setting::new(path, Source::Environment(BACKUP_DIR_ENV))
    } else if let Some(path) = file.backup_directory.clone().filter(|_| !redirected) {
        Setting::new(path, Source::File)
    } else {
        // Beside the database rather than beside the binary: backups belong with
        // the data they protect.
        // A bare filename's parent is the empty path, which joins to a relative
        // `backups` beside it in the working directory. That is still "beside the
        // database", so it is left alone.
        let beside = database
            .value
            .parent()
            .map(|parent| parent.join("backups"))
            .unwrap_or_else(|| outside.data_dir.join("backups"));
        Setting::new(
            beside,
            derived_or_default(redirected, &file.backup_directory),
        )
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

    let name = match file.name.clone().filter(|_| !redirected) {
        Some(name) => Setting::new(name, Source::File),
        // Taken from the database's filename, so a scratch database's backups are
        // named apart from the real one's and cannot evict them. The `redirected`
        // rule above is what makes that true even when the file sets `name`.
        None => Setting::new(
            database_stem(&database.value),
            derived_or_default(redirected, &file.name),
        ),
    };

    Config {
        database,
        backup_directory,
        backups_to_keep,
        backups,
        name,
        config_path,
        config_exists,
    }
}

/// Whether a derived value displaced a file setting or simply had none to displace.
///
/// Reported separately so that `draft-tracker config` can say "follows the
/// database" rather than "default", which would look like the file was ignored for
/// no reason.
fn derived_or_default<T>(redirected: bool, file_value: &Option<T>) -> Source {
    if redirected && file_value.is_some() {
        Source::Derived
    } else {
        Source::Default
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
        .join(DEFAULT_CONFIG_FILENAME))
}

/// A commented file showing every setting at its default, for `config --init`.
pub fn example_file(config: &Config) -> String {
    format!(
        "# draft-tracker configuration.\n\
         #\n\
         # Every setting is optional. A command line option beats an environment\n\
         # variable, which beats this file, which beats the built-in default.\n\
         # `draft-tracker config` shows what is in force and where it came from.\n\
         #\n\
         # Release builds read config.toml and debug builds read config.debug.toml,\n\
         # so working in the repository cannot reach the log a release build uses.\n\
         #\n\
         # `backup_directory` and `name` describe the database named below. Point\n\
         # the program at another database with --db and both are worked out from\n\
         # that path instead, so a scratch log cannot write into these backups.\n\
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

#[cfg(test)]
mod tests {
    use super::*;

    /// These are unit tests rather than integration tests because resolution has
    /// to be checked with the file's private shape in hand, and without reading
    /// the real environment.
    fn outside() -> Outside {
        Outside {
            data_dir: PathBuf::from("/data"),
            database_env: None,
            backup_directory_env: None,
        }
    }

    /// A config file describing a real log, as `config --init` would write it.
    fn real_file() -> FileConfig {
        FileConfig {
            database: Some(PathBuf::from("/pool/real/draft_tracker.db")),
            backup_directory: Some(PathBuf::from("/pool/real/backups")),
            backups_to_keep: Some(20),
            backups: Some(true),
            name: Some("draft_tracker".to_string()),
        }
    }

    fn resolved(overrides: &Overrides, file: &FileConfig) -> Config {
        resolve(
            overrides,
            file,
            &outside(),
            PathBuf::from("/cfg.toml"),
            true,
        )
    }

    fn pointing_at(path: &str) -> Overrides {
        Overrides {
            database: Some(PathBuf::from(path)),
            ..Overrides::default()
        }
    }

    #[test]
    fn the_file_decides_when_nothing_overrides_it() {
        let config = resolved(&Overrides::default(), &real_file());
        assert_eq!(config.database(), Path::new("/pool/real/draft_tracker.db"));
        assert_eq!(
            config.backup_directory.value,
            Path::new("/pool/real/backups")
        );
        assert_eq!(config.name.value, "draft_tracker");
        assert_eq!(config.name.source, Source::File);
    }

    /// The bug this rule exists for: a scratch database must not be able to write
    /// into the real backup directory, nor under the real filename prefix, where
    /// pruning would count it against the real backups and could evict them.
    #[test]
    fn a_flagged_database_takes_its_backups_with_it() {
        let config = resolved(&pointing_at("/tmp/demo.db"), &real_file());
        assert_eq!(config.database(), Path::new("/tmp/demo.db"));
        assert_eq!(config.backup_directory.value, Path::new("/tmp/backups"));
        assert_eq!(config.name.value, "demo");
    }

    #[test]
    fn displacing_a_file_setting_is_reported_as_following_the_database() {
        let config = resolved(&pointing_at("/tmp/demo.db"), &real_file());
        assert_eq!(config.name.source, Source::Derived);
        assert_eq!(config.backup_directory.source, Source::Derived);
        assert_eq!(config.database.source, Source::Flag);
    }

    /// With nothing in the file to displace, the same derived values are plain
    /// defaults. Saying "follows the database" there would imply a file setting
    /// had been overruled when none existed.
    #[test]
    fn deriving_without_a_file_setting_is_reported_as_default() {
        let file = FileConfig {
            database: Some(PathBuf::from("/pool/real/draft_tracker.db")),
            ..FileConfig::default()
        };
        let config = resolved(&pointing_at("/tmp/demo.db"), &file);
        assert_eq!(config.name.source, Source::Default);
        assert_eq!(config.backup_directory.source, Source::Default);
    }

    #[test]
    fn an_environment_database_displaces_them_too() {
        let mut outside = outside();
        outside.database_env = Some(PathBuf::from("/tmp/scratch.db"));
        let config = resolve(
            &Overrides::default(),
            &real_file(),
            &outside,
            PathBuf::from("/cfg.toml"),
            true,
        );
        assert_eq!(config.database.source, Source::Environment(DB_PATH_ENV));
        assert_eq!(config.backup_directory.value, Path::new("/tmp/backups"));
        assert_eq!(config.name.value, "scratch");
    }

    /// Stated for this run, so it is not an inherited description of another log.
    #[test]
    fn an_environment_backup_directory_still_wins_over_the_rule() {
        let mut outside = outside();
        outside.backup_directory_env = Some(PathBuf::from("/elsewhere"));
        let config = resolve(
            &pointing_at("/tmp/demo.db"),
            &real_file(),
            &outside,
            PathBuf::from("/cfg.toml"),
            true,
        );
        assert_eq!(config.backup_directory.value, Path::new("/elsewhere"));
        assert_eq!(
            config.backup_directory.source,
            Source::Environment(BACKUP_DIR_ENV)
        );
        // The name still follows the database; only the directory was stated.
        assert_eq!(config.name.value, "demo");
    }

    /// How the writer wants backups taken is true of whatever database is open, so
    /// redirecting the database must not quietly change the retention policy.
    #[test]
    fn policy_settings_survive_a_redirect() {
        let config = resolved(&pointing_at("/tmp/demo.db"), &real_file());
        assert_eq!(config.backups_to_keep.value, 20);
        assert_eq!(config.backups_to_keep.source, Source::File);
        assert!(config.backups_enabled());
        assert_eq!(config.backups.source, Source::File);
    }

    #[test]
    fn no_backup_turns_automatic_backups_off_whatever_the_file_says() {
        let overrides = Overrides {
            no_backup: true,
            ..Overrides::default()
        };
        let config = resolved(&overrides, &real_file());
        assert!(!config.backups_enabled());
        assert_eq!(config.backups.source, Source::Flag);
    }

    #[test]
    fn an_empty_file_falls_back_to_the_platform_directories() {
        let config = resolved(&Overrides::default(), &FileConfig::default());
        assert_eq!(
            config.database(),
            Path::new("/data").join(DEFAULT_DATABASE_FILENAME)
        );
        assert_eq!(config.backup_directory.value, Path::new("/data/backups"));
        assert_eq!(config.database.source, Source::Default);
    }

    /// "Beside the database" for a bare filename is the working directory.
    #[test]
    fn a_relative_database_keeps_its_backups_relative() {
        let config = resolved(&pointing_at("demo.db"), &real_file());
        assert_eq!(config.backup_directory.value, Path::new("backups"));
        assert_eq!(config.name.value, "demo");
    }

    #[test]
    fn a_database_with_no_readable_stem_still_gets_a_name() {
        let config = resolved(&pointing_at("/tmp/"), &real_file());
        assert!(!config.name.value.is_empty());
    }

    /// The dev/prod split: the two builds must not read the same file, or working
    /// in the repository resolves to whatever database the real file names.
    #[test]
    fn debug_and_release_read_different_configuration_files() {
        if cfg!(debug_assertions) {
            assert_eq!(DEFAULT_CONFIG_FILENAME, "config.debug.toml");
            assert_eq!(DEFAULT_DATABASE_FILENAME, "draft_tracker.debug.db");
        } else {
            assert_eq!(DEFAULT_CONFIG_FILENAME, "config.toml");
            assert_eq!(DEFAULT_DATABASE_FILENAME, "draft_tracker.db");
        }
        assert_ne!(DEFAULT_CONFIG_FILENAME, "");
    }

    #[test]
    fn the_default_config_path_uses_the_profile_filename() {
        let path = default_config_path().expect("a platform config directory");
        assert_eq!(
            path.file_name().and_then(|n| n.to_str()),
            Some(DEFAULT_CONFIG_FILENAME)
        );
        assert!(path.ends_with(Path::new("draft_tracker").join(DEFAULT_CONFIG_FILENAME)));
    }
}
