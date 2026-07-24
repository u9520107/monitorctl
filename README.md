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

Open **Developer PowerShell for VS 2022**, then build and run the Phase 1
probe:

```powershell
cargo run
```

The probe is read-only. It prints active Windows display paths, friendly
monitor names, target identifiers, and device paths.

## Phase 1 probe

```powershell
cargo run -- list
cargo run -- list --all
cargo run -- disable 0
cargo run -- disable 0 --apply
cargo run -- enable 1
cargo run -- enable 1 --apply
cargo run -- restore
cargo run -- restore --apply
```

`disable 0` validates only. `--apply` changes the selected active-display
index. `enable 1` uses index from `list --all`. The probe refuses to disable
the final active display. Run topology commands from local interactive Windows
desktop; remote or non-desktop sessions cannot call `SetDisplayConfig`.

`list --all` asks Windows for all available display paths, including inactive
ones, then shows each Windows-visible monitor once. `restore --apply` asks
Windows to apply its saved extended-display topology and is recovery path
after a disable.

## Status

Phase 1: validating native Windows display APIs. See
[roadmap](docs/roadmap.md).
