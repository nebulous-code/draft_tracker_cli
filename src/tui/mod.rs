//! The terminal interface.

pub mod app;
pub mod categories;
pub mod draw;
pub mod entry;
pub mod help;
pub mod history;
pub mod projects;
pub mod stats;
pub mod theme;

use std::io::IsTerminal;

use anyhow::{Context, Result, bail};
use ratatui::crossterm::event::{self, Event};

use crate::db::Db;
use app::{App, Flow};

/// Run the interface until the writer quits.
///
/// `ratatui::try_init` installs a panic hook that restores the terminal, so a
/// crash cannot leave the shell in raw mode with no echo.
pub fn run(db: Db) -> Result<()> {
    // Checked up front so that running the command in a pipe explains itself,
    // rather than failing with whatever errno raw mode happens to return.
    if !std::io::stdout().is_terminal() {
        bail!(
            "the interface needs a terminal; this looks like a pipe or redirect. \
             Try `draft-tracker status` or `draft-tracker import` instead."
        );
    }
    let mut terminal = ratatui::try_init().context("setting up the terminal")?;
    let result = event_loop(&mut terminal, db);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, db: Db) -> Result<()> {
    let mut app = App::new(db)?;
    loop {
        terminal.draw(|frame| draw::draw(frame, &mut app))?;
        // A blocking read: nothing changes on its own, so there is no reason to
        // wake up and redraw on a timer.
        match event::read()? {
            Event::Key(key) => {
                if app.on_key(key)? == Flow::Quit {
                    return Ok(());
                }
            }
            // Any other event just means the next draw should happen.
            _ => continue,
        }
    }
}
