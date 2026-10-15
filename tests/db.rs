//! Data layer tests. Every one runs against a fresh in-memory database.

use chrono::NaiveDate;
use draft_tracker::MIN_SESSION_HOURS;
use draft_tracker::db::models::{NewEntry, Status};
use draft_tracker::db::{Db, DbError, migrations, repo, stats};
use draft_tracker::streaks;
use rusqlite::params;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

/// A database with one type, one category and one open project.
fn seeded() -> (Db, i64) {
    let db = Db::open_in_memory().unwrap();
    let conn = db.conn();
    let type_id = repo::ensure_type(conn, "Fiction").unwrap();
    let cat_id = repo::ensure_category(conn, "Short Story", type_id).unwrap();
    let project_id =
        repo::insert_project(conn, "Debris Sea", Some(cat_id), Status::Open, None).unwrap();
    (db, project_id)
}

fn add(db: &Db, project_id: i64, date: NaiveDate, words: i64, hours: f64) -> i64 {
    repo::insert_entry(
        db.conn(),
        &NewEntry {
            entry_date: date,
            project_id,
            session_word_count: words,
            hours,
            notes: None,
        },
    )
    .unwrap()
}

// ------------------------------------------------------------ migrations

#[test]
fn migrations_bring_a_fresh_database_to_the_target_version() {
    let db = Db::open_in_memory().unwrap();
    assert_eq!(db.schema_version().unwrap(), migrations::target_version());
    assert!(migrations::target_version() >= 1);
}

#[test]
fn migrating_an_already_current_database_changes_nothing() {
    let mut db = Db::open_in_memory().unwrap();
    let before = db.schema_version().unwrap();
    migrations::migrate(db.conn_mut()).unwrap();
    assert_eq!(db.schema_version().unwrap(), before);
}

#[test]
fn a_database_from_a_newer_build_is_refused_rather_than_downgraded() {
    let db = Db::open_in_memory().unwrap();
    let future = migrations::target_version() + 5;
    db.conn()
        .pragma_update(None, "user_version", future)
        .unwrap();
    let mut db = db;
    let err = migrations::migrate(db.conn_mut()).unwrap_err();
    assert!(
        matches!(err, DbError::NewerSchema { .. }),
        "expected NewerSchema, got {err:?}"
    );
}

// ------------------------------------------------------------ constraints

#[test]
fn status_is_constrained_to_the_three_known_values() {
    let (db, _) = seeded();
    let err = db.conn().execute(
        "INSERT INTO projects (title, status) VALUES ('Nope', 'finished')",
        [],
    );
    assert!(err.is_err(), "an unknown status should violate the CHECK");
}

#[test]
fn every_status_round_trips_through_sql() {
    let db = Db::open_in_memory().unwrap();
    for status in Status::ALL {
        let id = repo::insert_project(db.conn(), &format!("project {status}"), None, status, None)
            .unwrap();
        assert_eq!(repo::get_project(db.conn(), id).unwrap().status, status);
    }
}

#[test]
fn a_session_shorter_than_the_minimum_is_rejected_before_it_reaches_sqlite() {
    let (db, project_id) = seeded();
    for hours in [0.0, 0.1, 0.24, -1.0] {
        let err = repo::insert_entry(
            db.conn(),
            &NewEntry {
                entry_date: day(2026, 1, 1),
                project_id,
                session_word_count: 100,
                hours,
                notes: None,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, DbError::SessionTooShort { .. }),
            "{hours} should be refused, got {err:?}"
        );
    }
}

/// `NaN` compares false against every bound, so a plain `<` check would let it
/// through and store a session whose hours can never be reasoned about.
#[test]
fn a_session_of_nan_hours_is_refused() {
    let (db, project_id) = seeded();
    let err = repo::insert_entry(
        db.conn(),
        &NewEntry {
            entry_date: day(2026, 1, 1),
            project_id,
            session_word_count: 100,
            hours: f64::NAN,
            notes: None,
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, DbError::SessionTooShort { .. }),
        "got {err:?}"
    );
    assert_eq!(repo::project_entry_count(db.conn(), project_id).unwrap(), 0);
}

