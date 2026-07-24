# Agent instructions

## Product boundary

Monitorctl changes only whether Windows includes a display in the active
desktop. Windows Display Settings owns monitor arrangement, resolution,
scaling, refresh rate, orientation, primary-display selection, and
extend/duplicate mode.

Profiles are named sets of active displays. They do not store display layout.

## Runtime behavior

- CLI commands enumerate current Windows display state, perform one action,
  then exit.
- Tray utility exists only for its menu and global hotkeys. It must not repair,
  restore, or maintain monitor state in background.
- Treat all monitors as Windows-visible displays. Do not add dock,
  connection-type, sleep, or unplug-specific behavior.
- Never disable the last active display.
- Missing profile display fails safely. Do not partially apply profiles in v1.
- Monitor selectors resolve exact alias, exact friendly name, then unique
  case-insensitive friendly-name substring. Ambiguous selectors fail.
- Store exact Windows monitor device path behind user aliases; never persist
  numeric display indexes.

## Implementation

- Use native Windows display APIs through Rust `windows` crate.
- Keep monitor topology logic in shared core when project gains CLI and tray
  binaries.
- No display-changing action without explicit CLI command, tray-menu action,
  or configured hotkey.
- Do not add settings UI, layout controls, or automatic profile switching
  unless requested.

## Verification

- Run `cargo fmt --check` and `cargo check` for Rust changes.
- Display-changing work requires explicit opt-in and manual Windows testing.
- Keep README and CONTRIBUTING toolchain instructions current when build
  requirements change.
