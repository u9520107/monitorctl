# Audio policy proposal

Status: revised design and implementation handoff; implementation not started
Created: 2026-08-27
Updated: 2026-09-13
Reviewed repository: branch `audio-policy-implementation`, commit `2c89510`

## Executive summary

This personal Windows utility should prevent NVIDIA driver updates from routing
sound to monitor speakers, and offer quick audio output selection in the existing
tray. Otherwise, preserve Windows audio selection behavior.

The ordered list is a live inventory with selection recency, not a configured
preference policy. Windows chooses normally. Monitorctl observes those choices
and uses the list only to recover from a suppressed NVIDIA default.

Use the existing Rust package, `windows` crate, tray, config writer, and startup
mechanism. Keep the undocumented default setter isolated. Prove discovery and
explicit selection before adding automatic correction. No new service, UI
framework, policy framework, or repository.

This revision supersedes the previous execution plan, including its static
priority commands, capture policy, strict enforcement, invalid-default repair,
and cross-process temporary overrides. Recommendations not explicitly settled
in discussion are identified below rather than presented as user decisions.

## Agreed behavior

### Available outputs and ordering

- Track currently available render endpoints by exact endpoint ID.
- Available means active and selectable. Disabled, unplugged, or not-present
  endpoints do not belong in the recovery list, even if diagnostics enumerate them.
- When an output becomes unavailable, remove its entry and its old position.
- When an output appears, append it at the bottom. Discovery alone never changes
  the Windows default or promotes the output.
- When Windows selects an eligible output, move it to the front without duplicates.
  It need not be newly connected. No inference about user intent is required.
- A returning output is a new arrival in this list; it does not reclaim its former
  position. If Windows selects it, promote it normally.
- NVIDIA outputs remain visible in the inventory and tray. Suppression affects
  promotion and automatic recovery eligibility, not Windows device availability.

### Suppression disabled

Accept every Windows selection, including NVIDIA, and move the selected output
to the front. Never perform automatic correction. The tool observes available
outputs and provides quick selection through the tray.

### Suppression enabled

Accept and promote non-NVIDIA selections. If Windows selects an NVIDIA output,
do not promote that selection; restore the first currently available non-NVIDIA
output in the list. Skip NVIDIA entries regardless of their stored position.
If no eligible output exists, leave the default unchanged and report the condition.

Do not repair a missing/null default, force a preferred device on reconnect, or
switch away from a valid non-NVIDIA default. A list change alone is not a reason
to call the setter. Automatic setters require a currently NVIDIA default and
enabled suppression.

### Examples

| Event | List effect | Windows action by monitorctl |
| --- | --- | --- |
| Focusrite selected, Realtek available | Focusrite before Realtek | None |
| Dragonfly appears, Windows keeps Focusrite | Append Dragonfly | None |
| Windows selects Dragonfly | Move Dragonfly to front | None |
| Focusrite disappears | Remove Focusrite | None just because it disappeared |
| Windows falls back to Realtek | Move Realtek to front | None |
| Windows falls back to NVIDIA, suppression on | Do not promote NVIDIA | Select first available non-NVIDIA output, e.g. Realtek |
| Windows selects NVIDIA, suppression off | Move NVIDIA to front | None |
| Only NVIDIA outputs remain, suppression on | Keep inventory, no eligible fallback | None; report unavailable fallback |
| Focusrite returns but Windows keeps Realtek | Append Focusrite | None |

The Realtek fallback does not depend on a manually configured preference entry:
it is already an available output in the live list. Name ambiguity cannot block
this recovery because entries carry exact endpoint IDs.

## Minimal v1 scope

- Render endpoint discovery, default queries, and relevant notifications.
- Live ordered output inventory and NVIDIA suppression toggle.
- Explicit output selection through CLI and tray.
- Optional corrective behavior hosted in the existing tray process.
- Read-only diagnostics and targeted tests of the actual NVIDIA failure mode.

Exclude capture/microphone switching, audio profiles, user-edited priority lists,
per-app routing, volume/mute, endpoint enable/disable, registry or driver mutation,
latency investigations, scheduled tasks, a service, and separate settings UI.
Do not expand display behavior: arrangement and other display settings remain
Windows-owned, and background monitor restoration remains prohibited.

