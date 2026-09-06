# Audio policy proposal

Status: execution plan
Branch: `audio-policy-proposal`
Created: 2026-08-27
Updated: 2026-09-05
Baseline commit: `4d63dda`

## Executive summary

Extending `monitorctl` with audio policy is a good fit for this personal
Windows utility. The problem is narrow, the existing Rust/Windows shape fits,
and a second repository or workspace would add no value now.

One hard risk controls the plan: Windows publicly documents audio endpoint
enumeration, default queries, metadata, and event notifications. It does not
document a user-mode API for setting the system default endpoint. The practical
setter is `IPolicyConfig::SetDefaultEndpoint`, an undocumented COM interface.
That ABI must be isolated, manually tested, and treated as optional risk.

This document is the implementation plan. Work proceeds in small phases so
each session leaves a usable, testable result. Do not begin the watcher until
read-only discovery, policy selection, and the setter have each worked alone.

Recommended direction:

1. Keep this repository and the `monitorctl` name.
2. Add an isolated `audio` module and optional TOML audio section.
3. Build a read-only API spike first.
4. Add explicit one-shot commands before any background watcher.
5. Add an event-driven watcher only after the setter works reliably on the target
   Windows 11 system.
6. Defer endpoint enable/disable and audio-inclusive profiles.

## How to use this plan across sessions

Each phase has a narrow scope, a deliverable, and an exit gate. A phase may take
more than one session; do not start the next phase just because code exists.

At the start of every session:

1. Read this document's phase status and the current Git status.
2. Reconfirm the phase exit gate before editing.
3. Work only on the current phase unless a blocker requires a plan update.

At the end of every session:

1. Run the phase's automated checks.
2. Record manual-test results, failures, and next action in the phase checklist.
3. Leave unrelated work untouched and leave changes uncommitted unless a commit
   is explicitly requested.

Phase status:

```text
[x] Planning and API research
[ ] Phase 0: repository and Windows baseline
[ ] Phase 1: audio domain model and config
[ ] Phase 2: read-only Core Audio discovery
[ ] Phase 3: adaptive policy engine and CLI
[ ] Phase 4: isolated default setter and one-shot enforcement
[ ] Phase 5: tray-hosted event watcher
[ ] Phase 6: tray controls, OSD, and startup behavior
[ ] Phase 7: Windows/NVIDIA validation and hardening
[ ] Phase 8: future profiles or endpoint disable experiment
```

The first implementation session should start at Phase 0, not by adding the
watcher or changing the tray.

## Latest behavioral model

The intended policy is an adaptive ordered priority list plus an optional
suppression rule:

```text
priority = [focusrite, realtek, nvidiahd]
suppress_nvidia = true
```

Behavior:

- The same user-facing priority list applies to render and capture where a
  matching endpoint exists. Windows flow remains separate internally because
  NVIDIA normally has no capture endpoint.
- With suppression disabled, Windows may select newly added devices normally;
  monitorctl does not fight that behavior.
- With suppression enabled, NVIDIA-like endpoints are treated as lowest
  priority for automatic policy without being disabled.
- If Windows selects a suppressed NVIDIA endpoint after driver recreation, the
  watcher restores the highest-priority available non-suppressed endpoint.
- If Windows selects a newly connected, non-suppressed endpoint, treat that as
  intentional and add it to the top of the learned priority list. Example:
  `[focusrite, realtek]` becomes `[dragonfly, focusrite, realtek]`.
- If the newly connected endpoint is suppressed NVIDIA, do not learn/promote it;
  restore the highest-priority available non-suppressed endpoint instead.
- If a user later selects another endpoint, the list keeps the resulting order;
  NVIDIA suppression still applies as an automatic overlay.
- If no non-suppressed priority endpoint is available, leave the current
  default unchanged and report the condition.
- Suppression does not hide, uninstall, or mutate the Windows device. Explicit
  disable/enable remains a separate future feature.

When an endpoint disappears, remove it from the active list and delete its
learned priority entry. If it later returns and Windows selects it, learn it
again at the top as a new device.

Suppression is an automatic-policy overlay, not a permanent prohibition. This
keeps intentional NVIDIA selection possible without making the normal Windows
device-switching behavior noisy.

## Problem and desired outcome

HDMI/DisplayPort monitors exposed through NVIDIA also expose render audio
endpoints. Driver installation can recreate or re-enable them, after which
Windows may choose one as default output.

Desired policy:

- Prefer outputs in explicit priority order, such as Focusrite 16i16, then
  built-in Realtek.
- Move away from NVIDIA monitor audio when policy enforcement runs.
- Recover after endpoint recreation or default-device changes.
- Never change monitor layout or display active state.
- Never choose an arbitrary audio endpoint when no safe priority endpoint is
  available.

## Scope

### In scope

