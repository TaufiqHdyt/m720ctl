//! The event taps: the active remapping tap and the listen-only `--watch` tap.

use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use core_foundation::base::TCFType;
use core_foundation::mach_port::CFMachPortRef;
use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions,
    CGEventTapPlacement, CGEventType, CallbackResult, EventField,
};

use crate::buttons::{self, Verdict};
use crate::config::Config;
use crate::natural;
use crate::scroll;
use crate::tilt::{self, Debounce, Dir, Horizontal};

extern "C" {
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

/// The live tap's mach port, so the callback can re-enable it if macOS
/// disables it (callback too slow, or secure input).
static TAP_PORT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

fn reenable_if_disabled(etype: CGEventType) -> bool {
    if matches!(etype, CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput) {
        eprintln!("m720ctl: event tap disabled by system ({etype:?}); re-enabling");
        let port = TAP_PORT.load(Ordering::Acquire);
        if !port.is_null() {
            unsafe { CGEventTapEnable(port as CFMachPortRef, true) };
        }
        return true;
    }
    false
}

/// Install `tap` on this thread's run loop and run forever.
fn run(tap: CGEventTap<'static>) -> ! {
    TAP_PORT.store(tap.mach_port().as_concrete_TypeRef() as *mut c_void, Ordering::Release);
    let source = tap
        .mach_port()
        .create_runloop_source(0)
        .expect("creating run loop source");
    CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopCommonModes });
    tap.enable();
    CFRunLoop::run_current();
    unreachable!("run loop exited");
}

/// Try to create the active remapping tap. Fails without Accessibility.
pub fn create_remap_tap(cfg: Config, verbose: bool) -> Result<CGEventTap<'static>, ()> {
    let mut events = vec![CGEventType::OtherMouseDown, CGEventType::OtherMouseUp, CGEventType::OtherMouseDragged];
    if cfg.shift_scroll_horizontal || cfg.tilt_enabled || cfg.horizontal_only_with_shift {
        events.push(CGEventType::ScrollWheel);
    }
    let debounce = Mutex::new(Debounce::new(Duration::from_millis(cfg.tilt_cooldown_ms)));
    CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        events,
        move |_proxy, etype, event: &CGEvent| {
            if reenable_if_disabled(etype) {
                return CallbackResult::Keep;
            }
            match etype {
                CGEventType::ScrollWheel => {
                    // Native horizontal events first; the Shift rewrite below
                    // only ever acts on vertical ones.
                    match classify_horizontal(event, &cfg) {
                        Horizontal::NotNative => {
                            // Direction first, so Shift + wheel goes sideways
                            // in the configured direction too.
                            natural::apply(event, cfg.natural_scroll);
                            scroll::rewrite(event, &cfg);
                            CallbackResult::Keep
                        }
                        Horizontal::Pass => {
                            natural::apply(event, cfg.natural_scroll);
                            CallbackResult::Keep
                        }
                        Horizontal::Block => {
                            if verbose {
                                eprintln!("m720ctl: blocked horizontal scroll (only Shift + wheel scrolls sideways)");
                            }
                            CallbackResult::Drop
                        }
                        Horizontal::Tilt(dir) => {
                            let fire = debounce
                                .lock()
                                .map(|mut d| d.hit(dir, Instant::now()))
                                .unwrap_or(false);
                            if fire {
                                let action = match dir {
                                    Dir::Left => cfg.tilt_left,
                                    Dir::Right => cfg.tilt_right,
                                };
                                // Always logged: rare, and it's how you tell a
                                // tilt was seen when an action doesn't happen.
                                eprintln!("m720ctl: tilt {dir:?} -> {action:?}");
                                action.run();
                            } else if verbose {
                                eprintln!("m720ctl: tilt {dir:?} repeat (ignored)");
                            }
                            // Drop every tilt event (repeats too) so apps don't
                            // also scroll sideways.
                            CallbackResult::Drop
                        }
                    }
                }
                CGEventType::OtherMouseDown | CGEventType::OtherMouseUp | CGEventType::OtherMouseDragged => {
                    let button = event.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER);
                    match buttons::classify(etype, button, &cfg) {
                        Verdict::Pass => CallbackResult::Keep,
                        Verdict::Swallow => CallbackResult::Drop,
                        Verdict::Fire(action) => {
                            eprintln!("m720ctl: button {button} -> {action:?}");
                            action.run();
                            CallbackResult::Drop
                        }
                    }
                }
                _ => CallbackResult::Keep,
            }
        },
    )
}

fn classify_horizontal(event: &CGEvent, cfg: &Config) -> Horizontal {
    // Classify in device direction, so tilt left is left whatever the system
    // natural scrolling setting is.
    let sign = if event.get_integer_value_field(natural::DIRECTION_INVERTED) != 0 { -1 } else { 1 };
    tilt::classify(
        sign * event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1),
        sign * event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_2),
        event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS) != 0,
        event.get_flags().contains(CGEventFlags::CGEventFlagShift),
        cfg.tilt_enabled,
        cfg.horizontal_only_with_shift,
    )
}

