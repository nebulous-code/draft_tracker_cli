//! Drawing. Reads state, never changes it.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};

use super::app::{App, Screen, Tone, mode_label};
use super::categories::{self, CategoryField, Pane, TypeField};
use super::entry::{Field, Mode};
use super::history::{EditField, History};
use super::projects::{self as projects_screen};
use super::{help, theme};

pub fn draw(frame: &mut Frame<'_>, app: &mut App) {
    let area = frame.area();
    frame.render_widget(Block::new().style(theme::base()), area);

    // Below this the form cannot be laid out without lying about what it shows.
    if area.width < 56 || area.height < 16 {
        let text = format!("{}x{} is too small. Needs 56x16.", area.width, area.height);
        frame.render_widget(
            Paragraph::new(text)
                .style(theme::warning())
                .alignment(Alignment::Center)
                // Without wrapping, the message saying the terminal is too
                // small is itself the thing that gets cut off.
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let [title_area, body_area, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_title(frame, app, title_area);
    match app.screen {
        Screen::Entry => draw_entry(frame, app, body_area),
        Screen::History => draw_history(frame, app, body_area),
        Screen::Projects => draw_projects(frame, app, body_area),
        Screen::Categories => draw_categories(frame, app, body_area),
        Screen::Stats => draw_stats(frame, app, body_area),
    }
    draw_status(frame, app, status_area);

    if app.help_visible {
        draw_help(frame, app, area);
    }
}

fn draw_title(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let mut spans = vec![
        Span::styled(" Draft Tracker ", theme::focused()),
        Span::styled("· ", theme::inactive()),
        Span::styled(app.screen.title(), Style::new().fg(theme::FG)),
    ];
    if app.form.include_closed {
        spans.push(Span::styled("  (all projects)", theme::label()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme::base()), area);
}

fn draw_entry(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let [form_area, picker_area] =
        Layout::horizontal([Constraint::Min(40), Constraint::Length(32)]).areas(area);
    draw_form(frame, app, form_area);
    draw_picker(frame, app, picker_area);
}

/// One labelled row: `Label   value   hint`.
fn field_line<'a>(
    label: &'a str,
    value: Span<'a>,
    hint: Option<Span<'a>>,
    focused: bool,
) -> Line<'a> {
    let marker = if focused { "▌" } else { " " };
    let label_style = if focused {
        theme::focused()
    } else {
        theme::label()
    };
    let mut spans = vec![
        Span::styled(marker, theme::focused()),
        Span::styled(format!("{label:<12}"), label_style),
        value,
    ];
    if let Some(hint) = hint {
        spans.push(Span::raw("  "));
        spans.push(hint);
    }
    Line::from(spans)
}

fn draw_form(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let form = &app.form;
    let preview = form.preview();
    let editing = form.mode == Mode::Insert;

    // A caret shows where typing would land, which a static value cannot.
    let text_value = |field: Field| -> Span<'_> {
        let raw = form.text_of(field);
        let focused = form.field == field;
        let shown = if focused && editing {
            format!("{raw}▏")
        } else if raw.is_empty() {
            "—".to_string()
        } else {
            raw.to_string()
        };
        let style = if raw.is_empty() && !(focused && editing) {
            theme::hint()
        } else {
            Style::new().fg(theme::FG)
        };
        Span::styled(shown, style)
    };

    let selected = form.selected_project();
    // Drawn in the same brackets the projects screen uses for a choice field, so
    // it reads as something you step rather than something you type into.
    let on_project = form.field == Field::Project;
    let project_value = match selected {
        Some(choice) if on_project => Span::styled(
            format!("\u{2039} {} \u{203a}", choice.title),
            theme::status(choice.status),
        ),
        Some(choice) => Span::styled(choice.title.clone(), theme::status(choice.status)),
        None => Span::styled("no project selected", theme::hint()),
    };
    let project_hint = selected.map(|choice| {
        if choice.is_first_entry() {
            Span::styled("first entry", theme::warning())
        } else {
            Span::styled(
                format!("logged so far {}", thousands(choice.logged_words)),
                theme::label(),
            )
        }
    });

    let mut lines = vec![
        Line::from(""),
        field_line(
            "Date",
            text_value(Field::Date),
            None,
            form.field == Field::Date,
        ),
        field_line(
            "Project",
            project_value,
            project_hint,
            form.field == Field::Project,
        ),
        field_line(
            "Hours",
            text_value(Field::Hours),
            None,
            form.field == Field::Hours,
        ),
        field_line(
            "Project WC",
            text_value(Field::ProjectWords),
            None,
            form.field == Field::ProjectWords,
        ),
        field_line(
            "Notes",
            text_value(Field::Notes),
            None,
            form.field == Field::Notes,
        ),
    ];

    // The one place Project WC means something different, said in full rather
    // than squeezed into a hint the column would truncate.
    if selected.is_some_and(|c| c.is_first_entry()) {
        lines.push(Line::from(Span::styled(
            "  first entry: type the words written today,",
            theme::warning(),
        )));
        lines.push(Line::from(Span::styled(
            "  not the manuscript total",
            theme::warning(),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  ──────── worked out for you ────────",
        theme::inactive(),
    )));

    lines.push(derived_line("Session WC", session_span(&preview)));
    lines.push(derived_line("Words/Hour", rate_span(&preview)));

    if let Some(words) = preview.session_words
        && words < 0
    {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                "  cut {} words — a revision day, submits normally",
                thousands(-words)
            ),
            theme::warning(),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(" Session ")
                .border_style(theme::inactive())
                .title_style(theme::label()),
        ),
        area,
    );
}

