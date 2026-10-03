//! What the importer did, in a form worth reading.

use std::fmt;

use chrono::NaiveDate;

/// A record of one import, printed whether or not anything was written.
///
/// Anything the importer changed or could not account for appears here. Silent
/// transformation of a writer's own records is the thing to avoid.
#[derive(Debug, Default)]
pub struct ImportReport {
    pub files_read: Vec<String>,
    pub rows_seen: usize,
    pub skipped_no_date: usize,
    pub skipped_no_work: usize,
    pub skipped_no_project: usize,
    pub rows_without_project: Vec<String>,
    /// Names whose repeated whitespace was collapsed: where, before, after.
    pub names_collapsed: Vec<(String, String, String)>,

    pub entries: usize,
    pub words: i64,
    pub hours: f64,
    pub types: usize,
    pub categories: usize,
    pub projects: usize,
    pub date_range: Option<(NaiveDate, NaiveDate)>,
    pub words_by_year: Vec<(i32, i64)>,
    /// Category, its type, how many projects sit in it, and their words.
    pub category_breakdown: Vec<(String, String, usize, i64)>,

    /// Sessions recorded below the minimum and raised to it: project, date, as recorded.
    pub hours_raised: Vec<(String, NaiveDate, f64)>,
    /// Projects whose first session implies words written before tracking began.
    pub pre_tracking_words: Vec<(String, i64)>,
    pub uncategorised_projects: Vec<String>,
    pub status_rows_without_entries: Vec<String>,
    pub projects_defaulted_open: Vec<String>,
}

impl ImportReport {
    pub fn skipped(&self) -> usize {
        self.skipped_no_date + self.skipped_no_work + self.skipped_no_project
    }

    /// True when nothing needed explaining. Useful for tests.
    pub fn is_clean(&self) -> bool {
        self.names_collapsed.is_empty()
            && self.hours_raised.is_empty()
            && self.pre_tracking_words.is_empty()
            && self.uncategorised_projects.is_empty()
            && self.status_rows_without_entries.is_empty()
    }
}

/// Lists a few items and then says how many more there are, so a report over a
/// messy journal stays readable.
fn sample<T: fmt::Display>(f: &mut fmt::Formatter<'_>, items: &[T], limit: usize) -> fmt::Result {
    for item in items.iter().take(limit) {
        writeln!(f, "    {item}")?;
    }
    if items.len() > limit {
        writeln!(f, "    ... and {} more", items.len() - limit)?;
    }
    Ok(())
}

impl fmt::Display for ImportReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Read {} file(s):", self.files_read.len())?;
        for name in &self.files_read {
            writeln!(f, "    {name}")?;
        }

        writeln!(f)?;
        writeln!(
            f,
            "{} entries, {} words, {:.2} hours",
            self.entries, self.words, self.hours
        )?;
        writeln!(
            f,
            "{} projects, {} categories, {} types",
            self.projects, self.categories, self.types
        )?;
        if let Some((first, last)) = self.date_range {
            // Stated because a CSV date can be read month-first, which is
            // ambiguous against day-first for the first twelve days of a month.
            writeln!(f, "dates {first} to {last}")?;
        }

        if !self.words_by_year.is_empty() {
            writeln!(f)?;
            writeln!(f, "Words by year:")?;
            for (year, words) in &self.words_by_year {
                writeln!(f, "    {year}  {words:>9}")?;
            }
        }

        if !self.category_breakdown.is_empty() {
            writeln!(f)?;
            writeln!(f, "Categories:")?;
            for (category, type_name, projects, words) in &self.category_breakdown {
                writeln!(
                    f,
                    "    {category:<20} {type_name:<12} {projects:>3} projects {words:>9} words"
                )?;
            }
        }

        writeln!(f)?;
        writeln!(f, "Skipped {} of {} rows:", self.skipped(), self.rows_seen)?;
        writeln!(f, "    {:>5}  no date", self.skipped_no_date)?;
        writeln!(f, "    {:>5}  no work logged", self.skipped_no_work)?;
        writeln!(f, "    {:>5}  no project named", self.skipped_no_project)?;
        if !self.rows_without_project.is_empty() {
            sample(f, &self.rows_without_project, 5)?;
        }

        if !self.names_collapsed.is_empty() {
            writeln!(f)?;
            writeln!(
                f,
                "Collapsed repeated whitespace in {} name(s); left alone each would \
                 have become a separate record:",
                self.names_collapsed.len()
            )?;
            let lines: Vec<String> = self
                .names_collapsed
                .iter()
                .map(|(at, before, after)| format!("{at}: {before:?} -> {after:?}"))
                .collect();
            sample(f, &lines, 10)?;
        }

        if !self.hours_raised.is_empty() {
            writeln!(f)?;
            writeln!(
                f,
                "Raised {} session(s) to the {:.2} hour minimum:",
                self.hours_raised.len(),
                crate::MIN_SESSION_HOURS
            )?;
            let lines: Vec<String> = self
                .hours_raised
                .iter()
                .map(|(project, date, was)| format!("{date}  {project} (recorded {was})"))
                .collect();
            sample(f, &lines, 10)?;
        }

        if !self.pre_tracking_words.is_empty() {
            writeln!(f)?;
            writeln!(
                f,
                "{} project(s) began before tracking; those words are not in any entry:",
                self.pre_tracking_words.len()
            )?;
            let lines: Vec<String> = self
                .pre_tracking_words
                .iter()
                .map(|(project, words)| {
                    format!("{project}: {words} words before the first session")
                })
                .collect();
            sample(f, &lines, 10)?;
            writeln!(f, "    Import the earlier journal to account for them.")?;
        }

        if !self.uncategorised_projects.is_empty() {
            writeln!(f)?;
            writeln!(
                f,
                "{} project(s) have no category and will show as Uncategorized:",
                self.uncategorised_projects.len()
            )?;
            sample(f, &self.uncategorised_projects, 10)?;
        }

        if !self.status_rows_without_entries.is_empty() {
            writeln!(f)?;
            writeln!(
                f,
                "{} project(s) named in a projects file have no sessions and were skipped:",
                self.status_rows_without_entries.len()
            )?;
            sample(f, &self.status_rows_without_entries, 10)?;
        }

        if !self.projects_defaulted_open.is_empty() {
            writeln!(f)?;
            writeln!(
                f,
                "{} project(s) had no status row and default to open:",
                self.projects_defaulted_open.len()
            )?;
            sample(f, &self.projects_defaulted_open, 10)?;
        }
        Ok(())
    }
}
