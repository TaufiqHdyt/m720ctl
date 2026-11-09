//! Actions a mapped button can trigger, implemented as synthesized keyboard
//! shortcuts (macOS's default Mission Control shortcuts — no private APIs).

use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode, KeyCode};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Ctrl + Up
    MissionControl,
    /// Ctrl + Down
    AppExpose,
    /// Ctrl + Left ("Move left a space")
    DesktopLeft,
    /// Ctrl + Right ("Move right a space")
    DesktopRight,
    /// Swallow the button and do nothing.
    None,
}

impl Action {
    pub fn run(self) {
        let key = match self {
            Action::MissionControl => KeyCode::UP_ARROW,
            Action::AppExpose => KeyCode::DOWN_ARROW,
            Action::DesktopLeft => KeyCode::LEFT_ARROW,
            Action::DesktopRight => KeyCode::RIGHT_ARROW,
            Action::None => return,
        };
        if let Err(e) = post_ctrl_arrow(key) {
            eprintln!("m720ctl: failed to post {self:?}: {e}");
        }
    }
}

fn post_ctrl_arrow(key: CGKeyCode) -> Result<(), &'static str> {
    // A physical arrow key press carries the NumericPad and SecondaryFn flags;
    // the Dock's space-switching hotkeys don't fire reliably without them.
    let flags = CGEventFlags::CGEventFlagControl
        | CGEventFlags::CGEventFlagNumericPad
        | CGEventFlags::CGEventFlagSecondaryFn;
    for down in [true, false] {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| "CGEventSourceCreate failed")?;
        let ev = CGEvent::new_keyboard_event(source, key, down)
            .map_err(|_| "CGEventCreateKeyboardEvent failed")?;
        ev.set_flags(flags);
        ev.post(CGEventTapLocation::HID);
    }
    Ok(())
}