- Windows render- and capture-endpoint enumeration.
- Current default render/capture endpoint by role.
- Endpoint identity, metadata, and aliases.
- Ordered audio-priority policy.
- Explicit one-shot default selection.
- Explicit one-shot policy enforcement.
- Optional event-driven audio watcher.
- Later, audio state in named profiles.

### Out of scope for v1

- Per-application or per-stream routing.
- Volume, mute, sample rate, channel layout, or exclusive mode.
- Monitor layout, resolution, scaling, refresh rate, orientation, or primary
  display changes.
- Automatic monitor profile switching.
- Automatic endpoint enable/disable.
- NVIDIA installer integration.
- Service, scheduled task, dedicated settings window, or new repository.

## Evaluation

### Repository fit

Current project has one Rust package and two binaries:

- `monitorctl`: explicit CLI operations.
- `monitorctl-tray`: menu and global hotkeys.

Core logic is in `src/lib.rs`; config is TOML at
`%LOCALAPPDATA%\\monitorctl\\monitorctl.toml`; Windows APIs use the `windows`
crate. Audio should follow this shape:

- Add `src/audio.rs` for discovery, identity, policy matching, default-role
  queries, and the isolated setter boundary.
- Extend `Config` with an optional audio section; old config remains valid.
- Keep CLI dispatch in `src/lib.rs` initially; no command-parser dependency.
- Add tray integration only after one-shot audio behavior works.
- Do not hide audio repair inside monitor menu rebuilds.

No new dependency is needed for the first implementation. Enable only the
smallest `windows` features needed for `Win32_Media_Audio`, COM, property
stores, and notification callbacks.

### Windows API decision matrix

| Need | API | Status | Proposal |
| --- | --- | --- | --- |
| Enumerate | `IMMDeviceEnumerator::EnumAudioEndpoints` | Documented | Active render/capture endpoints; all states for diagnostics. |
| ID | `IMMDevice::GetId` | Documented | Store as current endpoint identity component. |
| State | `IMMDevice::GetState` | Documented | Show active/disabled/not-present/unplugged; only active candidates. |
| Metadata | `OpenPropertyStore`, `IPropertyStore`, `PKEY_Device_FriendlyName`, `PKEY_Device_DeviceDesc`, `PKEY_Device_InstanceId`, `PKEY_Device_ContainerId` | Documented | Read only; use for display and cautious matching of explicit aliases. |
| Defaults | `GetDefaultAudioEndpoint` | Documented | Query render/capture defaults for each selected role. |
| Watch | `IMMNotificationClient` plus registration | Documented | Receive add/remove/state/property/default-role events. |
| Set default | `IPolicyConfig::SetDefaultEndpoint` via `CPolicyConfigClient` | Undocumented | One tiny adapter; manual-test on Windows 11. |
| Hard block | `PKEY_AudioDevice_NeverSetAsDefaultEndpoint` | Documented for driver setup, not app policy | Defer; poor fit for per-user v1. |
| Enable/disable | Device/control-panel mechanisms | Unsafe v1 | Defer. |

Official docs say notification callbacks must be nonblocking and must avoid
registration/unregistration and final COM releases inside callbacks. Callbacks
must only enqueue a signal; enumeration and enforcement happen elsewhere.

The official `PKEY_AudioDevice_NeverSetAsDefaultEndpoint` documentation is
important but limited: the property must be paired with
`PKEY_AudioEndpoint_Association` in the same endpoint subkey, and it blocks
both automatic and user selection. That describes a driver/endpoint setup
contract, not a clean user-level application API. Driver recreation may also
recreate the endpoint property.

### Default-correction API families

There are several things commonly called an audio “API,” but they solve
different problems:

| Family | What it does | Fit |
| --- | --- | --- |
| MMDevice API | Enumerates endpoints, reads state/properties, reads current defaults, and receives notifications. | Supported and required for discovery/watch. It does not provide the normal user-level global default setter. |
| `IPolicyConfigVista::SetDefaultEndpoint` | Sets a global endpoint for one `ERole` using an undocumented COM interface. | Practical legacy setter. Interface layout and COM identity must be declared manually. |
| `IPolicyConfig::SetDefaultEndpoint` | Same global role-setting operation through a newer undocumented PolicyConfig interface variant. | Practical primary candidate on the target Windows 11 system; still unsupported and ABI-sensitive. |
| `IAudioPolicyConfigFactory` | Persists per-application endpoint routing, including process/role-specific defaults. | Not the right operation: monitorctl wants the system-wide role defaults. Also undocumented. |
| `PKEY_AudioDevice_NeverSetAsDefaultEndpoint` | Driver/INF property that prevents automatic and user selection. | Too strong and wrong scope: it is not a per-user runtime correction mechanism. |
| Sound Settings/UI automation | Drives the user-facing settings surface. | Supported for the user, but fragile and unsuitable for silent event-driven repair. |

