//! Native horizontal wheel events (the M720's wheel tilt) -> actions or block.
//!
//! The M720 reports a tilt as a discrete horizontal scroll: `line(v=0, h=±1)`,
//! `continuous=0`. The sign macOS delivers depends on the system natural
//! scrolling setting, so callers pass the *device* direction (undo the
//! system's inversion first; see `natural.rs`). Holding the tilt repeats those events, so a tilt fires its
//! action once and further same-direction events are ignored until there's a
//! pause of `cooldown`.
//!
//! Horizontal scrolling is meant to come only from Shift + wheel (see
//! `scroll.rs`), so other discrete horizontal events are blocked by default.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
}

/// What to do with a scroll event, as far as horizontal scrolling goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Horizontal {
    /// Not a native discrete horizontal event (vertical wheel, trackpad...):
    /// carry on with normal handling (incl. the Shift + wheel rewrite).
    NotNative,
    /// A wheel tilt: run the tilt action (debounced) and drop the event.
    Tilt(Dir),
    /// Drop it: horizontal scrolling only comes from Shift + wheel.
    Block,
    /// Let it through as a sideways scroll.
    Pass,
}

/// Classify a scroll event. `line_v`/`line_h` are the line deltas in device
/// direction (as if natural scrolling were off).
///
/// Our own Shift rewrite happens later in the same callback, so it never shows
/// up here. Shift + tilt is not treated as a tilt: if it were, any Shift +
/// wheel event that arrived already horizontal could switch desktops.
pub fn classify(
    line_v: i64,
    line_h: i64,
    continuous: bool,
    shift: bool,
    tilt_enabled: bool,
    only_with_shift: bool,
) -> Horizontal {
    if continuous || line_v != 0 || line_h == 0 {
        return Horizontal::NotNative;
    }
    if tilt_enabled && !shift {
        // Device direction: positive horizontal delta = wheel pushed left
        // (classic scrolling shows the content to the left).
        return Horizontal::Tilt(if line_h > 0 { Dir::Left } else { Dir::Right });
    }
    if only_with_shift {
        Horizontal::Block
    } else {
        Horizontal::Pass
    }
}

/// Turns a stream of tilt events into one trigger per tilt.
#[derive(Debug)]
pub struct Debounce {
    cooldown: Duration,
    last: Option<(Dir, Instant)>,
}

impl Debounce {
    pub fn new(cooldown: Duration) -> Self {
        Debounce { cooldown, last: None }
    }

    /// Record a tilt event; returns true if it should fire the action.
    pub fn hit(&mut self, dir: Dir, now: Instant) -> bool {
        let repeat = matches!(
            self.last,
            Some((d, t)) if d == dir && now.saturating_duration_since(t) < self.cooldown
        );
        // Refresh the timestamp on every event, so holding the tilt keeps
        // extending the window and never fires again.
        self.last = Some((dir, now));
        !repeat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use Horizontal::*;

    /// classify with tilt on and horizontal-only-via-Shift on (the defaults).
    fn default(v: i64, h: i64, continuous: bool, shift: bool) -> Horizontal {
        classify(v, h, continuous, shift, true, true)
    }

    #[test]
    fn classifies_tilts() {
        assert_eq!(default(0, 1, false, false), Tilt(Dir::Left));
        assert_eq!(default(0, -1, false, false), Tilt(Dir::Right));
        // Vertical wheel (incl. Shift + wheel, rewritten later), trackpad,
        // empty event: not native horizontal, handled elsewhere.
        assert_eq!(default(-1, 0, false, false), NotNative);
        assert_eq!(default(-1, 0, false, true), NotNative);
        assert_eq!(default(0, 1, true, false), NotNative);
        assert_eq!(default(0, 0, false, false), NotNative);
    }

    #[test]
    fn horizontal_only_via_shift() {
        // Shift + tilt: no desktop switch, and no sideways scroll either.
        assert_eq!(default(0, 1, false, true), Block);
        // Tilt actions off: tilt still can't scroll sideways...
        assert_eq!(classify(0, -1, false, false, false, true), Block);
        // ...unless the restriction is turned off too.
        assert_eq!(classify(0, -1, false, false, false, false), Pass);
        assert_eq!(classify(0, 1, false, true, true, false), Pass);
        // Trackpad sideways scrolling is never touched.
        assert_eq!(classify(0, 5, true, false, false, true), NotNative);
    }

    #[test]
    fn one_trigger_per_tilt() {
        let ms = Duration::from_millis;
        let t0 = Instant::now();
        let mut d = Debounce::new(ms(300));
        assert!(d.hit(Dir::Left, t0));
        // Held: repeats every 50 ms for a second never re-fire.
        for i in 1..=20 {
            assert!(!d.hit(Dir::Left, t0 + ms(50 * i)), "repeat {i} fired");
        }
        // Opposite direction fires immediately.
        assert!(d.hit(Dir::Right, t0 + ms(1050)));
        // Same direction again after a pause fires.
        assert!(d.hit(Dir::Right, t0 + ms(1400)));
        // ...but not right after.
        assert!(!d.hit(Dir::Right, t0 + ms(1450)));
    }
}