#[test]
fn the_minimum_session_is_also_enforced_by_the_schema_itself() {
    let (db, project_id) = seeded();
    let direct = db.conn().execute(
        "INSERT INTO entries (entry_date, project_id, session_word_count, hours)
         VALUES ('2026-01-01', ?1, 100, 0.0)",
        params![project_id],
    );
    assert!(
        direct.is_err(),
        "the CHECK constraint is the backstop for the rule"
    );
}

#[test]
fn the_minimum_session_itself_is_accepted() {
    let (db, project_id) = seeded();
    add(&db, project_id, day(2026, 1, 1), 100, MIN_SESSION_HOURS);
    assert_eq!(repo::project_entry_count(db.conn(), project_id).unwrap(), 1);
}

#[test]
fn a_revision_day_that_cuts_words_is_recorded_as_negative() {
    let (db, project_id) = seeded();
    add(&db, project_id, day(2026, 1, 1), 1_000, 1.0);
    add(&db, project_id, day(2026, 1, 2), -322, 1.0);
    assert_eq!(
        repo::project_logged_total(db.conn(), project_id).unwrap(),
        678
    );
}

#[test]
fn an_entry_cannot_reference_a_project_that_does_not_exist() {
    let db = Db::open_in_memory().unwrap();
    let err = db.conn().execute(
        "INSERT INTO entries (entry_date, project_id, session_word_count, hours)
         VALUES ('2026-01-01', 999, 100, 1.0)",
        [],
    );
    assert!(
        err.is_err(),
        "foreign keys must be enforced, not merely declared"
    );
}

#[test]
fn deleting_a_project_takes_its_entries_with_it() {
    let (db, project_id) = seeded();
    add(&db, project_id, day(2026, 1, 1), 500, 1.0);
    db.conn()
        .execute("DELETE FROM projects WHERE id = ?1", params![project_id])
        .unwrap();
    let remaining: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(remaining, 0);
}

// ------------------------------------------------------------ project metrics

#[test]
fn days_written_counts_distinct_dates_not_entries() {
    let (db, project_id) = seeded();
    add(&db, project_id, day(2026, 1, 1), 300, 1.0);
    add(&db, project_id, day(2026, 1, 1), 200, 0.5);
    add(&db, project_id, day(2026, 1, 3), 100, 0.25);
    let m = repo::project_metrics(db.conn(), project_id).unwrap();
    assert_eq!(m.entry_count, 3);
    assert_eq!(m.days_written, 2);
    assert_eq!(m.total_word_count, 600);
    assert_eq!(m.hours_written, 1.75);
    assert_eq!(m.start_date, Some(day(2026, 1, 1)));
}

#[test]
fn an_open_project_has_no_end_date() {
    let (db, project_id) = seeded();
    add(&db, project_id, day(2026, 1, 1), 300, 1.0);
    let m = repo::project_metrics(db.conn(), project_id).unwrap();
    assert_eq!(m.end_date, None);
}

/// The distinction between completed and abandoned exists only so abandoned work
/// is not counted as finished. Every derived calculation, the end date included,
/// treats them identically.
#[test]
fn completed_and_abandoned_both_get_an_end_date_from_the_last_entry() {
    for status in [Status::Completed, Status::Abandoned] {
        let (db, project_id) = seeded();
        add(&db, project_id, day(2026, 1, 1), 300, 1.0);
        add(&db, project_id, day(2026, 2, 9), 400, 1.0);
        repo::set_project_status(db.conn(), project_id, status).unwrap();
        let m = repo::project_metrics(db.conn(), project_id).unwrap();
        assert_eq!(
            m.end_date,
            Some(day(2026, 2, 9)),
            "{status} should take its end date from the last entry"
        );
    }
}

