//! Reading source files into raw rows, and nothing else.
//!
//! This module knows the input contract and no rules beyond it. It carries no
//! knowledge of any particular writer's projects or categories.

use std::fmt;
use std::path::{Path, PathBuf};

use calamine::{Data, Ods, Reader, open_workbook};
use chrono::NaiveDate;

use super::ImportError;
use crate::db::models::Status;

/// The sheet an `.ods` workbook must contain.
pub const DATA_SHEET: &str = "Data";

/// Column headers the importer understands. Matching is by name, trimmed and
/// case-insensitive, taking the leftmost match.
pub mod headers {
    pub const DATE: &str = "Date";
    pub const PROJECT: &str = "Project";
    pub const CATEGORY: &str = "Category";
    pub const TYPE: &str = "Type";
    pub const DAY_WC: &str = "Day WC";
    pub const HOURS: &str = "Hours";
    pub const PROJECT_WC: &str = "Project WC";
    pub const NOTES: &str = "Notes";
    pub const STATUS: &str = "Status";
    pub const DESCRIPTION: &str = "Description";
}

/// Where a row came from, for reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub file: String,
    /// 1-based, counting the header, so it matches what an editor shows.
    pub row: usize,
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} row {}", self.file, self.row)
    }
}

/// One row of a writing log, before any rule has been applied.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryRow {
    pub date: Option<NaiveDate>,
    pub project: String,
    pub category: String,
    pub type_name: String,
    pub day_words: Option<f64>,
    pub hours: Option<f64>,
    pub project_words: Option<f64>,
    pub notes: String,
    pub origin: Origin,
}

impl EntryRow {
    /// True when the row records neither words, nor time, nor a running total.
    ///
    /// This is what retires missed-day bookkeeping without naming any sentinel:
    /// a row recording nothing is not a session, whatever it is called.
    pub fn records_no_work(&self) -> bool {
        fn blank_or_zero(v: Option<f64>) -> bool {
            v.is_none_or(|n| n == 0.0)
        }
        blank_or_zero(self.day_words)
            && blank_or_zero(self.hours)
            && blank_or_zero(self.project_words)
    }
}

/// One row of a projects file.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusRow {
    pub origin: Origin,
    pub project: String,
    pub status: Option<Status>,
    pub description: String,
}

/// Everything read from a directory.
#[derive(Debug, Default)]
pub struct Sources {
    pub entries: Vec<EntryRow>,
    pub statuses: Vec<StatusRow>,
    pub files_read: Vec<String>,
    /// Names whose internal whitespace was collapsed: where, before, after.
    pub names_collapsed: Vec<(Origin, String, String)>,
}

