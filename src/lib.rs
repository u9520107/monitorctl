use std::{
    collections::{BTreeMap, BTreeSet},
    mem::size_of,
};

use serde::{Deserialize, Serialize};

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_TARGET_DEVICE_NAME,
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ALL_PATHS,
    QUERY_DISPLAY_CONFIG_FLAGS, QueryDisplayConfig, SDC_ALLOW_CHANGES,
    SDC_ALLOW_PATH_ORDER_CHANGES, SDC_APPLY, SDC_SAVE_TO_DATABASE, SDC_TOPOLOGY_SUPPLIED,
    SDC_USE_SUPPLIED_DISPLAY_CONFIG, SDC_VALIDATE, SET_DISPLAY_CONFIG_FLAGS, SetDisplayConfig,
};

pub mod osd;

pub fn run_cli() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();

    match arguments.as_slice() {
        [command] if matches!(command.as_str(), "--help" | "-h" | "help") => print_help(help()),
        [command, topic] if command == "help" && topic == "profile" => print_help(profile_help()),
        [command, topic] if command == "help" && topic == "hotkey" => print_help(hotkey_help()),
        [command, topic] if command == "help" && topic == "osd" => print_help(osd_help()),
        [topic, command]
            if topic == "profile" && matches!(command.as_str(), "--help" | "-h" | "help") =>
        {
            print_help(profile_help())
        }
        [topic, command]
            if topic == "hotkey" && matches!(command.as_str(), "--help" | "-h" | "help") =>
        {
            print_help(hotkey_help())
        }
        [topic, command]
            if topic == "osd" && matches!(command.as_str(), "--help" | "-h" | "help") =>
        {
            print_help(osd_help())
        }
        [] => list(),
        [command] if command == "list" => list(),
        [command, selector] if command == "enable" => enable_display(selector),
        [command, selector] if command == "disable" => disable_display(selector),
        [command, selector] if command == "toggle" => toggle_display(selector),
        [command] if command == "restore" => restore_previous_active_set(),
        [command, action, name] if command == "profile" && action == "save" => save_profile(name),
        [command, action, name, active, selectors]
            if command == "profile" && action == "create" && active == "--active" =>
        {
            create_profile(name, &profile_selectors(selectors))
        }
        [command, action] if command == "profile" && action == "list" => list_profiles(),
        [command, action, name] if command == "profile" && action == "show" => show_profile(name),
        [command, action, name] if command == "profile" && action == "apply" => apply_profile(name),
        [command, action, name] if command == "profile" && action == "delete" => {
            delete_profile(name)
        }
        [command, action] if command == "hotkey" && action == "list" => list_hotkeys(),
        [command, action, key, target] if command == "hotkey" && action == "set" => {
            set_hotkey(key, target)
        }
        [command, action, key] if command == "hotkey" && action == "delete" => delete_hotkey(key),
        [command, action] if command == "osd" && action == "show" => show_osd("Monitorctl"),
        [command, action, message] if command == "osd" && action == "show" => show_osd(message),
        [command, action, opacity] if command == "osd" && action == "opacity" => {
            set_osd_opacity(opacity)
        }
        _ => Err(usage()),
    }
}

fn print_help(value: &str) -> Result<(), String> {
    print!("{value}");
    Ok(())
}

fn help() -> &'static str {
    "\
Monitorctl controls which Windows-visible displays are active.\n\
Windows Display Settings owns layout, resolution, scaling, refresh rate,\n\
orientation, primary display, and extend/duplicate mode.\n\
\n\
Usage: monitorctl <command>\n\
\n\
Commands:\n\
  list                         List displays, aliases, paths, and active state\n\
  enable <display>             Include one display in desktop\n\
  disable <display>            Remove one display; refuses last active display\n\
  toggle <display>             Enable inactive display or disable active display\n\
  restore                      Restore active set before last successful change\n\
  profile <command>            Manage named active-display sets\n\
  hotkey <command>             Manage tray global-hotkey configuration\n\
  osd <command>                Show OSD or set its opacity\n\
  help, --help, -h             Show this help\n\
\n\
Display selectors: exact alias, exact friendly name, then unique\n\
case-insensitive friendly-name substring. Ambiguous selectors fail.\n\
Aliases are configured in %LOCALAPPDATA%\\monitorctl\\monitorctl.toml.\n\
\n\
Profile workflow:\n\
  monitorctl profile save work\n\
  monitorctl profile create focus --active desk,laptop\n\
  monitorctl profile apply work\n\
  monitorctl profile show work\n\
\n\
Run `monitorctl profile --help` for profile commands.\n"
}

