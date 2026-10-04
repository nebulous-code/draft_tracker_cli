//! The command line logging path: the thing done most often.

use chrono::{Local, NaiveDate};
use draft_tracker::MIN_SESSION_HOURS;
use draft_tracker::cli::{self, LogError, Request};
use draft_tracker::db::models::{NewEntry, Status};
use draft_tracker::db::{Db, repo};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn request(project: &str, words: i64, hours: f64) -> Request {
    Request {
        project: project.to_string(),
        words,
        hours,
        date: None,
        notes: None,
        include_closed: false,
    }
}

/// Projects with overlapping names, one of them closed.
fn seeded() -> Db {
    let db = Db::open_in_memory().unwrap();
    {
        let conn = db.conn();
        let fiction = repo::ensure_type(conn, "Fiction").unwrap();
        let book = repo::ensure_category(conn, "SitR Book", fiction).unwrap();

        let fireside =
            repo::insert_project(conn, "Fireside 1", Some(book), Status::Open, None).unwrap();
        repo::insert_entry(
            conn,
            &NewEntry {
                entry_date: day(2026, 1, 12),
                project_id: fireside,
                session_word_count: 17_228,
                hours: 25.75,
                notes: None,
            },
        )
        .unwrap();

        repo::insert_project(conn, "Glowstones", Some(book), Status::Open, None).unwrap();
        repo::insert_project(conn, "Brand New", Some(book), Status::Open, None).unwrap();
        repo::insert_project(conn, "Flask Vol 1", Some(book), Status::Completed, None).unwrap();
        // A title that is wholly contained in another, to pin exact-match-wins.
        repo::insert_project(conn, "Flask", Some(book), Status::Open, None).unwrap();
    }
    db
}

fn summary(planned: &cli::Planned) -> String {
    let mut out = Vec::new();
    cli::write_summary(&mut out, planned).unwrap();
    String::from_utf8(out).unwrap()
}

// ------------------------------------------------------------ resolving

#[test]
fn a_partial_title_is_enough() {
    let db = seeded();
    let planned = cli::plan(&db, &request("fire", 17_900, 1.0)).unwrap();
    assert_eq!(planned.project_title, "Fireside 1");
}

#[test]
fn matching_ignores_case() {
    let db = seeded();
    assert_eq!(
        cli::plan(&db, &request("GLOWSTONES", 100, 1.0))
            .unwrap()
            .project_title,
        "Glowstones"
    );
}

/// Without this, a project whose name sits inside another's could never be
/// named: "Flask" would always be ambiguous against "Flask Vol 1".
#[test]
fn an_exact_title_wins_over_a_longer_one_containing_it() {
    let db = seeded();
    let planned = cli::plan(&db, &request("Flask", 500, 1.0)).unwrap();
    assert_eq!(planned.project_title, "Flask");
}

