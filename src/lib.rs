use std::{
    collections::{BTreeMap, BTreeSet},
    mem::size_of,
};

use serde::{Deserialize, Serialize};

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_TARGET_DEVICE_NAME,
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ALL_PATHS, QDC_ONLY_ACTIVE_PATHS,
    QUERY_DISPLAY_CONFIG_FLAGS, QueryDisplayConfig, SDC_ALLOW_PATH_ORDER_CHANGES, SDC_APPLY,
    SDC_TOPOLOGY_EXTEND, SDC_TOPOLOGY_SUPPLIED, SDC_VALIDATE, SET_DISPLAY_CONFIG_FLAGS,
    SetDisplayConfig,
};
use windows::Win32::Foundation::LUID;

pub fn run_probe() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let config = active_display_config()?;

    match arguments.as_slice() {
        [] => list_active_displays(&config),
        [command] if command == "list" => list_active_displays(&config),
        [command, all] if command == "list" && all == "--all" => {
            list_all_displays(&display_config(QDC_ALL_PATHS)?)
        }
        [command, selector] if command == "disable" => disable(&config, selector, false),
        [command, selector, apply] if command == "disable" && apply == "--apply" => {
            disable(&config, selector, true)
        }
        [command, selector] if command == "enable" => enable(&config, selector, false),
        [command, selector, apply] if command == "enable" && apply == "--apply" => {
            enable(&config, selector, true)
        }
        [command] if command == "restore" => restore(false),
        [command, apply] if command == "restore" && apply == "--apply" => restore(true),
        _ => Err(usage()),
    }
}