#[test]
fn rates_are_undefined_rather_than_zero_when_there_is_nothing_to_divide_by() {
    let (db, project_id) = seeded();
    let m = repo::project_metrics(db.conn(), project_id).unwrap();
    assert_eq!(m.total_word_count, 0);
    assert_eq!(m.hours_per_day(), None);
    assert_eq!(m.words_per_hour(), None);
}

#[test]
fn rates_divide_the_whole_project() {
    let (db, project_id) = seeded();
    add(&db, project_id, day(2026, 1, 1), 400, 0.25);
    add(&db, project_id, day(2026, 1, 2), 600, 0.75);
    let m = repo::project_metrics(db.conn(), project_id).unwrap();
    assert_eq!(m.words_per_hour(), Some(1_000.0));
    assert_eq!(m.hours_per_day(), Some(0.5));
}

// ------------------------------------------------------------ the entry form

#[test]
fn the_first_entry_on_a_project_takes_the_typed_number_as_the_session() {
    let (db, project_id) = seeded();
    let p = repo::preview_session(db.conn(), project_id, 1_289, 2.0).unwrap();
    assert!(p.first_entry);
    assert_eq!(p.session_word_count, 1_289);
    assert_eq!(p.words_per_hour, Some(644.5));
}

#[test]
fn later_entries_subtract_the_running_total_from_the_typed_manuscript_count() {
    let (db, project_id) = seeded();
    add(&db, project_id, day(2026, 1, 1), 1_289, 2.0);
    let p = repo::preview_session(db.conn(), project_id, 2_558, 2.0).unwrap();
    assert!(!p.first_entry);
    assert_eq!(p.session_word_count, 1_269);
}

#[test]
fn a_typed_count_below_the_running_total_previews_a_negative_session() {
    let (db, project_id) = seeded();
    add(&db, project_id, day(2026, 1, 1), 14_000, 2.0);
    let p = repo::preview_session(db.conn(), project_id, 12_670, 1.0).unwrap();
    assert_eq!(p.session_word_count, -1_330);
    assert_eq!(p.words_per_hour, Some(-1_330.0));
}

#[test]
fn the_session_reference_ignores_entry_order_so_back_dating_needs_no_recalculation() {
    let (db, project_id) = seeded();
    add(&db, project_id, day(2026, 3, 1), 500, 1.0);
    add(&db, project_id, day(2026, 1, 1), 500, 1.0);
    let p = repo::preview_session(db.conn(), project_id, 1_200, 1.0).unwrap();
    assert_eq!(p.session_word_count, 200);
}

// ------------------------------------------------------------ project lists

#[test]
fn the_entry_picker_offers_only_open_projects() {
    let db = Db::open_in_memory().unwrap();
    let conn = db.conn();
    repo::insert_project(conn, "Live", None, Status::Open, None).unwrap();
    repo::insert_project(conn, "Shipped", None, Status::Completed, None).unwrap();
    repo::insert_project(conn, "Given up", None, Status::Abandoned, None).unwrap();
    let open: Vec<_> = repo::list_open_projects(conn)
        .unwrap()
        .into_iter()
        .map(|p| p.title)
        .collect();
    assert_eq!(open, vec!["Live"]);
    assert_eq!(repo::list_projects(conn).unwrap().len(), 3);
}

#[test]
fn status_cycles_through_all_three_and_back() {
    assert_eq!(Status::Open.next(), Status::Completed);
    assert_eq!(Status::Completed.next(), Status::Abandoned);
    assert_eq!(Status::Abandoned.next(), Status::Open);
}

#[test]
fn a_status_is_parsed_after_trimming_and_without_regard_to_case() {
    assert_eq!("  Abandoned ".parse::<Status>().unwrap(), Status::Abandoned);
    assert_eq!("completed".parse::<Status>().unwrap(), Status::Completed);
    assert_eq!("OPEN".parse::<Status>().unwrap(), Status::Open);
    assert!("finished".parse::<Status>().is_err());
}

// ------------------------------------------------------------ dashboard