fn profile_help() -> &'static str {
    "\
Usage: monitorctl profile <command>\n\
\n\
Commands:\n\
  save <name>                         Save current active-display set\n\
  create <name> --active <a,b,...>    Create or replace profile from selectors\n\
  list                                List profile names\n\
  show <name>                         Show profile displays\n\
  apply <name>                        Apply profile as complete active set\n\
  delete <name>                       Delete profile\n\
\n\
`save` and `create` replace same-named profiles. `apply` fails without\n\
changing Windows state if any required display is unavailable. Profiles store\n\
only active-display paths; change layout in Windows Display Settings.\n"
}

fn hotkey_help() -> &'static str {
    "\
Usage: monitorctl hotkey <command>\n\
\n\
Commands:\n\
  list                                  List configured hotkeys\n\
  set <key> <action>                    Set or replace one hotkey\n\
  delete <key>                          Delete one hotkey\n\
\n\
Keys require one or more of ctrl, alt, shift, or win plus one letter or digit.\n\
Actions: restore, profile:<name>, or toggle:<display>. Missing profile or\n\
display targets warn but do not prevent saving. Restart monitorctl-tray after\n\
changes.\n"
}

fn osd_help() -> &'static str {
    "\
Usage: monitorctl osd <command>\n\
\n\
Commands:\n\
  show [message]               Show OSD for two seconds\n\
  opacity <0.10..=1.00>        Set OSD opacity (default 0.85)\n"
}

fn show_osd(message: &str) -> Result<(), String> {
    let config = load_config()?;
    unsafe { osd::show_blocking(message, 2_000, config.osd.opacity) }
}

fn set_osd_opacity(value: &str) -> Result<(), String> {
    let opacity = value
        .parse::<f32>()
        .map_err(|_| "OSD opacity must be a number between 0.10 and 1.00")?;
    osd::validate_opacity(opacity)?;
    let mut config = load_config()?;
    config.osd.opacity = opacity;
    save_config(&config)
}

fn list() -> Result<(), String> {
    let config = load_config()?;
    for display in discover_displays()? {
        let aliases = config
            .displays
            .iter()
            .filter_map(|(alias, path)| (path == &display.device_path).then_some(alias.as_str()))
            .collect::<Vec<_>>();
        println!(
            "{}\n  active: {}\n  alias: {}\n  path: {}",
            display_name(&display),
            display.active,
            if aliases.is_empty() {
                "-".into()
            } else {
                aliases.join(", ")
            },
            display.device_path,
        );
    }
    Ok(())
}

fn display_name(display: &Display) -> &str {
    if display.friendly_name.is_empty() {
        "(unnamed monitor)"
    } else {
        &display.friendly_name
    }
}

fn profile_selectors(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|selector| !selector.is_empty())
        .map(str::to_owned)
        .collect()
}

fn list_profiles() -> Result<(), String> {
    for name in load_config()?.profiles.keys() {
        println!("{name}");
    }
    Ok(())
}

fn list_hotkeys() -> Result<(), String> {
    for (key, action) in load_config()?.hotkeys {
        println!("{key} = {action}");
    }
    Ok(())
}

fn set_hotkey(key: &str, action: &str) -> Result<(), String> {
    validate_hotkey(key)?;
    validate_hotkey_action(action)?;
    let mut config = load_config()?;
    warn_missing_hotkey_target(&config, action);
    config.hotkeys.insert(key.into(), action.into());
    save_config(&config)
}