fn active_display_config() -> Result<DisplayConfig, String> {
    display_config(QDC_ONLY_ACTIVE_PATHS)
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

fn list_active_displays(config: &DisplayConfig) -> Result<(), String> {
    list_display_paths(config, "ACTIVE DISPLAY PATHS")
}

fn list_display_paths(config: &DisplayConfig, heading: &str) -> Result<(), String> {
    println!("{heading}");

    for (index, path) in config.paths.iter().enumerate() {
        let target = target_name(path)?;
        let friendly_name = utf16_string(&target.monitorFriendlyDeviceName);
        let device_path = utf16_string(&target.monitorDevicePath);

        println!(
            "{index}: {}\n  active: {}\n  target: adapter {} id {}\n  path: {}",
            if friendly_name.is_empty() {
                "(unnamed monitor)"
            } else {
                &friendly_name
            },
            path.flags & 1 != 0,
            luid(path.targetInfo.adapterId),
            path.targetInfo.id,
            device_path,
        );
    }

    Ok(())
}

fn list_all_displays(config: &DisplayConfig) -> Result<(), String> {
    println!("ALL WINDOWS-VISIBLE MONITORS");
    for (index, path) in visible_monitor_paths(config)?.iter().enumerate() {
        let target = target_name(path)?;
        let device_path = utf16_string(&target.monitorDevicePath);
        let friendly_name = utf16_string(&target.monitorFriendlyDeviceName);
        println!(
            "{index}: {}\n  active: {}\n  target: adapter {} id {}\n  path: {}",
            if friendly_name.is_empty() {
                "(unnamed monitor)"
            } else {
                &friendly_name
            },
            path.flags & 1 != 0,
            luid(path.targetInfo.adapterId),
            path.targetInfo.id,
            device_path,
        );
    }

    Ok(())
}

fn disable(config: &DisplayConfig, selector: &str, apply: bool) -> Result<(), String> {
    let display = select_display(selector)?;
    let index = config
        .paths
        .iter()
        .position(|path| {
            display_from_path(path).is_ok_and(|current| current.device_path == display.device_path)
        })
        .ok_or_else(|| format!("display {selector:?} is not active"))?;

    let mut paths = without_display(&config.paths, index)?;
    clear_mode_indices(&mut paths);
    let flags = SDC_TOPOLOGY_SUPPLIED | SDC_ALLOW_PATH_ORDER_CHANGES;

    set_display_config(&paths, SDC_VALIDATE | flags, "validation")?;
    if !apply {
        println!(
            "validated disable of display {selector}; rerun with --apply to change Windows topology"
        );
        return Ok(());
    }

    set_display_config(&paths, SDC_APPLY | flags, "apply")?;
    println!("disabled display {selector}");
    Ok(())
}

fn enable(config: &DisplayConfig, selector: &str, apply: bool) -> Result<(), String> {
    let display = select_display(selector)?;
    let target = *visible_monitor_paths(&display_config(QDC_ALL_PATHS)?)?
        .iter()
        .find(|path| {
            display_from_path(path).is_ok_and(|current| current.device_path == display.device_path)
        })
        .ok_or_else(|| format!("display {selector:?} is not available"))?;

    if config.paths.iter().any(|path| same_target(path, &target)) {
        return Err(format!("display {selector:?} is already active"));
    }

    let mut paths = config.paths.clone();
    let mut target = target;
    target.flags |= 1;
    paths.push(target);
    clear_mode_indices(&mut paths);

    let flags = SDC_TOPOLOGY_SUPPLIED | SDC_ALLOW_PATH_ORDER_CHANGES;
    set_display_config(&paths, SDC_VALIDATE | flags, "validation")?;
    if !apply {
        println!(
            "validated enable of display {selector}; rerun with --apply to change Windows topology"
        );
        return Ok(());
    }

    set_display_config(&paths, SDC_APPLY | flags, "apply")?;
    println!("enabled display {selector}");
    Ok(())
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

fn same_target(left: &DISPLAYCONFIG_PATH_INFO, right: &DISPLAYCONFIG_PATH_INFO) -> bool {
    left.targetInfo.adapterId == right.targetInfo.adapterId
        && left.targetInfo.id == right.targetInfo.id
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

fn restore(apply: bool) -> Result<(), String> {
    let flags = SDC_TOPOLOGY_EXTEND;
    set_topology(SDC_VALIDATE | flags, "restore validation")?;
    if !apply {
        println!(
            "validated Windows extended-topology restore; rerun with --apply to change Windows topology"
        );
        return Ok(());
    }

    set_topology(SDC_APPLY | flags, "restore apply")?;
    println!("restored Windows extended topology");
    Ok(())
}

fn set_topology(flags: SET_DISPLAY_CONFIG_FLAGS, operation: &str) -> Result<(), String> {
    let result = unsafe { SetDisplayConfig(None, None, flags) };
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

fn luid(value: LUID) -> String {
    format!("{:08x}:{:08x}", value.HighPart as u32, value.LowPart)
}

fn usage() -> String {
    "usage: monitorctl-probe [list [--all]] | disable <display> [--apply] | enable <display> [--apply] | restore [--apply]".into()
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_active: Option<Vec<String>>,
}

impl Config {
    pub fn from_toml(value: &str) -> Result<Self, String> {
        toml::from_str(value).map_err(|error| format!("invalid config: {error}"))
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
    visible_monitor_paths(&display_config(QDC_ALL_PATHS)?)?
        .iter()
        .map(display_from_path)
        .collect()
}

fn select_display(selector: &str) -> Result<Display, String> {
    let config = load_config()?;
    resolve_display(&discover_displays()?, &config.displays, selector).cloned()
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

fn without_display(
    paths: &[DISPLAYCONFIG_PATH_INFO],
    index: usize,
) -> Result<Vec<DISPLAYCONFIG_PATH_INFO>, String> {
    if paths.len() <= 1 {
        return Err("refusing to disable the last active display".into());
    }
    if index >= paths.len() {
        return Err(format!("display index {index} is not active"));
    }

    Ok(paths
        .iter()
        .enumerate()
        .filter_map(|(current, path)| (current != index).then_some(*path))
        .collect())
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

    let mut desired = active_paths(&displays);
    if active {
        desired.insert(display.device_path.clone());
    } else {
        desired.remove(&display.device_path);
    }
    apply_active_set(&mut config, &displays, &desired)
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

    config.previous_active = Some(active_paths(displays).into_iter().collect());
    save_config(config)?;

    let paths = visible_monitor_paths(&display_config(QDC_ALL_PATHS)?)?
        .into_iter()
        .filter_map(|mut path| {
            let display = display_from_path(&path).ok()?;
            desired.contains(&display.device_path).then(|| {
                path.flags |= 1;
                path
            })
        })
        .collect::<Vec<_>>();
    if paths.len() != desired.len() {
        return Err("display topology changed before apply; no change made".into());
    }
    let mut paths = paths;
    clear_mode_indices(&mut paths);
    let flags = SDC_TOPOLOGY_SUPPLIED | SDC_ALLOW_PATH_ORDER_CHANGES;
    set_display_config(&paths, SDC_VALIDATE | flags, "validation")?;
    set_display_config(&paths, SDC_APPLY | flags, "apply")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_to_remove_last_display() {
        assert!(without_display(&[DISPLAYCONFIG_PATH_INFO::default()], 0).is_err());
    }

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
}