pub fn run_remap(tap: CGEventTap<'static>) -> ! {
    run(tap)
}

/// `--watch`: print every mouse/scroll/key event without changing anything.
pub fn run_watch() -> Result<std::convert::Infallible, ()> {
    use CGEventType::*;
    let tap = CGEventTap::new(
        CGEventTapLocation::Session,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::ListenOnly,
        vec![
            LeftMouseDown, LeftMouseUp, RightMouseDown, RightMouseUp,
            OtherMouseDown, OtherMouseUp, ScrollWheel, KeyDown, KeyUp, FlagsChanged,
        ],
        |_proxy, etype, event: &CGEvent| {
            if !reenable_if_disabled(etype) {
                println!("{}", describe(etype, event));
            }
            CallbackResult::Keep
        },
    )?;
    println!("m720ctl --watch: press each mouse button and scroll (with and without Shift). Ctrl-C to quit.");
    run(tap)
}

fn describe(etype: CGEventType, e: &CGEvent) -> String {
    let flags = flag_names(e.get_flags());
    match etype {
        CGEventType::ScrollWheel => format!(
            "{etype:?}  line(v={}, h={})  point(v={}, h={})  continuous={}  natural={}  flags=[{flags}]",
            e.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_1),
            e.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_DELTA_AXIS_2),
            e.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1),
            e.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2),
            e.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS),
            e.get_integer_value_field(natural::DIRECTION_INVERTED),
        ),
        CGEventType::KeyDown | CGEventType::KeyUp | CGEventType::FlagsChanged => format!(
            "{etype:?}  keycode={}  flags=[{flags}]",
            e.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE),
        ),
        _ => format!(
            "{etype:?}  button={}  flags=[{flags}]",
            e.get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER),
        ),
    }
}

fn flag_names(f: CGEventFlags) -> String {
    let names = [
        (CGEventFlags::CGEventFlagShift, "shift"),
        (CGEventFlags::CGEventFlagControl, "ctrl"),
        (CGEventFlags::CGEventFlagAlternate, "opt"),
        (CGEventFlags::CGEventFlagCommand, "cmd"),
        (CGEventFlags::CGEventFlagSecondaryFn, "fn"),
        (CGEventFlags::CGEventFlagNumericPad, "numpad"),
    ];
    names
        .iter()
        .filter(|(bit, _)| f.contains(*bit))
        .map(|(_, n)| *n)
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_graphics::event::ScrollEventUnit;
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    /// A scroll event with the field values `--watch` prints.
    fn watched(v: i32, h: i32, natural: i64, flags: CGEventFlags) -> CGEvent {
        let src = CGEventSource::new(CGEventSourceStateID::Private).unwrap();
        let ev = CGEvent::new_scroll_event(src, ScrollEventUnit::LINE, 2, v, h, 0).unwrap();
        ev.set_integer_value_field(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS, 0);
        ev.set_integer_value_field(natural::DIRECTION_INVERTED, natural);
        ev.set_flags(flags);
        ev
    }

    /// The M720 tilt as captured on the user's Mac (system natural scrolling on):
    ///   ScrollWheel  line(v=0, h=1)   point(v=0, h=1)   continuous=0  natural=1  flags=[]
    ///   ScrollWheel  line(v=0, h=-1)  point(v=0, h=-1)  continuous=0  natural=1  flags=[]
    #[test]
    fn captured_m720_tilt_events_are_tilts() {
        let cfg = Config::default();
        let none = CGEventFlags::CGEventFlagNull;
        // natural=1 means macOS negated the deltas: h=1 is device h=-1 = right.
        assert_eq!(classify_horizontal(&watched(0, 1, 1, none), &cfg), Horizontal::Tilt(Dir::Right));
        assert_eq!(classify_horizontal(&watched(0, -1, 1, none), &cfg), Horizontal::Tilt(Dir::Left));
        // Same physical tilts with system natural scrolling off.
        assert_eq!(classify_horizontal(&watched(0, -1, 0, none), &cfg), Horizontal::Tilt(Dir::Right));
        assert_eq!(classify_horizontal(&watched(0, 1, 0, none), &cfg), Horizontal::Tilt(Dir::Left));
        // Shift + tilt is blocked, plain wheel is left for the other handlers.
        let shift = CGEventFlags::CGEventFlagShift;
        assert_eq!(classify_horizontal(&watched(0, 1, 1, shift), &cfg), Horizontal::Block);
        assert_eq!(classify_horizontal(&watched(-1, 0, 1, none), &cfg), Horizontal::NotNative);
    }
}
