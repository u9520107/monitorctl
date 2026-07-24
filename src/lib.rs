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

fn disable(config: &DisplayConfig, index: &str, apply: bool) -> Result<(), String> {
    let display = select_display(index)?;
    let index = config
        .paths
        .iter()
        .position(|path| {
            display_from_path(path).is_ok_and(|current| current.device_path == display.device_path)
        })
        .ok_or_else(|| format!("display {index:?} is not active"))?;

    let mut paths = without_display(&config.paths, index)?;
    clear_mode_indices(&mut paths);
    let flags = SDC_TOPOLOGY_SUPPLIED | SDC_ALLOW_PATH_ORDER_CHANGES;

    set_display_config(&paths, SDC_VALIDATE | flags, "validation")?;
    if !apply {
        println!(
            "validated disable of display {index}; rerun with --apply to change Windows topology"
        );
        return Ok(());
    }

    set_display_config(&paths, SDC_APPLY | flags, "apply")?;
    println!("disabled display {index}");
    Ok(())
}

fn enable(config: &DisplayConfig, index: &str, apply: bool) -> Result<(), String> {
    let display = select_display(index)?;
    let target = *visible_monitor_paths(&display_config(QDC_ALL_PATHS)?)?
        .iter()
        .find(|path| {
            display_from_path(path).is_ok_and(|current| current.device_path == display.device_path)
        })
        .ok_or_else(|| format!("display {index:?} is not available"))?;

    if config.paths.iter().any(|path| same_target(path, &target)) {
        return Err(format!("display index {index} is already active"));
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
            "validated enable of display {index}; rerun with --apply to change Windows topology"
        );
        return Ok(());
    }

    set_display_config(&paths, SDC_APPLY | flags, "apply")?;
    println!("enabled display {index}");
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
}

impl Config {
    pub fn from_toml(value: &str) -> Result<Self, String> {
        toml::from_str(value).map_err(|error| format!("invalid config: {error}"))
    }

    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|error| format!("cannot write config: {error}"))
    }
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
    let config = match std::fs::read_to_string("monitorctl.toml") {
        Ok(value) => Config::from_toml(&value)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Config::default(),
        Err(error) => return Err(format!("cannot read monitorctl.toml: {error}")),
    };
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