#[test]
fn an_ambiguous_fragment_is_refused_with_the_candidates() {
    let db = seeded();
    let error = cli::plan(&db, &request("n", 100, 1.0)).unwrap_err();
    match error {
        LogError::Ambiguous { matches, .. } => {
            assert!(matches.len() > 1, "got {matches:?}");
            assert!(
                matches.contains(&"Brand New".to_string()),
                "got {matches:?}"
            );
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }
}

#[test]
fn an_unknown_project_lists_the_open_ones() {
    let db = seeded();
    let error = cli::plan(&db, &request("nothing like this", 100, 1.0)).unwrap_err();
    match error {
        LogError::NoMatch { open, .. } => {
            assert!(open.contains(&"Fireside 1".to_string()), "got {open:?}");
            assert!(
                !open.contains(&"Flask Vol 1".to_string()),
                "a completed project is not an open one: {open:?}"
            );
        }
        other => panic!("expected NoMatch, got {other:?}"),
    }
}

/// Saying "that project is finished" is more use than "no such project".
#[test]
fn a_closed_project_says_so_rather_than_claiming_it_does_not_exist() {
    let db = seeded();
    let error = cli::plan(&db, &request("Flask Vol", 100, 1.0)).unwrap_err();
    match error {
        LogError::Closed { title, status } => {
            assert_eq!(title, "Flask Vol 1");
            assert_eq!(status, Status::Completed);
        }
        other => panic!("expected Closed, got {other:?}"),
    }
}

#[test]
fn any_allows_logging_against_a_closed_project() {
    let db = seeded();
    let mut req = request("Flask Vol", 500, 1.0);
    req.include_closed = true;
    assert_eq!(cli::plan(&db, &req).unwrap().project_title, "Flask Vol 1");
}

#[test]
fn an_empty_database_says_there_are_no_projects() {
    let db = Db::open_in_memory().unwrap();
    assert!(matches!(
        cli::plan(&db, &request("anything", 100, 1.0)).unwrap_err(),
        LogError::NoProjects
    ));
}

// ------------------------------------------------------------ the arithmetic

#[test]
fn a_typed_total_is_measured_against_what_is_logged() {
    let db = seeded();
    let planned = cli::plan(&db, &request("fire", 17_900, 1.0)).unwrap();
    assert!(!planned.first_entry);
    assert_eq!(planned.logged_before, 17_228);
    assert_eq!(planned.session_words, 672);
    assert_eq!(planned.words_per_hour(), Some(672.0));
}

#[test]
fn a_first_entry_takes_the_number_as_the_session() {
    let db = seeded();
    let planned = cli::plan(&db, &request("brand", 1_200, 2.0)).unwrap();
    assert!(planned.first_entry);
    assert_eq!(planned.session_words, 1_200);
    assert!(
        summary(&planned).contains("first entry: taken as this session, not a total"),
        "the one place the number means something else must say so:\n{}",
        summary(&planned)
    );
}

#[test]
fn a_revision_day_is_allowed_and_spelled_out() {
    let db = seeded();
    let planned = cli::plan(&db, &request("fire", 16_000, 1.0)).unwrap();
    assert_eq!(planned.session_words, -1_228);
    let text = summary(&planned);
    assert!(text.contains("-1,228"), "got:\n{text}");
    assert!(text.contains("This cuts 1,228 words"), "got:\n{text}");
}

#[test]
fn the_hours_minimum_is_refused_before_anything_else_happens() {
    let db = seeded();
    for hours in [0.0, 0.1, 0.24, -1.0, f64::NAN] {
        let error = cli::plan(&db, &request("fire", 17_900, hours)).unwrap_err();
        assert!(
            matches!(error, LogError::HoursTooShort(_)),
            "{hours} gave {error:?}"
        );
    }
    // Including when the project would not have resolved either: the rule is
    // checked first, so the message is about the real problem.
    assert!(matches!(
        cli::plan(&db, &request("nonsense", 1, 0.0)).unwrap_err(),
        LogError::HoursTooShort(_)
    ));
    assert!(cli::plan(&db, &request("fire", 17_900, MIN_SESSION_HOURS)).is_ok());
}

#[test]
fn the_summary_shows_the_reference_total_so_a_surprise_can_be_reasoned_about() {
    let db = seeded();
    let planned = cli::plan(&db, &request("fire", 17_900, 1.0)).unwrap();
    let text = summary(&planned);
    assert!(text.contains("logged so far 17,228"), "got:\n{text}");
    assert!(text.contains("Session WC   672"), "got:\n{text}");
}

// ------------------------------------------------------------ writing

#[test]
fn planning_writes_nothing() {
    let db = seeded();
    let before = repo::recent_entries(db.conn(), 100).unwrap().len();
    cli::plan(&db, &request("fire", 17_900, 1.0)).unwrap();
    assert_eq!(repo::recent_entries(db.conn(), 100).unwrap().len(), before);
}

#[test]
fn committing_writes_exactly_what_was_shown() {
    let db = seeded();
    let mut req = request("fire", 17_900, 1.5);
    req.notes = Some("a good morning".into());
    req.date = Some(day(2026, 2, 3));
    let planned = cli::plan(&db, &req).unwrap();
    let id = cli::commit(&db, &planned).unwrap();

    let entries = repo::recent_entries(db.conn(), 100).unwrap();
    let written = entries.iter().find(|e| e.id == id).expect("the new entry");
    assert_eq!(written.session_word_count, 672);
    assert_eq!(written.hours, 1.5);
    assert_eq!(written.entry_date, day(2026, 2, 3));
    assert_eq!(written.notes.as_deref(), Some("a good morning"));
    assert_eq!(
        repo::project_logged_total(db.conn(), planned.project_id).unwrap(),
        17_900
    );
}

#[test]
fn the_date_defaults_to_today() {
    let db = seeded();
    let planned = cli::plan(&db, &request("fire", 17_900, 1.0)).unwrap();
    assert_eq!(planned.date, Local::now().date_naive());
}

#[test]
fn two_sessions_in_a_row_each_measure_against_the_new_total() {
    let db = seeded();
    let first = cli::plan(&db, &request("fire", 17_900, 1.0)).unwrap();
    cli::commit(&db, &first).unwrap();
    let second = cli::plan(&db, &request("fire", 18_400, 1.0)).unwrap();
    assert_eq!(second.logged_before, 17_900);
    assert_eq!(second.session_words, 500);
}

// ------------------------------------------------------------ argument parsing

#[test]
fn a_word_count_may_carry_separators() {
    assert_eq!(cli::parse_words("17228"), Some(17_228));
    assert_eq!(cli::parse_words("17,228"), Some(17_228));
    assert_eq!(cli::parse_words("17_228"), Some(17_228));
    assert_eq!(cli::parse_words(" 500 "), Some(500));
    assert_eq!(cli::parse_words("-400"), Some(-400));
    assert_eq!(cli::parse_words("lots"), None);
    assert_eq!(cli::parse_words("1.5"), None);
}

#[test]
fn a_date_may_be_a_word_or_a_date() {
    assert_eq!(
        cli::parse_date_argument("today"),
        Some(Local::now().date_naive())
    );
    assert_eq!(
        cli::parse_date_argument("YESTERDAY"),
        Local::now().date_naive().pred_opt()
    );
    assert_eq!(
        cli::parse_date_argument("2026-01-14"),
        Some(day(2026, 1, 14))
    );
    assert_eq!(cli::parse_date_argument("01/14/26"), Some(day(2026, 1, 14)));
    assert_eq!(cli::parse_date_argument("the other day"), None);
}
