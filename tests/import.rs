//! Importer tests. Each builds a throwaway directory of source files.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use draft_tracker::MIN_SESSION_HOURS;
use draft_tracker::db::models::Status;
use draft_tracker::db::{Db, repo, stats};
use draft_tracker::import::{self, Existing, ImportError};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A directory that cleans itself up.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("draft_tracker_import_{}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn write(&self, name: &str, contents: &str) -> &Self {
        std::fs::write(self.0.join(name), contents).unwrap();
        self
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const HEADERS: &str = "Date,Project,Category,Type,Day WC,Hours,Project WC,Notes";

// ------------------------------------------------------------ input contract

#[test]
fn a_missing_required_header_names_itself() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        "Date,Project,Hours,Day WC\n2026-01-01,A,1.0,100\n",
    );
    let err = import::plan(dir.path()).unwrap_err();
    match err {
        ImportError::MissingHeader { header, .. } => assert_eq!(header, "Category"),
        other => panic!("expected MissingHeader, got {other:?}"),
    }
}

#[test]
fn a_file_with_neither_word_count_column_is_refused() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        "Date,Project,Category,Type,Hours\n2026-01-01,A,C,Fiction,1.0\n",
    );
    let err = import::plan(dir.path()).unwrap_err();
    assert!(
        matches!(err, ImportError::MissingWordCount { .. }),
        "got {err:?}"
    );
}

#[test]
fn headers_are_matched_trimmed_and_without_regard_to_case() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        " date , PROJECT ,Category, type , Day WC , hours ,Project WC, NOTES \n\
         2026-01-01,A,Short Story,Fiction,100,1.0,100,hello\n",
    );
    let plan = import::plan(dir.path()).unwrap();
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].words, 100);
    assert_eq!(plan.entries[0].notes.as_deref(), Some("hello"));
}

/// These sheets repeat Date, Day WC and Hours in a derived block further right.
#[test]
fn a_repeated_header_resolves_to_the_leftmost_column() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        "Date,Project,Category,Type,Day WC,Hours,Project WC,Notes,Date,Day WC,Hours\n\
         2026-01-01,A,Short Story,Fiction,100,1.0,100,,1999-01-01,99999,77\n",
    );
    let plan = import::plan(dir.path()).unwrap();
    assert_eq!(plan.entries[0].words, 100);
    assert_eq!(plan.entries[0].hours, 1.0);
    assert_eq!(plan.entries[0].date.to_string(), "2026-01-01");
}

#[test]
fn a_byte_order_mark_does_not_hide_the_first_header() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!("\u{feff}{HEADERS}\n2026-01-01,A,Short Story,Fiction,100,1.0,100,\n"),
    );
    assert_eq!(import::plan(dir.path()).unwrap().entries.len(), 1);
}

#[test]
fn csv_dates_are_read_as_iso_or_month_first() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!(
            "{HEADERS}\n\
             2023-11-01,A,Short Story,Fiction,100,1.0,100,\n\
             11/02/23,A,Short Story,Fiction,100,1.0,200,\n\
             11/03/2023,A,Short Story,Fiction,100,1.0,300,\n"
        ),
    );
    let plan = import::plan(dir.path()).unwrap();
    let dates: Vec<String> = plan.entries.iter().map(|e| e.date.to_string()).collect();
    assert_eq!(dates, vec!["2023-11-01", "2023-11-02", "2023-11-03"]);
}

// ------------------------------------------------------------ skip rules

#[test]
fn rows_with_nothing_to_record_are_skipped_without_naming_any_sentinel() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!(
            "{HEADERS}\n\
             ,Undated,Short Story,Fiction,100,1.0,100,\n\
             2026-01-02,Missed Day,Short Story,Fiction,0,0,0,nothing written\n\
             2026-01-03,,Short Story,Fiction,100,1.0,100,no project\n\
             2026-01-04,Real,Short Story,Fiction,500,1.0,500,kept\n"
        ),
    );
    let plan = import::plan(dir.path()).unwrap();
    assert_eq!(plan.report.skipped_no_date, 1);
    assert_eq!(plan.report.skipped_no_work, 1);
    assert_eq!(plan.report.skipped_no_project, 1);
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.projects.len(), 1);
    assert_eq!(plan.projects[0].title, "Real");
}

#[test]
fn time_spent_without_net_words_is_a_real_session() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!("{HEADERS}\n2026-01-01,A,Short Story,Fiction,0,0.25,0,editing\n"),
    );
    let plan = import::plan(dir.path()).unwrap();
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].words, 0);
    assert_eq!(plan.entries[0].hours, 0.25);
}

// ------------------------------------------------------------ word counts

