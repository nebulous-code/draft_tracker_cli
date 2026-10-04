//! Entry form behaviour, driven through the same key handler the terminal uses.

use chrono::{Datelike, NaiveDate};
use draft_tracker::db::models::{NewEntry, Status};
use draft_tracker::db::{Db, repo};
use draft_tracker::tui::app::{App, Flow, Screen, Tone};
use draft_tracker::tui::entry::{Field, Mode, Problem};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: ratatui::crossterm::event::KeyEventState::NONE,
    }
}

fn ch(c: char) -> KeyEvent {
    key(KeyCode::Char(c))
}

fn type_str(app: &mut App, text: &str) {
    for c in text.chars() {
        app.on_key(ch(c)).unwrap();
    }
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Three projects: one with history, one brand new, one completed.
fn app_with_projects() -> App {
    let db = Db::open_in_memory().unwrap();
    {
        let conn = db.conn();
        let fiction = repo::ensure_type(conn, "Fiction").unwrap();
        let book = repo::ensure_category(conn, "SitR Book", fiction).unwrap();

        let fireside =
            repo::insert_project(conn, "Fireside 1", Some(book), Status::Open, None).unwrap();
        repo::insert_entry(
            conn,
            &NewEntry {
                entry_date: day(2026, 1, 12),
                project_id: fireside,
                session_word_count: 15_429,
                hours: 22.75,
                notes: None,
            },
        )
        .unwrap();

        repo::insert_project(conn, "Brand New", Some(book), Status::Open, None).unwrap();
        repo::insert_project(conn, "Shipped", Some(book), Status::Completed, None).unwrap();
    }
    App::new(db).unwrap()
}

/// Move the cursor to a field by tabbing, which is what a writer does.
fn focus(app: &mut App, field: Field) {
    for _ in 0..Field::ORDER.len() {
        if app.form.field == field {
            return;
        }
        app.on_key(key(KeyCode::Tab)).unwrap();
    }
    panic!("could not focus {field:?}");
}

fn select_title(app: &mut App, title: &str) {
    focus(app, Field::Project);
    for _ in 0..app.form.visible().len() {
        if app.form.selected_project().map(|c| c.title.as_str()) == Some(title) {
            return;
        }
        app.on_key(ch('l')).unwrap();
    }
    panic!("{title} not in the picker");
}

// ------------------------------------------------------------ the picker

#[test]
fn the_picker_opens_on_open_projects_only() {
    let app = app_with_projects();
    let titles: Vec<&str> = app
        .form
        .visible()
        .iter()
        .map(|c| c.title.as_str())
        .collect();
    assert_eq!(titles, vec!["Brand New", "Fireside 1"]);
}

#[test]
fn capital_a_widens_the_picker_to_every_project() {
    let mut app = app_with_projects();
    app.on_key(ch('A')).unwrap();
    assert_eq!(app.form.visible().len(), 3);
    assert!(app.form.include_closed);
    app.on_key(ch('A')).unwrap();
    assert_eq!(app.form.visible().len(), 2);
}

#[test]
fn widening_the_picker_keeps_the_cursor_on_the_same_project() {
    let mut app = app_with_projects();
    select_title(&mut app, "Fireside 1");
    app.on_key(ch('A')).unwrap();
    assert_eq!(app.form.selected_project().unwrap().title, "Fireside 1");
}

#[test]
fn the_filter_narrows_the_picker_and_escape_clears_it() {
    let mut app = app_with_projects();
    app.on_key(ch('/')).unwrap();
    assert_eq!(app.form.mode, Mode::Filter);
    type_str(&mut app, "fire");
    assert_eq!(app.form.visible().len(), 1);
    assert_eq!(app.form.selected_project().unwrap().title, "Fireside 1");

    app.on_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.form.mode, Mode::Normal);
    app.on_key(key(KeyCode::Esc)).unwrap();
    assert!(app.form.filter.is_empty());
    assert_eq!(app.form.visible().len(), 2);
}

/// j and k walk the form like Tab. Reaching past the focused field to move the
/// project list was the first thing that went wrong in real use.
#[test]
fn j_and_k_walk_the_fields_and_never_touch_the_project_list() {
    let mut app = app_with_projects();
    app.form.field = Field::Date;
    app.on_key(ch('j')).unwrap();
    assert_eq!(app.form.field, Field::Project);
    app.on_key(ch('j')).unwrap();
    assert_eq!(app.form.field, Field::Hours);
    app.on_key(ch('k')).unwrap();
    assert_eq!(app.form.field, Field::Project);

    // Standing on Hours, j and k must not move the project underneath.
    focus(&mut app, Field::Hours);
    let before = app.form.selected_project().map(|c| c.title.clone());
    app.on_key(ch('j')).unwrap();
    app.on_key(ch('k')).unwrap();
    assert_eq!(app.form.selected_project().map(|c| c.title.clone()), before);
}

#[test]
fn h_and_l_step_the_project_and_gg_and_capital_g_jump() {
    let mut app = app_with_projects();
    focus(&mut app, Field::Project);
    assert_eq!(app.form.selected_index(), 0);
    app.on_key(ch('h')).unwrap();
    assert_eq!(
        app.form.selected_index(),
        1,
        "h from the top wraps to the bottom"
    );
    app.on_key(ch('l')).unwrap();
    assert_eq!(app.form.selected_index(), 0);
    app.on_key(ch('G')).unwrap();
    assert_eq!(app.form.selected_index(), 1);
    app.on_key(ch('g')).unwrap();
    app.on_key(ch('g')).unwrap();
    assert_eq!(app.form.selected_index(), 0);
}

#[test]
fn stepping_the_project_from_another_field_says_where_the_keys_apply() {
    let mut app = app_with_projects();
    focus(&mut app, Field::Hours);
    app.on_key(ch('l')).unwrap();
    let flash = app.flash.as_ref().expect("a reason was shown");
    assert!(
        flash.text.contains("h and l step the project"),
        "got {:?}",
        flash.text
    );
}

#[test]
fn the_project_field_is_reached_by_tab_and_entered_with_i() {
    let mut app = app_with_projects();
    app.form.field = Field::Date;
    app.on_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.form.field, Field::Project);
    // i on a choice field means "narrow it", which is the useful analogue of
    // typing when there are dozens of projects.
    app.on_key(ch('i')).unwrap();
    assert_eq!(app.form.mode, Mode::Filter);
    type_str(&mut app, "fire");
    assert_eq!(app.form.selected_project().unwrap().title, "Fireside 1");
}

// ------------------------------------------------------------ fields

#[test]
fn tab_cycles_forward_and_shift_tab_back() {
    let mut app = app_with_projects();
    app.form.field = Field::Date;
    app.on_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.form.field, Field::Project);
    app.on_key(key(KeyCode::BackTab)).unwrap();
    assert_eq!(app.form.field, Field::Date);
    app.on_key(key(KeyCode::BackTab)).unwrap();
    assert_eq!(app.form.field, Field::Notes, "cycles round");
}

#[test]
fn typing_only_reaches_a_field_in_insert_mode() {
    let mut app = app_with_projects();
    focus(&mut app, Field::Hours);
    type_str(&mut app, "1.5");
    assert_eq!(
        app.form.hours, "",
        "normal mode keys are commands, not text"
    );

    app.on_key(ch('i')).unwrap();
    assert_eq!(app.form.mode, Mode::Insert);
    type_str(&mut app, "1.5");
    assert_eq!(app.form.hours, "1.5");
    app.on_key(key(KeyCode::Backspace)).unwrap();
    assert_eq!(app.form.hours, "1.");
    app.on_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.form.mode, Mode::Normal);
}

#[test]
fn the_date_defaults_to_today() {
    let app = app_with_projects();
    assert_eq!(app.form.date, chrono::Local::now().date_naive().to_string());
}

// ------------------------------------------------------------ the preview

#[test]
fn a_typed_total_is_measured_against_what_is_already_logged() {
    let mut app = app_with_projects();
    select_title(&mut app, "Fireside 1");
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "15682");
    let preview = app.form.preview();
    assert!(!preview.first_entry);
    assert_eq!(preview.session_words, Some(253));
}

#[test]
fn a_projects_first_entry_takes_the_typed_number_as_the_session() {
    let mut app = app_with_projects();
    select_title(&mut app, "Brand New");
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "1200");
    let preview = app.form.preview();
    assert!(
        preview.first_entry,
        "nothing logged yet, so there is no total to subtract"
    );
    assert_eq!(preview.session_words, Some(1_200));
}

#[test]
fn words_per_hour_normalises_a_short_session() {
    let mut app = app_with_projects();
    select_title(&mut app, "Brand New");
    focus(&mut app, Field::Hours);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "0.25");
    app.on_key(key(KeyCode::Esc)).unwrap();
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "100");
    // 100 words in a quarter hour is 400 an hour.
    assert_eq!(app.form.preview().words_per_hour, Some(400.0));
}

#[test]
fn a_revision_day_previews_a_negative_session_and_a_negative_rate() {
    let mut app = app_with_projects();
    select_title(&mut app, "Fireside 1");
    focus(&mut app, Field::Hours);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "1");
    app.on_key(key(KeyCode::Esc)).unwrap();
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "14099");
    let preview = app.form.preview();
    assert_eq!(preview.session_words, Some(-1_330));
    assert_eq!(preview.words_per_hour, Some(-1_330.0));
}

#[test]
fn a_thousands_separator_in_a_typed_count_is_accepted() {
    let mut app = app_with_projects();
    select_title(&mut app, "Brand New");
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "1,200");
    assert_eq!(app.form.preview().session_words, Some(1_200));
}

// ------------------------------------------------------------ validation

#[test]
fn the_hours_minimum_is_refused_with_a_reason() {
    let mut app = app_with_projects();
    select_title(&mut app, "Brand New");
    focus(&mut app, Field::Hours);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "0");
    app.on_key(key(KeyCode::Esc)).unwrap();
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "500");
    app.on_key(key(KeyCode::Esc)).unwrap();

    assert!(
        app.form
            .problems()
            .contains(&Problem::HoursTooShort("0".into()))
    );
    // Submit is blocked and says why, rather than doing nothing.
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();
    let flash = app.flash.as_ref().expect("a reason was shown");
    assert_eq!(flash.tone, Tone::Error);
    assert!(flash.text.contains("minimum"), "got {:?}", flash.text);
    assert_eq!(
        repo::recent_entries(app.db.conn(), 10).unwrap().len(),
        1,
        "nothing written"
    );
}

#[test]
fn an_unparseable_field_is_named() {
    let mut app = app_with_projects();
    select_title(&mut app, "Brand New");
    focus(&mut app, Field::Date);
    app.on_key(ch('i')).unwrap();
    for _ in 0..12 {
        app.on_key(key(KeyCode::Backspace)).unwrap();
    }
    type_str(&mut app, "the 3rd");
    app.on_key(key(KeyCode::Esc)).unwrap();
    assert!(
        app.form
            .problems()
            .contains(&Problem::BadDate("the 3rd".into()))
    );
}

// ------------------------------------------------------------ submitting

