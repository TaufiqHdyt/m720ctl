# Changelog

All notable changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed
- While waiting for permissions under launchd, the agent exits every ~10 s so
  launchd restarts it; a running process often doesn't notice a new grant.

## [0.1.0] - 2026-11-09

### Added
- Shift + scroll wheel scrolls horizontally.
- Wheel tilt left/right switches to the desktop on the left/right (once per
  tilt); other sideways mouse scrolling is blocked by default.
- Mouse-wheel scroll direction independent of the system natural scrolling
  setting (`natural_scroll`, off by default).
- Optional remapping of extra mouse buttons to Mission Control, App Exposé,
  or desktop switching.
- `--watch` mode to inspect mouse events, `--print-config`.
- launchd login agent install/uninstall scripts.
