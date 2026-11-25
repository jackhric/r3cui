//! A value asked for here and shown straight away, in place of the device's report, until the device reports it too
//! or gives up. The receiver can take seconds to report a change (a minute for power), and the screen shouldn't
//! flick back to the old value meanwhile.

use std::time::{Duration, Instant};

pub(crate) struct Optimistic<T> {
    want: Option<(T, Instant)>,
    /// How long the asked value wins outright, whatever is reported: the slider's reports pass through levels on the
    /// way, one of which can match for a moment.
    hold: Duration,
    /// How long it wins while the report still differs.
    timeout: Duration,
}

impl<T: PartialEq + Clone> Optimistic<T> {
    pub(crate) const fn new(hold: Duration, timeout: Duration) -> Self {
        Self { want: None, hold, timeout }
    }

    pub(crate) fn ask(&mut self, value: T) {
        self.want = Some((value, Instant::now()));
    }

    /// What to show, given what the device reports.
    pub(crate) fn resolve(&self, reported: T) -> T {
        match &self.want {
            Some((want, _)) if self.pending(&reported) => want.clone(),
            _ => reported,
        }
    }

    /// Whether the asked value still stands against this report.
    pub(crate) fn pending(&self, reported: &T) -> bool {
        self.want.as_ref().is_some_and(|(want, at)| {
            let age = at.elapsed();
            age < self.hold || (want != reported && age < self.timeout)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asked_value_wins_until_reported_or_timed_out() {
        let mut o = Optimistic::new(Duration::ZERO, Duration::from_secs(60));
        assert_eq!(o.resolve(3), 3);
        o.ask(7);
        assert_eq!(o.resolve(3), 7);
        assert!(o.pending(&3));
        assert_eq!(o.resolve(7), 7);
        assert!(!o.pending(&7));
        let mut gone = Optimistic::new(Duration::ZERO, Duration::ZERO);
        gone.ask(7);
        assert_eq!(gone.resolve(3), 3);
    }

    #[test]
    fn hold_wins_even_over_a_matching_report() {
        let mut o = Optimistic::new(Duration::from_secs(60), Duration::from_secs(60));
        o.ask(7);
        assert!(o.pending(&7));
    }
}