/// Two years of entries across three projects: one completed in the later year,
/// one still open, and one abandoned with no category, so the uncategorised
/// bucket has something in it.
///
/// No word does two jobs here. A fixture where a project and a category share a
/// name reads ambiguously at every assertion, and would let one pass for the
/// wrong reason.
fn dashboard_fixture() -> Db {
    let db = Db::open_in_memory().unwrap();
    let conn = db.conn();
    let fiction = repo::ensure_type(conn, "Fiction").unwrap();
    let nonfic = repo::ensure_type(conn, "Non-Fiction").unwrap();
    let short_story = repo::ensure_category(conn, "Short Story", fiction).unwrap();
    let essay = repo::ensure_category(conn, "Essay", nonfic).unwrap();

    let completed = repo::insert_project(
        conn,
        "Winter Harbour",
        Some(short_story),
        Status::Completed,
        None,
    )
    .unwrap();
    let open =
        repo::insert_project(conn, "Against Outlines", Some(essay), Status::Open, None).unwrap();
    let abandoned =
        repo::insert_project(conn, "Nine Doors", None, Status::Abandoned, None).unwrap();

    repo::insert_entry(
        conn,
        &NewEntry {
            entry_date: day(2025, 5, 1),
            project_id: completed,
            session_word_count: 1_000,
            hours: 2.0,
            notes: None,
        },
    )
    .unwrap();
    repo::insert_entry(
        conn,
        &NewEntry {
            entry_date: day(2026, 2, 1),
            project_id: completed,
            session_word_count: 3_000,
            hours: 2.0,
            notes: None,
        },
    )
    .unwrap();
    repo::insert_entry(
        conn,
        &NewEntry {
            entry_date: day(2026, 3, 1),
            project_id: open,
            session_word_count: 500,
            hours: 1.0,
            notes: None,
        },
    )
    .unwrap();
    repo::insert_entry(
        conn,
        &NewEntry {
            entry_date: day(2026, 3, 2),
            project_id: abandoned,
            session_word_count: 250,
            hours: 1.0,
            notes: None,
        },
    )
    .unwrap();
    db
}

#[test]
fn totals_resolve_lifetime_when_no_year_is_given() {
    let db = dashboard_fixture();
    let all = stats::totals(db.conn(), None).unwrap();
    assert_eq!(all.words, 4_750);
    assert_eq!(all.hours, 6.0);
}

#[test]
fn the_same_query_filters_to_any_year() {
    let db = dashboard_fixture();
    assert_eq!(stats::totals(db.conn(), Some(2025)).unwrap().words, 1_000);
    assert_eq!(stats::totals(db.conn(), Some(2026)).unwrap().words, 3_750);
    assert_eq!(stats::totals(db.conn(), Some(2024)).unwrap().words, 0);
}

#[test]
fn abandoned_work_is_never_counted_as_completed() {
    let db = dashboard_fixture();
    assert_eq!(stats::completed_project_count(db.conn(), None).unwrap(), 1);
    // The abandoned project's last entry is in 2026, so a status-blind count
    // would report two here.
    assert_eq!(
        stats::completed_project_count(db.conn(), Some(2026)).unwrap(),
        1
    );
    // Winter Harbour's last entry is 2026, so it does not count toward 2025.
    assert_eq!(
        stats::completed_project_count(db.conn(), Some(2025)).unwrap(),
        0
    );
}

#[test]
fn the_projects_graph_carries_the_status_each_bar_is_coloured_by() {
    let db = dashboard_fixture();
    let bars = stats::project_words(db.conn(), Some(2026)).unwrap();
    let titles: Vec<_> = bars.iter().map(|b| b.title.as_str()).collect();
    assert_eq!(
        titles,
        vec!["Winter Harbour", "Against Outlines", "Nine Doors"]
    );
    assert_eq!(bars[0].status, Status::Completed);
    assert_eq!(bars[1].status, Status::Open);
    assert_eq!(bars[2].status, Status::Abandoned);
    assert_eq!(bars[0].words, 3_000);
}