#[test]
fn leader_enter_writes_the_entry_and_clears_the_session_fields() {
    let mut app = app_with_projects();
    select_title(&mut app, "Fireside 1");
    focus(&mut app, Field::Hours);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "1");
    app.on_key(key(KeyCode::Esc)).unwrap();
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "15682");
    app.on_key(key(KeyCode::Esc)).unwrap();
    focus(&mut app, Field::Notes);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "a good morning");
    app.on_key(key(KeyCode::Esc)).unwrap();

    let date = app.form.date.clone();
    app.on_key(ch(' ')).unwrap();
    assert!(app.leader_pending);
    app.on_key(key(KeyCode::Enter)).unwrap();

    let flash = app.flash.as_ref().expect("confirmation shown");
    assert_eq!(flash.tone, Tone::Success);
    assert!(flash.text.contains("253"), "got {:?}", flash.text);

    let entries = repo::recent_entries(app.db.conn(), 10).unwrap();
    assert_eq!(entries.len(), 2);
    let newest = entries
        .iter()
        .find(|e| e.session_word_count == 253)
        .expect("the new entry");
    assert_eq!(newest.hours, 1.0);
    assert_eq!(newest.notes.as_deref(), Some("a good morning"));

    // The session fields clear; the date stays, because another entry the same
    // day is the common next action.
    assert_eq!(app.form.hours, "");
    assert_eq!(app.form.project_words, "");
    assert_eq!(app.form.notes, "");
    assert_eq!(app.form.date, date);
}

#[test]
fn the_running_total_updates_so_the_next_entry_measures_correctly() {
    let mut app = app_with_projects();
    select_title(&mut app, "Fireside 1");
    assert_eq!(app.form.selected_project().unwrap().logged_words, 15_429);

    focus(&mut app, Field::Hours);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "1");
    app.on_key(key(KeyCode::Esc)).unwrap();
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "15682");
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();

    select_title(&mut app, "Fireside 1");
    assert_eq!(app.form.selected_project().unwrap().logged_words, 15_682);
}

#[test]
fn a_brand_new_project_stops_being_a_first_entry_after_one_submit() {
    let mut app = app_with_projects();
    select_title(&mut app, "Brand New");
    assert!(app.form.selected_project().unwrap().is_first_entry());

    focus(&mut app, Field::Hours);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "2");
    app.on_key(key(KeyCode::Esc)).unwrap();
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "900");
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();

    select_title(&mut app, "Brand New");
    let choice = app.form.selected_project().unwrap();
    assert!(!choice.is_first_entry());
    assert_eq!(choice.logged_words, 900);
}

// ------------------------------------------------------------ chrome

#[test]
fn question_mark_opens_the_help_and_any_key_closes_it() {
    let mut app = app_with_projects();
    app.on_key(ch('?')).unwrap();
    assert!(app.help_visible);
    app.on_key(ch('j')).unwrap();
    assert!(!app.help_visible);
    assert_eq!(app.form.selected_index(), 0, "the closing key is swallowed");
}

#[test]
fn quitting_is_immediate_when_nothing_has_been_typed() {
    let mut app = app_with_projects();
    assert_eq!(app.on_key(ch('q')).unwrap(), Flow::Quit);
}

#[test]
fn quitting_with_an_unsaved_entry_asks_first() {
    let mut app = app_with_projects();
    focus(&mut app, Field::Hours);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "1.5");
    app.on_key(key(KeyCode::Esc)).unwrap();

    assert_eq!(app.on_key(ch('q')).unwrap(), Flow::Continue);
    assert!(app.confirming_quit);
    assert_eq!(
        app.on_key(ch('n')).unwrap(),
        Flow::Continue,
        "anything else stays"
    );
    assert!(!app.confirming_quit);

    app.on_key(ch('q')).unwrap();
    assert_eq!(app.on_key(ch('q')).unwrap(), Flow::Quit);
}

#[test]
fn leader_keys_reach_every_screen() {
    let mut app = app_with_projects();
    for (leader_key, expected) in [
        ('h', Screen::History),
        ('p', Screen::Projects),
        ('c', Screen::Categories),
        ('s', Screen::Stats),
        ('e', Screen::Entry),
    ] {
        app.on_key(ch(' ')).unwrap();
        app.on_key(ch(leader_key)).unwrap();
        assert_eq!(
            app.screen, expected,
            "<leader>{leader_key} should reach {expected:?}"
        );
    }
}

#[test]
fn an_unbound_leader_key_says_so() {
    let mut app = app_with_projects();
    app.on_key(ch(' ')).unwrap();
    app.on_key(ch('z')).unwrap();
    assert_eq!(app.screen, Screen::Entry);
    assert!(app.flash.as_ref().unwrap().text.contains("not bound"));
}

#[test]
fn a_key_release_is_ignored_so_input_is_not_doubled() {
    let mut app = app_with_projects();
    focus(&mut app, Field::Hours);
    app.on_key(ch('i')).unwrap();
    let mut release = ch('7');
    release.kind = KeyEventKind::Release;
    app.on_key(release).unwrap();
    assert_eq!(app.form.hours, "");
}

// ------------------------------------------------------------ drawing

/// Render into a fixed-size buffer and return it as lines of text.
fn render(app: &mut App, width: u16, height: u16) -> Vec<String> {
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| draft_tracker::tui::draw::draw(frame, app))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

#[test]
fn the_entry_form_shows_the_figures_it_worked_out() {
    let mut app = app_with_projects();
    select_title(&mut app, "Fireside 1");
    focus(&mut app, Field::Hours);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "1");
    app.on_key(key(KeyCode::Esc)).unwrap();
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "15682");
    app.on_key(key(KeyCode::Esc)).unwrap();

    let screen = render(&mut app, 86, 22).join("\n");
    for expected in [
        "Draft Tracker",
        "Fireside 1",
        "Session WC",
        "253",
        "Words/Hour",
        // The reference total is on screen, so a surprising session can be
        // reasoned about without leaving the form.
        "logged so far 15,429",
        "<leader><CR> submit",
    ] {
        assert!(
            screen.contains(expected),
            "{expected:?} missing from:\n{screen}"
        );
    }
}

#[test]
fn a_first_entry_is_labelled_on_screen_rather_than_only_in_the_arithmetic() {
    let mut app = app_with_projects();
    select_title(&mut app, "Brand New");
    let screen = render(&mut app, 86, 22).join("\n");
    assert!(screen.contains("first entry"), "got:\n{screen}");
    assert!(
        screen.contains("type the words written today"),
        "the one place the field means something different must say so:\n{screen}"
    );
    assert!(
        screen.contains("not the manuscript total"),
        "got:\n{screen}"
    );
}

#[test]
fn a_negative_session_is_spelled_out_on_screen() {
    let mut app = app_with_projects();
    select_title(&mut app, "Fireside 1");
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "14099");
    let screen = render(&mut app, 86, 22).join("\n");
    assert!(screen.contains("-1,330"), "got:\n{screen}");
    assert!(screen.contains("cut 1,330 words"), "got:\n{screen}");
}

#[test]
fn the_help_overlay_lists_the_bindings() {
    let mut app = app_with_projects();
    app.on_key(ch('?')).unwrap();
    let screen = render(&mut app, 86, 26).join("\n");
    assert!(screen.contains("Keys"));
    assert!(screen.contains("<leader><CR>"));
    assert!(screen.contains("No binding uses a control key."));
}

#[test]
fn a_terminal_too_small_to_draw_says_so_instead_of_drawing_nonsense() {
    let mut app = app_with_projects();
    let screen = render(&mut app, 40, 10).join("\n");
    assert!(screen.contains("too small"), "got:\n{screen}");
    assert!(
        screen.contains("56x16"),
        "the required size must survive wrapping:\n{screen}"
    );
}

/// Not an assertion: prints the frame so the layout can be read by eye.
/// Run with `cargo test --test tui -- --nocapture show_the_entry_form`.
#[test]
fn show_the_first_entry_form() {
    let mut app = app_with_projects();
    select_title(&mut app, "Brand New");
    focus(&mut app, Field::Hours);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "0.5");
    app.on_key(key(KeyCode::Esc)).unwrap();
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "640");
    app.on_key(key(KeyCode::Esc)).unwrap();
    for line in render(&mut app, 86, 20) {
        println!("|{line}");
    }
}

#[test]
fn show_the_entry_form() {
    let mut app = app_with_projects();
    select_title(&mut app, "Fireside 1");
    let _ = &app;
    focus(&mut app, Field::Hours);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "1");
    app.on_key(key(KeyCode::Esc)).unwrap();
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "15682");
    app.on_key(key(KeyCode::Esc)).unwrap();
    focus(&mut app, Field::Project);
    for line in render(&mut app, 86, 20) {
        println!("|{line}");
    }
}

// ============================================================ entry history

use draft_tracker::tui::history::{EditField, EditProblem};

/// Two projects with a few sessions each, newest last in insertion order.
fn app_with_history() -> App {
    let db = Db::open_in_memory().unwrap();
    {
        let conn = db.conn();
        let fiction = repo::ensure_type(conn, "Fiction").unwrap();
        let book = repo::ensure_category(conn, "SitR Book", fiction).unwrap();
        let novel =
            repo::insert_project(conn, "Fireside 1", Some(book), Status::Open, None).unwrap();
        let story =
            repo::insert_project(conn, "Glowstones", Some(book), Status::Open, None).unwrap();

        let add = |project_id, d: (i32, u32, u32), words, hours, note: Option<&str>| {
            repo::insert_entry(
                conn,
                &NewEntry {
                    entry_date: day(d.0, d.1, d.2),
                    project_id,
                    session_word_count: words,
                    hours,
                    notes: note.map(str::to_string),
                },
            )
            .unwrap()
        };
        add(novel, (2026, 1, 12), 253, 1.0, Some("a slow start"));
        add(story, (2026, 1, 13), 880, 1.5, None);
        add(novel, (2026, 1, 14), 20_000, 1.0, Some("meant 2,000"));
    }
    App::new(db).unwrap()
}

fn to_history(app: &mut App) {
    app.on_key(ch(' ')).unwrap();
    app.on_key(ch('h')).unwrap();
    assert_eq!(app.screen, Screen::History);
}

fn select_entry_with_words(app: &mut App, words: i64) {
    for _ in 0..app.history.visible().len() {
        if app
            .history
            .selected_row()
            .map(|r| r.entry.session_word_count)
            == Some(words)
        {
            return;
        }
        app.on_key(ch('j')).unwrap();
    }
    panic!("no entry of {words} words");
}

#[test]
fn the_history_lists_newest_first() {
    let mut app = app_with_history();
    to_history(&mut app);
    let dates: Vec<String> = app
        .history
        .visible()
        .iter()
        .map(|r| r.entry.entry_date.to_string())
        .collect();
    assert_eq!(dates, vec!["2026-01-14", "2026-01-13", "2026-01-12"]);
}

#[test]
fn the_history_filters_by_project() {
    let mut app = app_with_history();
    to_history(&mut app);
    app.on_key(ch('/')).unwrap();
    type_str(&mut app, "glow");
    assert_eq!(app.history.visible().len(), 1);
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.history.visible().len(), 3);
}