## Implementation defaults and remaining decisions

These fill gaps left by the discussion. Record any adjustment here before coding
behavior that depends on it; do not silently resurrect the old plan.

### Roles

Recommendation: v1 quick selection controls render Console and Multimedia;
Communications and all capture defaults remain untouched. Observe both playback
roles, but do not let callback arrival order invent a user preference when their
defaults differ. Use Multimedia as the recency source for the single live list.
Automatic correction checks each managed role and changes only roles currently
pointing at NVIDIA. Re-query before writing; leave valid non-NVIDIA roles alone.

Validate this role choice during the read-only baseline. If separate playback-role
history is actually needed, resolve that requirement before watcher work rather
than adding multiple policy lists speculatively. The earlier all-three-role,
render-and-capture policy is not the implementation default for this revision.

### Intentional NVIDIA selection and toggle behavior

Recommendation: one persisted `suppress_nvidia` checkbox, off by default. With
suppression enabled, NVIDIA remains visible but tool selection is unavailable
with a clear instruction to turn suppression off first. CLI rejects that request
before writing. Windows selections of NVIDIA are still corrected while enabled.
Do not add transient overrides or cross-process exception state.

Turning suppression off leaves the current default unchanged and allows it to be
promoted, including NVIDIA. Turning it on immediately evaluates current defaults
for NVIDIA correction. It does not reorder existing entries merely to filter them.

### Startup and persistence

Recommendation: persist the observed ID order only as a restart seed using the
existing local storage mechanisms. It is not a durable preference list. On a
successful startup snapshot, discard saved IDs that are no longer active, retain
relative order of survivors, append newly observed outputs, then promote the
current eligible Multimedia default. While running, persist removals and actual
order changes, not every notification.

For outputs with no selection history, use a deterministic initial/appended order
(friendly name, then exact ID as tie-breaker). This is a fallback tie-break, not
inferred preference. On first launch with NVIDIA already default and multiple
other outputs, there is no historical choice to restore; the first eligible output
in that initialized list wins. Make that limitation visible in diagnostics.

Enumeration failure is not an empty inventory: retain previous state, perform no
correction from stale data, and report failure. A successful empty snapshot does
clear the list. Devices removed and returned while the tray was stopped cannot
be distinguished from continuously present devices; do not add infrastructure
to reconstruct events missed while the process was absent.

## Windows API and identity

Use documented MMDevice APIs for discovery, properties, current defaults, and
`IMMNotificationClient` notifications. Only active render endpoints are candidates.
Callbacks enqueue lightweight signals; they must not block, enumerate, write
config, set defaults, unregister callbacks, or release final COM references.

`IPolicyConfig::SetDefaultEndpoint` is the practical undocumented setter. Keep
manual interface declarations and unsafe COM calls in one small adapter. Test one
target Windows 11 interface variant first; add alternatives only for an observed
compatibility failure. Validate target and role before calls, query afterwards,
and report partial role failure honestly. Setting multiple roles is not atomic.

Exact current endpoint IDs drive the live list and tray actions. Re-enumeration
handles recreated endpoints as arrivals. Never persist numeric indexes or parse
opaque endpoint IDs. Duplicate friendly names are display/CLI concerns, not
ambiguous identities in the recovery list.

For explicit CLI selection, resolve exact endpoint ID, exact friendly name, then
unique case-insensitive friendly-name substring; ambiguous or missing selectors
fail before any change. Audio aliases and custom metadata-based identity migration
are not needed initially.

NVIDIA classification must survive endpoint recreation. Probe actual endpoint and
adapter metadata on this machine; do not assume monitor friendly names contain
"NVIDIA". Use a small evidenced classification rule and explain it in diagnostics.
Unknown classification must not trigger correction or be used as a known-safe
recovery candidate while suppression is enabled. It remains visible/selectable
with suppression disabled. If metadata cannot reliably distinguish outputs,
resolve that baseline blocker rather than guessing from opaque IDs.

Windows 11 24H2+ documents `PKEY_AudioEndpoint_StableId`, when available, for more
durable identity. It can be recorded during diagnostics but is not required for
this live inventory or a reason to restore deleted positions. Stable IDs may be
absent or change; do not build custom identity recovery for v1.

## CLI and tray

Keep the initial CLI surface small:

```text
monitorctl audio list [--all]
monitorctl audio default
monitorctl audio set-default <selector>
```

`list` shows current IDs, names, active state, managed-role defaults, observed
order when available, and NVIDIA classification. `--all` includes unavailable
endpoints for diagnostics only. Read-only commands do not mutate saved state.
`set-default` performs explicit validated selection and verifies managed roles.
The watcher observes successful CLI/Windows changes through normal notifications.

The tray provides an **Audio output** submenu listing active outputs, a marker
for the current Multimedia output, and **Suppress NVIDIA audio** checkbox.
Expose errors through existing mechanisms; do not add optional OSD settings in v1.
Revalidate a clicked endpoint ID because devices can disappear while a menu is open.

No `set-priority`, strict `enforce`, or foreground `audio watch` command initially.
Test the shared decision function without adding a public policy administration
surface. Reuse the existing startup installation and config writer/mutex. Serialize
config read-modify-write operations without holding locks across debounce waits.
Tray owns list updates; CLI reads the suppression setting before explicit selection.

Audio menu availability must not depend on successful monitor discovery. Existing
`show_menu` aborts on display enumeration failure; separate failure handling so
working audio controls remain accessible. Audio failure must likewise leave
monitor menus and hotkeys usable. Ensure a single tray/watcher owner across
processes, not merely one thread per tray instance.

## Watcher algorithm

1. Register notifications on a worker with its own COM apartment, then take an
   initial snapshot. Process notifications arriving during startup afterwards.
2. Coalesce event bursts; re-enumerate current state rather than trusting callback
   order or assuming that an add/default event expresses intent.
3. Reconcile the live list: delete unavailable IDs, append new active IDs, keep
   surviving relative order. Reclassify relevant metadata changes.
4. Observe the current Multimedia default and promote it if eligible for learning:
   any known active output with suppression off, known non-NVIDIA with it on.
5. If suppression is off, stop. If on, inspect each managed role; only a current
   NVIDIA default authorizes correction for that role.
6. Choose the first active, known non-NVIDIA ID. Re-check target availability,
   suppression setting, and that role's current default immediately before writing.
   If Windows has already selected a non-NVIDIA output, leave it alone.
7. Set and verify only affected roles. Fresh verified state feeds normal list
   reconciliation. Self-generated notifications must settle into no-op decisions.
8. On transient failure, allow at most three delayed retries over roughly three
   seconds per correction burst, then stop and report. Self-generated events must
   not reset the budget indefinitely. A later genuine device/default change can
   start a new evaluation. Never continually force a default through a timer.

A null/missing default never independently authorizes a setter. A later event
showing an NVIDIA default can. Bounded retries are a settling mechanism, not a
promise that an entire driver update finishes in three seconds.

On shutdown, unregister notifications outside callbacks before releasing COM
objects. Discovery/setter failures must not terminate the tray or affect displays.

## Execution plan and session handoff

Read this document and `AGENTS.md`, inspect Git status and the current code before
starting. Branch/commit above are review context, not instructions to reset work.
The next implementation session is expected to use GPT-5.6 Luna with high reasoning;
this document is self-contained and does not require access to the prior chat.

Use these small phases. Do not mark an exit gate passed without its evidence.
Manual audio-changing tests require explicit opt-in; document a pending gate when
that testing is not authorized or cannot be performed. Do not claim success from
compilation alone. Do not start automatic correction before the setter gate passes.

```text
[x] Design review and live-inventory behavior agreed
[ ] Phase 0: read-only baseline and remaining default validation
[ ] Phase 1: explicit default setter
[ ] Phase 2: tray quick selection
[ ] Phase 3: live list and suppression decision tests
[ ] Phase 4: tray observation and automatic correction
[ ] Phase 5: Windows/NVIDIA validation
```

### Phase 0: read-only baseline

- Inspect `Cargo.toml`, `src/lib.rs`, `src/tray.rs`, config serialization, and
  existing startup behavior. Confirm actual Windows build and Rust toolchain.
- Add minimal audio module and required `windows` features; implement read-only
  `audio list [--all]` and `audio default`.
- Record Focusrite, Realtek, NVIDIA, other outputs, states, current IDs, relevant
  metadata, classification evidence, and all render-role defaults.
