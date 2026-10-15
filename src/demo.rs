//! A database of invented data, for seeing the program work without real writing
//! in it.
//!
//! This lives in the library rather than in the example that calls it so that it
//! is covered by tests and cannot drift from the schema: a fixture that no longer
//! loads is worse than no fixture.
//!
//! The content is deliberate rather than random. Every screen needs something to
//! show, and the cases that are awkward to produce by hand are the ones most worth
//! having in a demonstration: a revision day that cuts words, a session recording
//! time but no net words, two entries for one project on one day, a project
//! spanning two calendar years, an abandoned project, a project with a single
//! entry, and more projects in one year than the stats graph can display at once.
//!
//! Word counts, hours and the shape of every session are deterministic, so the
//! same command always produces the same totals. The one project written daily is
//! dated from the clock instead, so that reseeding always produces a live streak
//! rather than one that expired whenever this was last edited. See [`Start`].

use chrono::{Datelike, Duration, Local, NaiveDate};

use crate::db::models::{NewEntry, Status};
use crate::db::{Db, DbError, repo};

#[derive(Debug, thiserror::Error)]
pub enum DemoError {
    #[error("that database already holds data; demonstration data goes in an empty one")]
    NotEmpty,

    #[error(transparent)]
    Db(#[from] DbError),
}

/// What was written.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub types: usize,
    pub categories: usize,
    pub projects: usize,
    pub entries: usize,
    pub words: i64,
    pub hours: f64,
    pub years: Vec<i32>,
}

/// When a project's first session falls.
enum Start {
    /// A fixed calendar date. The settled history uses these, so the year totals
    /// and the year-over-year graph are the same on every run.
    On(i32, u32, u32),
    /// Counted back from today, so the run always ends yesterday however long
    /// after this was written the generator is run.
    ///
    /// This is what makes a reseeded database show a working streak instead of an
    /// expired one, and it is deliberately the only thing in here that reads the
    /// clock. A project dated this way is also written on consecutive days, which
    /// is the rule the loop below applies: the two go together, since a run with
    /// gaps in it would be no streak at all.
    DaysBeforeToday(i64),
}

/// Titles are invented. They read like projects so the screens look like a real
/// log, and belong to nobody.
struct Spec {
    title: &'static str,
    category: &'static str,
    status: Status,
    /// First session.
    start: Start,
    sessions: usize,
    /// Words per session, low and high.
    words: (i64, i64),
}

const TYPES: [(&str, &str); 2] = [
    ("Fiction", "Stories, in any form"),
    ("Non-Fiction", "Essays, posts and anything true"),
];

const CATEGORIES: [(&str, &str); 5] = [
    ("Novel", "Fiction"),
    ("Short Story", "Fiction"),
    ("Flash Fiction", "Fiction"),
    ("Essay", "Non-Fiction"),
    ("Blog Post", "Non-Fiction"),
];

