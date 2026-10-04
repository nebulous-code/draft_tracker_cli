//! Command line entry point.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use draft_tracker::cli;
use draft_tracker::config::{DB_PATH_ENV, resolve_db_path};
use draft_tracker::db::{Db, stats};
use draft_tracker::import::{self, Existing};
use draft_tracker::tui;

const USAGE: &str = "\
draft-tracker - track writing sessions, projects and word counts

Usage:
  draft-tracker log <project> <words> <hours> [options]
  draft-tracker [--db <path>]
  draft-tracker [--db <path>] status
  draft-tracker [--db <path>] import <dir> [--reset] [--dry-run]
  draft-tracker --help

Logging a session:
  <project>     all or part of an open project's title, case insensitive
  <words>       the project's new total word count, or the session's words on
                a project's first entry
  <hours>       decimal hours, at least 0.25

  -d, --date <when>    today (the default), yesterday, or a date
  -n, --notes <text>   a note for the session
  -y, --yes            save without asking
      --any            allow a completed or abandoned project

Options:
  --db <path>   use this database instead of the default location
  --reset       let import replace data already in the database
  --dry-run     let import report what it would do, writing nothing
  --help, -h    show this message

With no command, the terminal interface opens on the entry form.
";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
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

/// Everything the argument list carried.
#[derive(Default)]
struct Args {
    db: Option<PathBuf>,
    command: Option<String>,
    positional: Vec<String>,
    reset: bool,
    dry_run: bool,
    yes: bool,
    any: bool,
    date: Option<String>,
    notes: Option<String>,
    help: bool,
}

fn parse_args() -> Result<Args> {
    let mut args = Args::default();
    let mut raw = std::env::args().skip(1);
    while let Some(arg) = raw.next() {
        let mut value = |flag: &str| -> Result<String> {
            raw.next().with_context(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "--help" | "-h" => args.help = true,
            "--db" => args.db = Some(PathBuf::from(value("--db")?)),
            "--reset" => args.reset = true,
            "--dry-run" => args.dry_run = true,
            "--yes" | "-y" => args.yes = true,
            "--any" => args.any = true,
            "--date" | "-d" => args.date = Some(value("--date")?),
            "--notes" | "-n" => args.notes = Some(value("--notes")?),
            other if other.starts_with('-') && other.len() > 1 && !is_negative_number(other) => {
                bail!("unknown option {other}; try --help")
            }
            other if args.command.is_none() => args.command = Some(other.to_string()),
            other => args.positional.push(other.to_string()),
        }
    }
    Ok(args)
}

/// `-500` is a word count on a revision day, not a flag.
fn is_negative_number(arg: &str) -> bool {
    arg.strip_prefix('-').is_some_and(|rest| {
        !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == ',')
    })
}

fn run() -> Result<ExitCode> {
    let args = parse_args()?;
    if args.help {
        let mut out = io::stdout().lock();
        out.write_all(USAGE.as_bytes())?;
        return Ok(ExitCode::SUCCESS);
    }

    let path = resolve_db_path(args.db.clone())?;
    match args.command.as_deref() {
        // The entry form is the daily path in the interface, so it opens by default.
        None => {
            let db = Db::open(&path).with_context(|| format!("opening {}", path.display()))?;
            tui::run(db)?;
            Ok(ExitCode::SUCCESS)
        }
        Some("log") => log(&path, &args),
        Some("status") => {
            let mut out = io::stdout().lock();
            let interactive = io::stdout().is_terminal();
            status(&path, &mut out, interactive)?;
            out.flush()?;
            Ok(ExitCode::SUCCESS)
        }
        Some("import") => {
            let [dir] = args.positional.as_slice() else {
                bail!("import needs exactly one directory; try --help");
            };
            let mut out = io::stdout().lock();
            import_journals(&path, Path::new(dir), args.reset, args.dry_run, &mut out)?;
            out.flush()?;
            Ok(ExitCode::SUCCESS)
        }
        Some(other) => bail!("unknown command {other}; try --help"),
    }
}

/// Log one session, after showing it and asking.
fn log(db_path: &Path, args: &Args) -> Result<ExitCode> {
    let [project, words, hours] = args.positional.as_slice() else {
        bail!(
            "log needs a project, a word count and hours, for example:\n  \
             draft-tracker log fireside 15682 1\nTry --help for the details."
        );
    };
    let request = cli::Request {
        project: project.clone(),
        words: cli::parse_words(words)
            .with_context(|| format!("{words:?} is not a whole number of words"))?,
        hours: hours
            .trim()
            .parse()
            .with_context(|| format!("{hours:?} is not a number of hours"))?,
        date: match &args.date {
            None => None,
            Some(raw) => Some(
                cli::parse_date_argument(raw)
                    .with_context(|| format!("{raw:?} is not a date; try 2026-01-14"))?,
            ),
        },
        notes: args.notes.clone(),
        include_closed: args.any,
    };

    let db = Db::open(db_path).with_context(|| format!("opening {}", db_path.display()))?;
    let planned = match cli::plan(&db, &request) {
        Ok(planned) => planned,
        Err(error) => {
            let mut err = io::stderr().lock();
            writeln!(err, "draft-tracker: {error}")?;
            cli::write_error_help(&mut err, &error)?;
            return Ok(ExitCode::FAILURE);
        }
    };

    let mut out = io::stdout().lock();
    cli::write_summary(&mut out, &planned)?;

    if !args.yes {
        write!(out, "\nSave this entry? [y/N] ")?;
        out.flush()?;
        match read_answer()? {
            Answer::Yes => {}
            Answer::No => {
                writeln!(out, "Nothing saved.")?;
                return Ok(ExitCode::SUCCESS);
            }
            // Nothing to read from: a cron job or a redirect from /dev/null.
            // Refusing is the safe reading of silence, and --yes is the way to
            // say yes in advance.
            Answer::NothingToRead => {
                writeln!(out)?;
                bail!("no answer to read; pass --yes to save without asking");
            }
        }
    }

    cli::commit(&db, &planned)?;
    let total = planned.logged_before + planned.session_words;
    writeln!(
        out,
        "Saved. {} is now at {} words.",
        planned.project_title,
        cli::thousands(total)
    )?;
    out.flush()?;
    Ok(ExitCode::SUCCESS)
}

enum Answer {
    Yes,
    No,
    NothingToRead,
}

/// Read one line and decide what it means. Anything but yes is a no.
///
/// Read from whatever stdin is rather than insisting on a terminal, so piping an
/// answer in works; end of input is distinguished from a no, because a writer
/// who answered nothing has not declined, they were never asked.
fn read_answer() -> Result<Answer> {
    let mut answer = String::new();
    let read = io::stdin().lock().read_line(&mut answer)?;
    if read == 0 {
        return Ok(Answer::NothingToRead);
    }
    Ok(match answer.trim().to_lowercase().as_str() {
        "y" | "yes" => Answer::Yes,
        _ => Answer::No,
    })
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

    // The open projects, because logging from the command line needs their names.
    let open = draft_tracker::db::repo::list_open_projects(conn)?;
    if !open.is_empty() {
        writeln!(out)?;
        writeln!(out, "Open projects")?;
        for project in open {
            let logged = draft_tracker::db::repo::project_logged_total(conn, project.id)?;
            writeln!(out, "  {:<30} {logged:>9}", project.title)?;
        }
    }
    Ok(())
}