#[test]
fn the_edit_panel_prefills_the_session_count_not_a_running_total() {
    let mut app = app_with_history();
    to_history(&mut app);
    select_entry_with_words(&mut app, 253);
    app.on_key(ch('i')).unwrap();
    let edit = app.history.edit.as_ref().expect("panel open");
    // 253 is the session; the project's running total is 20,253. Editing the
    // total here would be ambiguous, so the row's own figure is what is shown.
    assert_eq!(edit.session_words, "253");
    assert_eq!(edit.date, "2026-01-12");
    assert_eq!(edit.hours, "1");
    assert_eq!(edit.notes, "a slow start");
    assert_eq!(
        edit.field,
        EditField::SessionWords,
        "the field most likely to need fixing"
    );
}

/// The case the screen exists for.
#[test]
fn a_mistyped_word_count_can_be_corrected() {
    let mut app = app_with_history();
    let project_id = {
        let rows = repo::recent_entries_with_project(app.db.conn(), None).unwrap();
        rows.iter()
            .find(|r| r.entry.session_word_count == 20_000)
            .unwrap()
            .entry
            .project_id
    };
    assert_eq!(
        repo::project_logged_total(app.db.conn(), project_id).unwrap(),
        20_253
    );

    to_history(&mut app);
    select_entry_with_words(&mut app, 20_000);
    app.on_key(ch('i')).unwrap();
    app.on_key(ch('i')).unwrap(); // into insert on Session WC
    for _ in 0..6 {
        app.on_key(key(KeyCode::Backspace)).unwrap();
    }
    type_str(&mut app, "2000");
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();

    assert!(app.history.edit.is_none(), "the panel closes on save");
    assert_eq!(app.flash.as_ref().unwrap().tone, Tone::Success);
    assert_eq!(
        repo::project_logged_total(app.db.conn(), project_id).unwrap(),
        2_253
    );
    // And the entry form now measures against the corrected total.
    app.on_key(ch(' ')).unwrap();
    app.on_key(ch('e')).unwrap();
    select_title(&mut app, "Fireside 1");
    assert_eq!(app.form.selected_project().unwrap().logged_words, 2_253);
}

#[test]
fn an_edit_below_the_hours_minimum_is_refused_with_a_reason() {
    let mut app = app_with_history();
    to_history(&mut app);
    select_entry_with_words(&mut app, 253);
    app.on_key(ch('i')).unwrap();
    for _ in 0..EditField::ORDER.len() {
        if app.history.edit.as_ref().unwrap().field == EditField::Hours {
            break;
        }
        app.on_key(key(KeyCode::Tab)).unwrap();
    }
    app.on_key(ch('i')).unwrap();
    app.on_key(key(KeyCode::Backspace)).unwrap();
    type_str(&mut app, "0");
    app.on_key(key(KeyCode::Esc)).unwrap();

    assert!(
        app.history
            .edit
            .as_ref()
            .unwrap()
            .problems()
            .contains(&EditProblem::HoursTooShort("0".into()))
    );
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.flash.as_ref().unwrap().tone, Tone::Error);
    assert!(
        app.history.edit.is_some(),
        "the panel stays open so it can be fixed"
    );
    let stored = repo::recent_entries(app.db.conn(), 10).unwrap();
    assert!(stored.iter().all(|e| e.hours >= 0.25));
}

#[test]
fn escape_discards_an_edit_and_says_so() {
    let mut app = app_with_history();
    to_history(&mut app);
    select_entry_with_words(&mut app, 253);
    app.on_key(ch('i')).unwrap();
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "9");
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(key(KeyCode::Esc)).unwrap();

    assert!(app.history.edit.is_none());
    assert_eq!(app.flash.as_ref().unwrap().text, "edit discarded");
    assert_eq!(
        repo::project_logged_total(app.db.conn(), 1).unwrap(),
        20_253,
        "unchanged"
    );
}

#[test]
fn closing_an_unchanged_edit_says_nothing() {
    let mut app = app_with_history();
    to_history(&mut app);
    app.on_key(ch('i')).unwrap();
    app.on_key(key(KeyCode::Esc)).unwrap();
    assert!(app.history.edit.is_none());
    assert!(
        app.flash.is_none(),
        "nothing was lost, so nothing to report"
    );
}

#[test]
fn deleting_takes_two_d_keystrokes_and_a_confirmation() {
    let mut app = app_with_history();
    to_history(&mut app);
    select_entry_with_words(&mut app, 880);

    app.on_key(ch('d')).unwrap();
    assert!(!app.history.confirming_delete, "one d only arms it");
    app.on_key(ch('d')).unwrap();
    assert!(app.history.confirming_delete);

    app.on_key(ch('y')).unwrap();
    assert_eq!(app.history.visible().len(), 2);
    assert_eq!(repo::recent_entries(app.db.conn(), 10).unwrap().len(), 2);
    assert_eq!(app.flash.as_ref().unwrap().tone, Tone::Success);
}

#[test]
fn a_delete_is_abandoned_by_anything_other_than_y() {
    let mut app = app_with_history();
    to_history(&mut app);
    app.on_key(ch('d')).unwrap();
    app.on_key(ch('d')).unwrap();
    app.on_key(ch('n')).unwrap();
    assert!(!app.history.confirming_delete);
    assert_eq!(repo::recent_entries(app.db.conn(), 10).unwrap().len(), 3);
    assert_eq!(app.flash.as_ref().unwrap().text, "kept");
}

#[test]
fn a_stray_d_does_not_linger_into_the_next_keystroke() {
    let mut app = app_with_history();
    to_history(&mut app);
    app.on_key(ch('d')).unwrap();
    app.on_key(ch('j')).unwrap();
    app.on_key(ch('d')).unwrap();
    assert!(!app.history.confirming_delete, "d j d must not delete");
}

#[test]
fn deleting_an_entry_updates_the_projects_running_total() {
    let mut app = app_with_history();
    to_history(&mut app);
    select_entry_with_words(&mut app, 20_000);
    let project_id = app.history.selected_row().unwrap().entry.project_id;
    app.on_key(ch('d')).unwrap();
    app.on_key(ch('d')).unwrap();
    app.on_key(ch('y')).unwrap();
    assert_eq!(
        repo::project_logged_total(app.db.conn(), project_id).unwrap(),
        253
    );
}

#[test]
fn leaving_the_screen_discards_an_open_edit_rather_than_hiding_it() {
    let mut app = app_with_history();
    to_history(&mut app);
    app.on_key(ch('i')).unwrap();
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "5");
    app.on_key(key(KeyCode::Esc)).unwrap();

    app.on_key(ch(' ')).unwrap();
    app.on_key(ch('e')).unwrap();
    assert_eq!(app.screen, Screen::Entry);
    assert!(
        app.history.edit.is_none(),
        "an edit left open would be a trap on return"
    );
    assert_eq!(app.flash.as_ref().unwrap().text, "edit discarded");
}

#[test]
fn quitting_with_an_open_changed_edit_asks_first() {
    let mut app = app_with_history();
    to_history(&mut app);
    app.on_key(ch('i')).unwrap();
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "7");
    app.on_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.on_key(ch('q')).unwrap(), Flow::Continue);
    assert!(app.confirming_quit);
}

#[test]
fn an_entry_added_on_the_form_appears_in_the_history() {
    let mut app = app_with_history();
    select_title(&mut app, "Glowstones");
    focus(&mut app, Field::Hours);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "0.75");
    app.on_key(key(KeyCode::Esc)).unwrap();
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "1380");
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();

    to_history(&mut app);
    assert_eq!(app.history.visible().len(), 4);
    assert!(
        app.history
            .visible()
            .iter()
            .any(|r| r.entry.session_word_count == 500),
        "1380 typed against 880 logged is a 500 word session"
    );
}

#[test]
fn the_history_screen_renders_its_columns() {
    let mut app = app_with_history();
    to_history(&mut app);
    let screen = render(&mut app, 100, 18).join("\n");
    for expected in [
        "Entries (3)",
        "2026-01-14",
        "Fireside 1",
        "20,000",
        "1.00h",
        "meant 2,000",
    ] {
        assert!(
            screen.contains(expected),
            "{expected:?} missing from:\n{screen}"
        );
    }
    assert!(
        screen.contains("i edits"),
        "the status bar names the actions:\n{screen}"
    );
}

#[test]
fn the_edit_panel_renders_over_the_list() {
    let mut app = app_with_history();
    to_history(&mut app);
    select_entry_with_words(&mut app, 253);
    app.on_key(ch('i')).unwrap();
    let screen = render(&mut app, 100, 24).join("\n");
    assert!(screen.contains("Edit entry"), "got:\n{screen}");
    assert!(
        screen.contains("the words in this row, not a total"),
        "got:\n{screen}"
    );
    assert!(
        screen.contains("Entries (3)"),
        "the list stays visible:\n{screen}"
    );
}

#[test]
fn the_delete_prompt_names_what_would_go() {
    let mut app = app_with_history();
    to_history(&mut app);
    select_entry_with_words(&mut app, 20_000);
    app.on_key(ch('d')).unwrap();
    app.on_key(ch('d')).unwrap();
    let screen = render(&mut app, 100, 18).join("\n");
    assert!(
        screen.contains("delete 20,000 words on Fireside 1"),
        "got:\n{screen}"
    );
    assert!(screen.contains("y deletes"), "got:\n{screen}");
}

/// Not an assertion: prints the frame so the layout can be read by eye.
#[test]
fn show_the_history_screen() {
    let mut app = app_with_history();
    to_history(&mut app);
    select_entry_with_words(&mut app, 20_000);
    app.on_key(ch('i')).unwrap();
    for line in render(&mut app, 96, 20) {
        println!("|{line}");
    }
}

/// Runs only where the author's own (gitignored) journals are present: proves the
/// screens cope with the real volume, not just a handful of fixture rows.
#[test]
fn the_screens_handle_the_authors_real_journals() {
    let dir = std::path::Path::new("historic_trackers");
    if !dir.join("Writing Journal 2024.ods").exists() {
        eprintln!("skipping: {} not present", dir.display());
        return;
    }
    let mut db = Db::open_in_memory().unwrap();
    draft_tracker::import::run(&mut db, dir, draft_tracker::import::Existing::Refuse).unwrap();
    let mut app = App::new(db).unwrap();

    // Every entry is reachable, so G really does reach the oldest.
    to_history(&mut app);
    assert_eq!(app.history.visible().len(), 654);
    app.on_key(ch('G')).unwrap();
    assert_eq!(
        app.history
            .selected_row()
            .unwrap()
            .entry
            .entry_date
            .to_string(),
        "2023-11-01"
    );
    let screen = render(&mut app, 100, 24).join("\n");
    assert!(screen.contains("Entries (654)"), "got:\n{screen}");

    // The picker opens on the eight open projects, not all seventy-eight.
    app.on_key(ch(' ')).unwrap();
    app.on_key(ch('e')).unwrap();
    assert_eq!(app.form.visible().len(), 8);
    app.on_key(ch('A')).unwrap();
    assert_eq!(app.form.visible().len(), 78);
    let screen = render(&mut app, 100, 24).join("\n");
    assert!(screen.contains("Projects (78)"), "got:\n{screen}");
}

// ============================================================ projects screen

use draft_tracker::tui::categories::Pane;
use draft_tracker::tui::projects::Field as PField;

