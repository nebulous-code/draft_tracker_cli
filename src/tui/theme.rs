//! The melange palette, referenced by role rather than by colour.
//!
//! Widgets name a role, so the palette is defined once here and a change to the
//! scheme never has to be chased through the drawing code.

use ratatui::style::{Color, Modifier, Style};

const fn rgb(hex: u32) -> Color {
    Color::Rgb(
        (hex >> 16) as u8,
        ((hex >> 8) & 0xff) as u8,
        (hex & 0xff) as u8,
    )
}

// melange, group a: surfaces and text
pub const BG: Color = rgb(0x292522);
pub const FLOAT: Color = rgb(0x34302C);
pub const SELECTION: Color = rgb(0x403A36);
pub const UI: Color = rgb(0x867462);
pub const COMMENT: Color = rgb(0xC1A78E);
pub const FG: Color = rgb(0xECE1D7);

// group b: the bright accents
pub const RED: Color = rgb(0xD47766);
pub const YELLOW: Color = rgb(0xEBC06D);
pub const GREEN: Color = rgb(0x85B695);
pub const CYAN: Color = rgb(0x89B3B6);
pub const BLUE: Color = rgb(0xA3A9CE);
pub const MAGENTA: Color = rgb(0xCF9BC2);

// group c: the muted accents
pub const MUTED_RED: Color = rgb(0xBD8183);
pub const MUTED_YELLOW: Color = rgb(0xE49B5D);
pub const MUTED_GREEN: Color = rgb(0x78997A);
pub const MUTED_CYAN: Color = rgb(0x7B9695);
pub const MUTED_BLUE: Color = rgb(0x7F91B2);
pub const MUTED_MAGENTA: Color = rgb(0xB380B0);

/// Rotation for multi-series graphs.
///
/// Green is absent on purpose: it means "completed" on the projects graph, so
/// reusing it for an arbitrary series would make a colour mean two things.
pub const SERIES: [Color; 8] = [
    BLUE,
    MAGENTA,
    CYAN,
    YELLOW,
    MUTED_BLUE,
    MUTED_MAGENTA,
    MUTED_CYAN,
    MUTED_YELLOW,
];

pub fn base() -> Style {
    Style::new().fg(FG).bg(BG)
}

pub fn float() -> Style {
    Style::new().fg(FG).bg(FLOAT)
}

/// A field label, or any secondary text.
pub fn label() -> Style {
    Style::new().fg(COMMENT)
}

/// Borders and inactive chrome.
pub fn inactive() -> Style {
    Style::new().fg(UI)
}

/// The border or label of whatever currently has focus.
pub fn focused() -> Style {
    Style::new().fg(CYAN).add_modifier(Modifier::BOLD)
}

pub fn selected_row() -> Style {
    Style::new().bg(SELECTION).fg(FG)
}

/// A read-only value the application worked out.
pub fn derived() -> Style {
    Style::new().fg(BLUE)
}

pub fn warning() -> Style {
    Style::new().fg(YELLOW).add_modifier(Modifier::BOLD)
}

pub fn error() -> Style {
    Style::new().fg(RED).add_modifier(Modifier::BOLD)
}

pub fn success() -> Style {
    Style::new().fg(GREEN)
}

pub fn hint() -> Style {
    Style::new().fg(UI)
}

/// The colour a project's bar or status takes.
pub fn status(status: crate::db::models::Status) -> Style {
    use crate::db::models::Status;
    match status {
        Status::Open => Style::new().fg(FG),
        Status::Completed => Style::new().fg(GREEN),
        // The muted red, so abandoning a piece reads as a neutral outcome
        // rather than as an error.
        Status::Abandoned => Style::new().fg(MUTED_RED),
    }
}
