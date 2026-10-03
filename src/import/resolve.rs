//! Turning raw rows into records, by rules that apply to any writing log.

use std::collections::{BTreeMap, HashMap};

use chrono::{Datelike, NaiveDate};

use super::ImportError;
use super::report::ImportReport;
use super::source::{EntryRow, Sources};
use crate::MIN_SESSION_HOURS;
use crate::db::models::Status;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedProject {
    pub title: String,
    pub category: Option<String>,
    pub status: Status,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedEntry {
    pub project: String,
    pub date: NaiveDate,
    pub words: i64,
    pub hours: f64,
    pub notes: Option<String>,
}

/// Everything ready to be written, plus what happened on the way.
#[derive(Debug, Default)]
pub struct Resolved {
    /// Type name to its categories, ordered for stable insertion.
    pub types: Vec<String>,
    pub categories: Vec<(String, String)>,
    pub projects: Vec<ResolvedProject>,
    pub entries: Vec<ResolvedEntry>,
    pub report: ImportReport,
}

/// Pick the most common value, breaking ties alphabetically so that two runs
/// over the same data always agree.
fn majority(counts: &BTreeMap<String, usize>) -> Option<String> {
    counts
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(name, _)| name.clone())
}

pub fn resolve(sources: Sources) -> Result<Resolved, ImportError> {
    let Sources {
        entries: raw,
        statuses,
        files_read,
        names_collapsed,
    } = sources;
    let mut report = ImportReport {
        files_read,
        rows_seen: raw.len(),
        names_collapsed: names_collapsed
            .into_iter()
            .map(|(origin, before, after)| (origin.to_string(), before, after))
            .collect(),
        ..Default::default()
    };

    // ---- status rows, before anything depends on them
    let mut status_by_project: HashMap<String, (Status, Option<String>)> = HashMap::new();
    for row in &statuses {
        if let Some(previous) = status_by_project.get(&row.project) {
            return Err(ImportError::DuplicateStatusRow {
                project: row.project.clone(),
                first: previous.0,
                second: row.status.unwrap_or(Status::Open),
                at: row.origin.to_string(),
            });
        }
        let description = (!row.description.is_empty()).then(|| row.description.clone());
        status_by_project.insert(
            row.project.clone(),
            (row.status.unwrap_or(Status::Open), description),
        );
    }

    // ---- which rows are sessions at all
    let mut kept: Vec<(usize, EntryRow)> = Vec::new();
    for (index, row) in raw.into_iter().enumerate() {
        if row.date.is_none() {
            report.skipped_no_date += 1;
        } else if row.records_no_work() {
            report.skipped_no_work += 1;
        } else if row.project.is_empty() {
            report.skipped_no_project += 1;
            report.rows_without_project.push(row.origin.to_string());
        } else {
            kept.push((index, row));
        }
    }

    // ---- group by project, in date order, file order breaking ties
    let mut by_project: BTreeMap<String, Vec<(usize, EntryRow)>> = BTreeMap::new();
    for (index, row) in kept {
        by_project
            .entry(row.project.clone())
            .or_default()
            .push((index, row));
    }
    for rows in by_project.values_mut() {
        rows.sort_by(|a, b| a.1.date.cmp(&b.1.date).then_with(|| a.0.cmp(&b.0)));
    }

    let mut entries: Vec<ResolvedEntry> = Vec::new();
    let mut category_votes: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut type_votes: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();

    for (title, rows) in &by_project {
        // A manuscript that already existed when tracking began: reported so the
        // gap is visible, never invented as an entry or stored as a baseline.
        if let Some((_, first)) = rows.first()
            && let (Some(pwc), Some(day)) = (first.project_words, first.day_words)
        {
            let before = (pwc - day).round() as i64;
            if before > 0 {
                report.pre_tracking_words.push((title.clone(), before));
            }
        }

        // Either word-count column can drive an entry. `Day WC` is used when
        // present; otherwise the session is the rise in `Project WC`, which is
        // the same arithmetic the spreadsheets encode as `=I3-I2`.
        let mut running: Option<f64> = None;
        for (_, row) in rows {
            let words = match row.day_words {
                Some(day) => day,
                None => match row.project_words {
                    Some(total) => total - running.unwrap_or(0.0),
                    None => 0.0,
                },
            };
            if let Some(total) = row.project_words {
                running = Some(total);
            }

            let recorded = row.hours.unwrap_or(0.0);
            let hours = if recorded.is_nan() || recorded < MIN_SESSION_HOURS {
                report
                    .hours_raised
                    .push((title.clone(), row.date.expect("dated"), recorded));
                MIN_SESSION_HOURS
            } else {
                recorded
            };

            if !row.category.is_empty() {
                *category_votes
                    .entry(title.clone())
                    .or_default()
                    .entry(row.category.clone())
                    .or_default() += 1;
                if !row.type_name.is_empty() {
                    *type_votes
                        .entry(row.category.clone())
                        .or_default()
                        .entry(row.type_name.clone())
                        .or_default() += 1;
                }
            }

            entries.push(ResolvedEntry {
                project: title.clone(),
                date: row.date.expect("dated"),
                words: words.round() as i64,
                hours,
                notes: (!row.notes.is_empty()).then(|| row.notes.clone()),
            });
        }
    }

    // ---- a category's type is the majority of the rows carrying it
    let mut categories: Vec<(String, String)> = Vec::new();
    let mut types: Vec<String> = Vec::new();
    for category in category_votes
        .values()
        .flat_map(|v| v.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        let votes = type_votes.get(category).and_then(majority);
        let Some(type_name) = votes else {
            // Nothing anywhere states this category's type, and guessing one
            // would invent a taxonomy the writer never wrote.
            return Err(ImportError::UntypedCategory {
                category: category.clone(),
            });
        };
        if !types.contains(&type_name) {
            types.push(type_name.clone());
        }
        categories.push((category.clone(), type_name));
    }
    types.sort();
    categories.sort();

    // ---- projects
    let mut projects: Vec<ResolvedProject> = Vec::new();
    for title in by_project.keys() {
        let category = category_votes.get(title).and_then(majority);
        if category.is_none() {
            report.uncategorised_projects.push(title.clone());
        }
        let (status, description) = match status_by_project.get(title) {
            Some((status, description)) => (*status, description.clone()),
            None => {
                report.projects_defaulted_open.push(title.clone());
                (Status::Open, None)
            }
        };
        projects.push(ResolvedProject {
            title: title.clone(),
            category,
            status,
            description,
        });
    }

    // A projects file naming something with no sessions has nothing to track.
    for name in status_by_project.keys() {
        if !by_project.contains_key(name) {
            report.status_rows_without_entries.push(name.clone());
        }
    }
    report.status_rows_without_entries.sort();

    // ---- summary figures
    entries.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.project.cmp(&b.project)));
    report.entries = entries.len();
    report.words = entries.iter().map(|e| e.words).sum();
    report.hours = entries.iter().map(|e| e.hours).sum();
    report.types = types.len();
    report.categories = categories.len();
    report.projects = projects.len();
    report.date_range = match (entries.first(), entries.last()) {
        (Some(first), Some(last)) => Some((first.date, last.date)),
        _ => None,
    };
    let mut by_year: BTreeMap<i32, i64> = BTreeMap::new();
    for entry in &entries {
        *by_year.entry(entry.date.year()).or_default() += entry.words;
    }
    report.words_by_year = by_year.into_iter().collect();

    let mut words_by_project: BTreeMap<&str, i64> = BTreeMap::new();
    for entry in &entries {
        *words_by_project.entry(entry.project.as_str()).or_default() += entry.words;
    }
    let mut breakdown: BTreeMap<&str, (usize, i64)> = BTreeMap::new();
    for project in &projects {
        let key = project.category.as_deref().unwrap_or("Uncategorized");
        let slot = breakdown.entry(key).or_default();
        slot.0 += 1;
        slot.1 += words_by_project
            .get(project.title.as_str())
            .copied()
            .unwrap_or(0);
    }
    report.category_breakdown = breakdown
        .into_iter()
        .map(|(category, (count, words))| {
            let type_name = categories
                .iter()
                .find(|(name, _)| name == category)
                .map(|(_, t)| t.clone())
                .unwrap_or_default();
            (category.to_string(), type_name, count, words)
        })
        .collect();
    report
        .category_breakdown
        .sort_by(|a, b| b.3.cmp(&a.3).then_with(|| a.0.cmp(&b.0)));
    // Sorted by project then date: deterministic, and f64 is not Ord so the
    // recorded value cannot take part in the ordering anyway.
    report
        .hours_raised
        .sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    Ok(Resolved {
        types,
        categories,
        projects,
        entries,
        report,
    })
}
