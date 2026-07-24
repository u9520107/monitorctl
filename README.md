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

Run `monitorctl --help` for command reference, or `monitorctl profile --help`
for profile management. Typical profile workflow:

```powershell
cargo run -- list
cargo run -- profile save work
cargo run -- profile create focus --active desk,laptop
cargo run -- profile apply work
```

## Portable package

The portable Windows ZIP contains `monitorctl.exe`, `monitorctl-tray.exe`,
this README, and the license. Extract it anywhere, then run
`monitorctl-tray.exe` for the tray utility or `monitorctl.exe --help` for CLI
use. Configuration remains in `%LOCALAPPDATA%\monitorctl\monitorctl.toml`, so
upgrading the extracted folder does not replace profiles or aliases.

`profile save` and `profile create` replace any same-named profile. Use
`profile show <name>` to inspect one before applying or deleting it.

All display-changing commands apply immediately. Each command is explicit;
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

Create file to map aliases to paths printed by `list`:

```toml
[displays]
desk = "\\\\?\\MONITOR#..."

[profiles]
work = ["\\\\?\\MONITOR#..."]

[hotkeys]
"ctrl+alt+w" = "profile:work"

[osd]
opacity = 0.85
```

Profiles store exact Windows device paths, never display indexes or layout.
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
See [roadmap](docs/roadmap.md).