PowerShell modules and small third-party utilities generally wrap one of the
PolicyConfig interfaces; they do not provide a separate stable Windows setter.

The minimal implementation should therefore be:

1. Use documented MMDevice APIs for enumeration, defaults, and notifications.
2. Validate the endpoint ID and role in Rust.
3. Call one isolated `PolicyConfig` adapter for correction.
4. Set Console, Multimedia, and Communications explicitly for the selected
   flow, then re-query every role.
5. Keep any Vista/modern interface fallback inside that adapter, never in the
   watcher or policy code.

Do not start by trying every interface variant. First test one target Windows
11 build, record HRESULTs, and add a fallback only when an actual compatibility
case exists.

### Setter risk

`IPolicyConfig` is the go/no-go item:

- It is outside the documented MMDevice API.
- Its COM class and interface are not ordinary generated `windows` bindings.
- The vtable must be declared exactly for the target Windows variant.
- A wrong method order can call the wrong function or fail unpredictably.
- Community tools commonly set console, multimedia, and communications roles,
  but this must be tested, not assumed.
- Microsoft does not promise source compatibility for this interface.

For this personal tool, proceed with the isolated adapter because automatic
correction is valuable. If it fails on the target Windows 11 installation,
stop at read-only diagnostics and offer Windows Sound Settings as supported
fallback rather than spreading undocumented COM calls through the watcher.

## Enabled endpoint, driver latency, and stability

Research does not support a blanket claim that every enabled NVIDIA HDMI/DP
endpoint makes Windows unstable or measurably slower. It does support a
plausible, machine-specific failure mode:

- An endpoint being present in Windows is not the same as an audio stream being
  active. Windows tracks endpoint state separately from audio-session activity;
  inactive sessions contain streams that are not currently running.
- NVIDIA HD Audio and the Windows HD Audio bus participate in kernel driver
  interrupt/DPC processing. A faulty or poorly behaving driver can therefore
  contribute to audio glitches, stutter, or real-time-audio deadline misses.
- Community reports describe improvements after disabling NVIDIA HD Audio, but
  these are anecdotal and often also involve `nvlddmkm.sys`, `HDAudBus.sys`,
  Nahimic/APO components, power settings, or a driver revision. They do not show
  that an unused endpoint is the root cause on every system.
- Disabling an endpoint in Sound settings may not remove every underlying bus or
  GPU driver path. Device Manager/controller disablement and driver omission are
  stronger, more invasive actions with different side effects.

Microsoft describes DPC/ISR duration as a system-latency concern and recommends
kernel tracing to measure it. Therefore monitorctl should not infer latency from
endpoint presence or default status. If this becomes a requirement, add a
separate opt-in experiment:

1. Record a baseline with the exact NVIDIA, audio, and Focusrite driver versions.
2. Run the same idle and real-time-audio workload with NVIDIA HD Audio enabled.
3. Repeat with the exact endpoint/controller disablement recorded.
4. Compare total and highest ISR/DPC time, named drivers, audio dropouts, and
   system symptoms. Repeat after reboot and NVIDIA driver update.
5. Re-enable the device and confirm HDMI/DP audio recovery.

Use LatencyMon for a practical first pass, but treat ETW DPC/ISR tracing as the
stronger diagnosis when results matter. A positive result would justify a
separate future feature for disabling a known NVIDIA audio controller. It would
not justify making ordinary default-policy enforcement disable devices.

## Proposed policy model

Use three concepts:

- `priority`: ordered device selectors; first available match wins when
  monitorctl explicitly applies policy or repairs a suppressed default.
- `suppress_nvidia`: automatic exception that ranks NVIDIA-like endpoints below
  normal priority devices without disabling them.
- `roles`: default roles affected by set/enforce.

The runtime also maintains adaptive state: learned non-suppressed devices are
inserted at the top when Windows selects them, disappeared learned devices are
deleted, and an explicit selection can create a transient override. None of
that state disables devices or changes the user's suppression setting.

Illustrative TOML:

```toml
[audio]
roles = ["console", "multimedia", "communications"]
priority = ["focusrite", "realtek", "nvidiahd"]
suppress_nvidia = true

[audio.devices.focusrite]
id = "<endpoint-id>"
friendly_name = "Speakers (Focusrite 16i16)"
instance_id = "<instance-id>"
container_id = "<container-id>"
```

Exact schema remains open. Persist no numeric endpoint index.

### Selector resolution

Recommended order:

1. Exact configured alias.
2. Exact endpoint ID.
3. Exact friendly name.
4. Unique case-insensitive friendly-name substring.
5. Otherwise fail with matching candidates.

For learned entries, a disappeared ID is removed rather than re-identified.
Explicitly configured aliases may still use stored metadata, but only when the
replacement is unique. Friendly-name-only matching is unsafe when several
similar endpoints exist. Never silently accept ambiguity.