fn to_screen(app: &mut App, key_char: char, expected: Screen) {
    app.on_key(ch(' ')).unwrap();
    app.on_key(ch(key_char)).unwrap();
    assert_eq!(app.screen, expected);
}

fn select_project(app: &mut App, title: &str) {
    for _ in 0..app.projects.visible().len() {
        if app
            .projects
            .selected_row()
            .map(|r| r.project.title.as_str())
            == Some(title)
        {
            return;
        }
        app.on_key(ch('j')).unwrap();
    }
    panic!("{title} not in the project list");
}

fn focus_project_field(app: &mut App, field: PField) {
    for _ in 0..PField::ORDER.len() {
        if app.projects.edit.as_ref().unwrap().field == field {
            return;
        }
        app.on_key(key(KeyCode::Tab)).unwrap();
    }
    panic!("could not focus {field:?}");
}

#[test]
fn the_projects_screen_shows_the_derived_figures() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    let row = app
        .projects
        .visible()
        .iter()
        .find(|r| r.project.title == "Fireside 1")
        .copied()
        .cloned()
        .unwrap();
    assert_eq!(row.metrics.total_word_count, 20_253);
    assert_eq!(row.metrics.entry_count, 2);
    assert_eq!(row.category_name.as_deref(), Some("SitR Book"));
}

/// The projects screen opens on the open projects, because a writing life
/// accumulates finished work and the few still going are what matters.
#[test]
fn the_projects_screen_shows_only_open_projects_by_default() {
    let mut app = app_with_history();
    {
        let conn = app.db.conn();
        repo::insert_project(conn, "Shipped", None, Status::Completed, None).unwrap();
        repo::insert_project(conn, "Dropped", None, Status::Abandoned, None).unwrap();
    }
    to_screen(&mut app, 'p', Screen::Projects);
    assert!(!app.projects.show_all());
    let titles: Vec<&str> = app
        .projects
        .visible()
        .iter()
        .map(|r| r.project.title.as_str())
        .collect();
    assert_eq!(titles, vec!["Fireside 1", "Glowstones"]);
    assert_eq!(app.projects.total(), 4);

    app.on_key(ch('A')).unwrap();
    assert!(app.projects.show_all());
    assert_eq!(app.projects.visible().len(), 4);
    assert!(app.flash.as_ref().unwrap().text.contains("all projects"));

    app.on_key(ch('A')).unwrap();
    assert_eq!(app.projects.visible().len(), 2);
}

#[test]
fn the_title_says_which_view_so_a_short_list_is_not_mistaken_for_missing_data() {
    let mut app = app_with_history();
    {
        let conn = app.db.conn();
        repo::insert_project(conn, "Shipped", None, Status::Completed, None).unwrap();
    }
    to_screen(&mut app, 'p', Screen::Projects);
    let screen = render(&mut app, 100, 24).join("\n");
    assert!(screen.contains("Projects (2 open of 3)"), "got:\n{screen}");
    app.on_key(ch('A')).unwrap();
    let screen = render(&mut app, 100, 24).join("\n");
    assert!(screen.contains("Projects (all 3)"), "got:\n{screen}");
}

/// Marking a project completed in the open-only view would otherwise make it
/// vanish under the cursor, hiding what the keystroke did and stopping the cycle
/// halfway.
#[test]
fn a_project_stays_in_view_while_its_status_is_being_cycled() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    select_project(&mut app, "Fireside 1");

    app.on_key(ch('x')).unwrap();
    assert_eq!(
        app.projects
            .selected_row()
            .map(|r| r.project.title.as_str()),
        Some("Fireside 1"),
        "the row the cursor is on must not disappear mid-cycle"
    );
    assert_eq!(
        app.projects.selected_row().unwrap().project.status,
        Status::Completed
    );

    // And the cycle can be carried on to abandoned.
    app.on_key(ch('x')).unwrap();
    assert_eq!(
        app.projects.selected_row().unwrap().project.status,
        Status::Abandoned
    );
    app.on_key(ch('x')).unwrap();
    assert_eq!(
        app.projects.selected_row().unwrap().project.status,
        Status::Open
    );
}

#[test]
fn a_closed_project_leaves_the_open_view_once_the_cursor_moves_on() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    select_project(&mut app, "Fireside 1");
    app.on_key(ch('x')).unwrap(); // completed, held in view
    assert_eq!(app.projects.visible().len(), 2);

    app.on_key(ch('j')).unwrap();
    assert_eq!(
        app.projects
            .selected_row()
            .map(|r| r.project.title.as_str()),
        Some("Glowstones"),
        "the cursor lands where it was sent, not where the shrinking list moved it"
    );
    assert_eq!(
        app.projects.visible().len(),
        1,
        "the completed project has gone"
    );
    assert_eq!(app.projects.pinned(), None);
}

#[test]
fn an_empty_open_view_points_at_the_key_that_widens_it() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    for title in ["Fireside 1", "Glowstones"] {
        select_project(&mut app, title);
        app.on_key(ch('x')).unwrap();
        app.on_key(ch('G')).unwrap();
    }
    let screen = render(&mut app, 100, 24).join("\n");
    assert!(screen.contains("No open projects"), "got:\n{screen}");
    assert!(screen.contains("Press A"), "got:\n{screen}");
}

#[test]
fn x_cycles_a_projects_status_without_opening_a_panel() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    select_project(&mut app, "Fireside 1");
    assert_eq!(
        app.projects.selected_row().unwrap().project.status,
        Status::Open
    );

    app.on_key(ch('x')).unwrap();
    assert_eq!(
        app.projects.selected_row().unwrap().project.status,
        Status::Completed
    );
    assert!(app.projects.edit.is_none(), "no panel: this is a bulk pass");
    assert!(app.flash.as_ref().unwrap().text.contains("completed"));

    app.on_key(ch('x')).unwrap();
    assert_eq!(
        app.projects.selected_row().unwrap().project.status,
        Status::Abandoned
    );
    app.on_key(ch('x')).unwrap();
    assert_eq!(
        app.projects.selected_row().unwrap().project.status,
        Status::Open
    );
}

#[test]
fn a_status_change_reaches_the_entry_pickers_list_of_open_projects() {
    let mut app = app_with_history();
    assert_eq!(app.form.visible().len(), 2);
    to_screen(&mut app, 'p', Screen::Projects);
    select_project(&mut app, "Glowstones");
    app.on_key(ch('x')).unwrap(); // completed
    to_screen(&mut app, 'e', Screen::Entry);
    let titles: Vec<&str> = app
        .form
        .visible()
        .iter()
        .map(|c| c.title.as_str())
        .collect();
    assert_eq!(
        titles,
        vec!["Fireside 1"],
        "a completed project leaves the picker"
    );
}

#[test]
fn o_creates_a_project_ready_to_type_into() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    app.on_key(ch('o')).unwrap();
    let edit = app.projects.edit.as_ref().unwrap();
    assert!(edit.is_creating());
    assert_eq!(edit.field, PField::Title);
    assert_eq!(
        edit.mode,
        Mode::Insert,
        "nothing to read first, so start typing"
    );

    type_str(&mut app, "A Wizard at Rest");
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();

    assert!(app.projects.edit.is_none());
    assert!(
        repo::project_id_by_title(app.db.conn(), "A Wizard at Rest")
            .unwrap()
            .is_some(),
        "the project was written"
    );
    assert!(app.flash.as_ref().unwrap().text.contains("created"));
}

#[test]
fn a_project_needs_a_title() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    app.on_key(ch('o')).unwrap();
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(app.flash.as_ref().unwrap().tone, Tone::Error);
    assert!(
        app.projects.edit.is_some(),
        "the panel stays open to be fixed"
    );
}

#[test]
fn a_duplicate_title_is_reported_by_name() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    app.on_key(ch('o')).unwrap();
    type_str(&mut app, "Fireside 1");
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();
    let flash = app.flash.as_ref().unwrap();
    assert_eq!(flash.tone, Tone::Error);
    assert!(
        flash.text.contains("already exists"),
        "got {:?}",
        flash.text
    );
    assert_eq!(
        repo::list_projects(app.db.conn()).unwrap().len(),
        2,
        "nothing created"
    );
}

#[test]
fn h_and_l_step_the_category_and_status_choices() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    select_project(&mut app, "Fireside 1");
    app.on_key(ch('i')).unwrap();
    focus_project_field(&mut app, PField::Category);

    let before = app.projects.edit.as_ref().unwrap().category_id;
    app.on_key(ch('l')).unwrap();
    let after = app.projects.edit.as_ref().unwrap().category_id;
    assert_ne!(before, after);
    app.on_key(ch('h')).unwrap();
    assert_eq!(
        app.projects.edit.as_ref().unwrap().category_id,
        before,
        "h steps back"
    );

    // Uncategorized is reachable, since a project need not have a category.
    app.on_key(ch('h')).unwrap();
    assert_eq!(app.projects.edit.as_ref().unwrap().category_id, None);

    focus_project_field(&mut app, PField::Status);
    app.on_key(ch('l')).unwrap();
    assert_eq!(
        app.projects.edit.as_ref().unwrap().status,
        Status::Completed
    );
}

#[test]
fn typing_into_a_choice_field_explains_itself_instead_of_doing_nothing() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    app.on_key(ch('i')).unwrap();
    focus_project_field(&mut app, PField::Status);
    app.on_key(ch('i')).unwrap();
    assert_eq!(app.projects.edit.as_ref().unwrap().mode, Mode::Normal);
    assert!(app.flash.as_ref().unwrap().text.contains("h and l"));
}

/// The guard that matters: deleting would cascade to the entries.
#[test]
fn deleting_a_project_with_entries_is_refused_outright() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    select_project(&mut app, "Fireside 1");
    app.on_key(ch('d')).unwrap();
    app.on_key(ch('d')).unwrap();

    assert!(
        !app.projects.confirming_delete,
        "not even offered as a confirmation"
    );
    let flash = app.flash.as_ref().unwrap();
    assert_eq!(flash.tone, Tone::Error);
    assert!(flash.text.contains("2 entries"), "got {:?}", flash.text);
    assert!(
        flash.text.contains("abandoned"),
        "it names what to do instead"
    );
    assert_eq!(
        repo::recent_entries(app.db.conn(), 10).unwrap().len(),
        3,
        "entries intact"
    );
}

#[test]
fn deleting_a_project_with_no_entries_asks_and_then_deletes() {
    let mut app = app_with_history();
    repo::insert_project(app.db.conn(), "Never Started", None, Status::Open, None).unwrap();
    to_screen(&mut app, 'p', Screen::Projects);
    select_project(&mut app, "Never Started");
    app.on_key(ch('d')).unwrap();
    app.on_key(ch('d')).unwrap();
    assert!(app.projects.confirming_delete);
    app.on_key(ch('y')).unwrap();
    assert!(
        repo::project_id_by_title(app.db.conn(), "Never Started")
            .unwrap()
            .is_none()
    );
}

#[test]
fn the_projects_filter_matches_title_or_category() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    app.on_key(ch('/')).unwrap();
    type_str(&mut app, "glow");
    assert_eq!(app.projects.visible().len(), 1);
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(key(KeyCode::Esc)).unwrap();

    app.on_key(ch('/')).unwrap();
    type_str(&mut app, "sitr book");
    assert_eq!(
        app.projects.visible().len(),
        2,
        "both projects share the category"
    );
}

