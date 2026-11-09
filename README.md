# m720ctl

A small replacement for Logi Options for a **Logitech M720 Triathlon paired over
Bluetooth**, on **Apple Silicon macOS**. It does three things:

1. **Shift + scroll wheel → horizontal scroll**
2. **Wheel tilt left/right → Desktop Left / Desktop Right** (back/forward
   buttons keep working normally); optional button remapping, e.g. Mission Control
3. **Natural scrolling off for the mouse wheel** (configurable), while the
   trackpad keeps the system setting
4. Runs in the background as a **launchd login agent**

It doesn't talk to the mouse over HID++. macOS already delivers the M720's
wheel and buttons as normal input events, and m720ctl rewrites them with one
`CGEventTap`. It is about 700 lines of Rust, tests included, and uses no private APIs.

It does **not** do device filtering. The rewrite applies to any mouse's wheel
tilt, and extra buttons. Trackpad and Magic Mouse scrolling are left alone because
they send continuous scroll events, which m720ctl skips. Battery, SmartShift,
DPI, Flow, gestures, and cloud features are not included.

## Requirements

- Apple Silicon Mac, Rust toolchain (`cargo`)
- Logi Options **not running** (quit it, or remove its login item). If both run,
  both react to the same buttons and scroll events.
- Mission Control shortcuts turned on (they are by default): System Settings →
  Keyboard → Keyboard Shortcuts → Mission Control → *Mission Control* (⌃↑),
  *Move left a space* (⌃←), *Move right a space* (⌃→). m720ctl presses these
  shortcuts for you.

## 1. Find your button numbers

macOS numbers mouse buttons 0 = left, 1 = right, 2 = middle, 3 = back,
4 = forward. The M720's thumb **gesture button** has no fixed number, so check
what it sends first:

```sh
cargo build --release
./target/release/m720ctl --watch
```

Your terminal app needs **Input Monitoring** for this (System Settings →
Privacy & Security). Click every button and scroll with and without Shift.
You'll see lines like:

```
OtherMouseDown  button=3  flags=[]
ScrollWheel  line(v=-1, h=0)  point(v=-10, h=0)  continuous=0  flags=[shift]
```

If the gesture button shows up as `OtherMouseDown button=N`, map `N` in the
config. If it shows up as **key events** (some Logitech mice send a keyboard
shortcut when no Logitech software is running) or **shows nothing**, m720ctl
can't remap it. In that case, put Mission Control on another button such as
middle (`"2"`), or see *Limitations*.

## 2. Configure (optional)

Defaults: Shift+scroll on; wheel tilt left → Desktop Left, tilt right →
Desktop Right; no buttons remapped (back/forward stay back/forward). A tilt
switches once even if held; repeats closer than `tilt_cooldown_ms` (300) are
ignored. If the tilt directions feel swapped, swap `tilt_left`/`tilt_right`.
To change anything:

```sh
mkdir -p ~/.config/m720ctl
cp dist/config.example.toml ~/.config/m720ctl/config.toml
# edit, e.g. add:  "5" = "mission_control"
./target/release/m720ctl --print-config    # check the result
```

Actions: `mission_control`, `app_expose`, `desktop_left`, `desktop_right`,
`none` (blocks the button). A mapped button's press, release, and drag events
are all blocked, so apps never see that button.

## 3. Install as a login agent

```sh
./dist/install.sh
```

This builds the binary, signs it, installs it to `~/.local/bin/m720ctl`, writes
`~/Library/LaunchAgents/com.user.m720ctl.plist`, loads the agent, and opens the
two settings panes. In **System Settings → Privacy & Security**, turn on
`m720ctl` under both **Accessibility** and **Input Monitoring**. If it isn't
listed, click `+` and choose `~/.local/bin/m720ctl` (press ⌘⇧G in the file
picker to type the path).

You don't need to restart anything. Until both permissions are granted, the
agent exits every ~10 seconds and launchd starts it again, because a running
process often doesn't notice a new grant. It starts working within about 20
seconds after you grant them.

Check that it's running:

```sh
launchctl print gui/$(id -u)/com.user.m720ctl | grep state
tail -f ~/Library/Logs/m720ctl.log     # "event tap active" = working
```

After a config change, restart it with
`launchctl kickstart -k gui/$(id -u)/com.user.m720ctl`.

### Permissions after rebuilding

By default the binary is **ad-hoc signed**. macOS ties an ad-hoc binary's
permission grant to the exact build, so when a **changed** build is installed,
remove the old `m720ctl` entries with `-` and add them again. Re-running
`install.sh` on an unchanged build keeps the existing grant (signing is
deterministic, and the script skips the copy). To keep the grant across
rebuilds, sign with a real identity from your keychain:

```sh
M720CTL_SIGN_IDENTITY="Apple Development: you@example.com" ./dist/install.sh
```

## Uninstall

```sh
./dist/uninstall.sh
```

This removes the agent and the binary. Your config, the log file, and the
permission entries stay; the script tells you where they are.

## How it works

| File | Role |
|---|---|
| `src/main.rs` | Argument parsing; waits for permissions; starts the tap |
| `src/tap.rs` | Active tap (session level) + `--watch` listen-only tap; turns the tap back on if macOS disables it |
| `src/scroll.rs` | Shift + wheel: moves the vertical line, fixed-point, and pixel deltas to the horizontal axis and clears Shift |
| `src/natural.rs` | Mouse-wheel scroll direction (`natural_scroll`): undoes or applies the system's natural-scrolling inversion on discrete wheel events, using the event's own "inverted" flag (field 137) |
| `src/tilt.rs` | Recognizes wheel tilt (discrete horizontal scroll, no Shift) and fires once per tilt |
| `src/buttons.rs` | `OtherMouse*` + button number → pass / block / run action |
| `src/actions.rs` | Sends ⌃+arrow key presses (with the fn/numpad flags a real arrow key carries) at the HID level |
| `src/config.rs` | Defaults + `~/.config/m720ctl/config.toml` |
| `src/permissions.rs` | Accessibility / Input Monitoring checks and prompts |

Shift is removed from rewritten scroll events. Otherwise apps that already
turn Shift+scroll sideways would swap the axes back. By default, scrolls with
Cmd, Ctrl, or Opt held together with Shift are passed through unchanged
(`shift_only`).

## Background

`docs/` has notes from reverse engineering Logi Options' `LogiMgrDaemon`
(static analysis only): `REPORT.md` covers the app's architecture,
`device.md` covers how it talks to devices over HID++.

## Limitations

- **Gesture button.** If `--watch` doesn't show it as a mouse button, it is
  only reachable through Logitech's HID++ protocol: the device's reprogrammable
  controls feature, `0x1B04`, which can divert the button to software (see
  `docs/device.md`). That path isn't implemented.
- **Not limited to the M720.** Any other wheel mouse's Shift+scroll, tilt /
  horizontal wheel, and extra buttons are rewritten too.
- **Horizontal scrolling only via Shift + wheel.** Every other sideways scroll
  from a mouse is blocked (`horizontal_only_with_shift`, on by default): plain
  tilt switches desktops, Shift + tilt does nothing. Trackpad and Magic Mouse
  scrolling are not affected.
- **Secure input.** While a password field has focus, macOS may pause the
  tap. m720ctl turns it back on automatically.
