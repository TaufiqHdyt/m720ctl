//! Decide what to do with an `OtherMouse*` event.

use core_graphics::event::CGEventType;

use crate::actions::Action;
use crate::config::Config;

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Not a mapped button: let the event through.
    Pass,
    /// Mapped button, but not the press: drop it so no stray click/drag leaks.
    Swallow,
    /// Mapped button pressed: drop the event and run the action.
    Fire(Action),
}

pub fn classify(etype: CGEventType, button: i64, cfg: &Config) -> Verdict {
    let Some(&action) = cfg.buttons.get(&button) else {
        return Verdict::Pass;
    };
    match etype {
        CGEventType::OtherMouseDown => Verdict::Fire(action),
        CGEventType::OtherMouseUp | CGEventType::OtherMouseDragged => Verdict::Swallow,
        _ => Verdict::Pass,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapped_and_unmapped() {
        let cfg = Config {
            buttons: [(3, Action::DesktopLeft), (4, Action::DesktopRight)].into(),
            ..Config::default()
        };
        assert_eq!(classify(CGEventType::OtherMouseDown, 3, &cfg), Verdict::Fire(Action::DesktopLeft));
        assert_eq!(classify(CGEventType::OtherMouseUp, 3, &cfg), Verdict::Swallow);
        assert_eq!(classify(CGEventType::OtherMouseDragged, 4, &cfg), Verdict::Swallow);
        assert_eq!(classify(CGEventType::OtherMouseDown, 2, &cfg), Verdict::Pass);
        assert_eq!(classify(CGEventType::OtherMouseUp, 2, &cfg), Verdict::Pass);
        // Defaults: back/forward pass through untouched.
        let def = Config::default();
        assert_eq!(classify(CGEventType::OtherMouseDown, 3, &def), Verdict::Pass);
        assert_eq!(classify(CGEventType::OtherMouseDown, 4, &def), Verdict::Pass);
    }
}
