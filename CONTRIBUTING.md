# Contributing

Bug reports and pull requests are welcome.

## Reporting a bug

Please include:

- macOS version and Mac model (Apple Silicon only)
- mouse model and how it's connected (Bluetooth / Logi Bolt / Unifying)
- your `~/.config/m720ctl/config.toml`, if you have one
- the relevant part of `~/Library/Logs/m720ctl.log`
- for input problems, the output of `m720ctl --watch` while you reproduce it

## Development

```sh
cargo build
cargo test
cargo fmt
cargo clippy --all-targets -- -D warnings
```

CI runs the same checks. To try a change without installing it, stop the
agent (`launchctl bootout gui/$(id -u)/com.user.m720ctl`) and run
`cargo run --release -- --verbose` from a terminal that has Accessibility and
Input Monitoring permission. `./dist/install.sh` brings the agent back.

## Scope

m720ctl stays small on purpose: it rewrites input events with a `CGEventTap`
and uses only public APIs. Features that need Logitech's HID++ protocol, a
driver, or network access are probably out of scope; open an issue first to
discuss.
