//! Configuration resolution and backups.

use chrono::{Duration, Local};
use draft_tracker::backup::{self, Outcome};
use draft_tracker::config::{Config, Overrides, Source};
use draft_tracker::db::models::{NewEntry, Status};
use draft_tracker::db::{Db, repo};

/// A directory that cleans itself up, holding a database and its backups.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("dt_backup_{label}_{}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }

    fn config(&self, keep: usize) -> Config {
        Config::explicit(
            self.path("draft_tracker.db"),
            self.path("backups"),
            keep,
            "draft_tracker",
        )
    }

    /// A database with one project and one entry.
    fn seeded(&self) -> Db {
        let db = Db::open(self.path("draft_tracker.db")).unwrap();
        repo::insert_project(db.conn(), "Fireside 1", None, Status::Open, None).unwrap();
        add_entry(&db, 1_000);
        db
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn add_entry(db: &Db, words: i64) {
    let id = repo::project_id_by_title(db.conn(), "Fireside 1")
        .unwrap()
        .unwrap();
    repo::insert_entry(
        db.conn(),
        &NewEntry {
            entry_date: Local::now().date_naive(),
            project_id: id,
            session_word_count: words,
            hours: 1.0,
            notes: None,
        },
    )
    .unwrap();
}

fn bytes_of(path: &std::path::Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

// ------------------------------------------------------------ configuration

#[test]
fn settings_fall_back_to_defaults_when_there_is_no_file() {
    let scratch = Scratch::new("defaults");
    let missing = scratch.path("absent.toml");
    let config = Config::load(&Overrides {
        database: None,
        config_path: Some(missing.clone()),
        ..Default::default()
    })
    .unwrap();
    assert!(!config.config_exists);
    assert_eq!(config.config_path, missing);
    assert_eq!(config.database.source, Source::Default);
    assert_eq!(config.backups_to_keep.source, Source::Default);
    // Backups default to sitting beside the database they protect.
    assert_eq!(
        config.backup_directory.value,
        config.database.value.parent().unwrap().join("backups")
    );
    // And the name comes from the database's filename, whichever default that
    // is: a debug build resolves a different default database so that working in
    // the repository cannot reach an installed build's log.
    assert_eq!(
        config.name.value,
        config
            .database
            .value
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap()
    );
    assert!(config.name.value.starts_with("draft_tracker"));
}

#[test]
fn the_file_supplies_values_and_says_so() {
    let scratch = Scratch::new("file");
    let path = scratch.path("config.toml");
    std::fs::write(
        &path,
        format!(
            "database = \"{}\"\nbackup_directory = \"{}\"\nbackups_to_keep = 4\nname = \"journal\"\n",
            scratch.path("mine.db").display(),
            scratch.path("copies").display()
        ),
    )
    .unwrap();
    let config = Config::load(&Overrides {
        database: None,
        config_path: Some(path),
        ..Default::default()
    })
    .unwrap();
    assert!(config.config_exists);
    assert_eq!(config.database.value, scratch.path("mine.db"));
    assert_eq!(config.database.source, Source::File);
    assert_eq!(config.backup_directory.value, scratch.path("copies"));
    assert_eq!(config.backups_to_keep.value, 4);
    assert_eq!(config.name.value, "journal");
}

#[test]
fn a_flag_beats_the_file() {
    let scratch = Scratch::new("flag");
    let path = scratch.path("config.toml");
    std::fs::write(
        &path,
        format!(
            "database = \"{}\"\n",
            scratch.path("from_file.db").display()
        ),
    )
    .unwrap();
    let config = Config::load(&Overrides {
        database: Some(scratch.path("from_flag.db")),
        config_path: Some(path),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(config.database.value, scratch.path("from_flag.db"));
    assert_eq!(config.database.source, Source::Flag);
    // And the name follows the database that won, so a scratch database's
    // backups are named apart from the real one's.
    assert_eq!(config.name.value, "from_flag");
}

/// A hand-edited file should not lose a setting to a typo.
#[test]
fn an_unknown_key_is_refused_rather_than_ignored() {
    let scratch = Scratch::new("typo");
    let path = scratch.path("config.toml");
    std::fs::write(&path, "backups_to_keeep = 4\n").unwrap();
    let error = Config::load(&Overrides {
        database: None,
        config_path: Some(path),
        ..Default::default()
    })
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("config.toml"),
        "the message names the file: {error}"
    );
}

#[test]
fn the_generated_example_file_is_valid_and_round_trips() {
    let scratch = Scratch::new("example");
    let written = scratch.path("config.toml");
    let first = Config::load(&Overrides {
        database: Some(scratch.path("a.db")),
        config_path: Some(written.clone()),
        ..Default::default()
    })
    .unwrap();
    std::fs::write(&written, draft_tracker::config::example_file(&first)).unwrap();

    // Reading back what was generated must produce the same settings, now
    // attributed to the file.
    let second = Config::load(&Overrides {
        database: None,
        config_path: Some(written),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(second.database.value, first.database.value);
    assert_eq!(second.backup_directory.value, first.backup_directory.value);
    assert_eq!(second.backups_to_keep.value, first.backups_to_keep.value);
    assert_eq!(second.name.value, first.name.value);
    assert_eq!(second.database.source, Source::File);
}

// ------------------------------------------------------------ taking backups

#[test]
fn the_first_backup_is_written_and_holds_the_log() {
    let scratch = Scratch::new("first");
    let db = scratch.seeded();
    let config = scratch.config(5);

    let outcome = backup::run(&db, &config).unwrap();
    let path = outcome.written_path().expect("written").to_path_buf();
    assert!(path.exists());
    // The directory is created rather than required to exist.
    assert!(config.backup_directory.value.is_dir());

    let verified = backup::verify(&path).unwrap();
    assert_eq!(verified.integrity, "ok");
    assert!(verified.is_sound());
    assert_eq!(verified.entries, 1);
    assert_eq!(verified.words, 1_000);
    assert_eq!(verified.projects, 1);
}

/// The point of the whole scheme: opening the program to read does not consume a
/// generation of history.
#[test]
fn an_unchanged_log_is_not_backed_up_again() {
    let scratch = Scratch::new("unchanged");
    let db = scratch.seeded();
    let config = scratch.config(5);

    let first = backup::run(&db, &config).unwrap();
    let written = first.written_path().expect("written").to_path_buf();

    for _ in 0..3 {
        match backup::run(&db, &config).unwrap() {
            Outcome::Unchanged { matched } => assert_eq!(matched, written),
            Outcome::Written { path, .. } => panic!("wrote {path:?} with nothing changed"),
        }
    }
    assert_eq!(
        backup::list(&config.backup_directory.value, "draft_tracker")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_change_produces_a_new_backup() {
    let scratch = Scratch::new("changed");
    let db = scratch.seeded();
    let config = scratch.config(5);
    backup::run(&db, &config).unwrap();

    add_entry(&db, 250);
    let outcome = backup::run(&db, &config).unwrap();
    let path = outcome.written_path().expect("written").to_path_buf();
    assert_eq!(backup::verify(&path).unwrap().words, 1_250);
    assert_eq!(
        backup::list(&config.backup_directory.value, "draft_tracker")
            .unwrap()
            .len(),
        2
    );
}

/// Verifying a backup must not touch it. Opening a database the ordinary way sets
/// its journal mode, which rewrote the file and made it stop matching a fresh
/// copy, so every later backup was written as though something had changed.
#[test]
fn verifying_a_backup_leaves_it_byte_for_byte_alone() {
    let scratch = Scratch::new("verify");
    let db = scratch.seeded();
    let config = scratch.config(5);
    let path = backup::run(&db, &config)
        .unwrap()
        .written_path()
        .unwrap()
        .to_path_buf();

    let before = bytes_of(&path);
    backup::verify(&path).unwrap();
    assert_eq!(bytes_of(&path), before, "verifying rewrote the backup");

    // And the unchanged check still holds after a verification.
    assert!(matches!(
        backup::run(&db, &config).unwrap(),
        Outcome::Unchanged { .. }
    ));
}

#[test]
fn pruning_keeps_the_newest_and_discards_the_rest() {
    let scratch = Scratch::new("prune");
    let db = scratch.seeded();
    let config = scratch.config(3);

    let mut totals = Vec::new();
    for words in [100, 200, 300, 400, 500] {
        add_entry(&db, words);
        totals.push(repo::project_logged_total(db.conn(), 1).unwrap());
        backup::run(&db, &config).unwrap();
    }

    let kept = backup::list(&config.backup_directory.value, "draft_tracker").unwrap();
    assert_eq!(kept.len(), 3, "kept {kept:?}");
    // Oldest first, so the last is the newest, and it must match the live log.
    let newest = backup::verify(kept.last().unwrap()).unwrap();
    assert_eq!(newest.words, *totals.last().unwrap());
    // And the three kept are the three most recent states, in order.
    let words: Vec<i64> = kept
        .iter()
        .map(|p| backup::verify(p).unwrap().words)
        .collect();
    assert_eq!(words, totals[2..].to_vec(), "kept the wrong generations");
}

#[test]
fn a_retention_of_zero_still_keeps_the_backup_just_written() {
    let scratch = Scratch::new("zero");
    let db = scratch.seeded();
    let config = scratch.config(0);
    backup::run(&db, &config).unwrap();
    add_entry(&db, 10);
    backup::run(&db, &config).unwrap();
    let kept = backup::list(&config.backup_directory.value, "draft_tracker").unwrap();
    assert_eq!(
        kept.len(),
        1,
        "deleting what was just written cannot be what is meant"
    );
}

#[test]
fn backups_of_one_log_never_evict_another() {
    let scratch = Scratch::new("names");
    let db = scratch.seeded();
    let live = scratch.config(2);
    let scratch_db = Config::explicit(
        scratch.path("draft_tracker.db"),
        scratch.path("backups"),
        2,
        "dev",
    );

    for words in [1, 2, 3] {
        add_entry(&db, words);
        backup::run(&db, &live).unwrap();
        add_entry(&db, words);
        backup::run(&db, &scratch_db).unwrap();
    }
    assert_eq!(
        backup::list(&live.backup_directory.value, "draft_tracker")
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        backup::list(&live.backup_directory.value, "dev")
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn listing_ignores_anything_that_is_not_one_of_ours() {
    let scratch = Scratch::new("foreign");
    let db = scratch.seeded();
    let config = scratch.config(5);
    backup::run(&db, &config).unwrap();

    let directory = &config.backup_directory.value;
    std::fs::write(directory.join("notes.txt"), "hello").unwrap();
    std::fs::write(directory.join("something-else-2026.db"), "x").unwrap();
    assert_eq!(backup::list(directory, "draft_tracker").unwrap().len(), 1);
}

#[test]
fn listing_a_directory_that_does_not_exist_is_not_an_error() {
    let scratch = Scratch::new("absent");
    assert!(
        backup::list(&scratch.path("nowhere"), "draft_tracker")
            .unwrap()
            .is_empty()
    );
}

// ------------------------------------------------------------ explicit copies

#[test]
fn a_named_destination_is_written_exactly_there() {
    let scratch = Scratch::new("named");
    let db = scratch.seeded();
    let destination = scratch.path("archive/january.db");
    backup::write_to(&db, &destination).unwrap();
    assert!(destination.exists(), "the parent directory is created");
    assert_eq!(backup::verify(&destination).unwrap().words, 1_000);
}

/// `VACUUM INTO` refusing an existing file is the behaviour wanted: a mistyped
/// path cannot quietly replace an archive.
#[test]
fn a_named_destination_is_never_overwritten() {
    let scratch = Scratch::new("noclobber");
    let db = scratch.seeded();
    let destination = scratch.path("once.db");
    backup::write_to(&db, &destination).unwrap();
    let before = bytes_of(&destination);

    add_entry(&db, 999);
    assert!(backup::write_to(&db, &destination).is_err());
    assert_eq!(
        bytes_of(&destination),
        before,
        "the existing file is untouched"
    );
}

// ------------------------------------------------------------ failing safely

#[test]
fn a_failure_is_reported_as_a_message_rather_than_stopping_anything() {
    let scratch = Scratch::new("failure");
    let db = scratch.seeded();
    // A path that cannot be created, so the backup cannot be taken.
    let config = Config::explicit(
        scratch.path("draft_tracker.db"),
        "/proc/draft_tracker_cannot_exist/backups",
        5,
        "draft_tracker",
    );
    let message = backup::best_effort(&db, &config).unwrap_err();
    assert!(message.contains("backup directory"), "got {message:?}");
    // And the log itself is untouched and still usable.
    assert_eq!(repo::project_logged_total(db.conn(), 1).unwrap(), 1_000);
}

#[test]
fn a_backup_taken_later_sorts_after_one_taken_earlier() {
    let scratch = Scratch::new("ordering");
    let db = scratch.seeded();
    let config = scratch.config(10);

    let first = backup::run(&db, &config)
        .unwrap()
        .written_path()
        .unwrap()
        .to_path_buf();
    add_entry(&db, 5);
    let second = backup::run(&db, &config)
        .unwrap()
        .written_path()
        .unwrap()
        .to_path_buf();

    let listed = backup::list(&config.backup_directory.value, "draft_tracker").unwrap();
    assert_eq!(listed, vec![first, second], "oldest first");
}

#[test]
fn the_timestamp_in_a_name_is_the_time_it_was_taken() {
    let scratch = Scratch::new("stamp");
    let db = scratch.seeded();
    let config = scratch.config(5);
    let when = Local::now() - Duration::hours(3);
    let path = backup::run_at(&db, &config, when)
        .unwrap()
        .written_path()
        .unwrap()
        .to_path_buf();
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.contains(&when.format("%Y-%m-%dT%H%M%S").to_string()),
        "got {name:?}"
    );
}