fn delete_hotkey(key: &str) -> Result<(), String> {
    let mut config = load_config()?;
    if config.hotkeys.remove(key).is_none() {
        return Err(format!("hotkey {key:?} does not exist"));
    }
    save_config(&config)
}

pub fn validate_hotkey(key: &str) -> Result<(), String> {
    let mut has_modifier = false;
    let mut has_key = false;
    for part in key.split('+').map(|part| part.trim().to_ascii_lowercase()) {
        match part.as_str() {
            "ctrl" | "control" | "alt" | "shift" | "win" | "windows" => has_modifier = true,
            part if part.len() == 1 && part.as_bytes()[0].is_ascii_alphanumeric() && !has_key => {
                has_key = true
            }
            _ => return Err(format!("invalid hotkey {key:?}")),
        }
    }
    (has_modifier && has_key)
        .then_some(())
        .ok_or_else(|| format!("invalid hotkey {key:?}; use modifier+letter-or-digit"))
}

fn validate_hotkey_action(action: &str) -> Result<(), String> {
    match action {
        "restore" => Ok(()),
        action
            if action
                .strip_prefix("profile:")
                .is_some_and(|name| !name.is_empty()) =>
        {
            Ok(())
        }
        action
            if action
                .strip_prefix("toggle:")
                .is_some_and(|display| !display.is_empty()) =>
        {
            Ok(())
        }
        _ => Err(format!("invalid hotkey action {action:?}")),
    }
}

fn warn_missing_hotkey_target(config: &Config, action: &str) {
    if let Some(name) = action.strip_prefix("profile:") {
        if !config.profiles.contains_key(name) {
            eprintln!("warning: profile {name:?} does not exist yet; hotkey saved");
        }
    } else if let Some(selector) = action.strip_prefix("toggle:") {
        let available = discover_displays().and_then(|displays| {
            resolve_display(&displays, &config.displays, selector).map(|_| ())
        });
        if available.is_err() {
            eprintln!("warning: display {selector:?} is not available now; hotkey saved");
        }
    }
}

fn show_profile(name: &str) -> Result<(), String> {
    let config = load_config()?;
    let paths = config
        .profiles
        .get(name)
        .ok_or_else(|| format!("profile {name:?} does not exist"))?;
    println!("{name}");
    for path in paths {
        let alias = config
            .displays
            .iter()
            .find_map(|(alias, value)| (value == path).then_some(alias));
        println!("  {}", alias.map_or(path.as_str(), String::as_str));
    }
    Ok(())
}

fn display_config(flags: QUERY_DISPLAY_CONFIG_FLAGS) -> Result<DisplayConfig, String> {
    let mut path_count = 0;
    let mut mode_count = 0;

    unsafe {
        GetDisplayConfigBufferSizes(flags, &mut path_count, &mut mode_count)
            .ok()
            .map_err(|error| format!("GetDisplayConfigBufferSizes failed: {error}"))?;
    }

    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];

    unsafe {
        QueryDisplayConfig(
            flags,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
        .ok()
        .map_err(|error| format!("QueryDisplayConfig failed: {error}"))?;
    }

    paths.truncate(path_count as usize);
    Ok(DisplayConfig { paths })
}

fn visible_monitor_paths(config: &DisplayConfig) -> Result<Vec<DISPLAYCONFIG_PATH_INFO>, String> {
    let mut device_paths = BTreeSet::new();
    let mut monitors = Vec::new();

    for path in &config.paths {
        let target = target_name(path)?;
        let device_path = utf16_string(&target.monitorDevicePath);
        if !device_path.is_empty() && device_paths.insert(device_path) {
            monitors.push(*path);
        }
    }

    Ok(monitors)
}

fn clear_mode_indices(paths: &mut [DISPLAYCONFIG_PATH_INFO]) {
    for path in paths {
        path.sourceInfo.Anonymous.modeInfoIdx = u32::MAX;
        path.targetInfo.Anonymous.modeInfoIdx = u32::MAX;
    }
}

