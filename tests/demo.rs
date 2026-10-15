//! Tests for the demonstration data generator.
//!
//! The point of a checked-in fixture is that someone can clone the repository and
//! see the program working. That only holds if the fixture still loads, so these
//! tests assert both the shape of the data and the presence of the awkward cases
//! it exists to show.

use std::collections::BTreeSet;

use chrono::{Datelike, Duration, Local};
use draft_tracker::MIN_SESSION_HOURS;
use draft_tracker::db::models::Status;
use draft_tracker::db::{Db, repo, stats};
use draft_tracker::demo::{self, DemoError};
use rusqlite::Connection;

fn generated() -> (Db, demo::Summary) {
    let mut db = Db::open_in_memory().unwrap();
    let summary = demo::populate(&mut db).unwrap();
    (db, summary)
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

#[test]
fn populates_an_empty_database() {
    let (db, summary) = generated();
    let conn = db.conn();

    assert_eq!(
        summary.types,
        count(conn, "SELECT COUNT(*) FROM types") as usize
    );
    assert_eq!(
        summary.categories,
        count(conn, "SELECT COUNT(*) FROM categories") as usize
    );
    assert_eq!(
        summary.projects,
        count(conn, "SELECT COUNT(*) FROM projects") as usize
    );
    assert_eq!(
        summary.entries,
        count(conn, "SELECT COUNT(*) FROM entries") as usize
    );
}

#[test]
fn summary_totals_match_the_rows() {
    let (db, summary) = generated();
    let conn = db.conn();

    let words: i64 = conn
        .query_row("SELECT SUM(session_word_count) FROM entries", [], |r| {
            r.get(0)
        })
        .unwrap();
    let hours: f64 = conn
        .query_row("SELECT SUM(hours) FROM entries", [], |r| r.get(0))
        .unwrap();

    assert_eq!(summary.words, words);
    assert!((summary.hours - hours).abs() < 1e-9);
}

#[test]
fn types_are_only_fiction_and_non_fiction() {
    let (db, _) = generated();
    let mut names: Vec<String> = db
        .conn()
        .prepare("SELECT name FROM types")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec!["Fiction".to_string(), "Non-Fiction".to_string()]
    );
}

#[test]
fn every_category_has_a_type_and_every_project_a_category() {
    let (db, _) = generated();
    let conn = db.conn();
    assert_eq!(
        count(
            conn,
            "SELECT COUNT(*) FROM categories WHERE type_id IS NULL"
        ),
        0
    );
    assert_eq!(
        count(
            conn,
            "SELECT COUNT(*) FROM projects WHERE category_id IS NULL"
        ),
        0
    );
}

/// The settled history spans 2023 to 2026, and the daily project adds whichever
/// year or two its run falls in.
///
/// Spelled out as an exact set rather than as "four consecutive years", which is
/// what this asserted while every date was fixed. The daily project is dated from
/// the clock, so that assertion passed on the day it was written and would have
/// failed on 1 January 2027 with nothing in this crate having changed. A test that
/// breaks on a date rather than on a mistake is worse than no test, because the
/// failure teaches the reader to distrust the suite.
#[test]
fn covers_the_settled_history_and_the_run_up_to_today() {
    let (_, summary) = generated();

    // The run ends yesterday and is fourteen days long, so it touches yesterday's
    // year and the year thirteen days before it -- the same year, except across a
    // new year, where both belong on the list.
    let last = Local::now()
        .date_naive()
        .pred_opt()
        .expect("there is a day before today");
    let first = last - Duration::days(13);

    let expected: BTreeSet<i32> = [2023, 2024, 2025, 2026, first.year(), last.year()]
        .into_iter()
        .collect();
    let actual: BTreeSet<i32> = summary.years.iter().copied().collect();
    assert_eq!(actual, expected, "years: {:?}", summary.years);

    // Ascending, because the stats screen walks them in order.
    assert!(
        summary.years.windows(2).all(|p| p[0] < p[1]),
        "years: {:?}",
        summary.years
    );
}

#[test]
fn volume_is_modest_but_looks_like_real_work() {
    let (_, summary) = generated();
    assert!(
        (100..=400).contains(&summary.entries),
        "entries: {}",
        summary.entries
    );
    assert!(
        (50_000..=300_000).contains(&summary.words),
        "words: {}",
        summary.words
    );
    // A believable rate. A fixture that reads as 5,000 words an hour teaches the
    // wrong thing about the metric.
    let per_hour = summary.words as f64 / summary.hours;
    assert!(
        (300.0..=1_500.0).contains(&per_hour),
        "words per hour: {per_hour}"
    );
}

#[test]
fn all_three_statuses_are_represented() {
    let (db, _) = generated();
    let conn = db.conn();
    for status in Status::ALL {
        assert!(
            count(
                conn,
                &format!(
                    "SELECT COUNT(*) FROM projects WHERE status = '{}'",
                    status.as_str()
                )
            ) > 0,
            "no project with status {}",
            status.as_str()
        );
    }
}

