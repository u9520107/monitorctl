# Monitorctl

Rust tool for enabling, disabling, and switching Windows monitor profiles.
Windows remains responsible for display layout, resolution, scaling, refresh
rate, orientation, and primary-display selection.

## Development setup

Install these build dependencies:

| Dependency | Required for |
| --- | --- |
| [Rust](https://rustup.rs/) stable `x86_64-pc-windows-msvc` | Compile Monitorctl |
| Visual Studio Build Tools, **Desktop development with C++** | Provide MSVC `link.exe` |
| Windows SDK | Link against Windows display APIs |

Install Build Tools with winget when available:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --exact `
  --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Open **Developer PowerShell for VS 2022**, then build and run Monitorctl:

```powershell
cargo run
```

`list` is read-only. It prints Windows-visible monitors, friendly names,
aliases, active state, and device paths.

## CLI

Run `monitorctl --help` for command reference. Commands that change display
state apply immediately.

### Displays

```powershell
monitorctl list
monitorctl enable desk
monitorctl disable laptop
monitorctl toggle desk
monitorctl restore
```

`list` is read-only. `restore` restores the active-display set from before the
last successful display change. `disable` and `toggle` refuse to leave Windows
with no active displays.

### Active-display profiles

Save current active displays, create one from selectors, inspect it, apply it,
or delete it:

```powershell
monitorctl profile save work
monitorctl profile create focus --active desk,laptop
monitorctl profile list
monitorctl profile show work
monitorctl profile apply work
monitorctl profile delete work
```

`save` and `create` replace a same-named profile. `apply` fails without
changing Windows state when any saved display is unavailable.

### Hotkeys

Configure hotkeys for tray utility, then restart it:

```powershell
monitorctl hotkey list
monitorctl hotkey set ctrl+alt+w profile:work
monitorctl hotkey set ctrl+alt+d toggle:desk
monitorctl hotkey set ctrl+win+shift+1 color:m32q "HDR Cali"
monitorctl hotkey set ctrl+alt+r restore
monitorctl hotkey delete ctrl+alt+r
```

Keys require `ctrl`, `alt`, `shift`, or `win`, plus one letter or digit.
`toggle:<monitor>` and `color:<monitor> <file>` resolve monitor identity and
full installed filename when saved. Restart tray after every hotkey change.

### On-screen display

```powershell
monitorctl osd show
monitorctl osd show "Displays ready"
monitorctl osd opacity 0.85
```

## Portable package

The portable Windows package contains `monitorctl.exe`, `monitorctl-tray.exe`,
this README, and the license. Extract it anywhere, then run
`monitorctl-tray.exe` for tray utility or `monitorctl.exe --help` for CLI use.
Configuration remains in `%LOCALAPPDATA%\monitorctl\monitorctl.toml`, so
upgrading extracted folder does not replace profiles or aliases.

Create fresh package from source, then install it into
`%USERPROFILE%\tools\monitorctl`, add it to user `PATH`, and create login-startup
tray shortcut:

```powershell
.\scripts\package.ps1
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

`package.ps1` requires Rust, MSVC, and Windows SDK. `install.ps1` only installs
newest complete package already in `dist`; it does not build source code.

Existing `PATH` entries and `Monitorctl.lnk` startup shortcuts are left alone.
The tray utility starts after install; during an upgrade it is restarted after
files are copied. Use `-NoStart`, `-NoStartup`, or
`-InstallDirectory <path>` to change those defaults.

Each command is explicit;
Monitorctl has no background state repair. It refuses to disable final active
display. Run topology commands from local interactive Windows desktop; remote
or non-desktop sessions cannot call `SetDisplayConfig`.

## Monitor selection

Numeric indexes are Phase 1 probe behavior only; Windows does not guarantee
their order. Phase 2 commands resolve monitors in this order:

1. exact user alias;
2. exact friendly monitor name; then
3. case-insensitive unique substring of friendly monitor name.

Ambiguous partial names fail with matching monitor names. Monitorctl stores
the exact Windows monitor device path behind each alias.

Monitorctl stores aliases, profiles, hotkeys, and the previous active set in:

```text
%LOCALAPPDATA%\monitorctl\monitorctl.toml
```

Create file to map aliases to monitor identity printed by `list`:

```toml
[displays.desk]
path = "\\\\?\\DISPLAY#GBT3203#..."
friendly_name = "Gigabyte M32Q"

[displays.desk.serial]
manufacturer_id = 21532
product_code = 12803
serial = 2461

[hotkeys]
"ctrl+alt+w" = "profile:work"

[osd]
opacity = 0.85
```

Profiles and aliases store monitor identity, never display indexes or layout.
When EDID supplies a nonzero serial, Monitorctl matches manufacturer, product,
and serial across port or topology changes. Otherwise it prefers saved Windows
path, then uses an exact friendly-name match only when unique. Ambiguous
serial-less matches fail.

Old path-only config entries remain valid. Re-save a profile after upgrading to
capture serial and friendly-name identity data.

## Color profiles

ICC color profiles are independent from display profiles. They change only the
current user's Windows color association for one active monitor; they never
change monitor layout or active-display state. Windows advanced color uses an
advanced ICC profile; SDR uses a normal profile.

```powershell
monitorctl color import "C:\Profiles\Dell-U2723QE-2026.icc"
monitorctl color list
monitorctl color current desk
monitorctl color set desk "Dell-U2723QE"
```

Color commands require Windows 11. `color list` shows all installed `.icc` and
`.icm` files with detected channel or `unsupported`. `color import` installs an
ICC file unless identical bytes already exist. It rejects files whose name
collides with different installed contents. `color set` accepts an exact
filename or unique case-insensitive filename substring, checks profile channel,
then makes it current for that monitor. Windows' profile store is the source of
truth; changing display topology may require another explicit `color set`.

When Windows currently uses system color settings for a display, explicit
`color set` enables current-user settings for that display before applying the
profile. Tray's `Use system color settings` switches display back to Windows'
system settings; it does not remove installed ICC files.

`color current` reports `Windows default` when Windows reports no explicit ICC
association for monitor's active color channel.

## Tray utility

`monitorctl-tray` is a tray-only utility for Windows-visible monitors,
profiles, restore, and configured global hotkeys. It uses a display alias as
its menu label when configured; otherwise it uses the friendly name.
Run it with:

```powershell
cargo run --bin monitorctl-tray
```

Hotkeys map `ctrl`, `alt`, `shift`, or `win` plus one letter/digit to actions:

```toml
[hotkeys]
"ctrl+alt+w" = "profile:work"
"ctrl+alt+d" = "toggle:desk"
"ctrl+alt+r" = "restore"
```

Tray menu rebuilds on opening, so it reflects current Windows display state.
Manage those entries with `monitorctl hotkey list`, `monitorctl hotkey set`, and
`monitorctl hotkey delete`; restart the tray after changes.

Tray results use a native lower-center OSD. Configure opacity from `0.10` to
`1.00`, or preview it from CLI:

```powershell
monitorctl osd show
monitorctl osd show "Custom string"
monitorctl osd opacity 0.85
```

`osd show` stays open for two seconds, then exits. Tray success, error, and
hotkey-conflict messages show for five seconds.