fn derived_line<'a>(label: &'a str, value: Span<'a>) -> Line<'a> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled(format!("{label:<12}"), theme::label()),
        value,
    ])
}

fn session_span(preview: &super::entry::Preview) -> Span<'static> {
    match preview.session_words {
        None => Span::styled("—", theme::hint()),
        Some(words) if words < 0 => Span::styled(thousands(words), theme::warning()),
        Some(words) => Span::styled(thousands(words), theme::derived()),
    }
}

fn rate_span(preview: &super::entry::Preview) -> Span<'static> {
    match preview.words_per_hour {
        // Undefined is reserved for a zero denominator; negative is a real rate.
        None => Span::styled("—", theme::hint()),
        Some(rate) if rate < 0.0 => Span::styled(format!("{rate:.0}"), theme::warning()),
        Some(rate) => Span::styled(format!("{rate:.0}"), theme::derived()),
    }
}

fn draw_picker(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let form = &app.form;
    let visible = form.visible();
    let focused = form.field == Field::Project;

    let items: Vec<ListItem<'_>> = visible
        .iter()
        .map(|choice| {
            let mut spans = vec![Span::styled(
                choice.title.clone(),
                theme::status(choice.status),
            )];
            if choice.is_first_entry() {
                spans.push(Span::styled("  new", theme::warning()));
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let title = if form.filter.is_empty() {
        format!(" Projects ({}) ", visible.len())
    } else {
        format!(" Projects ({}) /{} ", visible.len(), form.filter)
    };
    let border = if focused {
        theme::focused()
    } else {
        theme::inactive()
    };

    if items.is_empty() {
        let message = if form.filter.is_empty() {
            "No open projects.\n\nPress A to include\ncompleted and\nabandoned ones."
        } else {
            "Nothing matches.\n\nEsc clears the filter."
        };
        frame.render_widget(
            Paragraph::new(message).style(theme::hint()).block(
                Block::bordered()
                    .title(title)
                    .border_style(border)
                    .title_style(theme::label()),
            ),
            area,
        );
        return;
    }

    let list = List::new(items)
        .block(
            Block::bordered()
                .title(title)
                .border_style(border)
                .title_style(theme::label()),
        )
        .highlight_style(theme::selected_row())
        .highlight_symbol("› ");
    let mut state = ListState::default().with_selected(Some(form.selected_index()));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_history(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    // The edit panel takes a fixed slice off the bottom; the list keeps the rest.
    let (list_area, edit_area) = if app.history.edit.is_some() {
        let [list, edit] =
            Layout::vertical([Constraint::Min(4), Constraint::Length(10)]).areas(area);
        (list, Some(edit))
    } else {
        (area, None)
    };
    draw_history_list(frame, app, list_area);
    if let Some(edit_area) = edit_area {
        // The picker only takes space while the project is the field in hand.
        let on_project = app
            .history
            .edit
            .as_ref()
            .is_some_and(|e| e.field == EditField::Project);
        if on_project {
            let [fields, picker] =
                Layout::horizontal([Constraint::Min(40), Constraint::Length(32)]).areas(edit_area);
            draw_edit_panel(frame, app, fields);
            draw_edit_project_picker(frame, app, picker);
        } else {
            draw_edit_panel(frame, app, edit_area);
        }
    }
}

/// Width given to the project column, so the columns line up down the page.
const PROJECT_COLUMN: usize = 24;

fn draw_history_list(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let rows = app.history.visible();
    let title = if app.history.filter.is_empty() {
        format!(" Entries ({}) ", rows.len())
    } else {
        format!(" Entries ({}) /{} ", rows.len(), app.history.filter)
    };
    let focused = app.history.edit.is_none();
    let border = if focused {
        theme::focused()
    } else {
        theme::inactive()
    };
    let block = Block::bordered()
        .title(title)
        .border_style(border)
        .title_style(theme::label());

    if rows.is_empty() {
        let message = if app.history.filter.is_empty() {
            "No entries yet.\n\nAdd one on the entry screen with <leader>e."
        } else {
            "No entries for that project.\n\nEsc clears the filter."
        };
        frame.render_widget(
            Paragraph::new(message).style(theme::hint()).block(block),
            area,
        );
        return;
    }

    // Notes get whatever is left after the fixed columns and the borders.
    let fixed = 10 + 2 + PROJECT_COLUMN + 2 + 8 + 2 + 6 + 2;
    let note_width = (area.width as usize).saturating_sub(fixed + 4);

    let items: Vec<ListItem<'_>> = rows
        .iter()
        .map(|row| {
            let (date, project, words, hours, notes) = History::row_cells(row, PROJECT_COLUMN);
            let word_style = if row.entry.session_word_count < 0 {
                theme::warning()
            } else {
                Style::new().fg(theme::FG)
            };
            ListItem::new(Line::from(vec![
                Span::styled(date, theme::label()),
                Span::raw("  "),
                Span::styled(
                    format!("{project:<PROJECT_COLUMN$}"),
                    theme::status(History::row_status(row)),
                ),
                Span::raw("  "),
                Span::styled(format!("{words:>8}"), word_style),
                Span::raw("  "),
                Span::styled(format!("{hours:>5}h"), theme::label()),
                Span::raw("  "),
                Span::styled(super::history::truncate(&notes, note_width), theme::hint()),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(theme::selected_row())
        .highlight_symbol("› ");
    let mut state = ListState::default().with_selected(Some(app.history.selected_index()));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_edit_panel(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let edit = app.history.edit.as_ref().expect("checked by caller");
    let editing = edit.mode == Mode::Insert;

    let value = |field: EditField| -> Span<'_> {
        let raw = edit.text_of(field);
        let focused = edit.field == field;
        let shown = if focused && editing {
            format!("{raw}▏")
        } else if raw.is_empty() {
            "—".to_string()
        } else {
            raw.to_string()
        };
        let style = if raw.is_empty() && !(focused && editing) {
            theme::hint()
        } else {
            Style::new().fg(theme::FG)
        };
        Span::styled(shown, style)
    };

    let on_project = edit.field == EditField::Project;
    let project_value = if on_project {
        Span::styled(
            format!("\u{2039} {} \u{203a}", edit.project_title),
            Style::new().fg(theme::FG),
        )
    } else {
        Span::styled(edit.project_title.clone(), theme::label())
    };
    let mut lines = vec![
        field_line(
            "Date",
            value(EditField::Date),
            None,
            edit.field == EditField::Date,
        ),
        field_line("Project", project_value, None, on_project),
        field_line(
            "Session WC",
            value(EditField::SessionWords),
            Some(Span::styled(
                "the words in this row, not a total",
                theme::label(),
            )),
            edit.field == EditField::SessionWords,
        ),
        field_line(
            "Hours",
            value(EditField::Hours),
            None,
            edit.field == EditField::Hours,
        ),
        field_line(
            "Notes",
            value(EditField::Notes),
            None,
            edit.field == EditField::Notes,
        ),
    ];
    if let Some(problem) = edit.problems().first() {
        lines.push(Line::from(Span::styled(
            format!("  {problem}"),
            theme::error(),
        )));
    } else if on_project {
        lines.push(Line::from(Span::styled(
            "  h l step the project  ·  i filters  ·  <leader><CR> saves",
            theme::hint(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  <leader><CR> saves  ·  Esc discards",
            theme::hint(),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines).style(theme::float()).block(
            Block::bordered()
                .title(" Edit entry ")
                .border_style(theme::focused())
                .title_style(theme::focused())
                .style(theme::float()),
        ),
        area,
    );
}

// ------------------------------------------------------------------ projects

const P_TITLE: usize = 26;
const P_CATEGORY: usize = 18;

fn draw_projects(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let [header, list_area, lower] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(10),
    ])
    .areas(area);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(
                "  {:<P_TITLE$}  {:<P_CATEGORY$}  {:<10}{:>9}{:>6}{:>8}{:>7}",
                "Title", "Category", "Status", "Words", "Days", "Hours", "W/Hr"
            ),
            theme::label(),
        )))
        .style(theme::base()),
        header,
    );

    let rows = app.projects.visible();
    // The title states which view this is, so a short list never looks like
    // missing data.
    let scope = if app.projects.show_all() {
        format!("all {}", app.projects.total())
    } else {
        format!("{} open of {}", rows.len(), app.projects.total())
    };
    let title = if app.projects.filter.is_empty() {
        format!(" Projects ({scope}) ")
    } else {
        format!(" Projects ({scope}) /{} ", app.projects.filter)
    };
    let focused = app.projects.edit.is_none();
    let block = Block::bordered()
        .title(title)
        .border_style(if focused {
            theme::focused()
        } else {
            theme::inactive()
        })
        .title_style(theme::label());

    if rows.is_empty() {
        let message = if !app.projects.filter.is_empty() {
            "Nothing matches.\n\nEsc clears the filter."
        } else if app.projects.total() > 0 {
            "No open projects.\n\nPress A to show all of them."
        } else {
            "No projects yet.\n\nPress o to create one."
        };
        frame.render_widget(
            Paragraph::new(message).style(theme::hint()).block(block),
            list_area,
        );
    } else {
        let items: Vec<ListItem<'_>> = rows
            .iter()
            .map(|row| {
                let m = &row.metrics;
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!(
                            "{:<P_TITLE$}",
                            super::history::truncate(&row.project.title, P_TITLE)
                        ),
                        Style::new().fg(theme::FG),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!(
                            "{:<P_CATEGORY$}",
                            super::history::truncate(
                                row.category_name.as_deref().unwrap_or("Uncategorized"),
                                P_CATEGORY
                            )
                        ),
                        theme::label(),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        format!("{:<10}", row.project.status.as_str()),
                        theme::status(row.project.status),
                    ),
                    Span::styled(
                        format!("{:>9}", thousands(m.total_word_count)),
                        Style::new().fg(theme::FG),
                    ),
                    Span::styled(format!("{:>6}", m.days_written), theme::label()),
                    Span::styled(format!("{:>8.2}", m.hours_written), theme::label()),
                    Span::styled(
                        format!(
                            "{:>7}",
                            m.words_per_hour()
                                .map(|r| format!("{r:.0}"))
                                .unwrap_or_else(|| "—".into())
                        ),
                        theme::derived(),
                    ),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(block)
            .highlight_style(theme::selected_row())
            .highlight_symbol("› ");
        let mut state = ListState::default().with_selected(Some(app.projects.selected_index()));
        frame.render_stateful_widget(list, list_area, &mut state);
    }

    if app.projects.edit.is_some() {
        draw_project_edit(frame, app, lower);
    } else {
        draw_project_details(frame, app, lower);
    }
}

fn draw_project_details(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let block = Block::bordered()
        .title(" Selected ")
        .border_style(theme::inactive())
        .title_style(theme::label());
    let Some(row) = app.projects.selected_row() else {
        frame.render_widget(Paragraph::new("").block(block), area);
        return;
    };
    let m = &row.metrics;
    let pair = |label: &'static str, value: String| -> Line<'static> {
        Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("{label:<16}"), theme::label()),
            Span::styled(value, Style::new().fg(theme::FG)),
        ])
    };
    let or_dash = |v: Option<String>| v.unwrap_or_else(|| "—".to_string());
    let lines = vec![
        Line::from(vec![
            Span::raw(" "),
            Span::styled(row.project.title.clone(), theme::focused()),
            Span::raw("  "),
            Span::styled(
                row.project.status.as_str(),
                theme::status(row.project.status),
            ),
        ]),
        pair(
            "Start / End",
            format!(
                "{} → {}",
                or_dash(m.start_date.map(|d| d.to_string())),
                or_dash(m.end_date.map(|d| d.to_string())),
            ),
        ),
        pair("Total words", thousands(m.total_word_count)),
        pair(
            "Days written",
            format!("{} over {} entries", m.days_written, m.entry_count),
        ),
        pair("Hours", format!("{:.2}", m.hours_written)),
        pair(
            "Hours per day",
            or_dash(m.hours_per_day().map(|v| format!("{v:.2}"))),
        ),
        pair(
            "Words per hour",
            or_dash(m.words_per_hour().map(|v| format!("{v:.0}"))),
        ),
        Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("{:<16}", "Description"), theme::label()),
            Span::styled(
                row.project
                    .description
                    .clone()
                    .unwrap_or_else(|| "—".into()),
                theme::hint(),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_project_edit(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let edit = app.projects.edit.as_ref().expect("checked by caller");
    let editing = edit.mode == Mode::Insert;
    let value = |field: projects_screen::Field| -> Span<'_> {
        if field.is_choice() {
            let text = match field {
                projects_screen::Field::Category => app.projects.category_name(edit.category_id),
                _ => edit.status.as_str().to_string(),
            };
            return Span::styled(format!("‹ {text} ›"), theme::derived());
        }
        let raw = edit.text_of(field);
        let focused = edit.field == field;
        let shown = if focused && editing {
            format!("{raw}▏")
        } else if raw.is_empty() {
            "—".to_string()
        } else {
            raw.to_string()
        };
        let style = if raw.is_empty() && !(focused && editing) {
            theme::hint()
        } else {
            Style::new().fg(theme::FG)
        };
        Span::styled(shown, style)
    };
    let f = |field: projects_screen::Field| {
        field_line(field.label(), value(field), None, edit.field == field)
    };
    let mut lines = vec![
        f(projects_screen::Field::Title),
        f(projects_screen::Field::Category),
        f(projects_screen::Field::Status),
        f(projects_screen::Field::Description),
    ];
    if let Some(problem) = edit.problems().first() {
        lines.push(Line::from(Span::styled(
            format!("  {problem}"),
            theme::error(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  h l step a choice  ·  <leader><CR> saves  ·  Esc discards",
            theme::hint(),
        )));
    }
    let title = if edit.is_creating() {
        " New project "
    } else {
        " Edit project "
    };
    frame.render_widget(
        Paragraph::new(lines).style(theme::float()).block(
            Block::bordered()
                .title(title)
                .border_style(theme::focused())
                .title_style(theme::focused())
                .style(theme::float()),
        ),
        area,
    );
}

// ---------------------------------------------------------------- categories

fn draw_categories(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let (panes, lower) = if app.categories.edit.is_some() {
        let [panes, edit] =
            Layout::vertical([Constraint::Min(4), Constraint::Length(8)]).areas(area);
        (panes, Some(edit))
    } else {
        (area, None)
    };
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)]).areas(panes);

    let editing = app.categories.edit.is_some();
    let pane_border = |pane: Pane| {
        if !editing && app.categories.pane == pane {
            theme::focused()
        } else {
            theme::inactive()
        }
    };

    let category_items: Vec<ListItem<'_>> = app
        .categories
        .category_rows()
        .iter()
        .map(|row| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<22}", super::history::truncate(&row.category.name, 22)),
                    Style::new().fg(theme::FG),
                ),
                Span::styled(format!("{:<14}", row.type_name), theme::label()),
                Span::styled(format!("{:>3} proj", row.project_count), theme::hint()),
            ]))
        })
        .collect();
    let category_block = Block::bordered()
        .title(format!(" Categories ({}) ", category_items.len()))
        .border_style(pane_border(Pane::Categories))
        .title_style(theme::label());
    if category_items.is_empty() {
        frame.render_widget(
            Paragraph::new("None yet.\n\nPress o to create one.")
                .style(theme::hint())
                .block(category_block),
            left,
        );
    } else {
        let mut state =
            ListState::default().with_selected(Some(app.categories.selected_category_index()));
        frame.render_stateful_widget(
            List::new(category_items)
                .block(category_block)
                .highlight_style(theme::selected_row())
                .highlight_symbol("› "),
            left,
            &mut state,
        );
    }

    let type_items: Vec<ListItem<'_>> = app
        .categories
        .type_rows()
        .iter()
        .map(|row| {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{:<16}", super::history::truncate(&row.type_.name, 16)),
                    Style::new().fg(theme::FG),
                ),
                Span::styled(format!("{:>3} cat", row.category_count), theme::hint()),
            ]))
        })
        .collect();
    let type_block = Block::bordered()
        .title(format!(" Types ({}) ", type_items.len()))
        .border_style(pane_border(Pane::Types))
        .title_style(theme::label());
    if type_items.is_empty() {
        frame.render_widget(
            Paragraph::new("None yet.\n\nPress o to create one.")
                .style(theme::hint())
                .block(type_block),
            right,
        );
    } else {
        let mut state =
            ListState::default().with_selected(Some(app.categories.selected_type_index()));
        frame.render_stateful_widget(
            List::new(type_items)
                .block(type_block)
                .highlight_style(theme::selected_row())
                .highlight_symbol("› "),
            right,
            &mut state,
        );
    }

    if let Some(lower) = lower {
        draw_category_edit(frame, app, lower);
    }
}