Core Audio endpoint IDs are the current machine identity, not a guaranteed
cross-driver identity. Store the ID plus friendly name, instance ID, and
container ID where available. Validate replacement behavior against actual
NVIDIA driver recreation before promising durable recovery.

## Proposed commands

Suggested surface:

```text
monitorctl audio list [--all]
monitorctl audio default
monitorctl audio set-default <selector>
monitorctl audio enforce [--dry-run]
monitorctl audio policy show
monitorctl audio policy set-priority <selector,...>
monitorctl audio policy suppress-nvidia <on|off>
```

Semantics:

- `audio list`: read-only. Show endpoint ID, friendly name, flow, state,
  role-default markers, alias, priority, and suppression classification.
- `audio list --all`: include non-active states for diagnosis; never use them as
  enforcement candidates.
- `audio default`: read-only current default by configured role.
- `audio set-default <selector>`: explicit user action through the isolated
  PolicyConfig adapter. Set the selected active endpoint for configured flows
  and roles. If the target is suppressed NVIDIA, create a temporary runtime
  exception so it can be first while it remains the current default; clear that
  exception when another endpoint is selected.
- `audio enforce`: choose highest-priority available non-suppressed endpoint.
  If no safe endpoint resolves uniquely, fail without changing anything.
- `--dry-run`: show current defaults, candidate, and reason without setter call.
- Policy-edit commands are optional. If they add too much surface, keep policy
  edits in TOML for this personal tool.

Separate `default` and `set-default` is safer than making one command both
read and write based on optional arguments.

## Decisions captured

- Q1: use one priority policy for render and capture endpoints; apply it to each
  flow independently where matching endpoints exist.
- Q2: enforce all three roles for each flow: Console, Multimedia, and
  Communications.
- Q3: priority is ordered. Focusrite wins when available; Realtek is next
  fallback; NVIDIA can remain in the list when not suppressed.
- Q4: automatic enforcement is protective, not strict. A newly connected
  non-suppressed output may become default intentionally; watcher repair targets
  suppressed NVIDIA outputs or invalid/missing defaults.
- Q5: explicit `audio enforce` is strict and selects the highest-priority
   available non-suppressed output. Background watcher stays protective.
- Q6: suppression protects automatic recovery only. Explicit `audio set-default`
   remains allowed; selecting suppressed NVIDIA creates a temporary effective
   top-priority exception, cleared when another endpoint is selected.
- Q7: consider future explicit NVIDIA audio disable/enable only after measured
  latency or stability benefit. Keep it separate from default enforcement and
  automatic watcher behavior.
- Q8: keep policy and future disable plumbing vendor-neutral, but initially
  target only explicitly configured NVIDIA HD Audio functions. Add AMD/Intel
  behavior only if real evidence requires it.
- Q9: use hybrid NVIDIA detection. Monitorctl may identify and suggest
  NVIDIA-like endpoints, but blocking requires explicit enablement.
- Q10: expose simple policy changes through CLI. Keep TOML as storage and
  advanced escape hatch; start GUI support through the existing tray rather
  than adding a separate settings window.
- Q11: keep GUI scope minimal. Use existing tray status/actions; defer a
  dedicated settings window.
- Q12: superseded by suppression model. Do not mutate Windows driver or
  endpoint properties to impose an OS-wide hard block.
- Q13: if no non-suppressed priority endpoint is available, leave current
  default unchanged and warn/log. Never choose an arbitrary fallback.
- Q14: priority changes should be available through simple CLI commands and
  should apply the newly selected first device immediately.
- Q15: newly added non-suppressed devices remain under normal Windows default
   selection behavior.
- Q16: priority is adaptive. A newly connected non-suppressed device that
   Windows selects is learned at the top; a disappeared device leaves the
   active list. NVIDIA suppression prevents NVIDIA devices from being learned
   this way.

## Enforcement algorithm

1. Load config.
2. Enumerate current endpoints and metadata.
3. Query defaults for configured roles and flows.
4. Resolve priority selectors against available endpoints.
5. If a temporary explicit-tool override is active and still current, retain it.
6. In watcher mode, if a newly added default is non-suppressed, learn it at the
   top of the active priority list.
7. In watcher mode, if default is suppressed NVIDIA or unavailable, choose the
   highest-priority available non-suppressed endpoint.
8. In explicit `audio enforce` mode, choose the highest-priority available
   non-suppressed endpoint.
9. If no safe endpoint resolves uniquely, fail closed with an actionable error.
10. Set endpoint for each configured role and flow through the isolated adapter.
11. Re-query and verify every requested role; report role-specific failure.

Do not choose arbitrary non-suppressed fallback. A typo or temporary absence must
not route sound somewhere surprising. Reuse the existing named mutex so audio
and monitor config/state operations serialize.

