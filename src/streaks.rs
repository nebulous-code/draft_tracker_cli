//! Runs of consecutive days with something logged.
//!
//! The arithmetic lives here rather than in SQL because the interesting part is
//! not the aggregation but what counts as a break in a run, and that reads far
//! better as a loop over dates than as a window function. It is also pure, so it
//! can be tested against hand-written calendars without a database.
//!
//! Both functions take the set of days that have an entry. Any entry counts: a
//! revision session that cut words still kept the appointment, and a streak is a
//! record of showing up rather than of output.
//!
//! Neither function knows about years. A year's longest streak is the longest run
//! among that year's days alone, which the caller gets by handing over only those
//! days. That is what clips a run at the year boundary: 31 December and 1 January
//! are consecutive on the calendar but cannot both be in one year's set, so a
//! streak that crosses new year contributes its two halves to the two years and
//! its whole length to the lifetime figure.

use chrono::NaiveDate;

/// Ascending and without repeats, which is what both loops below assume.
///
/// The queries that feed these functions already return `DISTINCT ... ORDER BY`,
/// so this is normally a copy. It is done anyway so that neither function can be
/// made to report nonsense by a caller that assembled its dates some other way.
fn in_order(days: &[NaiveDate]) -> Vec<NaiveDate> {
    let mut days = days.to_vec();
    days.sort_unstable();
    days.dedup();
    days
}

/// The longest run of consecutive days in `days`.
///
/// A single day is a run of one. An empty set is zero rather than one, because
/// "longest streak: 1 day" on a journal with nothing in it would be a claim about
/// a day that was never written.
pub fn longest(days: &[NaiveDate]) -> u32 {
    let mut best = 0;
    let mut run = 0;
    let mut previous: Option<NaiveDate> = None;
    for day in in_order(days) {
        run = match previous {
            Some(p) if p.succ_opt() == Some(day) => run + 1,
            _ => 1,
        };
        best = best.max(run);
        previous = Some(day);
    }
    best
}

/// The run of consecutive days still alive as of `today`.
///
/// Today not being written yet does not break the streak — the day is not over.
/// A run therefore counts as current if it reaches either today or yesterday, and
/// logging today extends it rather than starting it again. Written Monday to
/// Sunday and read on Monday, this is seven; log that Monday and it is eight; read
/// on Tuesday with Monday still empty, it is zero, because by then a whole day has
/// gone by unwritten.
///
/// Days after `today` are ignored. Entry dates are not checked against the
/// calendar, so a journal can hold one, and a date that has not arrived cannot
/// extend a streak that has not been lived. Such a day still counts towards its
/// own year's [`longest`], where it is a fact about that year's data rather than a
/// claim about now.
pub fn current(days: &[NaiveDate], today: NaiveDate) -> u32 {
    let days = in_order(days);
    let mut lived = days.iter().rev().filter(|d| **d <= today);

    let Some(end) = lived.next().copied() else {
        return 0;
    };
    if end != today && Some(end) != today.pred_opt() {
        return 0;
    }

    let mut run = 1;
    let mut expected = end.pred_opt();
    for day in lived {
        if Some(*day) != expected {
            break;
        }
        run += 1;
        expected = day.pred_opt();
    }
    run
}