fn set_display_config(
    paths: &[DISPLAYCONFIG_PATH_INFO],
    flags: SET_DISPLAY_CONFIG_FLAGS,
    operation: &str,
) -> Result<(), String> {
    let result = unsafe { SetDisplayConfig(Some(paths), None, flags) };
    if result == 0 {
        Ok(())
    } else if result == 5 {
        Err(format!(
            "SetDisplayConfig {operation} failed: access denied; run from the local interactive Windows desktop"
        ))
    } else {
        Err(format!("SetDisplayConfig {operation} failed: {result}"))
    }
}

fn target_name(path: &DISPLAYCONFIG_PATH_INFO) -> Result<DISPLAYCONFIG_TARGET_DEVICE_NAME, String> {
    let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME::default();
    target.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
        size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
        adapterId: path.targetInfo.adapterId,
        id: path.targetInfo.id,
    };

    unsafe {
        let result = DisplayConfigGetDeviceInfo(&mut target.header);
        if result != 0 {
            return Err(format!("DisplayConfigGetDeviceInfo failed: {result}"));
        }
    }

    Ok(target)
}

fn utf16_string(value: &[u16]) -> String {
    String::from_utf16_lossy(value.split(|unit| *unit == 0).next().unwrap_or_default())
}

fn usage() -> String {
    "invalid command; run `monitorctl --help`".into()
}

struct DisplayConfig {
    paths: Vec<DISPLAYCONFIG_PATH_INFO>,
}

/// User-managed monitor mappings. Device paths are Windows identifiers, never indexes.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub displays: BTreeMap<String, String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    pub hotkeys: BTreeMap<String, String>,
    #[serde(default)]
    pub osd: OsdConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_active: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OsdConfig {
    #[serde(default = "default_osd_opacity")]
    pub opacity: f32,
}

fn default_osd_opacity() -> f32 {
    0.85
}

impl Default for OsdConfig {
    fn default() -> Self {
        Self {
            opacity: default_osd_opacity(),
        }
    }
}

impl Config {
    pub fn from_toml(value: &str) -> Result<Self, String> {
        let config: Self =
            toml::from_str(value).map_err(|error| format!("invalid config: {error}"))?;
        osd::validate_opacity(config.osd.opacity)
            .map_err(|error| format!("invalid config: {error}"))?;
        Ok(config)
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|error| format!("cannot write config: {error}"))
    }
}

pub fn config_path() -> Result<std::path::PathBuf, String> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .ok_or("LOCALAPPDATA is not set; cannot locate monitorctl configuration")?;
    Ok(std::path::PathBuf::from(local_app_data)
        .join("monitorctl")
        .join("monitorctl.toml"))
}