#[test]
fn the_projects_screen_renders_its_columns_and_details() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    select_project(&mut app, "Fireside 1");
    let screen = render(&mut app, 100, 24).join("\n");
    for expected in [
        "Projects (2 open of 2)",
        "Fireside 1",
        "SitR Book",
        "open",
        "20,253",
        "Words per hour",
        "Selected",
    ] {
        assert!(
            screen.contains(expected),
            "{expected:?} missing from:\n{screen}"
        );
    }
    assert!(
        screen.contains("x cycles status"),
        "the status bar names the pass:\n{screen}"
    );
    // A label column exactly as wide as its longest label leaves no gap before
    // the value, which reads as one run-together word.
    assert!(
        screen.contains("Words per hour  "),
        "every details label needs a gap before its value:\n{screen}"
    );
}

// ============================================================ categories screen

#[test]
fn the_categories_screen_shows_both_panes_with_usage_counts() {
    let mut app = app_with_history();
    to_screen(&mut app, 'c', Screen::Categories);
    assert_eq!(app.categories.category_rows().len(), 1);
    assert_eq!(app.categories.category_rows()[0].project_count, 2);
    assert_eq!(app.categories.type_rows().len(), 1);
    assert_eq!(app.categories.type_rows()[0].category_count, 1);

    let screen = render(&mut app, 100, 20).join("\n");
    assert!(screen.contains("Categories (1)"), "got:\n{screen}");
    assert!(screen.contains("Types (1)"), "got:\n{screen}");
    assert!(screen.contains("2 proj"), "got:\n{screen}");
}

#[test]
fn tab_switches_between_the_category_and_type_panes() {
    let mut app = app_with_history();
    to_screen(&mut app, 'c', Screen::Categories);
    assert_eq!(app.categories.pane, Pane::Categories);
    app.on_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(app.categories.pane, Pane::Types);
    app.on_key(ch('h')).unwrap();
    assert_eq!(app.categories.pane, Pane::Categories);
}

#[test]
fn a_type_can_be_created_in_the_types_pane() {
    let mut app = app_with_history();
    to_screen(&mut app, 'c', Screen::Categories);
    app.on_key(key(KeyCode::Tab)).unwrap();
    app.on_key(ch('o')).unwrap();
    type_str(&mut app, "Creative NF");
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.categories.type_rows().len(), 2);
    assert!(
        repo::type_id_by_name(app.db.conn(), "Creative NF")
            .unwrap()
            .is_some()
    );
}

#[test]
fn a_category_is_created_under_a_type_and_can_be_moved_between_types() {
    let mut app = app_with_history();
    {
        let conn = app.db.conn();
        repo::ensure_type(conn, "Non-Fiction").unwrap();
    }
    to_screen(&mut app, 'c', Screen::Categories);
    app.on_key(ch('o')).unwrap();
    type_str(&mut app, "Non-Fiction Essays");
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(key(KeyCode::Tab)).unwrap(); // to the Type field
    let before = app
        .categories
        .type_name(match app.categories.edit.as_ref().unwrap() {
            draft_tracker::tui::categories::Edit::Category(e) => e.type_id,
            _ => None,
        });
    app.on_key(ch('l')).unwrap();
    let after = app
        .categories
        .type_name(match app.categories.edit.as_ref().unwrap() {
            draft_tracker::tui::categories::Edit::Category(e) => e.type_id,
            _ => None,
        });
    assert_ne!(before, after, "l steps to the next type");

    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();
    let rows = repo::list_categories_with_usage(app.db.conn()).unwrap();
    let created = rows
        .iter()
        .find(|r| r.category.name == "Non-Fiction Essays")
        .unwrap();
    assert_eq!(created.type_name, after);
}

#[test]
fn a_category_in_use_cannot_be_deleted_from_the_screen() {
    let mut app = app_with_history();
    to_screen(&mut app, 'c', Screen::Categories);
    app.on_key(ch('d')).unwrap();
    app.on_key(ch('d')).unwrap();
    assert!(!app.categories.confirming_delete);
    let flash = app.flash.as_ref().unwrap();
    assert_eq!(flash.tone, Tone::Error);
    assert!(
        flash.text.contains("2 projects use"),
        "got {:?}",
        flash.text
    );
    assert_eq!(
        repo::list_categories_with_usage(app.db.conn())
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn a_type_with_categories_under_it_cannot_be_deleted() {
    let mut app = app_with_history();
    to_screen(&mut app, 'c', Screen::Categories);
    app.on_key(key(KeyCode::Tab)).unwrap();
    app.on_key(ch('d')).unwrap();
    app.on_key(ch('d')).unwrap();
    assert!(!app.categories.confirming_delete);
    assert!(
        app.flash
            .as_ref()
            .unwrap()
            .text
            .contains("1 categories use")
    );
}

#[test]
fn an_unused_category_can_be_deleted_after_confirming() {
    let mut app = app_with_history();
    {
        let conn = app.db.conn();
        let type_id = repo::type_id_by_name(conn, "Fiction").unwrap().unwrap();
        repo::insert_category(conn, "Unused", type_id, None).unwrap();
    }
    to_screen(&mut app, 'c', Screen::Categories);
    for _ in 0..app.categories.category_rows().len() {
        if app.categories.selected_category().unwrap().category.name == "Unused" {
            break;
        }
        app.on_key(ch('j')).unwrap();
    }
    app.on_key(ch('d')).unwrap();
    app.on_key(ch('d')).unwrap();
    assert!(app.categories.confirming_delete);
    app.on_key(ch('y')).unwrap();
    assert!(
        repo::category_id_by_name(app.db.conn(), "Unused")
            .unwrap()
            .is_none(),
        "the unused category went"
    );
}

#[test]
fn renaming_a_category_shows_up_on_the_projects_screen() {
    let mut app = app_with_history();
    to_screen(&mut app, 'c', Screen::Categories);
    app.on_key(ch('i')).unwrap();
    app.on_key(ch('i')).unwrap();
    for _ in 0.."SitR Book".len() {
        app.on_key(key(KeyCode::Backspace)).unwrap();
    }
    type_str(&mut app, "SitR Novel");
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();

    to_screen(&mut app, 'p', Screen::Projects);
    assert!(
        app.projects
            .visible()
            .iter()
            .all(|r| r.category_name.as_deref() == Some("SitR Novel")),
        "the projects screen must not be showing a stale category name"
    );
}

#[test]
fn leaving_the_categories_screen_discards_an_open_edit() {
    let mut app = app_with_history();
    to_screen(&mut app, 'c', Screen::Categories);
    app.on_key(ch('i')).unwrap();
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "zz");
    app.on_key(key(KeyCode::Esc)).unwrap();
    to_screen(&mut app, 'e', Screen::Entry);
    assert!(app.categories.edit.is_none());
    assert_eq!(app.flash.as_ref().unwrap().text, "edit discarded");
}

#[test]
fn the_help_overlay_changes_with_the_screen() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    app.on_key(ch('?')).unwrap();
    let screen = render(&mut app, 90, 30).join("\n");
    assert!(screen.contains("cycle status"), "got:\n{screen}");
    assert!(
        screen.contains("refused if it has entries"),
        "got:\n{screen}"
    );
}

/// Not an assertion: prints the frames so the layouts can be read by eye.
#[test]
fn show_the_projects_screen() {
    let mut app = app_with_history();
    to_screen(&mut app, 'p', Screen::Projects);
    select_project(&mut app, "Fireside 1");
    for line in render(&mut app, 96, 20) {
        println!("|{line}");
    }
}

#[test]
fn show_the_categories_screen() {
    let mut app = app_with_history();
    to_screen(&mut app, 'c', Screen::Categories);
    app.on_key(ch('i')).unwrap();
    for line in render(&mut app, 96, 18) {
        println!("|{line}");
    }
}

// ============================================================ stats dashboard

/// Three years, two projects, one completed in 2025 and one abandoned.
fn app_with_years() -> App {
    let db = Db::open_in_memory().unwrap();
    {
        let conn = db.conn();
        let fiction = repo::ensure_type(conn, "Fiction").unwrap();
        let nonfic = repo::ensure_type(conn, "Non-Fiction").unwrap();
        let book = repo::ensure_category(conn, "SitR Book", fiction).unwrap();
        let essays = repo::ensure_category(conn, "Non-Fiction Essays", nonfic).unwrap();

        let novel =
            repo::insert_project(conn, "Trial of Mist", Some(book), Status::Completed, None)
                .unwrap();
        let essay =
            repo::insert_project(conn, "Utopia", Some(essays), Status::Abandoned, None).unwrap();
        let live =
            repo::insert_project(conn, "Fireside 1", Some(book), Status::Open, None).unwrap();

        let add = |project_id, d: (i32, u32, u32), words, hours| {
            repo::insert_entry(
                conn,
                &NewEntry {
                    entry_date: day(d.0, d.1, d.2),
                    project_id,
                    session_word_count: words,
                    hours,
                    notes: None,
                },
            )
            .unwrap()
        };
        add(novel, (2024, 6, 1), 10_000, 10.0);
        add(novel, (2025, 7, 10), 30_000, 20.0);
        add(essay, (2025, 5, 2), 3_000, 2.0);
        add(live, (2026, 1, 12), 1_800, 3.0);
    }
    App::new(db).unwrap()
}

fn to_stats(app: &mut App) {
    app.on_key(ch(' ')).unwrap();
    app.on_key(ch('s')).unwrap();
    assert_eq!(app.screen, Screen::Stats);
}

#[test]
fn the_bar_helper_never_loses_a_non_zero_value_or_overflows_its_width() {
    use draft_tracker::tui::draw::bar;
    assert_eq!(bar(0, 100, 10), "", "nothing is nothing");
    assert_eq!(
        bar(100, 100, 10).chars().count(),
        10,
        "a full bar fills the width"
    );
    assert!(
        !bar(1, 1_000_000, 10).is_empty(),
        "a tiny value still shows something"
    );
    assert!(bar(50, 100, 10).chars().count() <= 10);
    assert_eq!(bar(5, 100, 0), "", "no width, no bar");
    assert_eq!(bar(10, 0, 10), "", "no maximum to scale against");
    assert_eq!(bar(-5, 100, 10), "", "a negative total has no bar");
}

#[test]
fn the_dashboard_opens_on_the_current_year() {
    let mut app = app_with_years();
    to_stats(&mut app);
    assert_eq!(
        app.stats.selected_year(),
        Some(chrono::Local::now().date_naive().year())
    );
    assert!(app.stats.is_current_year());
}

#[test]
fn the_lifetime_figures_cover_everything() {
    let mut app = app_with_years();
    to_stats(&mut app);
    assert_eq!(app.stats.lifetime.words, 44_800);
    assert_eq!(app.stats.lifetime.hours, 35.0);
    // Abandoned work is not completed work.
    assert_eq!(app.stats.lifetime_completed, 1);
}

