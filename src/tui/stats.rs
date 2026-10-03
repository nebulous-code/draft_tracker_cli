//! The stats dashboard.
//!
//! Every figure comes from a query that takes an optional year, so the same code
//! serves the lifetime section and whichever year is selected. Nothing here
//! hardcodes "this year", which is what lets the screen step backwards.

use chrono::{Datelike, Local};

use crate::db::stats::{CategoryWords, ProjectWords, Totals};
use crate::db::{Db, DbError, stats};

/// Everything the dashboard draws, loaded in one pass.
pub struct Stats {
    /// The year the right-hand column and the two lower graphs describe.
    pub year: i32,
    /// Years that have entries, plus today's year, ascending.
    years: Vec<i32>,
    pub lifetime: Totals,
    pub lifetime_completed: i64,
    pub year_totals: Totals,
    pub year_completed: i64,
    pub year_projects: Vec<ProjectWords>,
    pub by_year: Vec<(i32, i64)>,
    pub year_categories: Vec<CategoryWords>,
}

impl Stats {
    pub fn load(db: &Db) -> Result<Self, DbError> {
        let today = Local::now().date_naive().year();
        let mut stats = Self {
            year: today,
            years: Vec::new(),
            lifetime: Totals {
                words: 0,
                hours: 0.0,
            },
            lifetime_completed: 0,
            year_totals: Totals {
                words: 0,
                hours: 0.0,
            },
            year_completed: 0,
            year_projects: Vec::new(),
            by_year: Vec::new(),
            year_categories: Vec::new(),
        };
        stats.reload(db)?;
        Ok(stats)
    }

    pub fn reload(&mut self, db: &Db) -> Result<(), DbError> {
        let conn = db.conn();
        let today = Local::now().date_naive().year();

        // The current year is always offered even with nothing logged in it yet,
        // so the screen a writer opens on is the one they are living in.
        let mut years = stats::years_with_entries(conn)?;
        if !years.contains(&today) {
            years.push(today);
        }
        years.sort_unstable();
        self.years = years;
        if !self.years.contains(&self.year) {
            self.year = today;
        }

        self.lifetime = stats::totals(conn, None)?;
        self.lifetime_completed = stats::completed_project_count(conn, None)?;
        self.by_year = stats::words_by_year(conn)?;
        self.reload_year(db)
    }

    /// Only the year-dependent half, for a step between years.
    fn reload_year(&mut self, db: &Db) -> Result<(), DbError> {
        let conn = db.conn();
        let year = Some(self.year);
        self.year_totals = stats::totals(conn, year)?;
        self.year_completed = stats::completed_project_count(conn, year)?;
        self.year_projects = stats::project_words(conn, year)?;
        self.year_categories = stats::category_words(conn, year)?;
        Ok(())
    }

    pub fn years(&self) -> &[i32] {
        &self.years
    }

    pub fn is_current_year(&self) -> bool {
        self.year == Local::now().date_naive().year()
    }

    /// Step to the previous year that has something in it.
    ///
    /// Stepping over empty years rather than through them: a dead step that
    /// shows nothing but zeroes is a keystroke wasted.
    pub fn previous_year(&mut self, db: &Db) -> Result<bool, DbError> {
        let Some(index) = self.years.iter().position(|y| *y == self.year) else {
            return Ok(false);
        };
        if index == 0 {
            return Ok(false);
        }
        self.year = self.years[index - 1];
        self.reload_year(db)?;
        Ok(true)
    }

    pub fn next_year(&mut self, db: &Db) -> Result<bool, DbError> {
        let Some(index) = self.years.iter().position(|y| *y == self.year) else {
            return Ok(false);
        };
        if index + 1 >= self.years.len() {
            return Ok(false);
        }
        self.year = self.years[index + 1];
        self.reload_year(db)?;
        Ok(true)
    }

    pub fn go_to_current_year(&mut self, db: &Db) -> Result<(), DbError> {
        self.year = Local::now().date_naive().year();
        self.reload_year(db)
    }

    /// Each category's share of the selected year, as a fraction.
    ///
    /// Shares are taken against the sum of the positive categories rather than
    /// the year's net total: a revision-heavy category can sum to less than zero,
    /// and a negative slice of a bar means nothing. Any such category is reported
    /// in the legend instead.
    pub fn category_shares(&self) -> Vec<(String, f64, i64)> {
        let positive: i64 = self
            .year_categories
            .iter()
            .map(|c| c.words)
            .filter(|w| *w > 0)
            .sum();
        if positive == 0 {
            return Vec::new();
        }
        self.year_categories
            .iter()
            .filter(|c| c.words > 0)
            .map(|c| {
                (
                    c.category.clone(),
                    c.words as f64 / positive as f64,
                    c.words,
                )
            })
            .collect()
    }

    /// Categories whose year total is zero or negative, so they have no slice.
    pub fn categories_without_a_share(&self) -> Vec<&CategoryWords> {
        self.year_categories
            .iter()
            .filter(|c| c.words <= 0)
            .collect()
    }
}
