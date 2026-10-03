//! Command line entry point.
//!
//! The terminal UI is not built yet. For now this opens the database, applies
//! migrations and reports what is there, which is enough to exercise the data
//! layer against a real file.

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use draft_tracker::config::{DB_PATH_ENV, resolve_db_path};
use draft_tracker::db::{Db, stats};
use draft_tracker::import::{self, Existing};
use draft_tracker::tui;

const USAGE: &str = "\
draft-tracker - track writing sessions, projects and word counts

Usage:
  draft-tracker [--db <path>]
  draft-tracker [--db <path>] status
  draft-tracker [--db <path>] import <dir> [--reset] [--dry-run]
  draft-tracker --help

Options:
  --db <path>   use this database instead of the default location
  --reset       let import replace data already in the database
  --dry-run     let import report what it would do, writing nothing
  --help, -h    show this message

With no command, the terminal interface opens on the entry form.

The default database is $XDG_DATA_HOME/draft_tracker/draft_tracker.db, or the
platform equivalent. It can also be set with the DRAFT_TRACKER_DB variable.
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        // A closed pipe is how `| head` ends a stream, not a failure. Rust
        // ignores SIGPIPE, so without this the process panics mid-write.
        Err(err) if is_broken_pipe(&err) => ExitCode::SUCCESS,
        Err(err) => {
            let _ = writeln!(io::stderr(), "draft-tracker: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
    })
}

fn run() -> Result<()> {
    let mut db_path: Option<PathBuf> = None;
    let mut command: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();
    let mut reset = false;
    let mut dry_run = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                let mut out = io::stdout().lock();
                out.write_all(USAGE.as_bytes())?;
                return Ok(());
            }
            "--db" => {
                let value = args.next().context("--db needs a path")?;
                db_path = Some(PathBuf::from(value));
            }
            "--reset" => reset = true,
            "--dry-run" => dry_run = true,
            other if other.starts_with('-') => bail!("unknown option {other}; try --help"),
            other if command.is_none() => command = Some(other.to_string()),
            other => positional.push(other.to_string()),
        }
    }

    let path = resolve_db_path(db_path)?;
    match command.as_deref() {
        // The entry form is the daily path, so it is what opens by default.
        None => {
            let db = Db::open(&path).with_context(|| format!("opening {}", path.display()))?;
            tui::run(db)
        }
        Some("status") => {
            let mut out = io::stdout().lock();
            let interactive = io::stdout().is_terminal();
            status(&path, &mut out, interactive)?;
            out.flush()?;
            Ok(())
        }
        Some("import") => {
            let [dir] = positional.as_slice() else {
                bail!("import needs exactly one directory; try --help");
            };
            let mut out = io::stdout().lock();
            import_journals(&path, Path::new(dir), reset, dry_run, &mut out)?;
            out.flush()?;
            Ok(())
        }
        Some(other) => bail!("unknown command {other}; try --help"),
    }
}

/// Import a directory of journals, or report what an import would do.
fn import_journals(
    db_path: &Path,
    dir: &Path,
    reset: bool,
    dry_run: bool,
    out: &mut impl Write,
) -> Result<()> {
    if dry_run {
        let resolved = import::plan(dir)
            .with_context(|| format!("reading journals from {}", dir.display()))?;
        writeln!(out, "Dry run: nothing was written.")?;
        writeln!(out)?;
        write!(out, "{}", resolved.report)?;
        return Ok(());
    }

    let mut db = Db::open(db_path).with_context(|| format!("opening {}", db_path.display()))?;
    let existing = if reset {
        Existing::Reset
    } else {
        Existing::Refuse
    };
    let report = import::run(&mut db, dir, existing)
        .with_context(|| format!("importing from {}", dir.display()))?;
    writeln!(out, "Imported into {}", db_path.display())?;
    writeln!(out)?;
    write!(out, "{report}")?;
    Ok(())
}

/// Write a summary of the database to `out`.
///
/// Takes the writer rather than printing directly so that a closed pipe surfaces
/// as an error to handle instead of a panic inside `println!`.
fn status(path: &Path, out: &mut impl Write, interactive: bool) -> Result<()> {
    let db = Db::open(path).with_context(|| format!("opening {}", path.display()))?;
    let conn = db.conn();

    writeln!(out, "database   {}", path.display())?;
    writeln!(out, "schema     {}", db.schema_version()?)?;

    if db.is_empty()? {
        writeln!(out)?;
        writeln!(
            out,
            "No projects or entries yet. Nothing has been imported."
        )?;
        if interactive {
            writeln!(out, "({DB_PATH_ENV} or --db selects a different database.)")?;
        }
        return Ok(());
    }

    let lifetime = stats::totals(conn, None)?;
    writeln!(out)?;
    writeln!(out, "Lifetime")?;
    writeln!(out, "  words      {}", lifetime.words)?;
    writeln!(out, "  hours      {:.2}", lifetime.hours)?;
    writeln!(
        out,
        "  completed  {}",
        stats::completed_project_count(conn, None)?
    )?;

    writeln!(out)?;
    writeln!(out, "Words by year")?;
    for (year, words) in stats::words_by_year(conn)? {
        writeln!(out, "  {year}  {words:>9}")?;
    }
    Ok(())
}