## Watcher design

Do not build watcher first. Prove setter and one-shot enforcement first.

Watcher behavior:

- Own a COM apartment on its thread.
- Create `MMDeviceEnumerator`, register one `IMMNotificationClient`.
- Handle default, added, removed, state, and property events.
- Do no blocking work inside callbacks.
- Coalesce notification bursts from driver installation.
- Re-enumerate and enforce outside callbacks.
- Re-check defaults to suppress self-trigger loops.
- Log old endpoint, new endpoint, reason, and role.
- Correct silently by default. If configured, reuse the existing lower-center
  OSD for a short correction message; do not add a second notification system.
- Treat an explicit CLI or tray selection as a transient override. Record it in
  shared local runtime state so the watcher can allow that one intentional
  selection; clear it when another endpoint becomes default or the endpoint is
  removed. Do not put this override in the durable policy list.
- Unregister before releasing callback/enumerator.

Use events, not continuous polling. Use bounded delayed retries after a driver
change; never infinite rapid retry.

### Process placement

1. Existing `monitorctl-tray`: background watcher at login, matching the
   current tray startup model.
2. `monitorctl audio watch`: optional foreground diagnostic/development mode,
   not required for normal use.
3. New `monitorctl-audio` binary: defer; extra package/startup surface.

Recommendation: build the watcher as shared core logic, host it in the existing
tray, and keep CLI commands one-shot for manual control. Do not add a service or
third binary until startup needs prove it.

Audio watching starts automatically with the tray. `suppress_nvidia` controls
whether it performs corrective audio changes; with suppression off, it can
observe and learn devices without repairing ordinary Windows choices. It must
never repair, restore, or maintain monitor state in the background. Monitor
actions remain explicit CLI, tray-menu, or configured-hotkey actions.

## Profiles later

Do not expand current monitor profiles in the first audio slice. They currently
contain only active display identities. Mixing audio in immediately creates
rollback and partial-availability questions.

After one-shot audio behavior is stable, profiles may store the ordered audio
priority/suppression policy. They do not pin a concrete endpoint in v1.

Application must resolve all requirements before any state change. Missing or
ambiguous display/audio requirements must fail without partial application.

## Safety

- Endpoint IDs and hardware metadata stay local.
- Do not write arbitrary endpoint property-store values in v1.
- Do not edit driver registry state in v1.
- Validate selectors and roles before setter calls.
- Missing/ambiguous priority endpoints are no-op failures.
- Never call undocumented setter from event callback.
- Re-query after changes and surface verification failure.
- Add dry-run before watcher enablement.
- Keep audio-only behavior separate from monitor state.

## Execution plan

Implementation order:

```text
0 baseline -> 1 model -> 2 discovery -> 3 policy/CLI -> 4 setter
  -> 5 watcher core -> 6 tray integration -> 7 validation
  -> 8 future extensions
```

Each phase below is a session-sized work package. Split a phase into multiple
sessions if its exit gate is not met. Never skip a gate to start the watcher.

### Phase 0: repository and Windows baseline

Goal: establish the smallest viable Windows API surface and capture the real
machine behavior before designing matching rules around guessed names.

Work:

- Confirm current branch, clean/dirty state, Rust toolchain, and existing tray
  startup behavior.
- Review `Cargo.toml`, `src/lib.rs`, `src/tray.rs`, `src/osd.rs`, and config
  serialization boundaries.
- Add only the required `windows` feature flags for COM, MMDevice, properties,
  and notification interfaces.
- Build a temporary read-only probe or test-only path. Do not add a setter,
  watcher, tray behavior, or persistent audio state yet.
- Record the actual Focusrite, Realtek, NVIDIA, and any other endpoint names,
  flows, states, IDs, instance IDs, and container IDs.
- Record which endpoint is default for Console, Multimedia, and Communications
  on render and capture.

Deliverable: a compiling API baseline and a short machine-specific probe
record in the session notes.

Exit gate:

- `cargo fmt --check` and `cargo check` pass.
- Read-only enumeration and default queries work on the target Windows 11
  machine.
- The endpoint metadata needed for aliases and NVIDIA classification is known.
- No Windows state was changed.

### Phase 1: audio domain model and configuration

Goal: make policy behavior deterministic and testable without Windows calls.

Work:

- Add the smallest shared audio model: flow, role, endpoint snapshot, selector,
  suppression classification, priority list, and transient explicit override.
- Add an optional `[audio]` TOML section with backward-compatible defaults:
  roles, priority, and `suppress_nvidia`.
- Keep learned priority entries in the existing local config/state boundary;
  delete learned entries when their endpoint disappears.
- Implement selector precedence: alias, exact endpoint ID, exact friendly name,
  then unique case-insensitive friendly-name substring.
- Implement ordered selection and the suppression overlay. Suppression must not
  mutate the configured order or disable a device.
