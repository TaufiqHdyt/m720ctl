//! Built-in defaults, optionally overridden by `~/.config/m720ctl/config.toml`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::actions::Action;

/// Runtime configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// Turn Shift + wheel into horizontal scrolling.
    pub shift_scroll_horizontal: bool,
    /// Flip the direction of the rewritten horizontal scroll.
    pub invert_horizontal: bool,
    /// Natural scrolling for the mouse wheel, regardless of the system
    /// setting (which then only applies to the trackpad / Magic Mouse).
    pub natural_scroll: bool,
    /// Only rewrite when Shift is the *only* modifier held (so Cmd/Ctrl/Opt +
    /// Shift + scroll still reaches apps untouched).
    pub shift_only: bool,
    /// macOS mouse button number (0 = left, 1 = right, 2 = middle, 3 = back,
    /// 4 = forward, ...) -> action. Unmapped buttons work normally.
    pub buttons: HashMap<i64, Action>,
    /// Horizontal scrolling only via Shift + wheel: block every other discrete
    /// (mouse) horizontal scroll, e.g. Shift + tilt, or tilt when
    /// `tilt_enabled` is off. Trackpad scrolling is never touched.
    pub horizontal_only_with_shift: bool,
    /// Turn wheel tilt into actions (the tilt's own sideways scroll is dropped).
    pub tilt_enabled: bool,
    /// Action for pushing the wheel left.
    pub tilt_left: Action,
    /// Action for pushing the wheel right.
    pub tilt_right: Action,
    /// A held tilt repeats; repeats closer together than this don't re-fire.
    pub tilt_cooldown_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            shift_scroll_horizontal: true,
            invert_horizontal: false,
            natural_scroll: false,
            shift_only: true,
            // Back/forward keep their normal behavior; desktops are on the tilt.
            buttons: HashMap::new(),
            horizontal_only_with_shift: true,
            tilt_enabled: true,
            tilt_left: Action::DesktopLeft,
            tilt_right: Action::DesktopRight,
            tilt_cooldown_ms: 300,
        }
    }
}

/// On-disk format. Every field is optional; missing ones keep the default.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    shift_scroll_horizontal: Option<bool>,
    invert_horizontal: Option<bool>,
    natural_scroll: Option<bool>,
    shift_only: Option<bool>,
    /// Replaces the default button table entirely when present.
    buttons: Option<HashMap<String, Action>>,
    horizontal_only_with_shift: Option<bool>,
    tilt_enabled: Option<bool>,
    tilt_left: Option<Action>,
    tilt_right: Option<Action>,
    tilt_cooldown_ms: Option<u64>,
}

pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/m720ctl/config.toml"))
}

/// Load the config. An explicit path must exist; the default path is optional.
pub fn load(explicit: Option<&Path>) -> Result<Config, String> {
    let path = match explicit {
        Some(p) => p.to_path_buf(),
        None => match default_path() {
            Some(p) if p.exists() => p,
            _ => return Ok(Config::default()),
        },
    };
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let parsed = parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    eprintln!("m720ctl: loaded config from {}", path.display());
    Ok(parsed)
}

fn parse(text: &str) -> Result<Config, String> {
    let file: FileConfig = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut cfg = Config::default();
    if let Some(v) = file.shift_scroll_horizontal {
        cfg.shift_scroll_horizontal = v;
    }
    if let Some(v) = file.invert_horizontal {
        cfg.invert_horizontal = v;
    }
    if let Some(v) = file.natural_scroll {
        cfg.natural_scroll = v;
    }
    if let Some(v) = file.shift_only {
        cfg.shift_only = v;
    }
    if let Some(v) = file.horizontal_only_with_shift {
        cfg.horizontal_only_with_shift = v;
    }
    if let Some(v) = file.tilt_enabled {
        cfg.tilt_enabled = v;
    }
    if let Some(v) = file.tilt_left {
        cfg.tilt_left = v;
    }
    if let Some(v) = file.tilt_right {
        cfg.tilt_right = v;
    }
    if let Some(v) = file.tilt_cooldown_ms {
        cfg.tilt_cooldown_ms = v;
    }
    if let Some(table) = file.buttons {
        let mut buttons = HashMap::new();
        for (key, action) in table {
            let n: i64 = key
                .trim()
                .parse()
                .map_err(|_| format!("[buttons] key {key:?} is not a button number"))?;
            if n <= 1 {
                // Left/right go through LeftMouse*/RightMouse* events, which
                // this tool deliberately never touches.
                return Err(format!(
                    "[buttons] button {n} (left/right click) can't be remapped"
                ));
            }
            buttons.insert(n, action);
        }
        cfg.buttons = buttons;
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_keeps_defaults() {
        let c = parse("").unwrap();
        assert!(c.shift_scroll_horizontal);
        assert!(c.buttons.is_empty(), "back/forward must stay unmapped");
        assert!(c.horizontal_only_with_shift);
        assert!(!c.natural_scroll, "natural scrolling must default to off");
        assert!(c.tilt_enabled);
        assert_eq!(c.tilt_left, Action::DesktopLeft);
        assert_eq!(c.tilt_right, Action::DesktopRight);
    }

    #[test]
    fn full_file() {
        let c = parse(
            r#"
            shift_scroll_horizontal = true
            invert_horizontal = true
            tilt_left = "desktop_right"
            tilt_right = "mission_control"
            tilt_cooldown_ms = 500
            horizontal_only_with_shift = false
            natural_scroll = true
            [buttons]
            "3" = "desktop_left"
            "4" = "desktop_right"
            "5" = "mission_control"
            "2" = "none"
            "#,
        )
        .unwrap();
        assert!(c.invert_horizontal);
        assert_eq!(c.tilt_left, Action::DesktopRight);
        assert_eq!(c.tilt_right, Action::MissionControl);
        assert_eq!(c.tilt_cooldown_ms, 500);
        assert!(!c.horizontal_only_with_shift);
        assert!(c.natural_scroll);
        assert_eq!(c.buttons.len(), 4);
        assert_eq!(c.buttons.get(&5), Some(&Action::MissionControl));
        assert_eq!(c.buttons.get(&2), Some(&Action::None));
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse("[buttons]\n\"x\" = \"desktop_left\"").is_err());
        assert!(parse("[buttons]\n\"1\" = \"desktop_left\"").is_err());
        assert!(parse("[buttons]\n\"3\" = \"launch_rockets\"").is_err());
        assert!(parse("typo_key = true").is_err());
    }
}
