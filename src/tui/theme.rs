//! Colours, referenced by role and inherited from the terminal.
//!
//! Nothing here names a colour by value. Every role resolves to one of the
//! sixteen ANSI colours, or to `Reset`, which emits the terminal's own default
//! foreground or background. The palette is therefore whatever the writer has
//! configured for everything else in their terminal, and the application looks
//! native in it rather than imposing a scheme of its own.
//!
//! This replaced a hardcoded melange palette. Two reasons, in order of weight.
//!
//! A solid background cannot be transparent. A terminal shows its background
//! through only where the application emits no background of its own; an explicit
//! colour is opaque even when its value matches the terminal's exactly. Painting
//! melange's `#292522` over the frame therefore defeated a transparent terminal,
//! and no amount of colour-matching could have fixed it. Only `Reset` can.
//!
//! The second is portability. A fixed palette assumes the background it was
//! designed against. Melange's pale tan secondary text is legible on melange's
//! dark surface and nearly invisible on a light one, so the hardcoded scheme was
//! only safe because it also painted the background it needed. Inheriting both
//! means the two always agree.
//!
//! What this gives up is a contrast guarantee: a writer whose palette is badly
//! balanced gets a badly balanced application, and there is no way to detect that
//! and compensate. Querying the terminal's background over OSC 11 is unreliable
//! and races the input loop, so the honest contract is the one vim keeps — the
//! palette is the writer's to get right.

use ratatui::style::{Color, Modifier, Style};

// The sixteen inherited slots, named by the role the old palette gave them.
//
// Melange was already arranged as six hues in a bright group and a muted group,
// which maps onto ANSI's normal and bright intensities without rearranging
// anything: the bright group takes the `Light*` variants, the muted group the
// plain ones.

/// The terminal's own text colour, for primary text.
const TEXT: Color = Color::Reset;
/// The terminal's own background, which is what lets it stay transparent.
const SURFACE: Color = Color::Reset;
/// ANSI 7. Labels and secondary text.
const DIM_TEXT: Color = Color::Gray;
/// ANSI 8. Borders and inactive chrome, dimmer than secondary text.
const CHROME: Color = Color::DarkGray;

// group b: the bright accents
const RED: Color = Color::LightRed;
const YELLOW: Color = Color::LightYellow;
const GREEN: Color = Color::LightGreen;
const CYAN: Color = Color::LightCyan;
const BLUE: Color = Color::LightBlue;
const MAGENTA: Color = Color::LightMagenta;

// group c: the muted accents
const MUTED_RED: Color = Color::Red;
const MUTED_YELLOW: Color = Color::Yellow;
const MUTED_CYAN: Color = Color::Cyan;
const MUTED_BLUE: Color = Color::Blue;
const MUTED_MAGENTA: Color = Color::Magenta;

/// Rotation for multi-series graphs.
///
/// Green is absent on purpose: it means "completed" on the projects graph, so
/// reusing it for an arbitrary series would make a colour mean two things. Red is
/// absent for the same reason, since it carries error and abandonment.
const SERIES: [Color; 8] = [
    BLUE,
    MAGENTA,
    CYAN,
    YELLOW,
    MUTED_BLUE,
    MUTED_MAGENTA,
    MUTED_CYAN,
    MUTED_YELLOW,
];

/// The nth series colour, wrapping round the rotation.
///
/// A function rather than the array itself, so the sixteen slots above are
/// genuinely private and a caller cannot acquire a `Color` to paint a background
/// with. It also puts the wrap next to the rotation it wraps rather than leaving
/// each caller to remember it.
pub fn series(index: usize) -> Style {
    Style::new().fg(SERIES[index % SERIES.len()])
}

/// The whole frame. Both halves are `Reset`, so a transparent terminal stays
/// transparent and a light one is not painted dark.
pub fn base() -> Style {
    Style::new().fg(TEXT).bg(SURFACE)
}

/// A popup surface.
///
/// Also `Reset` rather than a raised colour. There is no ANSI slot that reliably
/// reads as "slightly lifted from the background" — ANSI 8 is a dim grey on a
/// dark palette and would be dark-on-dark against a light one. Popups are
/// separated by their border instead.
///
/// Because this background is `Reset`, the style no longer hides anything behind
/// it: it sets colours and leaves the symbols alone. Most callers are panels that
/// a layout has carved out, so nothing is drawn behind them in the first place.
/// Anything that genuinely floats over the interface must render `Clear` first,
/// as the help and leader popups do, or the screen will show through its gaps.
pub fn float() -> Style {
    Style::new().fg(TEXT).bg(SURFACE)
}

/// Primary text: a value, a title, a row's main content.
///
/// Spelled as a role so that the sixteen slots above stay private to this module.
/// The melange version did not manage that — `FG` and `YELLOW` were public
/// constants and the drawing code reached for them directly in eighteen places,
/// which is why changing the palette was not the one-file change it should have
/// been. This role and [`key()`] exist to close that gap.
pub fn primary() -> Style {
    Style::new().fg(TEXT)
}

/// A key name in the help list or the leader menu.
pub fn key() -> Style {
    Style::new().fg(YELLOW)
}

/// A field label, or any secondary text.
pub fn label() -> Style {
    Style::new().fg(DIM_TEXT)
}

/// Borders and inactive chrome.
pub fn inactive() -> Style {
    Style::new().fg(CHROME)
}

/// The border or label of whatever currently has focus.
pub fn focused() -> Style {
    Style::new().fg(CYAN).add_modifier(Modifier::BOLD)
}

/// The selected row.
///
/// Reversed video rather than a background colour, because there is no inherited
/// colour that is guaranteed to differ from the terminal's background. Reversing
/// is defined in terms of whatever that background is, so it stands out on every
/// palette, and it makes the row opaque on a transparent terminal, which is what
/// a selection should look like.
pub fn selected_row() -> Style {
    Style::new().fg(TEXT).add_modifier(Modifier::REVERSED)
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
    Style::new().fg(CHROME)
}

/// The colour a project's bar or status takes.
pub fn status(status: crate::db::models::Status) -> Style {
    use crate::db::models::Status;
    match status {
        Status::Open => Style::new().fg(TEXT),
        Status::Completed => Style::new().fg(GREEN),
        // The muted red, so abandoning a piece reads as a neutral outcome rather
        // than as an error, which has the bright one.
        Status::Abandoned => Style::new().fg(MUTED_RED),
    }
}