- Implement the adaptive rules over fake endpoint data:
  - newly selected non-suppressed endpoint moves to the top;
  - newly selected suppressed NVIDIA endpoint is rejected for automatic policy;
  - disappeared learned endpoint is removed;
  - explicit NVIDIA selection creates a temporary override;
  - selecting another endpoint clears that override.

Deliverable: pure policy/config code with no COM dependency and focused tests.

Exit gate:

- Existing monitor config deserializes unchanged when `[audio]` is absent.
- Tests cover ordering, ambiguity, suppression, add/remove, and override rules.
- No test invokes a Windows setter or changes a device.

### Phase 2: read-only Core Audio discovery and CLI

Goal: expose the real endpoint state safely and make diagnostics useful before
any corrective action exists.

Work:

- Implement COM initialization and MMDevice enumeration for render and capture.
- Read active endpoints by default; support `--all` for disabled, unplugged,
  and not-present diagnostics.
- Extract friendly name, description, endpoint ID, instance ID, container ID,
  flow, and state.
- Query Console, Multimedia, and Communications defaults separately.
- Add:
  - `monitorctl audio list [--all]`
  - `monitorctl audio default`
  - `monitorctl audio policy show`
- Show enough information to diagnose the NVIDIA recreation case without
  exposing internal pointer/COM details.
- Use the actual machine to check plug/unplug, Sound Settings changes, reboot,
  and NVIDIA driver lifecycle behavior. Keep the probe read-only.

Deliverable: reliable read-only CLI and a real endpoint inventory.

Exit gate:

- Every endpoint shown has a stable current ID and useful metadata.
- Role/default output is correct for both flows.
- Ambiguous selectors are reported with candidates, never guessed.
- Manual diagnostics identify how NVIDIA IDs and metadata change after update.

### Phase 3: adaptive policy engine and one-shot policy CLI

Goal: persist the user's intent and learned device order without changing
Windows defaults yet.

Work:

- Connect Phase 1 policy logic to real endpoint snapshots from Phase 2.
- Persist the ordered list and suppression setting using the existing config
  writer and mutex.
- Add simple policy commands:
  - `monitorctl audio policy set-priority <selector,...>`
  - `monitorctl audio policy suppress-nvidia <on|off>`
- Make priority changes immediately select the new first policy candidate in
  the policy result, while keeping the actual default unchanged until Phase 4.
- Learn a newly connected non-suppressed endpoint only when Windows has selected
  it. Do not insert every merely enumerated endpoint.
- Remove learned entries on endpoint removal. Keep explicit aliases separate
  from learned entries.
- Add `audio enforce --dry-run` to explain the candidate, suppression reason,
  and no-op conditions without calling a setter.

Deliverable: persisted, explainable policy decisions with no default mutation.

Exit gate:

- A Dragonfly-style endpoint moves to the top only after Windows selects it.
- NVIDIA is not learned when suppression is enabled.
- Missing/ambiguous/no-safe-candidate cases fail closed.
- Dry-run output matches the pure policy tests.

### Phase 4: isolated default setter and explicit enforcement

Goal: prove safe correction manually before putting it behind events or tray
startup.

Work:

- Add one isolated `PolicyConfig` adapter for the tested Windows 11 variant.
- Keep all manually declared COM interfaces, CLSIDs/IIDs, HRESULT handling, and
  unsafe code inside that adapter.
- Validate that the target endpoint is active, belongs to the requested flow,
  and resolves uniquely before calling the setter.
- Implement role-specific setting for Console, Multimedia, and Communications.
- Add:
  - `monitorctl audio set-default <selector>`
  - real `monitorctl audio enforce`
- For explicit selection of suppressed NVIDIA, record the transient override so
  the tray watcher will honor the intentional choice.
- Re-query every role after setting and report partial role failure clearly.
- Never change endpoint visibility, driver properties, or enable/disable state.

Deliverable: manually usable correction with post-write verification.

Exit gate:

- Focusrite, Realtek, and NVIDIA can each be selected intentionally when active.
- All configured roles are verified after a change.
- `--dry-run` never invokes the setter.
- No arbitrary fallback occurs when a configured candidate is absent.
- Failures are contained within the adapter and do not crash the CLI.
- Manual Windows testing passes after reboot and endpoint recreation.

### Phase 5: event-driven watcher core

Goal: react to endpoint/default changes without continuous polling or fighting
normal Windows device selection.

Work:

- Implement `IMMNotificationClient` registration in a dedicated watcher thread
  with its own COM apartment.
- Have callbacks enqueue lightweight events only. Do enumeration, policy work,
  persistence, and setting outside callbacks.
- Handle endpoint added, removed, state, property, and default-role events.
- Coalesce event bursts with a short debounce, then retry up to three times over
  roughly three seconds while driver installation settles.