#[test]
fn stepping_back_moves_through_the_years_that_have_entries() {
    let mut app = app_with_years();
    to_stats(&mut app);
    assert_eq!(app.stats.selected_year(), Some(2026));

    app.on_key(ch('k')).unwrap();
    assert_eq!(app.stats.selected_year(), Some(2025));
    assert_eq!(app.stats.selected.words, 33_000);
    assert_eq!(
        app.stats.selected_completed, 1,
        "the novel's last entry is in 2025"
    );

    app.on_key(ch('k')).unwrap();
    assert_eq!(app.stats.selected_year(), Some(2024));
    assert_eq!(app.stats.selected.words, 10_000);
    assert_eq!(
        app.stats.selected_completed, 0,
        "its end date is 2025, not 2024"
    );

    // There is nothing before 2024, and the screen says so rather than sitting mute.
    app.on_key(ch('k')).unwrap();
    assert_eq!(app.stats.selected_year(), Some(2024));
    assert!(app.flash.as_ref().unwrap().text.contains("earliest"));
}

#[test]
fn stepping_forward_and_zero_return_to_the_current_year() {
    let mut app = app_with_years();
    to_stats(&mut app);
    app.on_key(ch('k')).unwrap();
    app.on_key(ch('k')).unwrap();
    assert_eq!(app.stats.selected_year(), Some(2024));
    app.on_key(ch('j')).unwrap();
    assert_eq!(app.stats.selected_year(), Some(2025));
    app.on_key(ch('0')).unwrap();
    assert_eq!(app.stats.selected_year(), Some(2026));
}

/// All time sits at the end of the sequence of years, so stepping forward from
/// the newest year arrives there rather than stopping dead.
#[test]
fn stepping_past_the_newest_year_reaches_all_time() {
    let mut app = app_with_years();
    to_stats(&mut app);
    assert_eq!(app.stats.selected_year(), Some(2026));
    app.on_key(ch('j')).unwrap();
    assert!(app.stats.is_lifetime());
    assert_eq!(app.stats.selected_year(), None);

    app.on_key(ch('j')).unwrap();
    assert!(app.stats.is_lifetime(), "there is nothing past all time");
    assert!(app.flash.as_ref().unwrap().text.contains("end"));

    app.on_key(ch('k')).unwrap();
    assert_eq!(
        app.stats.selected_year(),
        Some(2026),
        "stepping back leaves all time"
    );
}

#[test]
fn dollar_jumps_to_all_time_and_zero_comes_back() {
    let mut app = app_with_years();
    to_stats(&mut app);
    app.on_key(ch('$')).unwrap();
    assert!(app.stats.is_lifetime());
    // The lower half now covers every project and every category.
    assert_eq!(app.stats.selected.words, app.stats.lifetime.words);
    assert_eq!(
        app.stats.selected_projects.len(),
        3,
        "all three, not just 2026's one"
    );

    app.on_key(ch('0')).unwrap();
    assert_eq!(app.stats.selected_year(), Some(2026));
    assert_eq!(app.stats.selected_projects.len(), 1);
}

#[test]
fn all_time_shows_one_set_of_figures_rather_than_the_same_four_twice() {
    let mut app = app_with_years();
    to_stats(&mut app);
    let by_year = render(&mut app, 100, 30).join("\n");
    assert!(by_year.contains("Lifetime"), "got:\n{by_year}");
    assert!(by_year.contains("2026 (this year)"), "got:\n{by_year}");

    app.on_key(ch('$')).unwrap();
    let all_time = render(&mut app, 100, 30).join("\n");
    assert!(all_time.contains("All time"), "got:\n{all_time}");
    assert!(
        !all_time.contains("(this year)"),
        "no year panel while showing all time:\n{all_time}"
    );
    assert_eq!(
        all_time.matches("Words/Hour").count(),
        1,
        "the same four numbers must not appear twice:\n{all_time}"
    );
}

/// With every project a writer has started on one list, the graph has to scroll.
#[test]
fn h_and_l_scroll_the_project_graph() {
    let mut app = app_with_years();
    to_stats(&mut app);
    app.on_key(ch('$')).unwrap();
    // Draw first, so scrolling knows how many rows fit.
    let _ = render(&mut app, 100, 14);
    assert_eq!(app.stats.project_offset(), 0);

    app.on_key(ch('l')).unwrap();
    assert_eq!(app.stats.project_offset(), 1, "l scrolls down the list");
    app.on_key(ch('h')).unwrap();
    assert_eq!(app.stats.project_offset(), 0);
    app.on_key(ch('h')).unwrap();
    assert_eq!(app.stats.project_offset(), 0, "already at the top");

    app.on_key(ch('G')).unwrap();
    let at_end = app.stats.project_offset();
    app.on_key(ch('l')).unwrap();
    assert_eq!(
        app.stats.project_offset(),
        at_end,
        "G goes as far as there is to go"
    );
    app.on_key(ch('g')).unwrap();
    app.on_key(ch('g')).unwrap();
    assert_eq!(app.stats.project_offset(), 0, "gg returns to the top");
}

#[test]
fn changing_scope_returns_the_project_graph_to_the_top() {
    let mut app = app_with_years();
    to_stats(&mut app);
    app.on_key(ch('$')).unwrap();
    let _ = render(&mut app, 100, 14);
    app.on_key(ch('l')).unwrap();
    assert_eq!(app.stats.project_offset(), 1);
    app.on_key(ch('k')).unwrap();
    assert_eq!(app.stats.project_offset(), 0, "a new scope is a new list");
}

#[test]
fn the_year_graph_shows_only_that_years_projects_with_their_status() {
    let mut app = app_with_years();
    to_stats(&mut app);
    app.on_key(ch('k')).unwrap(); // 2025
    let bars = &app.stats.selected_projects;
    let titles: Vec<&str> = bars.iter().map(|b| b.title.as_str()).collect();
    assert_eq!(
        titles,
        vec!["Trial of Mist", "Utopia"],
        "largest first, 2026 excluded"
    );
    assert_eq!(bars[0].status, Status::Completed);
    assert_eq!(bars[1].status, Status::Abandoned);
}

#[test]
fn year_over_year_covers_every_year_with_entries() {
    let mut app = app_with_years();
    to_stats(&mut app);
    assert_eq!(
        app.stats.by_year,
        vec![(2024, 10_000), (2025, 33_000), (2026, 1_800)]
    );
}

#[test]
fn the_category_shares_add_up_to_the_whole_year() {
    let mut app = app_with_years();
    to_stats(&mut app);
    app.on_key(ch('k')).unwrap(); // 2025
    let shares = app.stats.category_shares();
    let total: f64 = shares.iter().map(|(_, share, _)| share).sum();
    assert!(
        (total - 1.0).abs() < 1e-9,
        "shares must sum to 1, got {total}"
    );
    let book = shares
        .iter()
        .find(|(name, _, _)| name == "SitR Book")
        .unwrap();
    assert!((book.1 - 30_000.0 / 33_000.0).abs() < 1e-9);
}

/// A revision-heavy category can net out below zero, and a negative slice of a
/// bar means nothing.
#[test]
fn a_category_that_nets_out_negative_gets_no_slice_and_is_reported() {
    let mut app = app_with_years();
    {
        let conn = app.db.conn();
        let fiction = repo::type_id_by_name(conn, "Fiction").unwrap().unwrap();
        let cuts = repo::ensure_category(conn, "Revision", fiction).unwrap();
        let project =
            repo::insert_project(conn, "Big Cut", Some(cuts), Status::Open, None).unwrap();
        repo::insert_entry(
            conn,
            &NewEntry {
                entry_date: day(2026, 2, 1),
                project_id: project,
                session_word_count: -500,
                hours: 1.0,
                notes: None,
            },
        )
        .unwrap();
    }
    to_stats(&mut app);
    app.stats.reload(&app.db).unwrap();

    let shares = app.stats.category_shares();
    assert!(
        shares.iter().all(|(name, _, _)| name != "Revision"),
        "a negative category has no slice: {shares:?}"
    );
    let total: f64 = shares.iter().map(|(_, share, _)| share).sum();
    assert!(
        (total - 1.0).abs() < 1e-9,
        "the remaining shares still sum to 1"
    );

    let ignored = app.stats.categories_without_a_share();
    assert_eq!(ignored.len(), 1);
    assert_eq!(ignored[0].category, "Revision");

    let screen = render(&mut app, 100, 26).join("\n");
    assert!(screen.contains("no share: Revision"), "got:\n{screen}");
}

#[test]
fn an_entry_added_on_the_form_shows_up_on_the_dashboard() {
    let mut app = app_with_years();
    to_stats(&mut app);
    let before = app.stats.lifetime.words;

    app.on_key(ch(' ')).unwrap();
    app.on_key(ch('e')).unwrap();
    select_title(&mut app, "Fireside 1");
    focus(&mut app, Field::Hours);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "1");
    app.on_key(key(KeyCode::Esc)).unwrap();
    focus(&mut app, Field::ProjectWords);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "2300");
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();

    to_stats(&mut app);
    assert_eq!(app.stats.lifetime.words, before + 500);
}

#[test]
fn a_status_change_moves_the_completed_count() {
    let mut app = app_with_years();
    to_stats(&mut app);
    assert_eq!(app.stats.lifetime_completed, 1);

    app.on_key(ch(' ')).unwrap();
    app.on_key(ch('p')).unwrap();
    select_project(&mut app, "Fireside 1");
    app.on_key(ch('x')).unwrap(); // open -> completed
    to_stats(&mut app);
    assert_eq!(app.stats.lifetime_completed, 2);
}

#[test]
fn the_dashboard_renders_every_section() {
    let mut app = app_with_years();
    to_stats(&mut app);
    let screen = render(&mut app, 100, 30).join("\n");
    for expected in [
        "Lifetime",
        "2026 (this year)",
        "Words",
        "Hours",
        "Completed",
        "Words/Hour",
        "2026 projects",
        "Year over year",
        "2026 by category",
        "44,800",
    ] {
        assert!(
            screen.contains(expected),
            "{expected:?} missing from:\n{screen}"
        );
    }
}

#[test]
fn the_category_bar_fills_its_width_exactly() {
    let mut app = app_with_years();
    to_stats(&mut app);
    app.on_key(ch('k')).unwrap(); // 2025 has two categories
    let width = 100u16;
    let lines = render(&mut app, width, 30);
    // The category bar is the only row whose entire content is solid blocks:
    // the project and year graphs carry a label and a number too.
    let bar_line = lines
        .iter()
        .find(|line| {
            let inner = line
                .trim_start_matches('\u{2502}')
                .trim_end_matches('\u{2502}');
            !inner.is_empty() && inner.chars().all(|c| c == '\u{2588}')
        })
        .expect("a segmented bar was drawn");
    let blocks = bar_line.chars().filter(|c| *c == '\u{2588}').count();
    assert_eq!(
        blocks,
        width as usize - 2,
        "largest-remainder sharing must leave no gap: {bar_line:?}"
    );
}

/// Not an assertion: prints the dashboard so the layout can be read by eye.
#[test]
fn show_the_stats_dashboard() {
    let mut app = app_with_years();
    to_stats(&mut app);
    app.on_key(ch('k')).unwrap();
    for line in render(&mut app, 92, 26) {
        println!("|{line}");
    }
}

