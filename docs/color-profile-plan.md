# Color profile support proposal

## Goal

Let user explicitly select ICC profiles for individual monitors
from CLI, tray menu, or configured hotkey.

Require Windows 11. Older Windows versions report color profile switching as
unsupported.

Color profiles remain separate from Monitorctl display profiles. Display
profiles only choose displays included in active desktop; they never change
layout or color settings.

## Model

Windows stores installed ICC files independently of monitors. Monitorctl keeps
a small alias library, then applies one alias to one monitor. Windows has
normal-color and advanced-color association lists; installed files are shared,
but aliases have one detected channel and are offered only for matching display
state. Display state is `sdr`, `wcg`, or `hdr`; WCG and HDR both use advanced
color association.

A profile can be generic and used on many monitors, or marked as intended for
one calibrated monitor. Windows does not enforce this distinction.

```toml
[color_profiles.desk-calibrated]
file = "Dell-U2723QE-2026.icc"
sha256 = "..."
channel = "normal"
intended_for = "\\\\?\\MONITOR#..."

[color_profiles.desk-advanced-calibrated]
file = "Dell-U2723QE-advanced.icm"
sha256 = "..."
channel = "advanced"
```

`intended_for` is optional and stores exact Windows monitor device path, never
a numeric display index. Omit it for profiles intended to be reusable.

## Commands

```powershell
monitorctl color list
monitorctl color alias <alias> <installed-file> [--intended-for <monitor>]
monitorctl color import <path> --as <alias> [--intended-for <monitor>]
monitorctl color current <monitor>
monitorctl color set <monitor> <alias> [--force]
```

`color list` shows installed files, detected channel, ICC descriptions where
available, and Monitorctl aliases. `color current` shows selected monitor's
`sdr`, `wcg`, or `hdr` state, current Windows association scope, and default
profile.

`color set` refuses a profile whose `intended_for` differs from selected
monitor unless caller passes `--force`. It also refuses profile channel that does
not match selected monitor's current display state: `sdr` requires `normal`;
`wcg` and `hdr` require `advanced`. Tray shows target mismatch and requires
confirmation. Generic profiles have no target restriction.

Aliases used by color hotkeys must not contain `:`. Monitor aliases used in
color hotkeys must also not contain `:`. Color hotkey action format is:

```toml
[hotkeys]
"ctrl+alt+1" = "color:desk:desk-calibrated"
```

## Windows application flow

For `color set`:

1. Resolve selector to configured exact monitor device path.
2. Re-enumerate current Windows display topology and map that path to its
   current adapter LUID and source ID. Persisted path is stable identity;
   color APIs receive live topology identifiers. Missing or ambiguous mapping
   fails before any association changes.
3. Confirm aliased profile remains installed, has expected SHA-256, and is a
   supported display ICC profile.
4. Query Windows display state: `sdr`, `wcg`, or `hdr`. SDR requires `normal`
   channel; WCG and HDR require `advanced` channel.
5. Add profile to current user's normal-color or advanced-color association
   list and set it as default using `ColorProfileAddDisplayAssociation`.
6. Read current user scope and default association back. Report success only
   after Windows reports current-user scope and requested installed file.

Monitorctl never creates system-wide associations. Windows' modern display
color APIs manage current-user scope directly; no legacy `WcsSetUsePerUserProfiles`
or `DISPLAY_DEVICE.DeviceName` mapping is used.

Preflight failures change nothing. Once Windows profile mutation begins,
Monitorctl reports the actual Windows result. It does not claim transactions
or automatic rollback for color changes.

## Import and aliases

`color alias` never copies a file. It verifies installed profile contents and
detects channel, then stores filename, SHA-256, and channel under user alias.

`color import` validates source file and calculates SHA-256 before calling
Windows install API. It detects channel before installation:

- MHC ICC profile: `advanced`;
- standard RGB display ICC: `normal`;
- unknown or unsupported profile: fail; no manual channel override in v1.

It enumerates installed profiles first:

- identical installed bytes: reuse existing installed filename;
- same installed filename but different bytes: fail; user must rename source;
- otherwise: install source file into Windows profile store.

Installing a profile is a committed Windows side effect. If profile install
succeeds but config alias save fails, command reports failure, names installed
file, and leaves it installed. It must not automatically uninstall it: another
user or app may use it. Retrying `color alias` completes setup.

## Tray behavior

Tray rebuilds its menu when opened. Per configured monitor, it labels current
state as SDR, wide gamut, or HDR and lists only aliases matching its required
channel. Click performs one explicit action; tray does not poll, repair,
restore, or automatically switch profiles.

## Out of scope

- Calibration loader or gamma-ramp control.
- Editing, deleting, or synchronizing Windows profiles.
- Automatic switching based on app, time, dock, or connection changes.
- Combining multiple color changes into a preset or existing display profile.