#[test]
fn a_session_is_derived_from_the_rise_in_the_running_total() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!(
            "{HEADERS}\n\
             2026-01-01,A,Short Story,Fiction,,1.0,1056,\n\
             2026-01-02,A,Short Story,Fiction,,1.0,1797,\n\
             2026-01-03,A,Short Story,Fiction,,1.0,1806,\n"
        ),
    );
    let plan = import::plan(dir.path()).unwrap();
    let words: Vec<i64> = plan.entries.iter().map(|e| e.words).collect();
    assert_eq!(words, vec![1056, 741, 9]);
}

#[test]
fn a_running_total_that_falls_derives_a_negative_session() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!(
            "{HEADERS}\n\
             2026-01-01,A,Short Story,Fiction,,1.0,3802,\n\
             2026-01-02,A,Short Story,Fiction,,1.0,3787,\n"
        ),
    );
    let plan = import::plan(dir.path()).unwrap();
    assert_eq!(plan.entries[1].words, -15);
}

#[test]
fn an_explicit_day_count_wins_over_the_running_total() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!(
            "{HEADERS}\n\
             2026-01-01,A,Short Story,Fiction,100,1.0,5000,\n\
             2026-01-02,A,Short Story,Fiction,250,1.0,9999,\n"
        ),
    );
    let plan = import::plan(dir.path()).unwrap();
    assert_eq!(
        plan.entries.iter().map(|e| e.words).collect::<Vec<_>>(),
        vec![100, 250]
    );
}

#[test]
fn words_written_before_tracking_began_are_reported_not_invented() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!("{HEADERS}\n2026-01-08,A,Short Story,Fiction,352,2.0,11902,\n"),
    );
    let plan = import::plan(dir.path()).unwrap();
    assert_eq!(
        plan.report.pre_tracking_words,
        vec![("A".to_string(), 11_550)]
    );
    // Reported, but no entry conjured up for it.
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].words, 352);
}

// ------------------------------------------------------------ the hours floor

#[test]
fn a_session_below_the_minimum_is_raised_and_reported() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!(
            "{HEADERS}\n\
             2026-01-01,A,Short Story,Fiction,78,0,78,no time recorded\n\
             2026-01-02,A,Short Story,Fiction,16,0.1,94,a tenth of an hour\n\
             2026-01-03,A,Short Story,Fiction,500,1.0,594,fine\n"
        ),
    );
    let plan = import::plan(dir.path()).unwrap();
    assert_eq!(plan.report.hours_raised.len(), 2);
    let hours: Vec<f64> = plan.entries.iter().map(|e| e.hours).collect();
    assert_eq!(hours, vec![MIN_SESSION_HOURS, MIN_SESSION_HOURS, 1.0]);
    // Raising is what lets the schema's floor stand without failing the import.
    let mut db = Db::open_in_memory().unwrap();
    import::run(&mut db, dir.path(), Existing::Refuse).unwrap();
}

// ------------------------------------------------------------ taxonomy

#[test]
fn a_projects_category_is_the_majority_of_its_rows() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!(
            "{HEADERS}\n\
             2026-01-01,A,SitR Book,Fiction,100,1.0,100,\n\
             2026-01-02,A,SitR Book,Fiction,100,1.0,200,\n\
             2026-01-03,A,Kids Book,Fiction,100,1.0,300,a mislabelled row\n"
        ),
    );
    let plan = import::plan(dir.path()).unwrap();
    assert_eq!(plan.projects[0].category.as_deref(), Some("SitR Book"));
}

#[test]
fn a_category_type_is_the_majority_of_the_rows_carrying_it() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!(
            "{HEADERS}\n\
             2026-01-01,A,SitR Blog Post,Creative NF,100,1.0,100,\n\
             2026-01-02,A,SitR Blog Post,Creative NF,100,1.0,200,\n\
             2026-01-03,A,SitR Blog Post,Non-fiction,100,1.0,300,a slip\n"
        ),
    );
    let plan = import::plan(dir.path()).unwrap();
    assert_eq!(
        plan.categories,
        vec![("SitR Blog Post".into(), "Creative NF".into())]
    );
    assert_eq!(plan.types, vec!["Creative NF"]);
}

#[test]
fn a_category_nothing_states_a_type_for_stops_the_import() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        "Date,Project,Category,Day WC,Hours\n2026-01-01,A,Mystery Category,100,1.0\n",
    );
    let err = import::plan(dir.path()).unwrap_err();
    match err {
        ImportError::UntypedCategory { category } => assert_eq!(category, "Mystery Category"),
        other => panic!("expected UntypedCategory, got {other:?}"),
    }
}

