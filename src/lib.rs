//! Draft Tracker: a writing log kept in SQLite and edited from a terminal UI.
//!
//! The data layer lives in [`db`]. Everything it exposes is deliberately free of
//! any particular writer's projects or categories, so the same code serves any
//! journal that meets the importer's input contract.

pub mod backup;
pub mod cli;
pub mod config;
pub mod db;
pub mod import;
pub mod tui;

/// The shortest session the application will record, in hours.
///
/// This is an accounting rule rather than an input guard. A minute spent is
/// counted as fifteen, because a session rounded down to no time at all makes a
/// words-per-hour figure either undefined or absurd and misrepresents the day.
/// It is enforced three times over: by a `CHECK` constraint on `entries`, by the
/// entry form before it will submit, and by the importer, which raises anything
/// lower rather than failing the import.
pub const MIN_SESSION_HOURS: f64 = 0.25;
