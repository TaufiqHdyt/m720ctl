//! m720ctl — minimal Logi Options replacement for a Bluetooth M720 on Apple
//! Silicon: Shift + wheel -> horizontal scroll, and extra mouse buttons ->
//! Mission Control / Desktop Left / Desktop Right. Works on ordinary macOS
//! input events via one CGEventTap; no HID++.

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
compile_error!("m720ctl only supports Apple Silicon macOS (aarch64-apple-darwin)");

mod actions;
mod buttons;
mod config;
mod natural;
mod permissions;
mod scroll;
mod tap;
mod tilt;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

const USAGE: &str = "\
m720ctl — Shift+scroll to horizontal and M720 button remapping

USAGE:
  m720ctl [--config PATH] [--verbose]   run the remapper (what launchd runs)
  m720ctl --watch                       print raw mouse/key events (discover button numbers)
  m720ctl --print-config [--config PATH] show the effective config
  m720ctl --help | --version

Config: ~/.config/m720ctl/config.toml (optional)";

enum Mode {
    Run,
    Watch,
    PrintConfig,
}

fn main() -> ExitCode {
    let mut mode = Mode::Run;
    let mut config_path: Option<PathBuf> = None;
    let mut verbose = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--watch" => mode = Mode::Watch,
            "--print-config" => mode = Mode::PrintConfig,
            "--verbose" | "-v" => verbose = true,
            "--config" => match args.next() {
                Some(p) => config_path = Some(p.into()),
                None => return usage_error("--config needs a path"),
            },
            "--help" | "-h" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            "--version" | "-V" => {
                println!("m720ctl {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            other => return usage_error(&format!("unknown argument {other:?}")),
        }
    }

    match mode {
        Mode::Watch => {
            if tap::run_watch().is_err() {
                eprintln!(
                    "m720ctl: could not create a listening event tap.\n{}",
                    permissions::help()
                );
                permissions::input_monitoring(true);
                return ExitCode::FAILURE;
            }
            unreachable!()
        }
        Mode::PrintConfig => match config::load(config_path.as_deref()) {
            Ok(cfg) => {
                println!("{cfg:#?}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("m720ctl: config error: {e}");
                ExitCode::FAILURE
            }
        },
        Mode::Run => run(config_path, verbose),
    }
}

fn run(config_path: Option<PathBuf>, verbose: bool) -> ExitCode {
    let cfg = match config::load(config_path.as_deref()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("m720ctl: config error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut mapping: Vec<_> = cfg.buttons.iter().collect();
    mapping.sort_by_key(|(n, _)| **n);
    eprintln!(
        "m720ctl {}: natural_scroll={} shift_scroll_horizontal={} invert_horizontal={} buttons={mapping:?} \
         horizontal_only_with_shift={} tilt={} (left={:?}, right={:?}, cooldown={}ms)",
        env!("CARGO_PKG_VERSION"),
        cfg.natural_scroll,
        cfg.shift_scroll_horizontal,
        cfg.invert_horizontal,
        cfg.horizontal_only_with_shift,
        cfg.tilt_enabled,
        cfg.tilt_left,
        cfg.tilt_right,
        cfg.tilt_cooldown_ms,
    );

    // Without Accessibility the tap can't be created. Instead of exiting (and
    // being restarted in a loop by launchd), ask once and wait for the grant.
    // A running process often doesn't see a grant made after it started, so
    // under launchd we exit and let KeepAlive start a fresh process instead
    // of polling forever. From a terminal we keep polling.
    let under_launchd = std::env::var("XPC_SERVICE_NAME").is_ok_and(|v| v == LAUNCHD_LABEL);
    let started = std::time::Instant::now();
    let mut first = true;
    let mut last_status = None;
    let tap = loop {
        if let Ok(tap) = tap::create_remap_tap(cfg.clone(), verbose) {
            break tap;
        }
        if first {
            eprintln!("m720ctl: waiting for permissions.\n{}", permissions::help());
            // Under launchd this runs on every relaunch; install.sh opens
            // the settings panes, so don't pop prompts in a loop.
            permissions::accessibility(!under_launchd);
            permissions::input_monitoring(!under_launchd);
            first = false;
        }
        // Log the current grant state right away and whenever it changes.
        let status = (
            permissions::accessibility(false),
            permissions::input_monitoring(false),
        );
        if last_status != Some(status) {
            eprintln!(
                "m720ctl: permissions: accessibility={}, input_monitoring={} (both must be true)",
                status.0, status.1
            );
            last_status = Some(status);
        }
        if under_launchd && started.elapsed() >= Duration::from_secs(10) {
            eprintln!(
                "m720ctl: no permissions yet; exiting so launchd restarts us with fresh state"
            );
            return ExitCode::from(1);
        }
        std::thread::sleep(Duration::from_secs(2));
    };

    eprintln!("m720ctl: event tap active");
    tap::run_remap(tap)
}

const LAUNCHD_LABEL: &str = "com.user.m720ctl";

fn usage_error(msg: &str) -> ExitCode {
    eprintln!("m720ctl: {msg}\n\n{USAGE}");
    ExitCode::from(2)
}
