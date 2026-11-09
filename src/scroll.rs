//! Shift + wheel -> horizontal scroll.

use core_graphics::event::{CGEvent, CGEventFlags, EventField};

use crate::config::Config;

const LINE_V: u32 = EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1;
const LINE_H: u32 = EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_2;
const FIXED_V: u32 = EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_1;
const FIXED_H: u32 = EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_2;
const POINT_V: u32 = EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1;
const POINT_H: u32 = EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2;

/// Should a scroll event with these modifier flags be rewritten?
pub fn wants_rewrite(flags: CGEventFlags, cfg: &Config) -> bool {
    if !cfg.shift_scroll_horizontal || !flags.contains(CGEventFlags::CGEventFlagShift) {
        return false;
    }
    let others = CGEventFlags::CGEventFlagControl
        | CGEventFlags::CGEventFlagAlternate
        | CGEventFlags::CGEventFlagCommand;
    !(cfg.shift_only && flags.intersects(others))
}

/// Rewrite `event` in place if it is a Shift + wheel scroll. Returns true if
/// the event was changed.
pub fn rewrite(event: &CGEvent, cfg: &Config) -> bool {
    let flags = event.get_flags();
    if !wants_rewrite(flags, cfg) {
        return false;
    }
    // Trackpads / Magic Mouse send continuous scrolls and already have a
    // horizontal axis; only touch classic wheel events (the M720's wheel).
    if event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS) != 0 {
        return false;
    }

    let line = event.get_integer_value_field(LINE_V);
    let fixed = event.get_double_value_field(FIXED_V);
    let point = event.get_integer_value_field(POINT_V);
    if line == 0 && fixed == 0.0 && point == 0 {
        // Nothing vertical to move (e.g. already horizontal); leave it alone.
        return false;
    }

    let sign = if cfg.invert_horizontal { -1 } else { 1 };
    // Write the line delta first: CoreGraphics may derive the other two
    // representations from it, so the explicit values below must come after.
    event.set_integer_value_field(LINE_V, 0);
    event.set_integer_value_field(LINE_H, sign * line);
    event.set_double_value_field(FIXED_V, 0.0);
    event.set_double_value_field(FIXED_H, sign as f64 * fixed);
    event.set_integer_value_field(POINT_V, 0);
    event.set_integer_value_field(POINT_H, sign * point);

    // Clear Shift so apps that natively turn Shift+scroll sideways don't
    // swap the axes back.
    event.set_flags(flags - CGEventFlags::CGEventFlagShift);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_graphics::event::ScrollEventUnit;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    fn wheel_event(lines: i32) -> CGEvent {
        let src = CGEventSource::new(CGEventSourceStateID::Private).unwrap();
        CGEvent::new_scroll_event(src, ScrollEventUnit::LINE, 1, lines, 0, 0).unwrap()
    }

    #[test]
    fn modifier_rules() {
        let cfg = Config::default();
        let shift = CGEventFlags::CGEventFlagShift;
        assert!(wants_rewrite(shift, &cfg));
        assert!(!wants_rewrite(CGEventFlags::CGEventFlagNull, &cfg));
        assert!(!wants_rewrite(
            shift | CGEventFlags::CGEventFlagCommand,
            &cfg
        ));
        let loose = Config {
            shift_only: false,
            ..Config::default()
        };
        assert!(wants_rewrite(
            shift | CGEventFlags::CGEventFlagCommand,
            &loose
        ));
        let off = Config {
            shift_scroll_horizontal: false,
            ..Config::default()
        };
        assert!(!wants_rewrite(shift, &off));
    }

    #[test]
    fn moves_vertical_to_horizontal() {
        let cfg = Config::default();
        let ev = wheel_event(-3);
        ev.set_flags(CGEventFlags::CGEventFlagShift);
        let (line, fixed, point) = (
            ev.get_integer_value_field(LINE_V),
            ev.get_double_value_field(FIXED_V),
            ev.get_integer_value_field(POINT_V),
        );
        assert_eq!(line, -3);
        assert!(fixed != 0.0 && point != 0, "fixed={fixed} point={point}");

        assert!(rewrite(&ev, &cfg));
        assert_eq!(ev.get_integer_value_field(LINE_V), 0);
        assert_eq!(ev.get_integer_value_field(LINE_H), line);
        assert_eq!(ev.get_double_value_field(FIXED_V), 0.0);
        assert_eq!(ev.get_double_value_field(FIXED_H), fixed);
        assert_eq!(ev.get_integer_value_field(POINT_V), 0);
        assert_eq!(ev.get_integer_value_field(POINT_H), point);
        assert!(!ev.get_flags().contains(CGEventFlags::CGEventFlagShift));
    }

    #[test]
    fn invert_and_passthrough() {
        let inv = Config {
            invert_horizontal: true,
            ..Config::default()
        };
        let ev = wheel_event(2);
        ev.set_flags(CGEventFlags::CGEventFlagShift);
        assert!(rewrite(&ev, &inv));
        assert_eq!(ev.get_integer_value_field(LINE_H), -2);

        // No Shift: untouched.
        let ev = wheel_event(2);
        assert!(!rewrite(&ev, &Config::default()));
        assert_eq!(ev.get_integer_value_field(LINE_V), 2);
    }
}