- Validate the proposed role scope and initial fallback ordering above. Keep
  unresolved product choices explicit before dependent phases.
- Update project `AGENTS.md` boundary during implementation to allow audio quick
  selection and opt-in audio correction, preserving all display restrictions.

Exit: formatting/build pass; real enumeration/default queries work; classification
is evidenced; no Windows state changed. Record machine results below.

### Phase 1: explicit setter

- Implement the isolated adapter and `audio set-default` with active-ID validation.
- Test explicit switching among available outputs with authorization, including
  NVIDIA with suppression disabled; verify each managed role afterwards.
- Contain failures and report partial results. No watcher yet.

Exit: build checks pass and authorized manual switching works on the target system.

### Phase 2: tray quick selection

- Reuse menu/action plumbing for active outputs and current selection marker.
- Keep audio and display discovery failures independent.
- Ensure stale menu selections fail safely and setter errors remain visible.

Exit: tray switches outputs through the shared setter; existing monitor behavior
still works; no automatic selection occurs.

### Phase 3: live list and suppression decisions

- Add optional audio config with suppression off by default and saved-order seed.
- Implement a small pure reconciliation/decision function with exact IDs.
- Test additions, promotion, removal, reconnection, suppression modes, unavailable
  fallback, duplicate names, classification failure, and startup reconciliation.
- Separate failed enumeration from successful empty enumeration.

Exit: tests encode the examples above; old monitor config loads unchanged; no
unit test invokes an audio or display setter.

### Phase 4: tray watcher

- Add single-owner notification worker, bounded debounce/retries, and clean shutdown.
- Observe and persist order in both modes; wire suppression checkbox and immediate
  evaluation on enable. Use the same validation for CLI/tray explicit selection.
- Guard against stale queued events, self-trigger loops, and concurrent CLI use.
- Keep correction conditional on current NVIDIA defaults only.

Exit: observation-only mode makes zero automatic setter calls; suppression repairs
NVIDIA selections; valid non-NVIDIA and null defaults cause no writes.

### Phase 5: real Windows validation

Test and record:

- Focusrite unplug with Windows fallback to Realtek: no tool correction.
- Focusrite unplug with NVIDIA fallback: correction to available Realtek.
- New Dragonfly-like output selected/not selected by Windows; reconnect behavior.
- NVIDIA selected with suppression off; both toggle directions.
- Only NVIDIA available; no default; failed enumeration; duplicate names.
- Different render-role defaults; untouched Communications and capture defaults
  under the proposed v1 role scope.
- Reboot, tray restart, duplicate tray launch, and concurrent CLI selection.
- NVIDIA endpoint recreation during an actual authorized driver update: record
  before/after metadata and correction result; synthetic tests are not equivalent.

Exit: real failure mode verified, no repeated correction loop, and no unexpected
Windows selection changes outside suppression or explicit tool actions.

For Rust changes run `cargo fmt --check`, `cargo check`, and focused relevant tests.
Keep README/CONTRIBUTING current if build or usage requirements change. Leave code
uncommitted unless the user explicitly authorizes a commit in that turn.

## Session record

- 2026-09-13: revised proposal only. No audio implementation or manual Windows
  testing performed. Next action: Phase 0.
- Record future sessions here with phase, files changed, checks, manual evidence,
  unresolved items, and the exact next action.

## Sources

- [Core Audio interfaces](https://learn.microsoft.com/en-us/windows/win32/coreaudio/core-audio-interfaces)
- [GetDefaultAudioEndpoint](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immdeviceenumerator-getdefaultaudioendpoint)
- [Device properties](https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-properties)
- [IMMNotificationClient and callback restrictions](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nn-mmdeviceapi-immnotificationclient)
- [OnDefaultDeviceChanged parameters](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immnotificationclient-ondefaultdevicechanged)
- [Endpoint device states](https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-state-xxx-constants)
- [Device roles](https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-roles)
- [Endpoint ID strings](https://learn.microsoft.com/en-us/windows/win32/coreaudio/endpoint-id-strings)
- [PKEY_AudioEndpoint_StableId](https://learn.microsoft.com/en-us/windows/win32/coreaudio/pkey-audioendpoint-stableid)
- [windows-rs PolicyConfig discussion](https://github.com/microsoft/windows-rs/issues/1355)