#[test]
fn words_by_year_is_ordered_oldest_first() {
    let db = dashboard_fixture();
    assert_eq!(
        stats::words_by_year(db.conn()).unwrap(),
        vec![(2025, 1_000), (2026, 3_750)]
    );
    assert_eq!(
        stats::years_with_entries(db.conn()).unwrap(),
        vec![2025, 2026]
    );
}

#[test]
fn an_uncategorised_project_is_bucketed_rather_than_dropped() {
    let db = dashboard_fixture();
    let dist = stats::category_words(db.conn(), Some(2026)).unwrap();
    let total: i64 = dist.iter().map(|c| c.words).sum();
    assert_eq!(total, stats::totals(db.conn(), Some(2026)).unwrap().words);
    assert!(
        dist.iter()
            .any(|c| c.category == "Uncategorized" && c.words == 250),
        "got {dist:?}"
    );
}

/// The streak arithmetic is tested on its own in `streaks.rs`. What matters here
/// is that the query hands it the right set: one row per day, in order, and
/// clipped to the year when one is given, since that clipping is the only reason
/// a year's longest streak differs from the lifetime one.
#[test]
fn entry_dates_come_back_distinct_and_in_order() {
    let db = dashboard_fixture();
    let conn = db.conn();
    // A second session on a day already logged, which must not appear twice.
    add(&db, 1, day(2026, 3, 1), 100, 1.0);

    assert_eq!(
        stats::entry_dates(conn, None).unwrap(),
        vec![
            day(2025, 5, 1),
            day(2026, 2, 1),
            day(2026, 3, 1),
            day(2026, 3, 2)
        ]
    );
    assert_eq!(
        stats::entry_dates(conn, Some(2025)).unwrap(),
        vec![day(2025, 5, 1)]
    );
    assert_eq!(
        stats::entry_dates(conn, Some(2026)).unwrap(),
        vec![day(2026, 2, 1), day(2026, 3, 1), day(2026, 3, 2)]
    );
    assert!(stats::entry_dates(conn, Some(2024)).unwrap().is_empty());
}

/// Any entry counts as a day written: a session that cut words, or that bought
/// time and produced none, still kept the appointment.
///
/// The rule lives in what this query selects rather than in the arithmetic, so a
/// `WHERE session_word_count > 0` added here would silently make the streak a
/// production measure and every test in `streaks.rs` would still pass. It breaks
/// on exactly the days that are hardest to show up for, which is why it is pinned
/// where the decision actually is.
#[test]
fn a_revision_day_and_a_wordless_day_both_count_towards_a_streak() {
    let db = Db::open_in_memory().unwrap();
    let conn = db.conn();
    let project =
        repo::insert_project(conn, "The Cost of Revision", None, Status::Open, None).unwrap();
    add(&db, project, day(2026, 4, 1), 900, 2.0);
    add(&db, project, day(2026, 4, 2), -1_228, 1.5); // a day spent cutting
    add(&db, project, day(2026, 4, 3), 0, 1.0); // a day spent thinking
    add(&db, project, day(2026, 4, 4), 400, 1.0);

    assert_eq!(
        stats::entry_dates(conn, None).unwrap().len(),
        4,
        "all four days are days at the desk"
    );
    assert_eq!(
        streaks::longest(&stats::entry_dates(conn, None).unwrap()),
        4,
        "a cut and a wordless day must not break the run"
    );
    assert_eq!(
        streaks::current(&stats::entry_dates(conn, None).unwrap(), day(2026, 4, 4)),
        4
    );
}

/// Clipping, end to end: the same run of days gives the two years their halves
/// and all time the whole, which is the rule the feature was asked for.
#[test]
fn a_new_year_run_is_clipped_by_the_year_filter_but_not_by_lifetime() {
    let db = Db::open_in_memory().unwrap();
    let conn = db.conn();
    let project = repo::insert_project(conn, "The Tin Road", None, Status::Open, None).unwrap();
    for i in 0..10i64 {
        // 28 December 2025 to 6 January 2026.
        add(
            &db,
            project,
            day(2025, 12, 28) + chrono::Duration::days(i),
            100,
            1.0,
        );
    }

    let longest = |year| streaks::longest(&stats::entry_dates(conn, year).unwrap());
    assert_eq!(longest(None), 10);
    assert_eq!(longest(Some(2025)), 4);
    assert_eq!(longest(Some(2026)), 6);
}