/// Trim a name and collapse runs of whitespace inside it to single spaces.
///
/// A project or category whose name differs from another only by a repeated
/// space is a typo every time, and left alone it silently becomes a second
/// record: a phantom category with no projects in it, or a project split in two.
/// OpenDocument encodes a second consecutive space as a `<text:s/>` element, so
/// this is easy to introduce in a spreadsheet and invisible while editing it.
/// The change is reported rather than made quietly.
pub fn collapse_whitespace(name: &str) -> String {
    let trimmed = name.trim();
    if !trimmed.contains("  ") && !trimmed.contains(['\t', '\n', '\r']) {
        return trimmed.to_string();
    }
    trimmed.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Locate a header by name: trimmed, case-insensitive, leftmost match wins.
///
/// Leftmost matters because these sheets repeat `Date`, `Day WC` and `Hours`
/// further right in a derived rolling-average block, and the first occurrence of
/// each is always the real one.
fn column_of(headers: &[String], name: &str) -> Option<usize> {
    headers
        .iter()
        .position(|h| h.trim().eq_ignore_ascii_case(name))
}

/// Resolve the columns an entries file needs, or say which is missing.
struct EntryColumns {
    date: usize,
    project: usize,
    category: usize,
    hours: usize,
    type_name: Option<usize>,
    day_words: Option<usize>,
    project_words: Option<usize>,
    notes: Option<usize>,
}

impl EntryColumns {
    fn resolve(headers: &[String], file: &str) -> Result<Self, ImportError> {
        let required = |name: &str| -> Result<usize, ImportError> {
            column_of(headers, name).ok_or_else(|| ImportError::MissingHeader {
                file: file.to_string(),
                header: name.to_string(),
            })
        };
        let day_words = column_of(headers, headers::DAY_WC);
        let project_words = column_of(headers, headers::PROJECT_WC);
        if day_words.is_none() && project_words.is_none() {
            return Err(ImportError::MissingWordCount {
                file: file.to_string(),
            });
        }
        Ok(Self {
            date: required(headers::DATE)?,
            project: required(headers::PROJECT)?,
            category: required(headers::CATEGORY)?,
            hours: required(headers::HOURS)?,
            type_name: column_of(headers, headers::TYPE),
            day_words,
            project_words,
            notes: column_of(headers, headers::NOTES),
        })
    }
}

// ----------------------------------------------------------------- cell values

fn cell_text(cell: Option<&Data>) -> String {
    match cell {
        None | Some(Data::Empty) => String::new(),
        Some(Data::String(s)) => s.trim().to_string(),
        Some(Data::Float(f)) => f.to_string(),
        Some(Data::Int(i)) => i.to_string(),
        Some(Data::Bool(b)) => b.to_string(),
        Some(Data::DateTimeIso(s)) => s.trim().to_string(),
        Some(other) => other.to_string().trim().to_string(),
    }
}

fn cell_number(cell: Option<&Data>) -> Option<f64> {
    match cell {
        None | Some(Data::Empty) => None,
        Some(Data::Float(f)) => Some(*f),
        Some(Data::Int(i)) => Some(*i as f64),
        Some(Data::String(s)) => parse_number(s),
        _ => None,
    }
}

/// Thousands separators are common in a hand-kept sheet.
fn parse_number(raw: &str) -> Option<f64> {
    let cleaned: String = raw
        .chars()
        .filter(|c| *c != ',' && !c.is_whitespace())
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse().ok()
}

/// Date formats accepted from a CSV.
///
/// ISO-8601 first. The month-first forms are what a spreadsheet writes from a US
/// locale; they are genuinely ambiguous against day-first for the first twelve
/// days of a month, which is why the report states the range it parsed.
const CSV_DATE_FORMATS: [&str; 3] = ["%Y-%m-%d", "%m/%d/%y", "%m/%d/%Y"];

pub fn parse_date_text(raw: &str) -> Option<NaiveDate> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    CSV_DATE_FORMATS
        .iter()
        .find_map(|f| NaiveDate::parse_from_str(raw, f).ok())
}

fn cell_date(cell: Option<&Data>) -> Option<NaiveDate> {
    match cell {
        Some(Data::DateTimeIso(s)) => parse_date_text(s),
        Some(Data::DateTime(dt)) => dt.as_datetime().map(|d| d.date()),
        Some(Data::String(s)) => parse_date_text(s),
        _ => None,
    }
}

// ----------------------------------------------------------------- reading

/// Read every `.ods` and `.csv` in `dir`, in a stable order.
pub fn read_dir(dir: &Path) -> Result<Sources, ImportError> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|source| ImportError::ReadDir {
            path: dir.to_path_buf(),
            source,
        })?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("ods") || e.eq_ignore_ascii_case("csv"))
        })
        .collect();
    // Sorted so that two runs over the same directory agree, which matters
    // because file order breaks ties when deriving a session from a running
    // total within one day.
    paths.sort();

    let mut sources = Sources::default();
    for path in paths {
        let name = file_name(&path);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if ext == "ods" {
            read_ods(&path, &name, &mut sources)?;
        } else {
            read_csv(&path, &name, &mut sources)?;
        }
        sources.files_read.push(name);
    }
    Ok(sources)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn read_ods(path: &Path, name: &str, out: &mut Sources) -> Result<(), ImportError> {
    let mut workbook: Ods<_> =
        open_workbook(path).map_err(|e: calamine::OdsError| ImportError::OpenWorkbook {
            file: name.to_string(),
            message: e.to_string(),
        })?;
    let range = workbook
        .worksheet_range(DATA_SHEET)
        .map_err(|e| ImportError::MissingSheet {
            file: name.to_string(),
            sheet: DATA_SHEET.to_string(),
            message: e.to_string(),
        })?;

    let mut rows = range.rows();
    let Some(header_row) = rows.next() else {
        return Ok(());
    };
    let headers: Vec<String> = header_row.iter().map(|c| cell_text(Some(c))).collect();
    let cols = EntryColumns::resolve(&headers, name)?;

    for (index, row) in rows.enumerate() {
        let at = |i: usize| row.get(i);
        let origin = Origin {
            file: name.to_string(),
            row: index + 2,
        };
        let project = tidy_name(cell_text(at(cols.project)), &origin, out);
        let category = tidy_name(cell_text(at(cols.category)), &origin, out);
        let type_name = cols
            .type_name
            .map(|i| tidy_name(cell_text(at(i)), &origin, out))
            .unwrap_or_default();
        out.entries.push(EntryRow {
            date: cell_date(at(cols.date)),
            project,
            category,
            type_name,
            day_words: cols.day_words.and_then(|i| cell_number(at(i))),
            hours: cell_number(at(cols.hours)),
            project_words: cols.project_words.and_then(|i| cell_number(at(i))),
            notes: cols.notes.map(|i| cell_text(at(i))).unwrap_or_default(),
            origin,
        });
    }
    Ok(())
}

