//! Kodi's highlight on the wall, as last reported or as a d-pad press will move it, so the wall moves on the press
//! rather than when Kodi next answers. Kodi's panel stops at the end of a row and moves a whole row up or down.

use crate::api::RemoteKey;
use std::time::{Duration, Instant};

/// How long a predicted move stands against Kodi's answers, which may predate the key press.
const HOLD: Duration = Duration::from_millis(400);

#[derive(Debug, Default)]
pub(crate) struct Cursor {
    at: usize,
    /// Titles on the wall; 0 while it isn't showing.
    len: usize,
    /// Kodi's column for the highlight; `None` until it has said.
    column: Option<usize>,
    /// Kodi's posters per row, once a report (or a vertical move) has shown it.
    columns: Option<usize>,
    /// Until when a prediction stands.
    held: Option<Instant>,
    /// Where an up or down press left from before the row width was known; the move it makes gives the width.
    vertical: Option<usize>,
}

/// Where Kodi says its highlight is.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Report {
    pub(crate) at: usize,
    pub(crate) len: usize,
    /// Kodi's `Container.Column`, `Row` and `Position` (the highlight among the cells on screen); -1 when unknown.
    pub(crate) column: i64,
    pub(crate) row: i64,
    pub(crate) position: i64,
}

impl Cursor {
    /// A d-pad key went to Kodi: where its highlight will go, when that's known.
    pub(crate) fn predict(&mut self, action: RemoteKey, now: Instant) -> Option<usize> {
        let (at, len, col) = (self.at, self.len, self.column?);
        if len == 0 {
            return None;
        }
        let (at, column) = match (action, self.columns) {
            (RemoteKey::Right, cols) if at + 1 < len && cols.is_none_or(|c| col + 1 < c) => (at + 1, col + 1),
            (RemoteKey::Left, _) if at > 0 && col > 0 => (at - 1, col - 1),
            (RemoteKey::Down, Some(cols)) if at + cols < len => (at + cols, col),
            (RemoteKey::Up, Some(cols)) if at >= cols => (at - cols, col),
            (RemoteKey::Up | RemoteKey::Down, None) => {
                self.vertical = Some(at);
                return None;
            }
            _ => return None,
        };
        (self.at, self.column, self.held) = (at, Some(column), Some(now + HOLD));
        Some(at)
    }

    /// Kodi's own report. `false` when it disagrees with a prediction that still stands: it may predate the press,
    /// so it's asked again shortly rather than shown.
    pub(crate) fn report(&mut self, r: Report, now: Instant) -> bool {
        self.len = r.len;
        if self.held.is_some_and(|t| now < t) && r.at != self.at {
            return false;
        }
        self.held = None;
        self.at = r.at;
        self.column = usize::try_from(r.column).ok();
        if r.row > 0 && r.column >= 0 && r.position >= r.column {
            self.columns = usize::try_from((r.position - r.column) / r.row).ok();
        } else if let Some(from) = self.vertical.filter(|&f| f != r.at) {
            self.columns = Some(from.abs_diff(r.at));
        }
        if self.columns.is_some() {
            self.vertical = None;
        }
        true
    }

    /// The wall is gone: no prediction until Kodi reports again.
    pub(crate) fn hide(&mut self) {
        self.len = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use RemoteKey::*;

    fn at(at: usize, column: i64, row: i64, position: i64) -> Report {
        Report { at, len: 30, column, row, position }
    }

    #[test]
    fn moves_within_a_row_and_stops_at_its_end() {
        let (mut c, t) = (Cursor::default(), Instant::now());
        assert_eq!(c.predict(Right, t), None, "nothing known yet");
        assert!(c.report(at(10, 4, 1, 10), t));
        assert_eq!(c.predict(Right, t), Some(11), "six to a row: column 5 is the last");
        assert_eq!(c.predict(Right, t), None);
        assert_eq!(c.predict(Down, t), Some(17));
        assert_eq!(c.predict(Up, t), Some(11));
        assert_eq!(c.predict(Left, t), Some(10));
    }

    #[test]
    fn a_vertical_move_from_the_top_row_teaches_the_width() {
        let (mut c, t) = (Cursor::default(), Instant::now());
        assert!(c.report(at(2, 2, 0, 2), t));
        assert_eq!(c.predict(Down, t), None, "the row width is unknown on the top row");
        assert!(c.report(at(8, 2, 1, 8), t + HOLD));
        assert_eq!(c.predict(Down, t + HOLD), Some(14));
    }

    #[test]
    fn stale_reports_wait_out_a_prediction() {
        let (mut c, t) = (Cursor::default(), Instant::now());
        assert!(c.report(at(10, 4, 1, 10), t));
        assert_eq!(c.predict(Right, t), Some(11));
        assert!(!c.report(at(10, 4, 1, 10), t + Duration::from_millis(100)), "from before the press");
        assert!(c.report(at(11, 5, 1, 11), t + Duration::from_millis(150)), "agrees");
        c.hide();
        assert_eq!(c.predict(Left, t), None);
    }
}
