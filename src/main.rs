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
    let under_launchd = std::env::var("XPC_SERVICE_NAME").is_ok_and(|v| v == LAUNCHD_LABEL);
    if under_launchd {
        trim_log();
    }
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
    let started = std::time::Instant::now();
    let mut first = true;
    let mut last_status = None;
    let tap = loop {
        if let Ok(tap) = tap::create_remap_tap(cfg.clone(), verbose) {
            break tap;
        }
        if first {
            if under_launchd {
                // We relaunch every ~10 s while waiting: keep it to one line.
                eprintln!(
                    "m720ctl: waiting for Accessibility + Input Monitoring for {}",
                    bin_path()
                );
            } else {
                eprintln!("m720ctl: waiting for permissions.\n{}", permissions::help());
            }
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

/// Where `dist/install.sh` points launchd's stdout/stderr.
const LOG_FILE: &str = "Library/Logs/m720ctl.log";
/// Trim the log once it's bigger than this...
const LOG_MAX: u64 = 1 << 20;
/// ...down to this much of its newest content.
const LOG_KEEP: usize = 256 << 10;

fn bin_path() -> String {
    std::env::current_exe().map_or_else(|_| "m720ctl".into(), |p| p.display().to_string())
}

/// Keep the launchd log from growing forever. Runs at startup, before we log
/// anything. launchd opens the log in append mode (restarts append to it), so
/// our stderr keeps writing at the new end after the truncate.
fn trim_log() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let path = PathBuf::from(home).join(LOG_FILE);
    if let Err(e) = trim_file(&path, LOG_MAX, LOG_KEEP) {
        eprintln!("m720ctl: couldn't trim {}: {e}", path.display());
    }
}

/// If `path` is over `max` bytes, keep only its last `keep` bytes, starting at
/// a line boundary. Returns whether it trimmed.
fn trim_file(path: &std::path::Path, max: u64, keep: usize) -> std::io::Result<bool> {
    use std::io::{Read, Seek, SeekFrom, Write};
    let mut f = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let len = f.metadata()?.len();
    if len <= max {
        return Ok(false);
    }
    let mut tail = Vec::with_capacity(keep);
    f.seek(SeekFrom::Start(len.saturating_sub(keep as u64)))?;
    f.read_to_end(&mut tail)?;
    // Drop the partial first line.
    let start = tail.iter().position(|&b| b == b'\n').map_or(0, |i| i + 1);
    f.set_len(0)?;
    f.seek(SeekFrom::Start(0))?;
    f.write_all(b"m720ctl: (log trimmed)\n")?;
    f.write_all(&tail[start..])?;
    Ok(true)
}

fn usage_error(msg: &str) -> ExitCode {
    eprintln!("m720ctl: {msg}\n\n{USAGE}");
    ExitCode::from(2)
}

#[cfg(test)]
mod tests {
    use super::trim_file;

    #[test]
    fn trims_big_log_to_whole_lines() {
        let path = std::env::temp_dir().join(format!("m720ctl-trim-{}.log", std::process::id()));
        let line = "m720ctl: tilt Left -> DesktopLeft\n"; // 34 bytes
        std::fs::write(&path, line.repeat(100)).unwrap();

        assert!(
            !trim_file(&path, 10_000, 200).unwrap(),
            "small file left alone"
        );
        assert!(trim_file(&path, 1_000, 200).unwrap());
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.starts_with("m720ctl: (log trimmed)\n"));
        let rest = out.trim_start_matches("m720ctl: (log trimmed)\n");
        assert!(
            rest.len() <= 200 && rest.lines().all(|l| l == line.trim_end()),
            "{rest:?}"
        );
        assert!(!rest.is_empty());

        std::fs::remove_file(&path).unwrap();
        assert!(!trim_file(&path, 1, 1).unwrap(), "missing file is fine");
    }
}
