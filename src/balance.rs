//! The game/chat balance, the same on every OS: what a move of the slider comes to, keeping the
//! outputs up to date, and logging their problems.

use std::sync::Mutex;

use crate::audio::GameChat;
use crate::hid::log;
use crate::settings::Settings;

static LAST_PROBLEM: Mutex<String> = Mutex::new(String::new());

/// Whether `problem` differs from the last one, which it then becomes.
fn is_new(last: &mut String, problem: &str) -> bool {
    let new = *last != problem;
    if new {
        *last = problem.to_string();
    }
    new
}

/// Writes a problem to the log, unless it's the one written last: the callers retry on every
/// status update. Keep output names out of it, since they can hold a serial number and the log
/// goes into device reports.
pub fn log_problem(problem: &str) {
    if is_new(&mut LAST_PROBLEM.lock().unwrap(), problem) {
        log(problem);
    }
}

/// The outputs work again, so the same problem is worth logging if it comes back.
pub fn problem_solved() {
    LAST_PROBLEM.lock().unwrap().clear();
}

/// What moving the slider comes to.
#[derive(Debug, PartialEq)]
pub struct Step {
    /// The new position of the slider.
    pub value: i32,
    /// The volumes when the balance is centered.
    pub base: (f32, f32),
    /// The volumes for this position.
    pub levels: (f32, f32),
}

/// Works out a move of the slider to `value` from `balance`. `None` if it needs the current volumes
/// and `read` can't get them: there's nothing to work the new ones out from, and a failed read says
/// nothing about what they are.
///
/// Leaving the center reads them again, in case they were changed in the system, unless `settled`
/// says a write is still on its way: what's there is then on its way to being `base` already.
pub fn step(value: i32, balance: i32, base: Option<(f32, f32)>, settled: bool, read: impl FnOnce() -> Option<(f32, f32)>) -> Option<Step> {
    let value = if value.abs() < 6 { 0 } else { value.clamp(-100, 100) };
    let base = match base {
        Some(base) if balance != 0 || value == 0 || !settled => base,
        _ => read()?,
    };
    let levels = (base.0 * (1.0 - value.max(0) as f32 / 100.0), base.1 * (1.0 - (-value).max(0) as f32 / 100.0));
    Some(Step { value, base, levels })
}

/// Brings `levels` up to date. Outputs that can't be read, or that refused a write, are dropped: the
/// window then says they're missing, and the next status update looks for them again. Returns
/// whether a write failed, in which case the volumes aren't what the slider says.
pub fn refresh(audio: &mut Option<GameChat>, levels: &mut (f32, f32)) -> bool {
    let Some(outputs) = audio else { return false };
    if outputs.take_write_failure() {
        *audio = None;
        return true;
    }
    match outputs.get() {
        Some(now) => *levels = now,
        None => *audio = None,
    }
    false
}

/// After a failed write: whatever the volumes are now becomes the new center, read on the next move.
pub fn recenter(settings: &mut Settings) {
    settings.balance = 0;
    settings.balance_base = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: (f32, f32) = (0.8, 0.4);

    #[test]
    fn a_failed_read_changes_nothing() {
        assert_eq!(step(50, 0, None, true, || None), None);
        assert_eq!(step(50, 0, Some(BASE), true, || None), None);
    }

    #[test]
    fn leaving_the_center_reads_the_volumes_again() {
        let now = (0.5, 0.5);
        assert_eq!(step(50, 0, Some(BASE), true, || Some(now)), Some(Step { value: 50, base: now, levels: (0.25, 0.5) }));
    }

    #[test]
    fn a_write_on_its_way_is_not_read_as_the_base() {
        // Dragged across the center: the restore hasn't reached the output, which is still low
        let step = step(50, 0, Some(BASE), false, || Some((0.1, 0.1))).unwrap();
        assert_eq!(step.base, BASE);
        assert_eq!(step.levels, (0.4, 0.4));
    }

    #[test]
    fn with_no_base_it_has_to_read_even_if_unsettled() {
        assert_eq!(step(0, 0, None, false, || Some(BASE)).unwrap().base, BASE);
        assert_eq!(step(0, 0, None, false, || None), None);
    }

    #[test]
    fn moving_off_center_keeps_the_base_without_reading() {
        let unread = || panic!("no read needed");
        assert_eq!(step(-100, 50, Some(BASE), true, unread), Some(Step { value: -100, base: BASE, levels: (0.8, 0.0) }));
        assert_eq!(step(3, 50, Some(BASE), true, unread), Some(Step { value: 0, base: BASE, levels: BASE }));
    }

    #[test]
    fn position_stays_in_range() {
        assert_eq!(step(500, 50, Some(BASE), true, || None).unwrap().value, 100);
    }

    #[test]
    fn a_problem_is_new_until_it_repeats_or_is_solved() {
        let mut last = String::new();
        assert!(is_new(&mut last, "down"));
        assert!(!is_new(&mut last, "down"));
        assert!(is_new(&mut last, "other"));
        last.clear();
        assert!(is_new(&mut last, "other"));
    }

    #[test]
    fn recentering_forgets_the_base() {
        let mut settings = Settings { balance: 40, balance_base: Some(BASE), ..Settings::default() };
        recenter(&mut settings);
        assert_eq!((settings.balance, settings.balance_base), (0, None));
    }
}
