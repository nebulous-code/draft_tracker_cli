//! The keybinding reminder, opened with `?`.
//!
//! Context-aware: the shared bindings first, then whatever the current screen
//! adds. No binding uses a control key, so the same keymap works on mac, linux
//! and windows.

use super::app::Screen;

pub struct Section {
    pub title: &'static str,
    pub bindings: &'static [(&'static str, &'static str)],
}

const GLOBAL: Section = Section {
    title: "Anywhere",
    bindings: &[
        ("?", "this help"),
        ("<leader>e", "entry"),
        ("<leader>h", "entry history"),
        ("<leader>p", "projects"),
        ("<leader>c", "categories"),
        ("<leader>s", "stats"),
        ("q", "quit"),
        ("<leader>", "the space bar"),
    ],
};

const ENTRY: Section = Section {
    title: "Entry form",
    bindings: &[
        ("Tab / Shift-Tab", "next / previous field"),
        ("j / k", "move through the project list"),
        ("gg / G", "first / last project"),
        ("i or a", "start typing in a field"),
        ("Esc", "stop typing"),
        ("/", "filter projects"),
        ("A", "show completed and abandoned projects too"),
        ("<leader><CR>", "submit the entry"),
    ],
};

const HISTORY: Section = Section {
    title: "Entry history",
    bindings: &[
        ("j / k", "move through the entries"),
        ("gg / G", "newest / oldest entry"),
        ("/", "filter by project"),
        ("Esc", "clear the filter"),
        ("i or <CR>", "edit the selected entry"),
        ("dd", "delete it, after confirming"),
        ("<leader><CR>", "save the open edit"),
    ],
};

const PROJECTS: Section = Section {
    title: "Projects",
    bindings: &[
        ("j / k", "move through the projects"),
        ("gg / G", "first / last"),
        ("/", "filter by title or category"),
        ("x", "cycle status: open, completed, abandoned"),
        ("i or <CR>", "edit the selected project"),
        ("o", "create a project"),
        ("dd", "delete it; refused if it has entries"),
        ("h / l", "step a choice field while editing"),
        ("<leader><CR>", "save the open edit"),
    ],
};

const CATEGORIES: Section = Section {
    title: "Categories and types",
    bindings: &[
        ("Tab or h / l", "switch between the two panes"),
        ("j / k", "move through the focused pane"),
        ("i or <CR>", "edit the selected record"),
        ("o", "create one in the focused pane"),
        ("dd", "delete it; refused if anything uses it"),
        ("<leader><CR>", "save the open edit"),
    ],
};

const STATS: Section = Section {
    title: "Stats",
    bindings: &[
        ("[ or h", "previous year with entries"),
        ("] or l", "next year"),
        ("0", "back to this year"),
    ],
};

pub fn sections(screen: Screen) -> Vec<&'static Section> {
    let screen_section = match screen {
        Screen::Entry => &ENTRY,
        Screen::History => &HISTORY,
        Screen::Projects => &PROJECTS,
        Screen::Categories => &CATEGORIES,
        Screen::Stats => &STATS,
    };
    vec![&GLOBAL, screen_section]
}

/// Width needed so no binding wraps.
pub fn widest_key(sections: &[&Section]) -> usize {
    sections
        .iter()
        .flat_map(|s| s.bindings.iter())
        .map(|(key, _)| key.len())
        .max()
        .unwrap_or(0)
}
