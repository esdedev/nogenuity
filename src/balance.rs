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

/// The outputs work again, so the same problem is worth logging if it comes back. Only for a status
/// update to say: with a headset unplugged, one drag logged the same failed write ten times when
/// any success in between counted.
fn problem_solved() {
    LAST_PROBLEM.lock().unwrap().clear();
}

/// What a move of the slider asks for.
#[derive(Debug, PartialEq)]
pub enum Move {
    /// It was centered and still is, as when grabbing it there: nothing to write, or the volumes
    /// would go back to a base that may be out of date.
    Stay,
    To(Step),
    /// It needs the current volumes and can't read them: there's nothing to work the new ones out
    /// from, and a failed read says nothing about what they are.
    Unreadable,
}

/// The slider's new position and what it comes to.
#[derive(Debug, PartialEq)]
pub struct Step {
    /// The new position of the slider.
    pub value: i32,
    /// The volumes when the balance is centered.
    pub base: (f32, f32),
    /// The volumes for this position.
    pub levels: (f32, f32),
}

/// Works out a move of the slider to `value` from `balance`.
///
/// Leaving the center reads them again, in case they were changed in the system, unless `settled`
/// says a write is still on its way: what's there is then on its way to being `base` already.
pub fn step(value: i32, balance: i32, base: Option<(f32, f32)>, settled: bool, read: impl FnOnce() -> Option<(f32, f32)>) -> Move {
    let value = if value.abs() < 6 { 0 } else { value.clamp(-100, 100) };
    if value == 0 && balance == 0 {
        return Move::Stay;
    }
    let base = match base {
        Some(base) if balance != 0 || value == 0 || !settled => base,
        _ => match read() {
            Some(base) => base,
            None => return Move::Unreadable,
        },
    };
    Move::To(Step { value, base, levels: levels_at(value, base) })
}

/// The volumes for a position of the slider.
fn levels_at(value: i32, base: (f32, f32)) -> (f32, f32) {
    (base.0 * (1.0 - value.max(0) as f32 / 100.0), base.1 * (1.0 - (-value).max(0) as f32 / 100.0))
}

/// Brings `levels` up to date. Outputs that can't be read, or that refused a write, are dropped: the
/// window then says they're missing, and the next status update looks for them again.
///
/// A refused write leaves the volumes different from what the slider says, which `unapplied` keeps
/// in mind: once outputs are back they get what the slider says. Taking whatever they're at as the
/// new center instead would lose the base for good, since they may be lowered by the balance.
pub fn refresh(audio: &mut Option<GameChat>, levels: &mut (f32, f32), unapplied: &mut bool, settings: &Settings) {
    let Some(outputs) = audio else { return };
    if outputs.take_write_failure() {
        *audio = None;
        *unapplied = true;
        return;
    }
    if std::mem::take(unapplied)
        && let Some(wanted) = wanted_levels(settings)
    {
        outputs.set(wanted.0, wanted.1);
        *levels = wanted;
        return;
    }
    match outputs.get() {
        Some(now) => {
            *levels = now;
            problem_solved();
        }
        None => *audio = None,
    }
}

/// The volumes the saved balance asks for, if it has a base to work from.
fn wanted_levels(settings: &Settings) -> Option<(f32, f32)> {
    Some(levels_at(settings.balance, settings.balance_base?))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: (f32, f32) = (0.8, 0.4);

    fn moved(step: Move) -> Step {
        match step {
            Move::To(step) => step,
            other => panic!("expected a move, got {other:?}"),
        }
    }

    #[test]
    fn a_failed_read_changes_nothing() {
        assert_eq!(step(50, 0, None, true, || None), Move::Unreadable);
        assert_eq!(step(50, 0, Some(BASE), true, || None), Move::Unreadable);
    }

    /// The volumes were changed in the system while centered, and the slider gets grabbed where it
    /// is: writing the base here would undo the change, and the next move would read that back.
    #[test]
    fn grabbing_it_at_the_center_writes_nothing() {
        let unread = || panic!("no read needed");
        for value in [-5, 0, 5] {
            assert_eq!(step(value, 0, Some(BASE), true, unread), Move::Stay, "{value}");
            assert_eq!(step(value, 0, None, true, unread), Move::Stay, "{value}");
        }
    }

    #[test]
    fn a_change_in_the_system_while_centered_becomes_the_base() {
        let changed = (0.1, 0.4);
        assert_eq!(step(0, 0, Some(BASE), true, || Some(changed)), Move::Stay);
        let step = moved(step(50, 0, Some(BASE), true, || Some(changed)));
        assert_eq!(step.base, changed);
        assert_eq!(step.levels, (0.05, 0.4));
    }

    #[test]
    fn leaving_the_center_reads_the_volumes_again() {
        let now = (0.5, 0.5);
        assert_eq!(step(50, 0, Some(BASE), true, || Some(now)), Move::To(Step { value: 50, base: now, levels: (0.25, 0.5) }));
    }

    #[test]
    fn a_write_on_its_way_is_not_read_as_the_base() {
        // Dragged across the center: the restore hasn't reached the output, which is still low
        let step = moved(step(50, 0, Some(BASE), false, || Some((0.1, 0.1))));
        assert_eq!(step.base, BASE);
        assert_eq!(step.levels, (0.4, 0.4));
    }

    #[test]
    fn with_no_base_it_has_to_read_even_if_unsettled() {
        assert_eq!(moved(step(50, 0, None, false, || Some(BASE))).base, BASE);
        assert_eq!(step(50, 0, None, false, || None), Move::Unreadable);
    }

    #[test]
    fn moving_off_center_keeps_the_base_without_reading() {
        let unread = || panic!("no read needed");
        assert_eq!(step(-100, 50, Some(BASE), true, unread), Move::To(Step { value: -100, base: BASE, levels: (0.8, 0.0) }));
        assert_eq!(step(3, 50, Some(BASE), true, unread), Move::To(Step { value: 0, base: BASE, levels: BASE }));
    }

    #[test]
    fn position_stays_in_range() {
        assert_eq!(moved(step(500, 50, Some(BASE), true, || None)).value, 100);
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

    /// Unplugged with the slider to one side: what comes back is that position over the same base,
    /// so returning to the center still gives the volumes from before the balance lowered them.
    #[test]
    fn what_a_failed_write_leaves_to_apply_keeps_the_base() {
        let settings = Settings { balance: -50, balance_base: Some(BASE), ..Settings::default() };
        assert_eq!(wanted_levels(&settings), Some((0.8, 0.2)));
    }

    #[test]
    fn with_no_base_there_is_nothing_to_apply() {
        let settings = Settings { balance: 0, balance_base: None, ..Settings::default() };
        assert_eq!(wanted_levels(&settings), None);
    }
}
