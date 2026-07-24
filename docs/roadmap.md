# Monitorctl roadmap

## Product boundary

Monitorctl controls whether Windows includes a display in the active desktop.
Windows Display Settings remains owner of layout, resolution, scaling, refresh
rate, orientation, primary display, and extend/duplicate mode.

Profiles store only named sets of active displays. They never store layout.

Safety rule: never disable the last active display.

## Phase 1: Windows API spike

Build a small Rust proof of concept to confirm native Windows APIs can:

- enumerate displays with friendly names and active state;
- disable and enable one external display;
- restore Windows' remembered placement on re-enable; and
- refuse an operation that would leave no active display.

Verify that each invocation re-enumerates current Windows-visible displays
after external topology changes. Do not add monitoring, automatic repair, or
connection-type-specific behavior.

Gate: do not build user interfaces until this works reliably on target setup.

## Phase 2: Shared core

Create `monitorctl-core` for display discovery and state changes.

- Find configured displays and report active state.
- Enable, disable, and toggle a display.
- Apply a profile as an active-display set.
- Save prior active set before a change for `restore`.
- Return clear errors for absent displays and unsafe requests.

Use one small local configuration file for display matching, aliases, profiles,
and fixed hotkeys.

Start by replacing Phase 1 numeric indexes with aliases and monitor-name
selectors. Resolve exact alias, exact name, then unique case-insensitive
partial name; ambiguous partial names fail.

## Phase 3: CLI

Provide scriptable commands:

```powershell
monitorctl list
monitorctl enable <display>
monitorctl disable <display>
monitorctl toggle <display>

monitorctl profile save <name>
monitorctl profile create <name> --active <display,...>
monitorctl profile list
monitorctl profile show <name>
monitorctl profile apply <name>
monitorctl profile delete <name>

monitorctl restore
```

`list` must show usable display names, identifiers, and active state. Profile
apply fails safely when a required display is missing; no partial application
in v1.

## Phase 4: Tray utility

Build a tray-only utility using shared core and configuration. No settings
window in v1.

Menu:

```text
Monitors
  Dell 27"          Toggle
  LG Ultrawide       Toggle
────────────
Profiles
  Work
  Focus
  Presentation
────────────
Restore previous set
Quit
```

Requirements:

- show current on/off state for each configured display;
- toggle displays and apply profiles from menu;
- use fixed, manually configured global hotkeys;
- refresh after Windows display-topology changes;
- show short success or error notifications; and
- disable unavailable or unsafe actions.

Tray marks a profile active only when current active-display set exactly
matches it.

## Phase 5: Reliability and packaging

Exercise concurrent actions, display-driver restarts, dock changes, monitor
removal, sleep/resume, and hotkey conflicts. Serialize topology changes so two
actions cannot race.

Package a portable executable first. Add installer and start-on-login only if
personal use needs them.

## First milestone

Complete Phase 1, then decide whether monitor identity and Windows layout
restoration are reliable enough for CLI and tray work.
