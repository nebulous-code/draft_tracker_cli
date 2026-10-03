//! The stats dashboard.
//!
//! Every figure comes from a query that takes an optional year, so the same code
//! serves the lifetime column and whichever scope is selected. Nothing here
//! hardcodes "this year", which is what lets the screen step backwards and what
//! makes an all-time view free.

use chrono::{Datelike, Local};

use crate::db::stats::{CategoryWords, ProjectWords, Totals};
use crate::db::{Db, DbError, stats};

/// What the lower half of the dashboard describes.
///
/// Lifetime sits at the end of the sequence of years, so stepping walks
/// 2023, 2024, 2025, 2026, all time: `$` jumps to it the way `$` reaches the end
/// of a line, and `0` returns to the year the writer is living in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Year(i32),
    Lifetime,
}

impl Scope {
    /// The year to filter a query by, or `None` for every year.
    pub fn year(self) -> Option<i32> {
        match self {
            Scope::Year(year) => Some(year),
            Scope::Lifetime => None,
        }
    }

    pub fn label(self) -> String {
        match self {
            Scope::Year(year) => year.to_string(),
            Scope::Lifetime => "All time".to_string(),
        }
    }
}

/// Everything the dashboard draws, loaded in one pass.
pub struct Stats {
    pub scope: Scope,
    /// Years that have entries, plus today's year, ascending.
    years: Vec<i32>,
    pub lifetime: Totals,
    pub lifetime_completed: i64,
    pub selected: Totals,
    pub selected_completed: i64,
    pub selected_projects: Vec<ProjectWords>,
    pub by_year: Vec<(i32, i64)>,
    pub selected_categories: Vec<CategoryWords>,
    /// First project shown in the graph, so a long list can be walked through.
    project_offset: usize,
}

impl Stats {
    pub fn load(db: &Db) -> Result<Self, DbError> {
        let mut stats = Self {
            scope: Scope::Year(Local::now().date_naive().year()),
            years: Vec::new(),
            lifetime: Totals {
                words: 0,
                hours: 0.0,
            },
            lifetime_completed: 0,
            selected: Totals {
                words: 0,
                hours: 0.0,
            },
            selected_completed: 0,
            selected_projects: Vec::new(),
            by_year: Vec::new(),
            selected_categories: Vec::new(),
            project_offset: 0,
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
        if let Scope::Year(year) = self.scope
            && !self.years.contains(&year)
        {
            self.scope = Scope::Year(today);
        }

        self.lifetime = stats::totals(conn, None)?;
        self.lifetime_completed = stats::completed_project_count(conn, None)?;
        self.by_year = stats::words_by_year(conn)?;
        self.reload_scope(db)
    }

    /// Only the scope-dependent half, for a step between years.
    fn reload_scope(&mut self, db: &Db) -> Result<(), DbError> {
        let conn = db.conn();
        let year = self.scope.year();
        self.selected = stats::totals(conn, year)?;
        self.selected_completed = stats::completed_project_count(conn, year)?;
        self.selected_projects = stats::project_words(conn, year)?;
        self.selected_categories = stats::category_words(conn, year)?;
        self.project_offset = 0;
        Ok(())
    }

    pub fn years(&self) -> &[i32] {
        &self.years
    }

    pub fn selected_year(&self) -> Option<i32> {
        self.scope.year()
    }

    pub fn is_lifetime(&self) -> bool {
        self.scope == Scope::Lifetime
    }

    pub fn is_current_year(&self) -> bool {
        self.scope == Scope::Year(Local::now().date_naive().year())
    }

    /// Everything that can be selected, oldest year first and all time last.
    fn sequence(&self) -> Vec<Scope> {
        self.years
            .iter()
            .map(|y| Scope::Year(*y))
            .chain(std::iter::once(Scope::Lifetime))
            .collect()
    }

    /// Step the selection. `-1` is towards older years, `1` towards all time.
    ///
    /// Years with nothing in them are stepped over rather than through: a
    /// keystroke that lands on a screen of zeroes is a keystroke wasted.
    pub fn step(&mut self, direction: i64, db: &Db) -> Result<bool, DbError> {
        let sequence = self.sequence();
        let Some(index) = sequence.iter().position(|s| *s == self.scope) else {
            return Ok(false);
        };
        let target = if direction < 0 {
            index.checked_sub(1)
        } else {
            (index + 1 < sequence.len()).then_some(index + 1)
        };
        let Some(target) = target else {
            return Ok(false);
        };
        self.scope = sequence[target];
        self.reload_scope(db)?;
        Ok(true)
    }

    pub fn go_to_current_year(&mut self, db: &Db) -> Result<(), DbError> {
        self.scope = Scope::Year(Local::now().date_naive().year());
        self.reload_scope(db)
    }

    pub fn go_to_lifetime(&mut self, db: &Db) -> Result<(), DbError> {
        self.scope = Scope::Lifetime;
        self.reload_scope(db)
    }

    // ---------------------------------------------------------- project graph

    pub fn project_offset(&self) -> usize {
        self.project_offset
    }

    /// Scroll the project graph, which matters most in the all-time view where
    /// every project a writer has ever started is on the list.
    pub fn scroll_projects(&mut self, step: i64, visible: usize) -> bool {
        let len = self.selected_projects.len();
        let furthest = len.saturating_sub(visible.max(1));
        let target = self.project_offset as i64 + step;
        let clamped = target.clamp(0, furthest as i64) as usize;
        let moved = clamped != self.project_offset;
        self.project_offset = clamped;
        moved
    }

    // ---------------------------------------------------------- categories

    /// Each category's share of the selected scope, as a fraction.
    ///
    /// Shares are taken against the sum of the positive categories rather than
    /// the net total: a revision-heavy category can sum to less than zero, and a
    /// negative slice of a bar means nothing. Any such category is named in the
    /// legend instead.
    pub fn category_shares(&self) -> Vec<(String, f64, i64)> {
        let positive: i64 = self
            .selected_categories
            .iter()
            .map(|c| c.words)
            .filter(|w| *w > 0)
            .sum();
        if positive == 0 {
            return Vec::new();
        }
        self.selected_categories
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

    /// Categories whose total is zero or negative, so they have no slice.
    pub fn categories_without_a_share(&self) -> Vec<&CategoryWords> {
        self.selected_categories
            .iter()
            .filter(|c| c.words <= 0)
            .collect()
    }
}
