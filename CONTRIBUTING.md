# Contributing

## Toolchain

- Rust stable, `x86_64-pc-windows-msvc`
- Visual Studio Build Tools: **Desktop development with C++**
- MSVC build tools and Windows SDK installed

## Checks

Run from **Developer PowerShell for VS 2022**:

```powershell
cargo fmt --check
cargo check
cargo run
```

`cargo run` is currently read-only. Keep every display-changing experiment
explicitly opt-in and manually test it on real Windows hardware.

## Scope

Do not add display-layout controls. Monitorctl controls only whether Windows
includes a display in the active desktop.
