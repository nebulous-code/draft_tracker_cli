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

use std::time::Duration;

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

/// How long the leader key waits before showing its menu.
///
/// Short enough to feel immediate, long enough that a sequence typed at speed
/// never flashes a popup. Zero would show it on every press, including the
/// hundreds of `<leader><CR>` submissions where it would only flicker.
const LEADER_HINT_DELAY: Duration = Duration::from_millis(50);

fn event_loop(terminal: &mut ratatui::DefaultTerminal, db: Db) -> Result<()> {
    let mut app = App::new(db)?;
    loop {
        terminal.draw(|frame| draw::draw(frame, &mut app))?;

        // Nothing changes on its own, so the loop blocks on input. The one
        // exception is a pending leader key, which turns into a menu if the
        // writer pauses rather than carrying straight on.
        if app.leader_pending && !app.leader_hint && !event::poll(LEADER_HINT_DELAY)? {
            app.leader_hint = true;
            continue;
        }

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