fn draw_category_edit(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let edit = app.categories.edit.as_ref().expect("checked by caller");
    let editing = edit.mode() == Mode::Insert;

    let text_span = |raw: &str, focused: bool| -> Span<'static> {
        let shown = if focused && editing {
            format!("{raw}▏")
        } else if raw.is_empty() {
            "—".to_string()
        } else {
            raw.to_string()
        };
        let style = if raw.is_empty() && !(focused && editing) {
            theme::hint()
        } else {
            Style::new().fg(theme::FG)
        };
        Span::styled(shown, style)
    };

    let (title, mut lines) = match edit {
        categories::Edit::Category(e) => {
            let lines = vec![
                field_line(
                    CategoryField::Name.label(),
                    text_span(
                        e.text_of(CategoryField::Name),
                        e.field == CategoryField::Name,
                    ),
                    None,
                    e.field == CategoryField::Name,
                ),
                field_line(
                    CategoryField::Type.label(),
                    Span::styled(
                        format!("‹ {} ›", app.categories.type_name(e.type_id)),
                        theme::derived(),
                    ),
                    None,
                    e.field == CategoryField::Type,
                ),
                field_line(
                    CategoryField::Description.label(),
                    text_span(
                        e.text_of(CategoryField::Description),
                        e.field == CategoryField::Description,
                    ),
                    None,
                    e.field == CategoryField::Description,
                ),
            ];
            (
                if e.id.is_none() {
                    " New category "
                } else {
                    " Edit category "
                },
                lines,
            )
        }
        categories::Edit::Type(e) => {
            let lines = vec![
                field_line(
                    TypeField::Name.label(),
                    text_span(e.text_of(TypeField::Name), e.field == TypeField::Name),
                    None,
                    e.field == TypeField::Name,
                ),
                field_line(
                    TypeField::Description.label(),
                    text_span(
                        e.text_of(TypeField::Description),
                        e.field == TypeField::Description,
                    ),
                    None,
                    e.field == TypeField::Description,
                ),
            ];
            (
                if e.id.is_none() {
                    " New type "
                } else {
                    " Edit type "
                },
                lines,
            )
        }
    };
    if let Some(problem) = edit.problems().first() {
        lines.push(Line::from(Span::styled(
            format!("  {problem}"),
            theme::error(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  <leader><CR> saves  ·  Esc discards",
            theme::hint(),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines).style(theme::float()).block(
            Block::bordered()
                .title(title)
                .border_style(theme::focused())
                .title_style(theme::focused())
                .style(theme::float()),
        ),
        area,
    );
}

// -------------------------------------------------------------------- stats

/// Eighth-width blocks, so a bar is not quantised to whole characters.
const EIGHTHS: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

pub fn bar(value: i64, max: i64, width: usize) -> String {
    if max <= 0 || value <= 0 || width == 0 {
        return String::new();
    }
    let eighths = (value as f64 / max as f64 * width as f64 * 8.0).round() as usize;
    let full = (eighths / 8).min(width);
    let remainder = eighths % 8;
    let mut out = EIGHTHS[7].to_string().repeat(full);
    if full < width && remainder > 0 {
        out.push(EIGHTHS[remainder - 1]);
    }
    // A non-zero value should never render as nothing at all.
    if out.is_empty() {
        out.push(EIGHTHS[0]);
    }
    out
}

fn draw_stats(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let yoy_height = (app.stats.by_year.len() as u16 + 2).clamp(3, 9);
    let category_height = category_panel_height(&app.stats, area.width);
    // The category bar sits above the year graph: it describes the selected
    // scope, like everything above it, where the year graph spans all of them.
    let [kpi_area, projects_area, category_area, yoy_area] = Layout::vertical([
        Constraint::Length(6),
        Constraint::Min(4),
        Constraint::Length(category_height),
        Constraint::Length(yoy_height),
    ])
    .areas(area);

    draw_kpis(frame, app, kpi_area);
    draw_scope_projects(frame, app, projects_area);
    draw_category_bar(frame, app, category_area);
    draw_year_over_year(frame, app, yoy_area);
}

fn kpi_panel<'a>(
    title: String,
    totals: &crate::db::stats::Totals,
    completed: i64,
) -> Paragraph<'a> {
    let kpi = |label: &'static str, value: String| -> Line<'static> {
        Line::from(vec![
            Span::raw(" "),
            Span::styled(format!("{label:<12}"), theme::label()),
            Span::styled(value, Style::new().fg(theme::FG)),
        ])
    };
    let lines = vec![
        kpi("Words", thousands(totals.words)),
        kpi("Hours", format!("{:.2}", totals.hours)),
        kpi("Completed", completed.to_string()),
        kpi(
            "Words/Hour",
            totals
                .words_per_hour()
                .map(|r| format!("{r:.0}"))
                .unwrap_or_else(|| "—".to_string()),
        ),
    ];
    Paragraph::new(lines).block(
        Block::bordered()
            .title(title)
            .border_style(theme::inactive())
            .title_style(theme::focused()),
    )
}