- Apply the behavior matrix:
  - suppression off: observe and learn normal Windows choices; do not repair;
  - suppression on + suppressed NVIDIA default: restore best safe candidate;
  - suppression on + new non-NVIDIA default: learn it and leave it selected;
  - explicit tool override: honor it until another endpoint becomes default;
  - endpoint removal: delete its learned entry and clear stale override state.
- Re-check defaults after every correction to suppress self-trigger loops.
- Log old endpoint, new endpoint, role, reason, retry, and failure.
- Keep `monitorctl audio watch` as a foreground diagnostic harness for this
  core, even though normal use will be tray-hosted.

Deliverable: watcher core that can be exercised without changing tray startup.

Exit gate:

- NVIDIA driver recreation is corrected when suppression is enabled.
- Newly connected normal devices remain under Windows control and are learned.
- No continuous polling, callback blocking, infinite retries, or correction loop.
- Clean watcher shutdown unregisters notifications and releases COM safely.

### Phase 6: tray integration, OSD, and startup

Goal: make the watcher part of normal monitorctl use without changing the
existing monitor product boundary.

Work:

- Start the shared watcher automatically from `monitorctl-tray` at login,
  matching existing tray startup behavior.
- Do not add a watcher enable switch. `suppress_nvidia` controls corrective
  intervention; suppression off still permits observation/learning.
- Keep CLI commands one-shot for manual list, default, policy, enforce, and
  set-default operations.
- Add minimal tray status/actions only after the watcher is stable. Do not add
  a settings window.
- Keep corrections silent by default. If configured, reuse the existing OSD for
  a short message; do not create a new notification system.
- Share transient explicit-selection state between CLI and tray through local
  runtime state, not the durable priority policy.
- Ensure tray monitor menus, hotkeys, and OSD behavior remain unchanged when
  audio discovery or correction fails.

Deliverable: automatic tray-hosted audio policy with safe degradation.

Exit gate:

- Tray starts exactly one watcher and shuts it down cleanly.
- Existing monitor behavior remains unchanged.
- Audio failures are logged/visible through the existing mechanisms but do not
  break the tray.
- Manual CLI commands remain usable while the tray is running.

### Phase 7: Windows/NVIDIA validation and hardening

Goal: validate the real failure mode and remove operational surprises.

Test matrix:

- Clean boot with Focusrite and Realtek available.
- NVIDIA endpoints present at boot, absent at boot, and recreated after a
  driver update.
- Suppression on and off.
- Focusrite first, Realtek first, and no safe configured endpoint.
- Dragonfly or another new non-suppressed device plugged in and selected by
  Windows, then unplugged.
- Manual Windows Sound Settings changes to every role.
- Explicit CLI/tray selection of NVIDIA while suppression is enabled, followed
  by selection of another endpoint.
- Disabled, unplugged, not-present, duplicate-name, and ambiguous-selector
  cases.
- Reboot, tray restart, watcher restart, and concurrent CLI use.

Acceptance:

- Automatic correction occurs only for suppressed NVIDIA or invalid defaults.
- Normal Windows switching remains normal for non-suppressed devices.
- Learned order matches the documented adaptive rules.
- No endpoint is disabled, hidden, uninstalled, or assigned arbitrary state.
- All automated checks pass: `cargo fmt --check`, `cargo check`, and tests.
- Manual results and any Windows-version limitations are recorded here before
  calling v1 complete.

### Phase 8: future extensions, only if justified

Do not block v1 on these items:

- Add policy-only audio state to monitor profiles.
- Measure NVIDIA audio/controller DPC/ISR impact with the exact driver stack.
- If measurements justify it, design a separate explicit enable/disable
  experiment for configured NVIDIA HD Audio functions. Keep the internals
  vendor-neutral and never make default correction disable devices.
- Consider other vendors only when real evidence requires it.
- Do not add a service, third binary, endpoint pinning, or rename unless actual
  use creates a concrete need.

### Verification

Automated checks: selector precedence/ambiguity, ordered selection, suppression
classification, missing endpoint fail-closed behavior, no-op without setter,
role parsing, unique identity refresh, old-config deserialization, and watcher
loop suppression where practical.

Run `cargo fmt --check` and `cargo check`. Audio-changing behavior requires
manual Windows testing, just as display-changing behavior does.

## Remaining validation questions

The product decisions are resolved. These questions require the real Windows
machine or can be answered during implementation.

### Product behavior

1. Resolved: use one priority policy for render and capture; apply it to each
   flow independently where matching endpoints exist.
2. Resolved: enforce Console, Multimedia, and Communications roles for each
   flow.
3. Resolved: priority is ordered. Focusrite wins when available; Realtek is
   next fallback; NVIDIA may remain in the list when not suppressed.
4. Resolved for automatic watcher: intervene only for suppressed/invalid
   defaults; allow newly connected non-suppressed outputs to remain default.