/// Four years of work, weighted so that no year looks like any other and 2025
/// holds more projects than the stats graph can show at once.
const PROJECTS: [Spec; 19] = [
    // 2023
    Spec {
        title: "Winter Harbour",
        category: "Novel",
        status: Status::Completed,
        start: Start::On(2023, 2, 6),
        sessions: 48,
        words: (700, 1900),
    },
    Spec {
        title: "Paper Birds",
        category: "Short Story",
        status: Status::Completed,
        start: Start::On(2023, 5, 15),
        sessions: 6,
        words: (600, 1400),
    },
    Spec {
        title: "On Keeping Time",
        category: "Essay",
        status: Status::Completed,
        start: Start::On(2023, 8, 3),
        sessions: 3,
        words: (800, 1600),
    },
    Spec {
        title: "The Orchard Gate",
        category: "Short Story",
        status: Status::Completed,
        start: Start::On(2023, 10, 9),
        sessions: 5,
        words: (500, 1500),
    },
    // 2024
    Spec {
        title: "Quiet Machines",
        category: "Short Story",
        status: Status::Completed,
        start: Start::On(2024, 3, 4),
        sessions: 4,
        words: (700, 1800),
    },
    Spec {
        title: "Matchlight",
        category: "Flash Fiction",
        status: Status::Completed,
        start: Start::On(2024, 4, 18),
        sessions: 1,
        words: (900, 900),
    },
    Spec {
        title: "Nine Doors",
        category: "Short Story",
        status: Status::Abandoned,
        start: Start::On(2024, 6, 11),
        sessions: 3,
        words: (300, 900),
    },
    Spec {
        title: "Notes from a Desk",
        category: "Blog Post",
        status: Status::Completed,
        start: Start::On(2024, 7, 2),
        sessions: 1,
        words: (1100, 1100),
    },
    // Spans 2024 into 2025, so a project's start and end sit in different years.
    Spec {
        title: "The Tin Road",
        category: "Novel",
        status: Status::Completed,
        start: Start::On(2024, 10, 7),
        sessions: 42,
        words: (650, 1800),
    },
    // 2025: seven projects, more than the graph shows at once
    Spec {
        title: "Against Outlines",
        category: "Essay",
        status: Status::Completed,
        start: Start::On(2025, 3, 11),
        sessions: 2,
        words: (900, 1700),
    },
    Spec {
        title: "The Last Ferry",
        category: "Short Story",
        status: Status::Completed,
        start: Start::On(2025, 4, 1),
        sessions: 5,
        words: (600, 1600),
    },
    Spec {
        title: "Thumbprint",
        category: "Flash Fiction",
        status: Status::Completed,
        start: Start::On(2025, 5, 20),
        sessions: 1,
        words: (750, 750),
    },
    Spec {
        title: "Glasshouse",
        category: "Short Story",
        status: Status::Completed,
        start: Start::On(2025, 7, 8),
        sessions: 7,
        words: (550, 1500),
    },
    Spec {
        title: "Why I Keep a Log",
        category: "Blog Post",
        status: Status::Completed,
        start: Start::On(2025, 9, 16),
        sessions: 1,
        words: (1300, 1300),
    },
    Spec {
        title: "Low Tide",
        category: "Short Story",
        status: Status::Open,
        start: Start::On(2025, 11, 4),
        sessions: 2,
        words: (400, 1200),
    },
    // 2026
    Spec {
        title: "Reading Slowly",
        category: "Blog Post",
        status: Status::Completed,
        start: Start::On(2026, 1, 13),
        sessions: 1,
        words: (950, 950),
    },
    Spec {
        title: "Salt Lantern",
        category: "Novel",
        status: Status::Open,
        start: Start::On(2026, 1, 26),
        sessions: 14,
        words: (700, 1700),
    },
    Spec {
        title: "The Cost of Revision",
        category: "Essay",
        status: Status::Open,
        start: Start::On(2026, 2, 17),
        sessions: 2,
        words: (850, 1500),
    },
    // Written every day, so the dashboard always has a streak to show. Dated back
    // from the clock rather than pinned to a calendar date: a fixed run expires,
    // and a demonstration whose headline feature reads zero demonstrates nothing.
    //
    // Fourteen sessions ending yesterday, which leaves today deliberately empty so
    // that logging a session moves the figure on by one.
    Spec {
        title: "Daybook",
        category: "Blog Post",
        status: Status::Open,
        start: Start::DaysBeforeToday(14),
        sessions: 14,
        words: (350, 900),
    },
];

/// Hours a session might take, in quarter hours, which is how the application
/// counts time.
const HOURS: [f64; 7] = [0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 2.5];

/// A fixed-seed generator, so the data is the same every time.
///
/// An xorshift rather than a dependency: nothing here needs statistical quality,
/// only repeatability.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A number in `low..=high`.
    fn between(&mut self, low: i64, high: i64) -> i64 {
        if high <= low {
            return low;
        }
        low + (self.next() % (high - low + 1) as u64) as i64
    }

    fn pick<T: Copy>(&mut self, options: &[T]) -> T {
        options[(self.next() % options.len() as u64) as usize]
    }
}