pub fn load_config() -> Result<Config, String> {
    let path = config_path()?;
    match std::fs::read_to_string(&path) {
        Ok(value) => Config::from_toml(&value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

pub fn save_config(config: &Config) -> Result<(), String> {
    let path = config_path()?;
    let directory = path.parent().expect("config path has parent");
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    std::fs::write(&path, config.to_toml()?)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Display {
    pub friendly_name: String,
    pub device_path: String,
    pub active: bool,
}

pub fn discover_displays() -> Result<Vec<Display>, String> {
    let mut displays = visible_monitor_paths(&display_config(QDC_ALL_PATHS)?)?
        .iter()
        .map(display_from_path)
        .collect::<Result<Vec<_>, _>>()?;
    sort_displays(&mut displays);
    Ok(displays)
}

fn sort_displays(displays: &mut [Display]) {
    displays.sort_by(|left, right| {
        left.friendly_name
            .cmp(&right.friendly_name)
            .then(left.device_path.cmp(&right.device_path))
    });
}

/// Resolve alias, then exact friendly name, then one case-insensitive name substring.
pub fn resolve_display<'a>(
    displays: &'a [Display],
    aliases: &BTreeMap<String, String>,
    selector: &str,
) -> Result<&'a Display, String> {
    if let Some(device_path) = aliases.get(selector) {
        return displays
            .iter()
            .find(|display| display.device_path == *device_path)
            .ok_or_else(|| format!("display alias {selector:?} is not present"));
    }

    if let Some(display) = displays
        .iter()
        .find(|display| display.friendly_name == selector)
    {
        return Ok(display);
    }

    let selector = selector.to_lowercase();
    let mut matches = displays
        .iter()
        .filter(|display| display.friendly_name.to_lowercase().contains(&selector));
    let Some(display) = matches.next() else {
        return Err(format!("no display matches {selector:?}"));
    };
    if matches.next().is_some() {
        let names = displays
            .iter()
            .filter(|candidate| candidate.friendly_name.to_lowercase().contains(&selector))
            .map(|candidate| candidate.friendly_name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "display selector {selector:?} is ambiguous: {names}"
        ));
    }
    Ok(display)
}

fn display_from_path(path: &DISPLAYCONFIG_PATH_INFO) -> Result<Display, String> {
    let target = target_name(path)?;
    Ok(Display {
        friendly_name: utf16_string(&target.monitorFriendlyDeviceName),
        device_path: utf16_string(&target.monitorDevicePath),
        active: path.flags & 1 != 0,
    })
}

pub fn enable_display(selector: &str) -> Result<(), String> {
    change_display(selector, true)
}

pub fn disable_display(selector: &str) -> Result<(), String> {
    change_display(selector, false)
}

pub fn toggle_display(selector: &str) -> Result<(), String> {
    let config = load_config()?;
    let displays = discover_displays()?;
    let display = resolve_display(&displays, &config.displays, selector)?;
    change_display(selector, !display.active)
}

/// Toggle a Windows display identified by its exact device path.
///
/// This is for UI callers that already selected one discovered display.
pub fn toggle_display_path(device_path: &str) -> Result<(), String> {
    let mut config = load_config()?;
    let displays = discover_displays()?;
    let display = displays
        .iter()
        .find(|display| display.device_path == device_path)
        .ok_or_else(|| format!("display path is not available: {device_path}"))?;
    change_display_path(&mut config, &displays, device_path, !display.active)
}

fn change_display(selector: &str, active: bool) -> Result<(), String> {
    let mut config = load_config()?;
    let displays = discover_displays()?;
    let display = resolve_display(&displays, &config.displays, selector)?;
    if display.active == active {
        return Err(format!(
            "display {selector:?} is already {}",
            if active { "active" } else { "inactive" }
        ));
    }
    change_display_path(&mut config, &displays, &display.device_path, active)
}

fn change_display_path(
    config: &mut Config,
    displays: &[Display],
    device_path: &str,
    active: bool,
) -> Result<(), String> {
    let mut desired = active_paths(&displays);
    if active {
        desired.insert(device_path.into());
    } else {
        desired.remove(device_path);
    }
    apply_active_set(config, displays, &desired)
}

pub fn create_profile(name: &str, selectors: &[String]) -> Result<(), String> {
    if name.is_empty() {
        return Err("profile name cannot be empty".into());
    }
    if selectors.is_empty() {
        return Err("profile must include at least one display".into());
    }

    let mut config = load_config()?;
    let displays = discover_displays()?;
    let mut paths = BTreeSet::new();
    for selector in selectors {
        let display = resolve_display(&displays, &config.displays, selector)?;
        paths.insert(display.device_path.clone());
    }
    config
        .profiles
        .insert(name.into(), paths.into_iter().collect());
    save_config(&config)
}

pub fn save_profile(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("profile name cannot be empty".into());
    }

    let mut config = load_config()?;
    let active = active_paths(&discover_displays()?);
    if active.is_empty() {
        return Err("no active displays to save".into());
    }
    config
        .profiles
        .insert(name.into(), active.into_iter().collect());
    save_config(&config)
}