#[test]
fn includes_a_revision_day_that_cuts_words() {
    let (db, _) = generated();
    assert!(
        count(
            db.conn(),
            "SELECT COUNT(*) FROM entries WHERE session_word_count < 0"
        ) > 0,
        "nothing shows that a session can remove words"
    );
}

#[test]
fn includes_a_session_with_time_but_no_words() {
    let (db, _) = generated();
    assert!(
        count(
            db.conn(),
            "SELECT COUNT(*) FROM entries WHERE session_word_count = 0"
        ) > 0,
        "nothing shows a session that spent time without a net gain"
    );
}

#[test]
fn includes_two_sessions_for_one_project_on_one_day() {
    let (db, _) = generated();
    assert!(
        count(
            db.conn(),
            "SELECT COUNT(*) FROM (SELECT 1 FROM entries
               GROUP BY project_id, entry_date HAVING COUNT(*) > 1)"
        ) > 0,
        "nothing shows that a day can hold two sessions on one project"
    );
}

#[test]
fn includes_a_project_with_a_single_entry() {
    let (db, _) = generated();
    assert!(
        count(
            db.conn(),
            "SELECT COUNT(*) FROM (SELECT 1 FROM entries GROUP BY project_id HAVING COUNT(*) = 1)"
        ) > 0,
        "nothing shows a project finished in one sitting"
    );
}

#[test]
fn includes_a_project_spanning_two_calendar_years() {
    let (db, _) = generated();
    assert!(
        count(
            db.conn(),
            "SELECT COUNT(*) FROM (SELECT 1 FROM entries GROUP BY project_id
               HAVING strftime('%Y', MIN(entry_date)) <> strftime('%Y', MAX(entry_date)))"
        ) > 0,
        "nothing shows a project carrying across a year boundary"
    );
}

/// The stats screen's project graph scrolls, and the category legend wraps. Both
/// only show up when there is more to display than fits, so the fixture has to be
/// big enough to overflow them.
#[test]
fn there_are_more_projects_than_the_stats_graph_can_list() {
    let (db, summary) = generated();
    assert!(
        summary.projects >= 15,
        "only {} projects, so the graph never needs to scroll",
        summary.projects
    );

    let busiest: i64 = db
        .conn()
        .query_row(
            "SELECT MAX(n) FROM (SELECT COUNT(DISTINCT project_id) n FROM entries
               GROUP BY strftime('%Y', entry_date))",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        busiest >= 6,
        "the busiest year holds only {busiest} projects"
    );

    // Five categories wrap the legend onto a second line on a normal terminal.
    assert!(
        summary.categories >= 5,
        "categories: {}",
        summary.categories
    );
}

#[test]
fn every_session_respects_the_quarter_hour_floor() {
    let (db, _) = generated();
    let shortest: f64 = db
        .conn()
        .query_row("SELECT MIN(hours) FROM entries", [], |row| row.get(0))
        .unwrap();
    assert!(
        shortest >= MIN_SESSION_HOURS,
        "shortest session: {shortest}"
    );
}

#[test]
fn the_screens_have_something_to_show() {
    let (db, _) = generated();
    let conn = db.conn();

    // Projects screen.
    let projects = repo::list_projects_with_metrics(conn).unwrap();
    assert!(projects.len() >= 10);
    assert!(
        projects
            .iter()
            .any(|p| p.metrics.words_per_hour().is_some()),
        "no project has a derivable rate"
    );

    // History screen.
    assert!(repo::recent_entries_with_project(conn, None).unwrap().len() >= 50);

    // Stats, for every year the data covers and for all time.
    for year in [None, Some(2024), Some(2025)] {
        let totals = stats::totals(conn, year).unwrap();
        assert!(totals.words > 0, "no words for {year:?}");
        assert!(totals.hours > 0.0, "no hours for {year:?}");
    }
}

/// Two runs produce the same summary.
///
/// Determinism given a fixed today, rather than across all time: the daily
/// project's dates come from the clock. That is the property worth having, since
/// it is what keeps the word and hour totals -- and so every figure in the
/// README -- reproducible from the command alone.
#[test]
fn generation_is_deterministic() {
    let (_, first) = generated();
    let (_, second) = generated();
    assert_eq!(first, second);
}

#[test]
fn refuses_a_database_that_already_holds_entries() {
    let mut db = Db::open_in_memory().unwrap();
    demo::populate(&mut db).unwrap();
    assert!(matches!(demo::populate(&mut db), Err(DemoError::NotEmpty)));
}

#[test]
fn refuses_a_database_holding_only_a_taxonomy() {
    let mut db = Db::open_in_memory().unwrap();
    repo::ensure_type(db.conn(), "Fiction").unwrap();
    assert!(matches!(demo::populate(&mut db), Err(DemoError::NotEmpty)));
}

#[test]
fn a_refusal_leaves_the_database_untouched() {
    let mut db = Db::open_in_memory().unwrap();
    repo::ensure_type(db.conn(), "Fiction").unwrap();
    assert!(demo::populate(&mut db).is_err());
    assert_eq!(count(db.conn(), "SELECT COUNT(*) FROM projects"), 0);
    assert_eq!(count(db.conn(), "SELECT COUNT(*) FROM types"), 1);
}