#[test]
fn a_blank_category_is_filled_in_from_the_projects_other_rows() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!(
            "{HEADERS}\n\
             2026-01-01,A,Short Story,Fiction,100,1.0,100,\n\
             2026-01-02,A,,,100,1.0,200,only the counts changed\n"
        ),
    );
    let plan = import::plan(dir.path()).unwrap();
    assert_eq!(plan.projects[0].category.as_deref(), Some("Short Story"));
    assert!(plan.report.uncategorised_projects.is_empty());
}

/// A second consecutive space is invisible in a spreadsheet and would otherwise
/// create a phantom category with no projects in it.
#[test]
fn repeated_whitespace_in_a_name_is_collapsed_and_reported() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!(
            "{HEADERS}\n\
             2026-01-01,A,SitR Book,Fiction,100,1.0,100,\n\
             2026-01-02,A,\"SitR  Book\",Fiction,100,1.0,200,typo\n"
        ),
    );
    let plan = import::plan(dir.path()).unwrap();
    assert_eq!(plan.categories.len(), 1, "got {:?}", plan.categories);
    assert_eq!(plan.report.names_collapsed.len(), 1);
    let (_, before, after) = &plan.report.names_collapsed[0];
    assert_eq!(
        (before.as_str(), after.as_str()),
        ("SitR  Book", "SitR Book")
    );
}

// ------------------------------------------------------------ projects file

#[test]
fn a_file_with_project_and_status_but_no_date_sets_statuses() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!(
            "{HEADERS}\n\
             2026-01-01,Shipped,Short Story,Fiction,100,1.0,100,\n\
             2026-01-02,Dropped,Short Story,Fiction,100,1.0,100,\n\
             2026-01-03,Live,Short Story,Fiction,100,1.0,100,\n"
        ),
    );
    dir.write(
        "whatever_its_called.csv",
        "Project,Words,Status,Description\n\
         Shipped,100,completed,a finished piece\n\
         Dropped,100,Abandoned ,\n\
         Live,100,open,\n",
    );
    let plan = import::plan(dir.path()).unwrap();
    let by_title = |t: &str| {
        plan.projects
            .iter()
            .find(|p| p.title == t)
            .expect("project present")
            .clone()
    };
    assert_eq!(by_title("Shipped").status, Status::Completed);
    assert_eq!(
        by_title("Shipped").description.as_deref(),
        Some("a finished piece")
    );
    // Trimmed and case-insensitive, because this file is edited by hand.
    assert_eq!(by_title("Dropped").status, Status::Abandoned);
    assert_eq!(by_title("Live").status, Status::Open);
    assert!(plan.report.projects_defaulted_open.is_empty());
}

#[test]
fn an_unrecognised_status_stops_the_import_naming_the_row() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!("{HEADERS}\n2026-01-01,A,Short Story,Fiction,1,1.0,1,\n"),
    );
    dir.write("status.csv", "Project,Status\nA,finished\n");
    let err = import::plan(dir.path()).unwrap_err();
    match err {
        ImportError::BadStatus {
            project,
            value,
            row,
            ..
        } => {
            assert_eq!(
                (project.as_str(), value.as_str(), row),
                ("A", "finished", 2)
            );
        }
        other => panic!("expected BadStatus, got {other:?}"),
    }
}

#[test]
fn two_status_rows_for_one_project_is_an_error_rather_than_a_silent_override() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!("{HEADERS}\n2026-01-01,A,Short Story,Fiction,1,1.0,1,\n"),
    );
    dir.write("status.csv", "Project,Status\nA,completed\nA,abandoned\n");
    let err = import::plan(dir.path()).unwrap_err();
    assert!(
        matches!(err, ImportError::DuplicateStatusRow { .. }),
        "got {err:?}"
    );
}

#[test]
fn a_project_with_no_status_row_defaults_to_open_and_is_reported() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!(
            "{HEADERS}\n\
             2026-01-01,Named,Short Story,Fiction,1,1.0,1,\n\
             2026-01-02,Unnamed,Short Story,Fiction,1,1.0,1,\n"
        ),
    );
    dir.write("status.csv", "Project,Status\nNamed,completed\n");
    let plan = import::plan(dir.path()).unwrap();
    assert_eq!(plan.report.projects_defaulted_open, vec!["Unnamed"]);
    let unnamed = plan.projects.iter().find(|p| p.title == "Unnamed").unwrap();
    assert_eq!(unnamed.status, Status::Open);
}

#[test]
fn a_status_row_for_a_project_with_no_sessions_is_reported_and_skipped() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!("{HEADERS}\n2026-01-01,Real,Short Story,Fiction,1,1.0,1,\n"),
    );
    dir.write("status.csv", "Project,Status\nReal,open\nGhost,completed\n");
    let plan = import::plan(dir.path()).unwrap();
    assert_eq!(plan.report.status_rows_without_entries, vec!["Ghost"]);
    assert_eq!(plan.projects.len(), 1);
}