// ------------------------------------------------------------ housekeeping

#[test]
fn a_fresh_database_reports_itself_empty_and_stops_being_empty_once_used() {
    let (db, project_id) = seeded();
    assert!(!db.is_empty().unwrap(), "seeding created a project");
    add(&db, project_id, day(2026, 1, 1), 10, 1.0);
    let mut db = db;
    db.reset().unwrap();
    assert!(db.is_empty().unwrap());
    assert_eq!(db.schema_version().unwrap(), migrations::target_version());
}

#[test]
fn reopening_a_file_preserves_what_was_written() {
    let dir = std::env::temp_dir().join(format!("draft_tracker_test_{}", std::process::id()));
    let path = dir.join("nested").join("draft_tracker.db");
    let _ = std::fs::remove_dir_all(&dir);

    let project_id = {
        let db = Db::open(&path).unwrap();
        let id = repo::insert_project(db.conn(), "Persisted", None, Status::Open, None).unwrap();
        repo::insert_entry(
            db.conn(),
            &NewEntry {
                entry_date: day(2026, 1, 1),
                project_id: id,
                session_word_count: 42,
                hours: 0.25,
                notes: Some("kept".into()),
            },
        )
        .unwrap();
        id
    };

    let db = Db::open(&path).unwrap();
    assert_eq!(
        repo::get_project(db.conn(), project_id).unwrap().title,
        "Persisted"
    );
    assert_eq!(
        repo::project_logged_total(db.conn(), project_id).unwrap(),
        42
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ------------------------------------------------------------ delete guards

/// `entries` cascades on delete, so without a guard removing a project would
/// quietly destroy the record of the work done on it.
#[test]
fn a_project_with_entries_cannot_be_deleted() {
    let (db, project_id) = seeded();
    add(&db, project_id, day(2026, 1, 1), 500, 1.0);
    let err = repo::delete_project(db.conn(), project_id).unwrap_err();
    match err {
        DbError::ProjectHasEntries { entries, title } => {
            assert_eq!(entries, 1);
            assert_eq!(title, "Debris Sea");
        }
        other => panic!("expected ProjectHasEntries, got {other:?}"),
    }
    assert_eq!(
        repo::list_projects(db.conn()).unwrap().len(),
        1,
        "still there"
    );
    assert_eq!(
        repo::recent_entries(db.conn(), 10).unwrap().len(),
        1,
        "entry kept"
    );
}

#[test]
fn a_project_with_no_entries_can_be_deleted() {
    let (db, project_id) = seeded();
    repo::delete_project(db.conn(), project_id).unwrap();
    assert!(repo::list_projects(db.conn()).unwrap().is_empty());
}

#[test]
fn a_category_in_use_cannot_be_deleted() {
    let db = Db::open_in_memory().unwrap();
    let conn = db.conn();
    let type_id = repo::ensure_type(conn, "Fiction").unwrap();
    let cat_id = repo::ensure_category(conn, "Short Story", type_id).unwrap();
    repo::insert_project(conn, "A", Some(cat_id), Status::Open, None).unwrap();
    let err = repo::delete_category(conn, cat_id).unwrap_err();
    assert!(
        matches!(err, DbError::CategoryInUse { projects: 1, .. }),
        "got {err:?}"
    );

    repo::update_project(conn, 1, "A", None, Status::Open, None).unwrap();
    repo::delete_category(conn, cat_id).unwrap();
}

#[test]
fn a_type_in_use_cannot_be_deleted() {
    let db = Db::open_in_memory().unwrap();
    let conn = db.conn();
    let type_id = repo::ensure_type(conn, "Fiction").unwrap();
    let cat_id = repo::ensure_category(conn, "Short Story", type_id).unwrap();
    let err = repo::delete_type(conn, type_id).unwrap_err();
    assert!(
        matches!(err, DbError::TypeInUse { categories: 1, .. }),
        "got {err:?}"
    );

    repo::delete_category(conn, cat_id).unwrap();
    repo::delete_type(conn, type_id).unwrap();
}

#[test]
fn a_duplicate_name_is_reported_as_a_clash_rather_than_a_constraint_message() {
    let db = Db::open_in_memory().unwrap();
    repo::insert_project(db.conn(), "Taken", None, Status::Open, None).unwrap();
    let err = repo::insert_project(db.conn(), "Taken", None, Status::Open, None).unwrap_err();
    let mapped = repo::name_taken(err, "Taken");
    assert!(
        matches!(mapped, DbError::NameTaken(ref n) if n == "Taken"),
        "got {mapped:?}"
    );
    assert_eq!(mapped.to_string(), "\"Taken\" already exists");
}

// ------------------------------------------------------------ screen queries

#[test]
fn the_projects_query_returns_metrics_in_one_pass() {
    let (db, project_id) = seeded();
    add(&db, project_id, day(2026, 1, 1), 400, 0.25);
    add(&db, project_id, day(2026, 1, 1), 200, 0.25);
    add(&db, project_id, day(2026, 1, 3), 400, 0.5);
    repo::insert_project(db.conn(), "Untouched", None, Status::Abandoned, None).unwrap();

    let rows = repo::list_projects_with_metrics(db.conn()).unwrap();
    assert_eq!(rows.len(), 2);
    let sea = rows
        .iter()
        .find(|r| r.project.title == "Debris Sea")
        .unwrap();
    assert_eq!(sea.category_name.as_deref(), Some("Short Story"));
    assert_eq!(sea.type_name.as_deref(), Some("Fiction"));
    assert_eq!(sea.metrics.total_word_count, 1_000);
    assert_eq!(sea.metrics.days_written, 2);
    assert_eq!(sea.metrics.entry_count, 3);
    assert_eq!(sea.metrics.hours_written, 1.0);
    assert_eq!(sea.metrics.words_per_hour(), Some(1_000.0));
    assert_eq!(sea.metrics.end_date, None, "still open");

    // A project with no entries still appears, with nothing to divide by.
    let other = rows
        .iter()
        .find(|r| r.project.title == "Untouched")
        .unwrap();
    assert_eq!(other.metrics.total_word_count, 0);
    assert_eq!(other.metrics.words_per_hour(), None);
    assert_eq!(other.category_name, None);
}

#[test]
fn usage_counts_come_back_with_categories_and_types() {
    let db = Db::open_in_memory().unwrap();
    let conn = db.conn();
    let fiction = repo::ensure_type(conn, "Fiction").unwrap();
    repo::ensure_type(conn, "Non-Fiction").unwrap();
    let story = repo::ensure_category(conn, "Short Story", fiction).unwrap();
    repo::ensure_category(conn, "Flash Fiction", fiction).unwrap();
    repo::insert_project(conn, "A", Some(story), Status::Open, None).unwrap();
    repo::insert_project(conn, "B", Some(story), Status::Open, None).unwrap();

    let cats = repo::list_categories_with_usage(conn).unwrap();
    let story_row = cats
        .iter()
        .find(|c| c.category.name == "Short Story")
        .unwrap();
    assert_eq!(story_row.project_count, 2);
    assert_eq!(story_row.type_name, "Fiction");
    let flash_row = cats
        .iter()
        .find(|c| c.category.name == "Flash Fiction")
        .unwrap();
    assert_eq!(flash_row.project_count, 0);

    let types = repo::list_types_with_usage(conn).unwrap();
    assert_eq!(
        types
            .iter()
            .find(|t| t.type_.name == "Fiction")
            .unwrap()
            .category_count,
        2
    );
    assert_eq!(
        types
            .iter()
            .find(|t| t.type_.name == "Non-Fiction")
            .unwrap()
            .category_count,
        0
    );
}
