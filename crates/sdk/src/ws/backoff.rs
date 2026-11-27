use std::time::Duration;

const STEPS: [Duration; 5] = [Duration::from_millis(500), Duration::from_secs(1), Duration::from_secs(2), Duration::from_secs(4), Duration::from_secs(8)];
const SETTLED: Duration = Duration::from_secs(15);

/// A connection that stayed up this long is healthy: the next failure starts the backoff over.
pub const STABLE_AFTER: Duration = Duration::from_secs(30);

/// How long to wait after failed attempt number `attempt` (counting from 1): 0.5, 1, 2, 4, 8 s, then 15 s.
pub fn delay_for(attempt: u32) -> Duration {
    STEPS.get(attempt.saturating_sub(1) as usize).copied().unwrap_or(SETTLED)
}

#[derive(Debug, Default, Clone)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts the next attempt and returns its number, from 1.
    pub fn begin_attempt(&mut self) -> u32 {
        self.attempt += 1;
        self.attempt
    }

    /// The wait after the current attempt failed or its connection ended.
    pub fn delay(&self) -> Duration {
        delay_for(self.attempt)
    }

    /// Reports that a connection ended after `up_for`; one that lasted starts the backoff over.
    pub fn connection_ended(&mut self, up_for: Duration) {
        if up_for > STABLE_AFTER {
            self.attempt = 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_then_settles() {
        let secs = |a| delay_for(a).as_secs_f64();
        assert_eq!([secs(0), secs(1), secs(2), secs(3), secs(4), secs(5), secs(6), secs(60)], [0.5, 0.5, 1.0, 2.0, 4.0, 8.0, 15.0, 15.0]);
    }

    #[test]
    fn counts_attempts_and_a_long_connection_starts_over() {
        let mut b = Backoff::new();
        assert_eq!([b.begin_attempt(), b.begin_attempt(), b.begin_attempt()], [1, 2, 3]);
        assert_eq!(b.delay(), Duration::from_secs(2));
        b.connection_ended(STABLE_AFTER);
        assert_eq!(b.delay(), Duration::from_secs(2), "exactly the limit is not long enough");
        b.connection_ended(STABLE_AFTER + Duration::from_millis(1));
        assert_eq!(b.delay(), Duration::from_millis(500));
        assert_eq!(b.begin_attempt(), 2);
    }
}