/// Write the demonstration data. The database must be empty.
///
/// Everything goes in one transaction: a generator that fails halfway and leaves
/// a partial database behind would be worse than one that fails outright.
pub fn populate(db: &mut Db) -> Result<Summary, DemoError> {
    if !db.is_empty()? || !taxonomy_empty(db.conn())? {
        return Err(DemoError::NotEmpty);
    }
    let tx = db.conn_mut().transaction().map_err(DbError::from)?;
    let conn = &*tx;
    let mut rng = Rng(0x5eed_1234_abcd_ef01);
    let today = Local::now().date_naive();

    for (name, description) in TYPES {
        repo::insert_type(conn, name, Some(description))?;
    }
    for (category, type_name) in CATEGORIES {
        let type_id = repo::type_id_by_name(conn, type_name)?.expect("types inserted first");
        repo::insert_category(conn, category, type_id, None)?;
    }

    let mut entries = 0usize;
    let mut words_total = 0i64;
    let mut hours_total = 0.0f64;
    let mut years: Vec<i32> = Vec::new();

    for (index, spec) in PROJECTS.iter().enumerate() {
        let category_id = repo::category_id_by_name(conn, spec.category)?;
        let project_id = repo::insert_project(conn, spec.title, category_id, spec.status, None)?;

        let mut date = match spec.start {
            Start::On(y, m, d) => {
                NaiveDate::from_ymd_opt(y, m, d).expect("the specs hold real dates")
            }
            // One short of the session count, so the last session lands yesterday.
            Start::DaysBeforeToday(n) => today - Duration::days(n),
        };

        for session in 0..spec.sessions {
            let (words, hours, note) = session_shape(&mut rng, spec, index, session);
            repo::insert_entry(
                conn,
                &NewEntry {
                    entry_date: date,
                    project_id,
                    session_word_count: words,
                    hours,
                    notes: note.map(str::to_string),
                },
            )?;
            entries += 1;
            words_total += words;
            hours_total += hours;
            if !years.contains(&date.year()) {
                years.push(date.year());
            }

            // A second entry on the same day for one project, once, because the
            // application allows it and nothing else would exercise it.
            if index == 0 && session == 3 {
                repo::insert_entry(
                    conn,
                    &NewEntry {
                        entry_date: date,
                        project_id,
                        session_word_count: 420,
                        hours: 0.5,
                        notes: Some("came back to it after dinner".into()),
                    },
                )?;
                entries += 1;
                words_total += 420;
                hours_total += 0.5;
            }

            // A project dated from the clock is the daily one and runs without
            // gaps; everything else is spaced the way real work is.
            date += Duration::days(match spec.start {
                Start::DaysBeforeToday(_) => 1,
                Start::On(..) => rng.between(1, 4),
            });
        }
    }

    years.sort_unstable();
    tx.commit().map_err(DbError::from)?;

    Ok(Summary {
        types: TYPES.len(),
        categories: CATEGORIES.len(),
        projects: PROJECTS.len(),
        entries,
        words: words_total,
        hours: hours_total,
        years,
    })
}

/// What a single session looks like, including the awkward ones.
fn session_shape(
    rng: &mut Rng,
    spec: &Spec,
    index: usize,
    session: usize,
) -> (i64, f64, Option<&'static str>) {
    // A revision day: the manuscript got shorter, which is real work.
    if index == 0 && session == 20 {
        return (
            -1_340,
            1.0,
            Some("cut the whole second chapter, and it reads better"),
        );
    }
    // Time spent with nothing to show for it, which the floor on hours exists for.
    if index == 8 && session == 12 {
        return (
            0,
            0.25,
            Some("read back over yesterday and changed nothing"),
        );
    }
    // A project abandoned mid-thought.
    if matches!(spec.status, Status::Abandoned) && session + 1 == spec.sessions {
        return (
            rng.between(spec.words.0, spec.words.1),
            rng.pick(&HOURS),
            Some("no idea where this goes"),
        );
    }
    let words = rng.between(spec.words.0, spec.words.1);
    let hours = rng.pick(&HOURS);
    (words, hours, None)
}

/// `is_empty` only looks at projects and entries, because that is all the
/// importer cares about. A leftover category would collide here, so it is checked
/// too.
fn taxonomy_empty(conn: &rusqlite::Connection) -> Result<bool, DbError> {
    let count: i64 = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM types) + (SELECT COUNT(*) FROM categories)",
        [],
        |row| row.get(0),
    )?;
    Ok(count == 0)
}