/// Runs only where the author's own (gitignored) journals are present: the real
/// test of the dashboard layout is 78 projects and 10 categories, not a fixture.
#[test]
fn the_dashboard_copes_with_the_authors_real_journals() {
    let dir = std::path::Path::new("historic_trackers");
    if !dir.join("Writing Journal 2024.ods").exists() {
        eprintln!("skipping: {} not present", dir.display());
        return;
    }
    let mut db = Db::open_in_memory().unwrap();
    draft_tracker::import::run(&mut db, dir, draft_tracker::import::Existing::Refuse).unwrap();
    let mut app = App::new(db).unwrap();
    to_stats(&mut app);

    assert_eq!(app.stats.lifetime.words, 466_665);
    assert_eq!(app.stats.lifetime.hours, 593.50);
    assert_eq!(app.stats.lifetime_completed, 68);
    assert_eq!(app.stats.years(), [2023, 2024, 2025, 2026]);

    // 2024 has 46 projects: more than fit, so the graph must say what it cut
    // rather than quietly showing a partial picture.
    app.on_key(ch('k')).unwrap();
    app.on_key(ch('k')).unwrap();
    assert_eq!(app.stats.selected_year(), Some(2024));
    assert_eq!(app.stats.selected_projects.len(), 46);
    let _ = render(&mut app, 100, 30);
    let screen = render(&mut app, 100, 30).join("\n");
    // The title carries the window, so a scrolled graph never looks like the
    // whole list.
    assert!(screen.contains("2024 projects (1"), "got:\n{screen}");
    assert!(screen.contains("of 46)"), "got:\n{screen}");

    let shares = app.stats.category_shares();
    let total: f64 = shares.iter().map(|(_, s, _)| s).sum();
    assert!((total - 1.0).abs() < 1e-9, "shares sum to {total}");

    // Every category must appear in the legend in full. A legend that silently
    // drops its last entries is worse than one that takes an extra line.
    app.on_key(ch('j')).unwrap(); // 2025, six categories
    let screen = render(&mut app, 104, 30).join("\n");
    for (name, _, _) in app.stats.category_shares() {
        assert!(
            screen.contains(&name),
            "{name:?} missing from the legend:\n{screen}"
        );
    }
}

#[test]
fn show_the_real_dashboard() {
    let dir = std::path::Path::new("historic_trackers");
    if !dir.join("Writing Journal 2024.ods").exists() {
        return;
    }
    let mut db = Db::open_in_memory().unwrap();
    draft_tracker::import::run(&mut db, dir, draft_tracker::import::Existing::Refuse).unwrap();
    let mut app = App::new(db).unwrap();
    to_stats(&mut app);
    app.on_key(ch('k')).unwrap(); // 2025, the busiest full year
    for line in render(&mut app, 104, 28) {
        println!("|{line}");
    }
}

#[test]
fn show_the_real_projects_screen() {
    let dir = std::path::Path::new("historic_trackers");
    if !dir.join("Writing Journal 2024.ods").exists() {
        return;
    }
    let mut db = Db::open_in_memory().unwrap();
    draft_tracker::import::run(&mut db, dir, draft_tracker::import::Existing::Refuse).unwrap();
    let mut app = App::new(db).unwrap();
    to_screen(&mut app, 'p', Screen::Projects);
    for line in render(&mut app, 96, 20) {
        println!("|{line}");
    }
}

// ============================================================ moving an entry

fn focus_edit_field(app: &mut App, field: EditField) {
    for _ in 0..EditField::ORDER.len() {
        if app.history.edit.as_ref().unwrap().field == field {
            return;
        }
        app.on_key(key(KeyCode::Tab)).unwrap();
    }
    panic!("could not focus {field:?}");
}

#[test]
fn j_and_k_walk_the_fields_of_every_edit_panel() {
    // History.
    let mut app = app_with_history();
    to_history(&mut app);
    app.on_key(ch('i')).unwrap();
    let first = app.history.edit.as_ref().unwrap().field;
    app.on_key(ch('j')).unwrap();
    assert_ne!(app.history.edit.as_ref().unwrap().field, first);
    app.on_key(ch('k')).unwrap();
    assert_eq!(app.history.edit.as_ref().unwrap().field, first);

    // Projects.
    to_screen(&mut app, 'p', Screen::Projects);
    app.on_key(ch('i')).unwrap();
    let first = app.projects.edit.as_ref().unwrap().field;
    app.on_key(ch('j')).unwrap();
    assert_ne!(app.projects.edit.as_ref().unwrap().field, first);
    app.on_key(ch('k')).unwrap();
    assert_eq!(app.projects.edit.as_ref().unwrap().field, first);

    // Categories.
    to_screen(&mut app, 'c', Screen::Categories);
    app.on_key(ch('i')).unwrap();
    let before = render(&mut app, 100, 24).join("\n");
    app.on_key(ch('j')).unwrap();
    let after = render(&mut app, 100, 24).join("\n");
    assert_ne!(
        before, after,
        "j must move the cursor in the category panel too"
    );
}

#[test]
fn an_entry_can_be_moved_to_another_project() {
    let mut app = app_with_history();
    let (fireside, glowstones) = {
        let conn = app.db.conn();
        (
            repo::project_id_by_title(conn, "Fireside 1")
                .unwrap()
                .unwrap(),
            repo::project_id_by_title(conn, "Glowstones")
                .unwrap()
                .unwrap(),
        )
    };
    assert_eq!(
        repo::project_logged_total(app.db.conn(), fireside).unwrap(),
        20_253
    );
    assert_eq!(
        repo::project_logged_total(app.db.conn(), glowstones).unwrap(),
        880
    );

    to_history(&mut app);
    select_entry_with_words(&mut app, 253);
    app.on_key(ch('i')).unwrap();
    focus_edit_field(&mut app, EditField::Project);
    assert_eq!(
        app.history.edit.as_ref().unwrap().project_title,
        "Fireside 1"
    );

    // h and l step it, exactly as on the entry form.
    app.on_key(ch('l')).unwrap();
    assert_eq!(
        app.history.edit.as_ref().unwrap().project_title,
        "Glowstones"
    );
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();

    assert_eq!(app.flash.as_ref().unwrap().tone, Tone::Success);
    // The words move with the entry: both totals change.
    assert_eq!(
        repo::project_logged_total(app.db.conn(), fireside).unwrap(),
        20_000
    );
    assert_eq!(
        repo::project_logged_total(app.db.conn(), glowstones).unwrap(),
        1_133
    );
}

#[test]
fn the_project_picker_only_appears_while_the_project_field_is_in_hand() {
    let mut app = app_with_history();
    to_history(&mut app);
    app.on_key(ch('i')).unwrap();

    focus_edit_field(&mut app, EditField::Hours);
    let without = render(&mut app, 100, 24).join("\n");
    assert!(
        !without.contains("Move to"),
        "the picker must stay out of the way:\n{without}"
    );

    focus_edit_field(&mut app, EditField::Project);
    let with = render(&mut app, 100, 24).join("\n");
    assert!(with.contains("Move to"), "got:\n{with}");
    assert!(
        with.contains("Glowstones"),
        "the other projects are listed:\n{with}"
    );
    assert!(
        with.contains("Edit entry"),
        "the fields stay visible:\n{with}"
    );
}

#[test]
fn the_picker_offers_closed_projects_too_because_that_is_where_a_mistake_hides() {
    let mut app = app_with_history();
    {
        let conn = app.db.conn();
        repo::insert_project(conn, "Shipped Long Ago", None, Status::Completed, None).unwrap();
    }
    to_history(&mut app);
    app.on_key(ch('i')).unwrap();
    focus_edit_field(&mut app, EditField::Project);
    let titles: Vec<&str> = app
        .history
        .edit_choices()
        .iter()
        .map(|c| c.title.as_str())
        .collect();
    assert!(titles.contains(&"Shipped Long Ago"), "got {titles:?}");
}

#[test]
fn the_picker_can_be_filtered_by_typing() {
    let mut app = app_with_history();
    to_history(&mut app);
    select_entry_with_words(&mut app, 253);
    app.on_key(ch('i')).unwrap();
    focus_edit_field(&mut app, EditField::Project);
    app.on_key(ch('i')).unwrap();
    assert_eq!(app.history.edit.as_ref().unwrap().mode, Mode::Filter);
    type_str(&mut app, "glow");
    assert_eq!(app.history.edit_choices().len(), 1);
    // Typing to find a project lands on it; no extra keystroke needed.
    assert_eq!(
        app.history.edit.as_ref().unwrap().project_title,
        "Glowstones"
    );

    // Widening the filter leaves the found project pending rather than snapping
    // back to where it started.
    for _ in 0..4 {
        app.on_key(key(KeyCode::Backspace)).unwrap();
    }
    assert_eq!(app.history.edit.as_ref().unwrap().filter, "");
    assert!(app.history.edit_choices().len() > 1);
    assert_eq!(
        app.history.edit.as_ref().unwrap().project_title,
        "Glowstones"
    );

    // Esc leaves filtering, and h/l then step the narrowed list.
    app.on_key(key(KeyCode::Esc)).unwrap();
    assert_eq!(app.history.edit.as_ref().unwrap().mode, Mode::Normal);
    app.on_key(ch('l')).unwrap();
    assert_ne!(
        app.history.edit.as_ref().unwrap().project_title,
        "Glowstones",
        "l steps on from wherever the filter left the cursor"
    );
}

#[test]
fn leaving_the_project_field_forgets_its_filter() {
    let mut app = app_with_history();
    to_history(&mut app);
    app.on_key(ch('i')).unwrap();
    focus_edit_field(&mut app, EditField::Project);
    app.on_key(ch('i')).unwrap();
    type_str(&mut app, "glow");
    assert_eq!(app.history.edit.as_ref().unwrap().filter, "glow");
    app.on_key(key(KeyCode::Esc)).unwrap();
    app.on_key(key(KeyCode::Tab)).unwrap();
    assert_eq!(
        app.history.edit.as_ref().unwrap().filter,
        "",
        "a filter is a way of finding something, not a setting to carry around"
    );
}

#[test]
fn moving_the_project_counts_as_an_unsaved_change() {
    let mut app = app_with_history();
    to_history(&mut app);
    app.on_key(ch('i')).unwrap();
    focus_edit_field(&mut app, EditField::Project);
    app.on_key(ch('l')).unwrap();
    assert!(app.history.edit.as_ref().unwrap().is_changed());
    assert_eq!(app.on_key(ch('q')).unwrap(), Flow::Continue);
    assert!(
        app.confirming_quit,
        "quitting must not throw the move away silently"
    );
}

/// Not assertions: print the frames so the layouts can be read by eye.
#[test]
fn show_the_entry_move_picker() {
    let mut app = app_with_history();
    to_history(&mut app);
    select_entry_with_words(&mut app, 253);
    app.on_key(ch('i')).unwrap();
    focus_edit_field(&mut app, EditField::Project);
    for line in render(&mut app, 96, 20) {
        println!("|{line}");
    }
}

