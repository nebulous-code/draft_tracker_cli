//! End-to-end tests that run the built binary, rather than calling into the
//! library. These are the only tests that exercise argument parsing, exit codes
//! and the confirmation prompt the way a shell does.

use std::process::{Command, Stdio};

/// Cargo hands integration tests the path to the binary it built.
const BIN: &str = env!("CARGO_BIN_EXE_draft-tracker");

struct TempDb(std::path::PathBuf);

impl TempDb {
    fn new(label: &str) -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!("dt_bin_{label}_{}_{n}.db", std::process::id())))
    }

    fn path(&self) -> &str {
        self.0.to_str().expect("a utf-8 temp path")
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

struct Output {
    status: i32,
    stdout: String,
    stderr: String,
}

fn run_with_stdin(args: &[&str], stdin: Option<&str>) -> Output {
    use std::io::Write;
    let mut child = Command::new(BIN)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary runs");
    if let Some(text) = stdin {
        child
            .stdin
            .as_mut()
            .expect("piped stdin")
            .write_all(text.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().expect("it finishes");
    Output {
        status: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn run(args: &[&str]) -> Output {
    run_with_stdin(args, None)
}

#[test]
fn version_reports_the_build_it_came_from() {
    for flag in ["--version", "-V"] {
        let out = run(&[flag]);
        assert_eq!(out.status, 0, "{flag}: {}", out.stderr);
        let line = out.stdout.trim();
        assert!(line.starts_with("draft-tracker "), "got {line:?}");
        assert!(line.contains(env!("CARGO_PKG_VERSION")), "got {line:?}");
        // Built from a checkout in development, or "unknown" from a tarball;
        // either way the profile is always stated.
        assert!(
            line.contains("debug)") || line.contains("release)"),
            "the profile distinguishes an installed copy from a working build: {line:?}"
        );
    }
}

#[test]
fn help_explains_itself_and_succeeds() {
    let out = run(&["--help"]);
    assert_eq!(out.status, 0);
    for expected in ["Usage:", "log <project>", "--version", "import"] {
        assert!(
            out.stdout.contains(expected),
            "{expected:?} missing from --help"
        );
    }
}

#[test]
fn an_unknown_option_fails_rather_than_being_ignored() {
    let out = run(&["--nonsense"]);
    assert_eq!(out.status, 1);
    assert!(
        out.stderr.contains("unknown option"),
        "got {:?}",
        out.stderr
    );
}

#[test]
fn an_unknown_command_fails() {
    let out = run(&["frobnicate"]);
    assert_eq!(out.status, 1);
    assert!(
        out.stderr.contains("unknown command"),
        "got {:?}",
        out.stderr
    );
}

#[test]
fn status_on_a_fresh_database_says_there_is_nothing_in_it() {
    let db = TempDb::new("fresh");
    let out = run(&["--db", db.path(), "status"]);
    assert_eq!(out.status, 0, "{}", out.stderr);
    assert!(
        out.stdout.contains("No projects or entries yet"),
        "got {:?}",
        out.stdout
    );
}

#[test]
fn logging_needs_its_three_arguments() {
    let db = TempDb::new("args");
    let out = run(&["--db", db.path(), "log", "something"]);
    assert_eq!(out.status, 1);
    assert!(
        out.stderr.contains("needs a project"),
        "got {:?}",
        out.stderr
    );
}

#[test]
fn logging_against_an_empty_database_says_there_are_no_projects() {
    let db = TempDb::new("empty");
    let out = run(&["--db", db.path(), "log", "anything", "100", "1"]);
    assert_eq!(out.status, 1);
    assert!(
        out.stderr.contains("no projects yet"),
        "got {:?}",
        out.stderr
    );
}

#[test]
fn an_answer_can_be_piped_in() {
    let db = TempDb::new("piped");
    // A database with one project, created through the library the binary shares.
    {
        use draft_tracker::db::models::Status;
        use draft_tracker::db::{Db, repo};
        let handle = Db::open(db.path()).unwrap();
        repo::insert_project(handle.conn(), "Glasshouse", None, Status::Open, None).unwrap();
    }

    let declined = run_with_stdin(
        &["--db", db.path(), "log", "glass", "640", "1"],
        Some("n\n"),
    );
    assert_eq!(declined.status, 0);
    assert!(
        declined.stdout.contains("Nothing saved"),
        "got {:?}",
        declined.stdout
    );

    let accepted = run_with_stdin(
        &["--db", db.path(), "log", "glass", "640", "1"],
        Some("y\n"),
    );
    assert_eq!(accepted.status, 0, "{}", accepted.stderr);
    assert!(
        accepted.stdout.contains("Saved."),
        "got {:?}",
        accepted.stdout
    );
    assert!(accepted.stdout.contains("640"), "got {:?}", accepted.stdout);

    // And with nothing to read, it refuses rather than guessing.
    let silent = run(&["--db", db.path(), "log", "glass", "1200", "1"]);
    assert_eq!(silent.status, 1);
    assert!(
        silent.stderr.contains("no answer to read"),
        "got {:?}",
        silent.stderr
    );
}
