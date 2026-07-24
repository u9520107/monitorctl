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
```

Profiles store exact Windows device paths, never display indexes or layout.
## Status

Phase 3 CLI provides display discovery, aliases, enable/disable/toggle,
profiles, and previous-active-set restore. See [roadmap](docs/roadmap.md).
