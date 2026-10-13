//! Streak arithmetic, checked against hand-written calendars.
//!
//! These run without a database on purpose. The rules about what breaks a run and
//! what a run is measured against are the whole feature, and they are easier to
//! be exhaustive about when a test case is a list of dates.

use chrono::{Datelike, NaiveDate};
use draft_tracker::streaks;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// `n` consecutive days starting at `start`.
fn run(start: NaiveDate, n: u32) -> Vec<NaiveDate> {
    (0..n as i64)
        .map(|i| start + chrono::Duration::days(i))
        .collect()
}

// ------------------------------------------------------------ longest

#[test]
fn an_empty_journal_has_no_streak() {
    assert_eq!(streaks::longest(&[]), 0);
}

#[test]
fn a_single_day_is_a_streak_of_one() {
    assert_eq!(streaks::longest(&[day(2026, 3, 4)]), 1);
}

#[test]
fn consecutive_days_accumulate() {
    assert_eq!(streaks::longest(&run(day(2026, 3, 1), 9)), 9);
}

#[test]
fn a_missed_day_breaks_the_run() {
    let days = [day(2026, 3, 1), day(2026, 3, 2), day(2026, 3, 4)];
    assert_eq!(streaks::longest(&days), 2);
}

#[test]
fn the_longest_of_several_runs_wins_wherever_it_sits() {
    let mut days = run(day(2026, 1, 1), 3);
    days.extend(run(day(2026, 2, 1), 11));
    days.extend(run(day(2026, 3, 1), 4));
    assert_eq!(streaks::longest(&days), 11);
}

#[test]
fn a_run_crosses_a_month_boundary() {
    assert_eq!(streaks::longest(&run(day(2026, 1, 28), 8)), 8);
}

/// February is the one month whose length a naive "+1 day" might get wrong.
#[test]
fn a_run_crosses_the_end_of_february_in_a_leap_year() {
    assert_eq!(streaks::longest(&run(day(2024, 2, 27), 5)), 5);
}

#[test]
fn two_sessions_on_one_day_are_one_day_written() {
    let days = [day(2026, 3, 1), day(2026, 3, 1), day(2026, 3, 2)];
    assert_eq!(streaks::longest(&days), 2);
}

#[test]
fn dates_handed_over_out_of_order_still_read_correctly() {
    let days = [day(2026, 3, 3), day(2026, 3, 1), day(2026, 3, 2)];
    assert_eq!(streaks::longest(&days), 3);
}

// ------------------------------------------------------------ the year boundary

/// The behaviour asked for: a run that crosses new year gives each year its own
/// half and the lifetime figure the whole thing. Clipping falls out of handing the
/// function only one year's days, so this is checked as the caller will use it.
#[test]
fn a_streak_across_new_year_splits_between_the_years_but_not_in_lifetime() {
    // 1 November 2025 to 29 January 2026: 61 days in 2025, 29 in 2026, 90 in all.
    let days = run(day(2025, 11, 1), 90);
    let in_2025: Vec<_> = days.iter().copied().filter(|d| d.year() == 2025).collect();
    let in_2026: Vec<_> = days.iter().copied().filter(|d| d.year() == 2026).collect();

    assert_eq!(streaks::longest(&days), 90, "lifetime sees the whole run");
    assert_eq!(streaks::longest(&in_2025), 61);
    assert_eq!(streaks::longest(&in_2026), 29);
    assert_eq!(in_2025.len() + in_2026.len(), 90, "no day counted twice");
}

// ------------------------------------------------------------ current

#[test]
fn nothing_logged_at_all_is_a_streak_of_zero() {
    assert_eq!(streaks::current(&[], day(2026, 3, 10)), 0);
}

/// The case from the brief: written Monday to Sunday, read on the following
/// Monday before anything is logged. The day is not over, so the run stands.
#[test]
fn today_being_unwritten_does_not_break_the_streak() {
    let week = run(day(2026, 3, 2), 7); // Monday 2 March to Sunday 8 March
    assert_eq!(streaks::current(&week, day(2026, 3, 9)), 7);
}

#[test]
fn logging_today_extends_the_streak_rather_than_restarting_it() {
    let mut week = run(day(2026, 3, 2), 7);
    week.push(day(2026, 3, 9));
    assert_eq!(streaks::current(&week, day(2026, 3, 9)), 8);
}

/// One whole day gone by unwritten is what ends a run.
#[test]
fn a_full_missed_day_resets_the_streak_to_zero() {
    let week = run(day(2026, 3, 2), 7);
    assert_eq!(streaks::current(&week, day(2026, 3, 10)), 0);
}

#[test]
fn a_streak_of_one_counts_whether_it_is_today_or_yesterday() {
    assert_eq!(streaks::current(&[day(2026, 3, 9)], day(2026, 3, 9)), 1);
    assert_eq!(streaks::current(&[day(2026, 3, 9)], day(2026, 3, 10)), 1);
}

#[test]
fn only_the_run_that_reaches_now_is_counted() {
    let mut days = run(day(2026, 1, 1), 30);
    days.extend(run(day(2026, 3, 7), 3)); // ends Monday 9 March
    assert_eq!(
        streaks::current(&days, day(2026, 3, 9)),
        3,
        "a longer run earlier in the year is not the current one"
    );
}

#[test]
fn a_current_streak_counts_back_across_new_year() {
    let days = run(day(2025, 12, 28), 10); // to 6 January 2026
    assert_eq!(streaks::current(&days, day(2026, 1, 6)), 10);
}

// ------------------------------------------------------------ future dates

/// Entry dates are not checked against the calendar, so one can sit in the
/// future. It cannot extend a run that has not been lived.
#[test]
fn a_future_date_does_not_extend_the_current_streak() {
    let mut days = run(day(2026, 3, 7), 3); // to Monday 9 March
    days.push(day(2026, 12, 25));
    assert_eq!(streaks::current(&days, day(2026, 3, 9)), 3);
}

#[test]
fn a_future_date_cannot_bridge_a_gap_that_already_broke_the_streak() {
    let mut days = run(day(2026, 3, 2), 7); // to Sunday 8 March
    days.push(day(2026, 3, 11)); // tomorrow, from the reader's point of view
    assert_eq!(
        streaks::current(&days, day(2026, 3, 10)),
        0,
        "Monday 9 March was missed; a date past today does not cover for it"
    );
}

#[test]
fn a_journal_holding_only_future_dates_has_no_current_streak() {
    assert_eq!(
        streaks::current(&run(day(2027, 1, 1), 5), day(2026, 3, 9)),
        0
    );
}

/// The other half of the rule: a future day is still a fact about its own year's
/// data, so it is left in the longest-streak figure rather than filtered out.
#[test]
fn a_future_date_still_counts_towards_a_longest_streak() {
    assert_eq!(streaks::longest(&run(day(2027, 1, 1), 5)), 5);
}