fn draw_kpis(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let stats = &app.stats;
    // In the all-time view the selected scope *is* lifetime, so one panel holds
    // everything rather than printing the same four numbers twice.
    if stats.is_lifetime() {
        frame.render_widget(
            kpi_panel(
                " All time ".to_string(),
                &stats.lifetime,
                stats.lifetime_completed,
            ),
            area,
        );
        return;
    }
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);
    frame.render_widget(
        kpi_panel(
            " Lifetime ".to_string(),
            &stats.lifetime,
            stats.lifetime_completed,
        ),
        left,
    );
    let title = if stats.is_current_year() {
        format!(" {} (this year) ", stats.scope.label())
    } else {
        format!(" {} ", stats.scope.label())
    };
    frame.render_widget(
        kpi_panel(title, &stats.selected, stats.selected_completed),
        right,
    );
}

/// Titles are cut to ten characters, as the requirements ask.
const GRAPH_LABEL: usize = 10;

fn draw_scope_projects(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let visible = (area.height as usize).saturating_sub(2).max(1);
    // Remembered so h and l scroll by what is actually on screen.
    app.project_graph_rows = visible;

    let stats = &app.stats;
    let total = stats.selected_projects.len();
    let offset = stats.project_offset().min(total.saturating_sub(1));
    let shown = total.min(offset + visible);

    let title = if total > visible {
        format!(
            " {} projects ({}–{} of {}) ",
            stats.scope.label(),
            offset + 1,
            shown,
            total
        )
    } else {
        format!(" {} projects ({}) ", stats.scope.label(), total)
    };
    let block = Block::bordered()
        .title(title)
        .border_style(theme::inactive())
        .title_style(theme::label());

    if stats.selected_projects.is_empty() {
        frame.render_widget(
            Paragraph::new(format!("Nothing written in {}.", stats.scope.label()))
                .style(theme::hint())
                .block(block),
            area,
        );
        return;
    }

    let words_column = 10;
    let bar_width = (area.width as usize).saturating_sub(GRAPH_LABEL + words_column + 5);
    // Scaled against the largest in the whole list, not the window, so scrolling
    // does not silently rescale the bars under the writer.
    let max = stats
        .selected_projects
        .iter()
        .map(|p| p.words)
        .max()
        .unwrap_or(1);

    let lines: Vec<Line<'_>> = stats
        .selected_projects
        .iter()
        .skip(offset)
        .take(visible)
        .map(|project| {
            Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    format!(
                        "{:<GRAPH_LABEL$}",
                        super::history::truncate(&project.title, GRAPH_LABEL)
                    ),
                    Style::new().fg(theme::FG),
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{:<bar_width$}", bar(project.words, max, bar_width)),
                    theme::status(project.status),
                ),
                Span::styled(
                    format!("{:>words_column$}", thousands(project.words)),
                    theme::label(),
                ),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_year_over_year(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let stats = &app.stats;
    let block = Block::bordered()
        .title(" Year over year ")
        .border_style(theme::inactive())
        .title_style(theme::label());
    if stats.by_year.is_empty() {
        frame.render_widget(
            Paragraph::new("No entries yet.")
                .style(theme::hint())
                .block(block),
            area,
        );
        return;
    }
    let words_column = 10;
    let bar_width = (area.width as usize).saturating_sub(4 + 1 + words_column + 5);
    let max = stats.by_year.iter().map(|(_, w)| *w).max().unwrap_or(1);

    let lines: Vec<Line<'_>> = stats
        .by_year
        .iter()
        .map(|(year, words)| {
            // In the all-time view no single year is selected, so none is lit.
            let selected = stats.selected_year() == Some(*year);
            Line::from(vec![
                Span::raw(" "),
                Span::styled(
                    format!("{year:<4}"),
                    if selected {
                        theme::focused()
                    } else {
                        theme::label()
                    },
                ),
                Span::raw(" "),
                Span::styled(
                    format!("{:<bar_width$}", bar(*words, max, bar_width)),
                    if selected {
                        theme::derived()
                    } else {
                        theme::inactive()
                    },
                ),
                Span::styled(
                    format!("{:>words_column$}", thousands(*words)),
                    theme::label(),
                ),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// One legend entry's text, without its colour swatch.
fn legend_text(name: &str, share: f64, words: i64) -> String {
    format!(" {name} {:.1}% ({})", share * 100.0, thousands(words))
}

/// Pack legend entries into lines that fit `width`, returning the indices on each.
///
/// Shared by the height calculation and the drawing, so the panel can never be
/// one line too short for its own legend.
fn legend_rows(shares: &[(String, f64, i64)], width: usize) -> Vec<Vec<usize>> {
    let mut rows: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut used = 1usize; // the leading gutter
    for (index, (name, share, words)) in shares.iter().enumerate() {
        let entry = 1 + legend_text(name, *share, *words).chars().count();
        let needed = entry + if current.is_empty() { 0 } else { 2 };
        if !current.is_empty() && used + needed > width {
            rows.push(std::mem::take(&mut current));
            used = 1 + entry;
        } else {
            used += needed;
        }
        current.push(index);
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

/// Two rows of blocks, so the colours read as bands rather than a thin line.
const CATEGORY_BAR_ROWS: u16 = 2;

/// Height the category panel needs: borders, its legend, the bar, and any note.
fn category_panel_height(stats: &super::stats::Stats, width: u16) -> u16 {
    let shares = stats.category_shares();
    if shares.is_empty() {
        return 3;
    }
    let rows = legend_rows(&shares, width.saturating_sub(2) as usize).len() as u16;
    let note = u16::from(!stats.categories_without_a_share().is_empty());
    (2 + CATEGORY_BAR_ROWS + rows + note).clamp(5, 10)
}

/// The category split, drawn as one bar divided by share.
///
/// The key sits above the bar: it is read first, and a bar with its legend
/// underneath puts the explanation after the thing needing explaining.
fn draw_category_bar(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let stats = &app.stats;
    let block = Block::bordered()
        .title(format!(" {} by category ", stats.scope.label()))
        .border_style(theme::inactive())
        .title_style(theme::label());

    let shares = stats.category_shares();
    if shares.is_empty() {
        frame.render_widget(
            Paragraph::new("Nothing to divide up.")
                .style(theme::hint())
                .block(block),
            area,
        );
        return;
    }

    let width = (area.width as usize).saturating_sub(2);
    // Cells go out largest-remainder first, so the segments add up to the full
    // width instead of leaving a gap to rounding.
    let mut cells: Vec<usize> = shares
        .iter()
        .map(|(_, share, _)| (share * width as f64).floor() as usize)
        .collect();
    let mut leftover = width.saturating_sub(cells.iter().sum::<usize>());
    let mut order: Vec<usize> = (0..shares.len()).collect();
    order.sort_by(|a, b| {
        let fa = shares[*a].1 * width as f64;
        let fb = shares[*b].1 * width as f64;
        (fb - fb.floor())
            .partial_cmp(&(fa - fa.floor()))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for index in order {
        if leftover == 0 {
            break;
        }
        cells[index] += 1;
        leftover -= 1;
    }

    let colour = |index: usize| Style::new().fg(theme::SERIES[index % theme::SERIES.len()]);

    let mut lines: Vec<Line<'_>> = Vec::new();
    for row in legend_rows(&shares, width) {
        let mut spans = vec![Span::raw(" ")];
        for (position, index) in row.iter().enumerate() {
            if position > 0 {
                spans.push(Span::raw("  "));
            }
            let (name, share, words) = &shares[*index];
            spans.push(Span::styled(EIGHTHS[7].to_string(), colour(*index)));
            spans.push(Span::styled(
                legend_text(name, *share, *words),
                theme::label(),
            ));
        }
        lines.push(Line::from(spans));
    }

    let bar_spans: Vec<Span<'_>> = shares
        .iter()
        .enumerate()
        .filter(|(index, _)| cells[*index] > 0)
        .map(|(index, _)| Span::styled(EIGHTHS[7].to_string().repeat(cells[index]), colour(index)))
        .collect();
    for _ in 0..CATEGORY_BAR_ROWS {
        lines.push(Line::from(bar_spans.clone()));
    }

    let ignored = stats.categories_without_a_share();
    if !ignored.is_empty() {
        let names: Vec<String> = ignored
            .iter()
            .map(|c| format!("{} ({})", c.category, thousands(c.words)))
            .collect();
        lines.push(Line::from(Span::styled(
            format!(" no share: {}", names.join(", ")),
            theme::warning(),
        )));
    }
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// The project picker, shown beside the edit panel while the project is in hand.
fn draw_edit_project_picker(frame: &mut Frame<'_>, app: &mut App, area: Rect) {
    let filter = app
        .history
        .edit
        .as_ref()
        .map(|e| e.filter.clone())
        .unwrap_or_default();
    let selected = app.history.edit_choice_index();
    let choices = app.history.edit_choices();

    let title = if filter.is_empty() {
        format!(" Move to ({}) ", choices.len())
    } else {
        format!(" Move to ({}) /{filter} ", choices.len())
    };
    let block = Block::bordered()
        .title(title)
        .border_style(theme::focused())
        .title_style(theme::label())
        .style(theme::float());

    if choices.is_empty() {
        frame.render_widget(
            Paragraph::new("Nothing matches.\n\nEsc stops filtering.")
                .style(theme::hint())
                .block(block),
            area,
        );
        return;
    }

    let items: Vec<ListItem<'_>> = choices
        .iter()
        .map(|choice| {
            ListItem::new(Line::from(Span::styled(
                choice.title.clone(),
                theme::status(choice.status),
            )))
        })
        .collect();
    let mut state = ListState::default().with_selected(selected);
    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(theme::selected_row())
            .highlight_symbol("\u{203a} "),
        area,
        &mut state,
    );
}

fn draw_status(frame: &mut Frame<'_>, app: &App, area: Rect) {
    // The dashboard is read-only, so a mode indicator there says nothing and
    // costs room on the one line that has to fit.
    let mut spans = if app.screen == Screen::Stats {
        vec![Span::raw(" ")]
    } else {
        vec![
            Span::styled(format!(" {} ", mode_label(app)), theme::selected_row()),
            Span::raw(" "),
        ]
    };

    let prompt = if app.history.confirming_delete {
        app.history.selected_row().map(|row| {
            format!(
                "delete {} words on {} ({})?",
                thousands(row.entry.session_word_count),
                row.project_title,
                row.entry.entry_date
            )
        })
    } else if app.projects.confirming_delete {
        app.projects
            .selected_row()
            .map(|row| format!("delete the project {}?", row.project.title))
    } else if app.categories.confirming_delete {
        match app.categories.pane {
            Pane::Categories => app
                .categories
                .selected_category()
                .map(|row| format!("delete the category {}?", row.category.name)),
            Pane::Types => app
                .categories
                .selected_type()
                .map(|row| format!("delete the type {}?", row.type_.name)),
        }
    } else {
        None
    };
    if let Some(prompt) = prompt {
        spans.push(Span::styled(
            format!("{prompt}  y deletes, anything else keeps it"),
            theme::error(),
        ));
        frame.render_widget(Paragraph::new(Line::from(spans)).style(theme::base()), area);
        return;
    }

    if let Some(flash) = &app.flash {
        let style = match flash.tone {
            Tone::Info => theme::label(),
            Tone::Success => theme::success(),
            Tone::Warning => theme::warning(),
            Tone::Error => theme::error(),
        };
        spans.push(Span::styled(flash.text.clone(), style));
    } else {
        let (where_am_i, action) = match app.screen {
            Screen::History => match app.history.edit.as_ref() {
                Some(edit) => (edit.field.label(), "<leader><CR> save"),
                None => ("browsing", "i edits  ·  dd deletes"),
            },
            Screen::Projects => match app.projects.edit.as_ref() {
                Some(edit) => (edit.field.label(), "<leader><CR> save"),
                None => ("browsing", "x cycles status  ·  i edits  ·  o new"),
            },
            Screen::Categories => match app.categories.edit.as_ref() {
                Some(edit) => (edit.field_label(), "<leader><CR> save"),
                None => (
                    app.categories.pane.title(),
                    "Tab switches pane  ·  i edits  ·  o new",
                ),
            },
            Screen::Stats => (
                if app.stats.is_lifetime() {
                    "all time"
                } else {
                    "by year"
                },
                "j k year  ·  h l scroll  ·  0 now  ·  $ all time",
            ),
            _ => (app.form.field.label(), "<leader><CR> submit"),
        };
        spans.push(Span::styled(where_am_i, theme::label()));
        spans.push(Span::styled("  ·  ", theme::inactive()));
        spans.push(Span::styled(action, theme::hint()));
        spans.push(Span::styled("  ·  ", theme::inactive()));
        spans.push(Span::styled("? help", theme::hint()));
        spans.push(Span::styled("  ·  ", theme::inactive()));
        spans.push(Span::styled("q quit", theme::hint()));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme::base()), area);
}

fn draw_help(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let sections = help::sections(app.screen);
    let key_width = help::widest_key(&sections).max(8);

    let mut lines: Vec<Line<'_>> = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        if index > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(section.title, theme::focused())));
        for (key, description) in section.bindings {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{key:<key_width$}"), Style::new().fg(theme::YELLOW)),
                Span::raw("  "),
                Span::styled(*description, Style::new().fg(theme::FG)),
            ]));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "No binding uses a control key.",
        theme::hint(),
    )));

    let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
    // Sized to its content: a help popup that truncates the binding it is
    // explaining is worse than no popup.
    let widest_line = lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|s| s.content.chars().count())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(0) as u16;
    let width = (widest_line + 4).min(area.width.saturating_sub(2)).max(24);
    let [popup] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(area);
    let [popup] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).style(theme::float()).block(
            Block::bordered()
                .title(" Keys ")
                .border_style(theme::focused())
                .title_style(theme::focused())
                .style(theme::float()),
        ),
        popup,
    );
}

/// 17228 reads as 17,228. Word counts are compared at a glance all day.
pub fn thousands(n: i64) -> String {
    let negative = n < 0;
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    if negative { format!("-{out}") } else { out }
}