/// Collapse a name's whitespace, recording the change if there was one.
fn tidy_name(raw: String, origin: &Origin, out: &mut Sources) -> String {
    let tidied = collapse_whitespace(&raw);
    if tidied != raw {
        out.names_collapsed
            .push((origin.clone(), raw, tidied.clone()));
    }
    tidied
}

fn read_csv(path: &Path, name: &str, out: &mut Sources) -> Result<(), ImportError> {
    let text = std::fs::read_to_string(path).map_err(|source| ImportError::ReadFile {
        file: name.to_string(),
        source,
    })?;
    // Spreadsheet applications write a byte-order mark when saving UTF-8 CSV.
    // Without stripping it the first header silently fails to match.
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);

    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| ImportError::Csv {
            file: name.to_string(),
            message: e.to_string(),
        })?
        .iter()
        .map(|h| h.trim().to_string())
        .collect();
    if headers.is_empty() {
        return Ok(());
    }

    // A projects file carries Project and Status but no Date. Detection is by
    // header rather than filename so neither file has to be named a particular way.
    let is_projects_file = column_of(&headers, headers::DATE).is_none()
        && column_of(&headers, headers::PROJECT).is_some()
        && column_of(&headers, headers::STATUS).is_some();

    if is_projects_file {
        let project = column_of(&headers, headers::PROJECT).expect("checked above");
        let status = column_of(&headers, headers::STATUS).expect("checked above");
        let description = column_of(&headers, headers::DESCRIPTION);
        for (index, record) in reader.records().enumerate() {
            let record = record.map_err(|e| ImportError::Csv {
                file: name.to_string(),
                message: e.to_string(),
            })?;
            let get = |i: usize| record.get(i).unwrap_or("").trim().to_string();
            let title = collapse_whitespace(&get(project));
            if title.is_empty() {
                continue;
            }
            let raw_status = get(status);
            let parsed = if raw_status.is_empty() {
                None
            } else {
                Some(
                    raw_status
                        .parse::<Status>()
                        .map_err(|_| ImportError::BadStatus {
                            file: name.to_string(),
                            row: index + 2,
                            project: title.clone(),
                            value: raw_status.clone(),
                        })?,
                )
            };
            out.statuses.push(StatusRow {
                origin: Origin {
                    file: name.to_string(),
                    row: index + 2,
                },
                project: title,
                status: parsed,
                description: description.map(get).unwrap_or_default(),
            });
        }
        return Ok(());
    }

    let cols = EntryColumns::resolve(&headers, name)?;
    for (index, record) in reader.records().enumerate() {
        let record = record.map_err(|e| ImportError::Csv {
            file: name.to_string(),
            message: e.to_string(),
        })?;
        let text_at = |i: usize| record.get(i).unwrap_or("").trim().to_string();
        let num_at = |i: usize| parse_number(record.get(i).unwrap_or(""));
        let origin = Origin {
            file: name.to_string(),
            row: index + 2,
        };
        let project = tidy_name(text_at(cols.project), &origin, out);
        let category = tidy_name(text_at(cols.category), &origin, out);
        let type_name = cols
            .type_name
            .map(|i| tidy_name(text_at(i), &origin, out))
            .unwrap_or_default();
        out.entries.push(EntryRow {
            date: parse_date_text(record.get(cols.date).unwrap_or("")),
            project,
            category,
            type_name,
            day_words: cols.day_words.and_then(num_at),
            hours: num_at(cols.hours),
            project_words: cols.project_words.and_then(num_at),
            notes: cols.notes.map(text_at).unwrap_or_default(),
            origin,
        });
    }
    Ok(())
}