#[test]
fn show_the_all_time_dashboard() {
    let dir = std::path::Path::new("historic_trackers");
    if !dir.join("Writing Journal 2024.ods").exists() {
        return;
    }
    let mut db = Db::open_in_memory().unwrap();
    draft_tracker::import::run(&mut db, dir, draft_tracker::import::Existing::Refuse).unwrap();
    let mut app = App::new(db).unwrap();
    to_stats(&mut app);
    app.on_key(ch('$')).unwrap();
    for line in render(&mut app, 104, 28) {
        println!("|{line}");
    }
}

/// The status bar is the one line that must never be cut off: it is where the
/// keys are announced.
#[test]
fn the_status_bar_fits_on_every_screen() {
    let mut app = app_with_years();
    for (leader_key, expected) in [
        ('e', Screen::Entry),
        ('h', Screen::History),
        ('p', Screen::Projects),
        ('c', Screen::Categories),
        ('s', Screen::Stats),
    ] {
        app.on_key(ch(' ')).unwrap();
        app.on_key(ch(leader_key)).unwrap();
        assert_eq!(app.screen, expected);
        let lines = render(&mut app, 90, 30);
        let status = lines.last().expect("a status bar");
        assert!(
            status.ends_with("q quit"),
            "{expected:?} status bar is cut off at 90 columns: {status:?}"
        );
    }
}

// ============================================================ the leader menu

#[test]
fn the_leader_menu_lists_what_leader_can_be_followed_by() {
    let mut app = app_with_history();
    app.on_key(ch(' ')).unwrap();
    assert!(app.leader_pending);
    // The event loop decides when to show it; the draw only honours the flag.
    assert!(!app.leader_hint, "not until the writer has paused");

    app.leader_hint = true;
    let screen = render(&mut app, 90, 24).join("\n");
    assert!(screen.contains("<leader>"), "got:\n{screen}");
    for expected in [
        "entry history",
        "projects",
        "categories",
        "stats",
        "submit this entry",
    ] {
        assert!(
            screen.contains(expected),
            "{expected:?} missing from:\n{screen}"
        );
    }
    assert!(
        screen.contains("every key for this screen"),
        "got:\n{screen}"
    );
}

#[test]
fn the_leader_menu_names_what_enter_would_do_on_this_screen() {
    let mut app = app_with_history();
    to_history(&mut app);
    app.on_key(ch('i')).unwrap();
    app.on_key(ch(' ')).unwrap();
    app.leader_hint = true;
    let screen = render(&mut app, 90, 24).join("\n");
    assert!(screen.contains("save the open edit"), "got:\n{screen}");
    assert!(!screen.contains("submit this entry"), "got:\n{screen}");
}

#[test]
fn a_leader_sequence_still_works_while_the_menu_is_showing() {
    let mut app = app_with_history();
    app.on_key(ch(' ')).unwrap();
    app.leader_hint = true;
    // The menu must not swallow the key the way the full help does.
    app.on_key(ch('p')).unwrap();
    assert_eq!(app.screen, Screen::Projects);
    assert!(
        !app.leader_hint,
        "the menu goes once the sequence completes"
    );
    assert!(!app.leader_pending);
}

#[test]
fn escape_dismisses_the_leader_menu_without_doing_anything() {
    let mut app = app_with_history();
    let before = app.screen;
    app.on_key(ch(' ')).unwrap();
    app.leader_hint = true;
    app.on_key(key(KeyCode::Esc)).unwrap();
    assert!(!app.leader_pending);
    assert!(!app.leader_hint);
    assert_eq!(app.screen, before);
    assert!(app.flash.is_none(), "cancelling is not worth a message");
}

#[test]
fn leader_question_mark_opens_the_full_reference() {
    let mut app = app_with_history();
    app.on_key(ch(' ')).unwrap();
    app.leader_hint = true;
    app.on_key(ch('?')).unwrap();
    assert!(app.help_visible);
    assert!(!app.leader_hint);
    let screen = render(&mut app, 90, 30).join("\n");
    assert!(
        screen.contains("Keys"),
        "the full reference, not the menu:\n{screen}"
    );
    assert!(
        screen.contains("No binding uses a control key."),
        "got:\n{screen}"
    );
}

#[test]
fn the_full_help_takes_precedence_over_the_leader_menu() {
    let mut app = app_with_history();
    app.help_visible = true;
    app.leader_hint = true;
    let screen = render(&mut app, 90, 30).join("\n");
    assert!(screen.contains("Keys"), "got:\n{screen}");
    assert!(
        !screen.contains("every key for this screen"),
        "only one popup:\n{screen}"
    );
}

#[test]
fn an_unbound_leader_key_still_reports_itself() {
    let mut app = app_with_history();
    app.on_key(ch(' ')).unwrap();
    app.leader_hint = true;
    app.on_key(ch('z')).unwrap();
    assert!(app.flash.as_ref().unwrap().text.contains("not bound"));
    assert!(!app.leader_hint);
}

#[test]
fn show_the_leader_menu() {
    let mut app = app_with_history();
    app.on_key(ch(' ')).unwrap();
    app.leader_hint = true;
    for line in render(&mut app, 76, 18) {
        println!("|{line}");
    }
}

// ==================================================== a second session writing

/// A database on disk, so two connections can exist at once.
struct TempDb(std::path::PathBuf);

impl TempDb {
    fn new(label: &str) -> Self {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("dt_{label}_{}_{n}.db", std::process::id()));
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
        Self(path)
    }

    fn open(&self) -> Db {
        Db::open(&self.0).unwrap()
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

/// One project with 17,228 words logged, and an app already looking at it.
fn app_on_disk(temp: &TempDb) -> App {
    {
        let db = temp.open();
        let id = repo::insert_project(db.conn(), "Fireside 1", None, Status::Open, None).unwrap();
        repo::insert_entry(
            db.conn(),
            &NewEntry {
                entry_date: day(2026, 1, 12),
                project_id: id,
                session_word_count: 17_228,
                hours: 20.0,
                notes: None,
            },
        )
        .unwrap();
    }
    App::new(temp.open()).unwrap()
}

/// Simulate the command line, or a second interface, on its own connection.
fn other_session_logs(temp: &TempDb, words: i64) {
    let other = temp.open();
    let id = repo::project_id_by_title(other.conn(), "Fireside 1")
        .unwrap()
        .unwrap();
    repo::insert_entry(
        other.conn(),
        &NewEntry {
            entry_date: day(2026, 10, 3),
            project_id: id,
            session_word_count: words,
            hours: 1.0,
            notes: None,
        },
    )
    .unwrap();
}

fn type_into(app: &mut App, field: Field, text: &str) {
    focus(app, field);
    app.on_key(ch('i')).unwrap();
    type_str(app, text);
    app.on_key(key(KeyCode::Esc)).unwrap();
}

/// The scenario: an interface left open in one terminal while a session is
/// logged from another. The session count is a difference against the project's
/// total, so a stale total would inflate it without complaint.
#[test]
fn a_submit_against_a_stale_total_is_refused_and_recalculated() {
    let temp = TempDb::new("stale");
    let mut app = app_on_disk(&temp);
    select_title(&mut app, "Fireside 1");
    assert_eq!(app.form.selected_project().unwrap().logged_words, 17_228);

    other_session_logs(&temp, 672);

    // The writer types the manuscript's true new total, expecting 100 words.
    type_into(&mut app, Field::Hours, "1");
    type_into(&mut app, Field::ProjectWords, "18000");
    // Against the stale total this previews 772, which is the bug being guarded.
    assert_eq!(app.form.preview().session_words, Some(772));

    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();

    let flash = app.flash.as_ref().expect("a warning");
    assert_eq!(flash.tone, Tone::Warning);
    assert!(
        flash.text.contains("another session"),
        "got {:?}",
        flash.text
    );
    assert_eq!(
        repo::recent_entries(app.db.conn(), 10).unwrap().len(),
        2,
        "the refused submit wrote nothing"
    );
    // The figures now reflect reality, and the number on screen is the number
    // that would be written.
    assert_eq!(app.form.selected_project().unwrap().logged_words, 17_900);
    assert_eq!(app.form.preview().session_words, Some(100));
}

#[test]
fn submitting_again_writes_the_corrected_session() {
    let temp = TempDb::new("corrected");
    let mut app = app_on_disk(&temp);
    select_title(&mut app, "Fireside 1");
    other_session_logs(&temp, 672);
    type_into(&mut app, Field::Hours, "1");
    type_into(&mut app, Field::ProjectWords, "18000");

    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap(); // refused, recalculated
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap(); // accepted

    assert_eq!(app.flash.as_ref().unwrap().tone, Tone::Success);
    let entries = repo::recent_entries(app.db.conn(), 10).unwrap();
    assert_eq!(entries.len(), 3);
    assert!(
        entries.iter().any(|e| e.session_word_count == 100),
        "the written session is the corrected one: {entries:?}"
    );
    assert!(
        !entries.iter().any(|e| e.session_word_count == 772),
        "the inflated figure must never reach the database"
    );
    assert_eq!(
        repo::project_logged_total(app.db.conn(), 1).unwrap(),
        18_000
    );
}

/// The check must not fire on this session's own writes, or every second entry
/// of the day would be refused once for no reason.
#[test]
fn two_submits_in_a_row_are_not_mistaken_for_outside_interference() {
    let temp = TempDb::new("own");
    let mut app = app_on_disk(&temp);
    select_title(&mut app, "Fireside 1");

    type_into(&mut app, Field::Hours, "1");
    type_into(&mut app, Field::ProjectWords, "17500");
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(
        app.flash.as_ref().unwrap().tone,
        Tone::Success,
        "first submit"
    );

    select_title(&mut app, "Fireside 1");
    type_into(&mut app, Field::Hours, "1");
    type_into(&mut app, Field::ProjectWords, "18000");
    app.on_key(ch(' ')).unwrap();
    app.on_key(key(KeyCode::Enter)).unwrap();
    assert_eq!(
        app.flash.as_ref().unwrap().tone,
        Tone::Success,
        "a second submit must go straight through: {:?}",
        app.flash
    );
    assert_eq!(
        repo::project_logged_total(app.db.conn(), 1).unwrap(),
        18_000
    );
}

#[test]
fn changing_screen_picks_up_what_another_session_wrote() {
    let temp = TempDb::new("screen");
    let mut app = app_on_disk(&temp);
    to_history(&mut app);
    assert_eq!(app.history.visible().len(), 1);

    other_session_logs(&temp, 672);

    app.on_key(ch(' ')).unwrap();
    app.on_key(ch('p')).unwrap();
    assert_eq!(app.screen, Screen::Projects);
    assert!(
        app.flash.as_ref().unwrap().text.contains("another session"),
        "got {:?}",
        app.flash
    );
    assert_eq!(
        app.projects
            .visible()
            .iter()
            .find(|r| r.project.title == "Fireside 1")
            .unwrap()
            .metrics
            .total_word_count,
        17_900
    );

    to_history(&mut app);
    assert_eq!(app.history.visible().len(), 2, "the new entry is listed");
}

#[test]
fn changing_screen_says_nothing_when_nothing_changed_outside() {
    let temp = TempDb::new("quiet");
    let mut app = app_on_disk(&temp);
    to_history(&mut app);
    app.on_key(ch(' ')).unwrap();
    app.on_key(ch('p')).unwrap();
    assert!(
        app.flash.is_none(),
        "no news is not worth a message: {:?}",
        app.flash
    );
}