5. Resolved: explicit `set-default` remains allowed for suppressed targets;
   suppression does not disable devices or reject explicit user choice.
6. Resolved: no OS-wide hard block or driver/property mutation in this policy.
7. Resolved: if no non-suppressed priority endpoint exists, leave current
   default unchanged and warn/log. Never choose an arbitrary fallback.
8. Resolved: explicit `audio enforce` selects the highest-priority available
   non-suppressed output; background watcher stays protective.
9. Resolved direction: changing priority should select the new first device
   immediately; one-shot `set-default` remains available for temporary choice.

### Identity/config

10. What exact Focusrite, Realtek, and NVIDIA endpoint names appear on this
    machine? A read-only probe will answer this.
11. Resolved direction: simple policy changes should have CLI commands; TOML
    remains storage and advanced escape hatch.
12. Resolved: use existing tray status/actions; defer a dedicated settings
    window.
13. Resolved: use hybrid NVIDIA detection. Suggest NVIDIA-like endpoints, but
    require explicit enablement before suppression.
14. Persist metadata refresh automatically, or only after confirmation?
    Recommendation: persist only unique replacements.
15. Are there multiple similar endpoints that make substrings ambiguous? Which
    aliases do you want?

16. Resolved: delete a learned endpoint when it disappears. If it is later
    reconnected and Windows selects it, learn it again at the top.

### Setter/watcher

17. Resolved: automatic correction is preferred. Use an isolated undocumented
    PolicyConfig adapter, with read-only detection remaining independently
    usable and Sound Settings as fallback if the adapter fails.
18. Resolved: manually test correction after Windows/NVIDIA driver updates.
19. Resolved: host the background watcher in the existing tray at login. Keep
    `audio watch` as optional foreground diagnostic/development mode; CLI
    commands remain primarily one-shot manual controls.
20. Resolved: the tray starts audio watching automatically. No separate watcher
    enable switch; `suppress_nvidia` controls whether corrective intervention is
    active.
21. Resolved: corrections are silent by default. Reuse the existing OSD as an
    optional short message; do not add another notification mechanism.
22. Resolved: debounce driver-update events, then retry up to three times over
    roughly three seconds. Stop and report failure; never retry indefinitely.

### Future scope

23. Resolved direction: consider endpoint/controller disable/enable later as an
    explicit opt-in feature, only after measurement shows a latency or stability
    benefit. Initially target explicitly configured NVIDIA HD Audio functions;
    keep mechanism vendor-neutral and separate from default policy.
24. Resolved: profiles store the audio priority/suppression policy, not a
    concrete endpoint ID. Endpoint pinning is not crucial for this use case;
    audio devices are usually semantically unique, unlike identical monitors.
    Revisit concrete endpoint pinning only if a real profile workflow requires
    it.
25. Resolved: keep the `monitorctl` name. No rename is needed for the audio
     extension.

## Proposed defaults if you want speed

Assume Windows 11 desktop, render and capture, all three roles, adaptive priority
list, explicit NVIDIA suppression, no auto-hard-block, fail-closed
missing/ambiguous matches, read-only list/default commands, isolated
undocumented setter, one-shot enforcement first, automatic tray watcher,
optional foreground `audio watch`, no disable in v1, CLI policy commands,
existing-tray GUI before any separate settings window, policy-only profiles,
no endpoint pinning, no rename/workspace/service/dependency.

## Sources

- [Core Audio interfaces](https://learn.microsoft.com/en-us/windows/win32/coreaudio/core-audio-interfaces)
- [GetDefaultAudioEndpoint](https://learn.microsoft.com/en-us/windows/win32/api/mmdeviceapi/nf-mmdeviceapi-immdeviceenumerator-getdefaultaudioendpoint)
- [Device properties](https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-properties)
- [Device events](https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-events)
- [IMMNotificationClient](https://learn.microsoft.com/en-gb/windows/win32/api/mmdeviceapi/nn-mmdeviceapi-immnotificationclient)
- [Endpoint device states](https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-state-xxx-constants)
- [Device roles](https://learn.microsoft.com/en-us/windows/win32/coreaudio/device-roles)
- [Endpoint ID strings](https://learn.microsoft.com/en-us/windows/win32/coreaudio/endpoint-id-strings)
- [PKEY_AudioDevice_NeverSetAsDefaultEndpoint](https://github.com/MicrosoftDocs/windows-driver-docs/blob/staging/windows-driver-docs-pr/audio/pkey-audiodevice-neversetasdefaultendpoint.md)
- [IMMDeviceEnumerator in windows 0.62.2](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/Media/Audio/struct.IMMDeviceEnumerator.html)
- [windows-rs issue on undocumented PolicyConfig](https://github.com/microsoft/windows-rs/issues/1355)
