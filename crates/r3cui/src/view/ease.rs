//! Easing along a cubic Bézier, as CSS's `cubic-bezier()` and Slint's: for curves tuned elsewhere, kept exact.

/// `[x1, y1, x2, y2]`: the two control points, from (0, 0) to (1, 1). A `y` outside 0..1 overshoots.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bezier(pub [f32; 4]);

impl Bezier {
    /// The eased value at `t`, held to 0 before the start and 1 after the end.
    pub fn at(self, t: f32) -> f32 {
        let [x1, y1, x2, y2] = self.0;
        if t <= 0.0 {
            return 0.0;
        }
        if t >= 1.0 {
            return 1.0;
        }
        let poly = |p1: f32, p2: f32| {
            let c = 3.0 * p1;
            let b = 3.0 * (p2 - p1) - c;
            (1.0 - c - b, b, c)
        };
        let ((ax, bx, cx), (ay, by, cy)) = (poly(x1, x2), poly(y1, y2));
        let x = |s: f32| ((ax * s + bx) * s + cx) * s;
        let y = |s: f32| ((ay * s + by) * s + cy) * s;
        // Newton's method finds the parameter for `t` quickly; where the slope is flat, halving finishes it.
        let mut s = t;
        for _ in 0..8 {
            let (err, slope) = (x(s) - t, (3.0 * ax * s + 2.0 * bx) * s + cx);
            if err.abs() < 1e-5 {
                return y(s);
            }
            if slope.abs() < 1e-6 {
                break;
            }
            s -= err / slope;
        }
        let (mut lo, mut hi) = (0.0, 1.0);
        s = t;
        for _ in 0..30 {
            let at = x(s);
            if (at - t).abs() < 1e-5 {
                break;
            }
            if at < t { lo = s } else { hi = s }
            s = (lo + hi) / 2.0;
        }
        y(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_hold_and_known_curves_match() {
        let ease_out_cubic = Bezier([0.215, 0.61, 0.355, 1.0]);
        assert_eq!(ease_out_cubic.at(-1.0), 0.0);
        assert_eq!(ease_out_cubic.at(2.0), 1.0);
        let linear = Bezier([0.25, 0.25, 0.75, 0.75]);
        assert!((linear.at(0.3) - 0.3).abs() < 1e-3);
        // Close to the formula it approximates.
        let t = 0.4f32;
        assert!((ease_out_cubic.at(t) - (1.0 - (1.0 - t).powi(3))).abs() < 0.03);
        let overshoot = Bezier([0.0, 1.6, 0.64, 1.0]);
        assert!((1..9).map(|k| overshoot.at(k as f32 / 10.0)).any(|y| y > 1.0));
    }
}
