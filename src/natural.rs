//! Natural scrolling for the mouse wheel, independent of the system setting.
//!
//! macOS has one "Natural scrolling" switch for trackpad and mouse. When it's
//! on, the window server negates scroll deltas before our tap sees them and
//! marks the event with field 137, which is exactly what
//! `-[NSEvent isDirectionInvertedFromDevice]` reads for scroll events
//! (confirmed by disassembling AppKit). We flip discrete (mouse wheel) events
//! to the configured direction and leave continuous ones (trackpad, Magic
//! Mouse) on the system setting.

use core_graphics::event::{CGEvent, EventField};

/// CGEvent field: scroll deltas were inverted from the device direction.
pub const DIRECTION_INVERTED: u32 = 137;

const INT_FIELDS: [u32; 4] = [
    EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1,
    EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_2,
    EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1,
    EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2,
];
const FIXED_FIELDS: [u32; 2] = [
    EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_1,
    EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_2,
];

/// Does the event need flipping to match `natural`?
pub fn needs_flip(continuous: bool, inverted: bool, natural: bool) -> bool {
    !continuous && inverted != natural
}

/// Make a discrete wheel event scroll in the configured direction. Returns
/// true if the event was changed.
pub fn apply(event: &CGEvent, natural: bool) -> bool {
    let continuous =
        event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS) != 0;
    let inverted = event.get_integer_value_field(DIRECTION_INVERTED) != 0;
    if !needs_flip(continuous, inverted, natural) {
        return false;
    }
    // Read everything first: writing the line deltas can make CoreGraphics
    // re-derive the other representations, so explicit values go last.
    let ints = INT_FIELDS.map(|f| event.get_integer_value_field(f));
    let fixed = FIXED_FIELDS.map(|f| event.get_double_value_field(f));
    for (f, v) in INT_FIELDS.iter().zip(ints) {
        event.set_integer_value_field(*f, -v);
    }
    for (f, v) in FIXED_FIELDS.iter().zip(fixed) {
        event.set_double_value_field(*f, -v);
    }
    // Keep isDirectionInvertedFromDevice truthful for apps that read it.
    event.set_integer_value_field(DIRECTION_INVERTED, natural as i64);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_graphics::event::ScrollEventUnit;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    fn wheel_event(v: i32, h: i32) -> CGEvent {
        let src = CGEventSource::new(CGEventSourceStateID::Private).unwrap();
        CGEvent::new_scroll_event(src, ScrollEventUnit::LINE, 2, v, h, 0).unwrap()
    }

    #[test]
    fn flip_rules() {
        // Mouse wheel, system natural on, want off: flip.
        assert!(needs_flip(false, true, false));
        // Mouse wheel already matches: leave.
        assert!(!needs_flip(false, true, true));
        assert!(!needs_flip(false, false, false));
        // Mouse wheel, system natural off, want on: flip.
        assert!(needs_flip(false, false, true));
        // Trackpad / Magic Mouse: never touched.
        assert!(!needs_flip(true, true, false));
        assert!(!needs_flip(true, false, true));
    }

    #[test]
    fn negates_all_deltas_and_marks_event() {
        let ev = wheel_event(-3, 2);
        ev.set_integer_value_field(DIRECTION_INVERTED, 1); // as if system natural on
        let ints = INT_FIELDS.map(|f| ev.get_integer_value_field(f));
        let fixed = FIXED_FIELDS.map(|f| ev.get_double_value_field(f));
        assert!(ints.iter().all(|v| *v != 0), "{ints:?}");

        assert!(apply(&ev, false));
        assert_eq!(
            INT_FIELDS.map(|f| ev.get_integer_value_field(f)),
            ints.map(|v| -v)
        );
        assert_eq!(
            FIXED_FIELDS.map(|f| ev.get_double_value_field(f)),
            fixed.map(|v| -v)
        );
        assert_eq!(ev.get_integer_value_field(DIRECTION_INVERTED), 0);

        // Second pass is a no-op: it now matches the config.
        assert!(!apply(&ev, false));
    }

    #[test]
    fn leaves_matching_and_continuous_events() {
        let ev = wheel_event(-3, 0);
        ev.set_integer_value_field(DIRECTION_INVERTED, 0);
        assert!(!apply(&ev, false));
        assert_eq!(
            ev.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1),
            -3
        );

        let ev = wheel_event(-3, 0);
        ev.set_integer_value_field(DIRECTION_INVERTED, 1);
        ev.set_integer_value_field(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS, 1);
        assert!(!apply(&ev, false));
        assert_eq!(
            ev.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1),
            -3
        );
    }
}