// ------------------------------------------------------------ writing

#[test]
fn a_project_tracked_across_two_files_becomes_one_record() {
    let dir = Dir::new();
    dir.write(
        "a_2025.csv",
        &format!("{HEADERS}\n2025-12-31,Long Novel,SitR Book,Fiction,1000,2.0,1000,\n"),
    );
    dir.write(
        "b_2026.csv",
        &format!("{HEADERS}\n2026-01-01,Long Novel,SitR Book,Fiction,500,1.0,1500,\n"),
    );
    let mut db = Db::open_in_memory().unwrap();
    import::run(&mut db, dir.path(), Existing::Refuse).unwrap();
    let projects = repo::list_projects(db.conn()).unwrap();
    assert_eq!(projects.len(), 1);
    let metrics = repo::project_metrics(db.conn(), projects[0].id).unwrap();
    assert_eq!(metrics.total_word_count, 1_500);
    assert_eq!(metrics.entry_count, 2);
}

#[test]
fn an_import_refuses_a_database_that_already_holds_data_unless_told_to_reset() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!("{HEADERS}\n2026-01-01,A,Short Story,Fiction,100,1.0,100,\n"),
    );
    let mut db = Db::open_in_memory().unwrap();
    import::run(&mut db, dir.path(), Existing::Refuse).unwrap();

    let err = import::run(&mut db, dir.path(), Existing::Refuse).unwrap_err();
    assert!(matches!(err, ImportError::NotEmpty), "got {err:?}");

    import::run(&mut db, dir.path(), Existing::Reset).unwrap();
    assert_eq!(
        stats::totals(db.conn(), None).unwrap().words,
        100,
        "no doubling"
    );
}

#[test]
fn a_failed_import_leaves_the_database_untouched() {
    let dir = Dir::new();
    // The second file lacks a required header, so resolution fails after the
    // first has been read. Nothing should be written.
    dir.write(
        "a.csv",
        &format!("{HEADERS}\n2026-01-01,A,Short Story,Fiction,100,1.0,100,\n"),
    );
    dir.write("b.csv", "Date,Project,Hours,Day WC\n2026-01-02,B,1.0,50\n");
    let mut db = Db::open_in_memory().unwrap();
    assert!(import::run(&mut db, dir.path(), Existing::Refuse).is_err());
    assert!(db.is_empty().unwrap(), "a failed import must write nothing");
}

#[test]
fn two_runs_over_the_same_sources_agree_exactly() {
    let dir = Dir::new();
    dir.write(
        "log.csv",
        &format!(
            "{HEADERS}\n\
             2026-01-01,A,SitR Book,Fiction,100,1.0,100,\n\
             2026-01-01,B,Kids Book,Fiction,200,1.0,200,\n\
             2026-01-02,A,SitR Book,Fiction,300,1.0,400,\n"
        ),
    );
    let first = import::plan(dir.path()).unwrap();
    let second = import::plan(dir.path()).unwrap();
    assert_eq!(first.entries, second.entries);
    assert_eq!(first.projects, second.projects);
    assert_eq!(first.categories, second.categories);
    assert_eq!(first.types, second.types);
}

// ------------------------------------------------------------ the real journals

/// Runs only where the author's own (gitignored) journals are present.
#[test]
fn the_authors_journals_import_to_known_figures() {
    let dir = Path::new("historic_trackers");
    if !dir.join("Writing Journal 2024.ods").exists() {
        eprintln!("skipping: {} not present", dir.display());
        return;
    }
    let mut db = Db::open_in_memory().unwrap();
    let report = import::run(&mut db, dir, Existing::Refuse).unwrap();

    assert_eq!(report.entries, 654);
    assert_eq!(report.words, 466_665);
    assert_eq!(report.hours, 593.50);
    assert_eq!(report.projects, 78);
    assert_eq!(report.categories, 10);
    assert_eq!(report.types, 3);
    assert_eq!(report.skipped_no_work, 239);
    assert_eq!(
        report.words_by_year,
        vec![
            (2023, 34_261),
            (2024, 244_148),
            (2025, 186_457),
            (2026, 1_799)
        ]
    );

    // Nothing is left unaccounted for: every word belongs to a dated entry.
    assert!(
        report.pre_tracking_words.is_empty(),
        "{:?}",
        report.pre_tracking_words
    );
    assert!(report.uncategorised_projects.is_empty());

    let conn = db.conn();
    assert_eq!(stats::totals(conn, None).unwrap().words, 466_665);
    assert_eq!(stats::completed_project_count(conn, None).unwrap(), 68);
    assert_eq!(repo::list_open_projects(conn).unwrap().len(), 8);
}