pub fn delete_profile(name: &str) -> Result<(), String> {
    let mut config = load_config()?;
    if config.profiles.remove(name).is_none() {
        return Err(format!("profile {name:?} does not exist"));
    }
    save_config(&config)
}

pub fn apply_profile(name: &str) -> Result<(), String> {
    let mut config = load_config()?;
    let profile = config
        .profiles
        .get(name)
        .cloned()
        .ok_or_else(|| format!("profile {name:?} does not exist"))?;
    let displays = discover_displays()?;
    let desired = profile.iter().cloned().collect::<BTreeSet<_>>();
    if desired.is_empty() {
        return Err(format!("profile {name:?} has no displays"));
    }
    let present = displays
        .iter()
        .map(|display| display.device_path.as_str())
        .collect::<BTreeSet<_>>();
    let missing = desired
        .iter()
        .filter(|path| !present.contains(path.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "profile {name:?} requires unavailable displays: {}",
            missing.join(", ")
        ));
    }
    apply_active_set(&mut config, &displays, &desired)
}

pub fn restore_previous_active_set() -> Result<(), String> {
    let mut config = load_config()?;
    let desired = config
        .previous_active
        .clone()
        .ok_or("no previous active-display set saved")?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let displays = discover_displays()?;
    let present = displays
        .iter()
        .map(|display| display.device_path.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(missing) = desired.iter().find(|path| !present.contains(path.as_str())) {
        return Err(format!(
            "previous active-display set requires unavailable display: {missing}"
        ));
    }
    apply_active_set(&mut config, &displays, &desired)
}

fn active_paths(displays: &[Display]) -> BTreeSet<String> {
    displays
        .iter()
        .filter(|display| display.active)
        .map(|display| display.device_path.clone())
        .collect()
}

fn apply_active_set(
    config: &mut Config,
    displays: &[Display],
    desired: &BTreeSet<String>,
) -> Result<(), String> {
    if desired.is_empty() {
        return Err("refusing to disable the last active display".into());
    }
    let previous_active = active_paths(displays);
    if desired == &previous_active {
        return Ok(());
    }
    let available = displays
        .iter()
        .map(|display| display.device_path.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(missing) = desired
        .iter()
        .find(|path| !available.contains(path.as_str()))
    {
        return Err(format!("display path is not available: {missing}"));
    }

    let topology = display_config(QDC_ALL_PATHS)?;
    let saved_flags = SDC_TOPOLOGY_SUPPLIED | SDC_ALLOW_PATH_ORDER_CHANGES;
    let best_mode_flags = SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_ALLOW_CHANGES;
    let (paths, flags) = match validated_paths(&topology, desired, saved_flags) {
        Ok(paths) => (paths, saved_flags),
        Err(_) => (
            validated_paths(&topology, desired, best_mode_flags)?,
            best_mode_flags,
        ),
    };

    let old_previous_active = config.previous_active.clone();
    config.previous_active = Some(previous_active.into_iter().collect());
    save_config(config)?;
    if let Err(error) = set_display_config(
        &paths,
        SDC_APPLY
            | flags
            | if flags == best_mode_flags {
                SDC_SAVE_TO_DATABASE
            } else {
                SET_DISPLAY_CONFIG_FLAGS(0)
            },
        "apply",
    ) {
        config.previous_active = old_previous_active;
        return match save_config(config) {
            Ok(()) => Err(error),
            Err(restore_error) => Err(format!(
                "{error}; also could not restore previous active-display set: {restore_error}"
            )),
        };
    }
    Ok(())
}

fn validated_paths(
    topology: &DisplayConfig,
    desired: &BTreeSet<String>,
    flags: SET_DISPLAY_CONFIG_FLAGS,
) -> Result<Vec<DISPLAYCONFIG_PATH_INFO>, String> {
    let mut candidates = BTreeMap::<String, Vec<DISPLAYCONFIG_PATH_INFO>>::new();
    for path in &topology.paths {
        let display = display_from_path(path)?;
        if desired.contains(&display.device_path) {
            let mut path = *path;
            path.flags |= 1;
            candidates
                .entry(display.device_path)
                .or_default()
                .push(path);
        }
    }
    if candidates.len() != desired.len() {
        return Err("display topology changed before apply; no change made".into());
    }
    for paths in candidates.values_mut() {
        paths.sort_by_key(|path| std::cmp::Reverse(path.flags & 1));
    }
    let candidates = desired
        .iter()
        .map(|path| candidates.remove(path).expect("desired path was grouped"))
        .collect::<Vec<_>>();
    let mut selected = Vec::with_capacity(candidates.len());
    validate_candidates(&candidates, &mut selected, flags)
        .ok_or_else(|| "no valid display path combination found; no change made".into())
}

fn validate_candidates(
    candidates: &[Vec<DISPLAYCONFIG_PATH_INFO>],
    selected: &mut Vec<DISPLAYCONFIG_PATH_INFO>,
    flags: SET_DISPLAY_CONFIG_FLAGS,
) -> Option<Vec<DISPLAYCONFIG_PATH_INFO>> {
    if let Some((paths, rest)) = candidates.split_first() {
        for path in paths {
            selected.push(*path);
            if let Some(valid) = validate_candidates(rest, selected, flags) {
                return Some(valid);
            }
            selected.pop();
        }
        None
    } else {
        let mut paths = selected.clone();
        clear_mode_indices(&mut paths);
        (unsafe { SetDisplayConfig(Some(&paths), None, SDC_VALIDATE | flags) } == 0)
            .then_some(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(name: &str, path: &str) -> Display {
        Display {
            friendly_name: name.into(),
            device_path: path.into(),
            active: true,
        }
    }

    #[test]
    fn resolves_alias_before_name() {
        let displays = [display("Desk", "first"), display("Other", "desk")];
        let aliases = BTreeMap::from([("Desk".into(), "desk".into())]);
        assert_eq!(
            resolve_display(&displays, &aliases, "Desk")
                .unwrap()
                .friendly_name,
            "Other"
        );
    }

    #[test]
    fn rejects_ambiguous_partial_name() {
        let displays = [display("Dell Left", "left"), display("Dell Right", "right")];
        assert!(resolve_display(&displays, &BTreeMap::new(), "dell").is_err());
    }

    #[test]
    fn lists_displays_by_friendly_name() {
        let mut displays = [display("Gigabyte", "second"), display("Dell", "first")];
        sort_displays(&mut displays);
        assert_eq!(displays[0].friendly_name, "Dell");
    }

    #[test]
    fn parses_profile_selectors() {
        assert_eq!(profile_selectors(" desk, laptop ,,"), ["desk", "laptop"]);
    }

    #[test]
    fn ignores_active_set_no_op_without_saving_restore_state() {
        let displays = [display("Desk", "desk")];
        let desired = BTreeSet::from(["desk".into()]);
        let mut config = Config::default();

        apply_active_set(&mut config, &displays, &desired).unwrap();

        assert_eq!(config.previous_active, None);
    }

    #[test]
    fn help_covers_profile_workflow() {
        assert!(help().contains("monitorctl profile save work"));
        assert!(profile_help().contains("create <name>"));
        assert!(hotkey_help().contains("set <key> <action>"));
        assert!(osd_help().contains("opacity"));
    }

    #[test]
    fn validates_hotkey_syntax() {
        assert!(validate_hotkey("ctrl+alt+w").is_ok());
        assert!(validate_hotkey("w").is_err());
        assert!(validate_hotkey("ctrl+f1").is_err());
    }

    #[test]
    fn defaults_and_validates_osd_opacity() {
        assert_eq!(Config::default().osd.opacity, 0.85);
        assert_eq!(Config::from_toml("[osd]").unwrap().osd.opacity, 0.85);
        assert!(Config::from_toml("[osd]\nopacity = 0.09").is_err());
    }
}
