//! Values asked from the headset at most once per interval (battery, settings): each
//! unanswered query waits up to 1 s, and the state is read every few seconds.

use std::cell::Cell;
use std::time::{Duration, Instant};

pub struct Every<T> {
    every: Duration,
    last: Cell<Option<(Instant, T)>>,
}

impl<T: Copy> Every<T> {
    pub const fn new(every: Duration) -> Self {
        Every { every, last: Cell::new(None) }
    }

    /// The last value if recent; otherwise the one from `read`, which is stored even if it says the
    /// headset did not reply (so each state read does not wait again). If `read` could not ask
    /// (something transient: None), the previous one is kept and it is retried next time.
    pub fn get(&self, read: impl FnOnce() -> Option<T>) -> Option<T> {
        match self.last.get() {
            Some((at, v)) if at.elapsed() < self.every => Some(v),
            last => match read() {
                Some(v) => {
                    self.set(v);
                    Some(v)
                }
                None => last.map(|(_, v)| v),
            },
        }
    }

    /// Records a known value (e.g. what was just set).
    pub fn set(&self, v: T) {
        self.last.set(Some((Instant::now(), v)));
    }

    /// Forgets the value (the headset was turned off).
    pub fn clear(&self) {
        self.last.set(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_only_when_old() {
        let e = Every::new(Duration::from_secs(60));
        assert_eq!(e.get(|| Some(1)), Some(1));
        // Recent: not asked
        assert_eq!(e.get(|| panic!("should not ask")), Some(1));
        e.set(2);
        assert_eq!(e.get(|| None), Some(2));
        e.clear();
        assert_eq!(e.get(|| None), None);
    }

    #[test]
    fn a_failed_read_keeps_the_last_value() {
        let e = Every::new(Duration::ZERO);
        assert_eq!(e.get(|| Some(5)), Some(5));
        // Expired and unable to ask: the previous one
        assert_eq!(e.get(|| None), Some(5));
        assert_eq!(e.get(|| Some(6)), Some(6));
    }
}
