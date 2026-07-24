#![allow(unsafe_op_in_unsafe_fn)]

use std::{collections::BTreeSet, mem::size_of};

use monitorctl_core::{Display, discover_displays, load_config};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::{
            Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey},
            Shell::{
                NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
                Shell_NotifyIconW,
            },
            WindowsAndMessaging::{
                AppendMenuW, CW_USEDEFAULT, CreateIconFromResourceEx, CreatePopupMenu,
                CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW,
                GetCursorPos, GetMessageW, HMENU, IMAGE_FLAGS, MENU_ITEM_FLAGS, MF_CHECKED,
                MF_DISABLED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MSG, PostQuitMessage,
                RegisterClassW, SetForegroundWindow, TPM_RIGHTBUTTON, TrackPopupMenu,
                TranslateMessage, WM_APP, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY, WM_HOTKEY,
                WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
            },
        },
    },
    core::{PCWSTR, Result as WindowsResult},
};

const CLASS_NAME: &str = "monitorctl-tray";
const TRAY_MESSAGE: u32 = WM_APP + 1;
const MENU_MONITOR_BASE: u32 = 1_000;
const MENU_PROFILE_BASE: u32 = 2_000;
const MENU_RESTORE: u32 = 3_000;
const MENU_QUIT: u32 = 3_001;
const HOTKEY_BASE: i32 = 4_000;
const TRAY_ICON: &[u8] = include_bytes!("../assets/monitorctl.ico");

fn main() -> WindowsResult<()> {
    unsafe {
        let class_name = wide(CLASS_NAME);
        let class = WNDCLASSW {
            lpszClassName: PCWSTR(class_name.as_ptr()),
            lpfnWndProc: Some(window_proc),
            ..Default::default()
        };
        RegisterClassW(&class);
        let window = CreateWindowExW(
            Default::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(class_name.as_ptr()),
            Default::default(),
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            None,
            None,
            None,
            None,
        )?;
        monitorctl_core::osd::initialize(false).map_err(|error| {
            windows::core::Error::new(windows::core::HRESULT(0x8000_4005_u32 as i32), error)
        })?;
        add_icon(window)?;
        register_hotkeys(window);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}

#[allow(unsafe_op_in_unsafe_fn)]
unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        TRAY_MESSAGE
            if matches!(
                lparam.0 as u32,
                WM_LBUTTONUP | WM_RBUTTONUP | WM_CONTEXTMENU
            ) =>
        {
            show_menu(window);
            LRESULT(0)
        }
        WM_COMMAND => {
            run_menu_action(window, (wparam.0 & 0xffff) as u32);
            LRESULT(0)
        }
        WM_HOTKEY => {
            run_hotkey(window, wparam.0 as i32);
            LRESULT(0)
        }
        WM_DESTROY => {
            delete_icon(window);
            unregister_hotkeys(window);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(window, message, wparam, lparam),
    }
}

unsafe fn show_menu(window: HWND) {
    let Ok(state) = menu_state() else {
        show_osd("Cannot read monitor state", 5_000);
        return;
    };
    let Ok(menu) = CreatePopupMenu() else {
        return;
    };
    append(menu, MF_STRING | MF_DISABLED, 0, "Monitors");
    for (index, display) in state.displays.iter().enumerate() {
        let unavailable = display.path.is_none();
        let unsafe_disable = display.active && state.active_count == 1;
        let flags = if unavailable || unsafe_disable {
            MF_STRING | MF_GRAYED
        } else {
            MF_STRING
        };
        let label = format!(
            "{}{}",
            display.label,
            if display.active { "  On" } else { "  Off" }
        );
        append(menu, flags, MENU_MONITOR_BASE + index as u32, &label);
    }
    append(menu, MF_SEPARATOR, 0, "");
    append(menu, MF_STRING | MF_DISABLED, 0, "Profiles");
    for (index, profile) in state.profiles.iter().enumerate() {
        let flags = if profile.available {
            MF_STRING
                | if profile.active {
                    MF_CHECKED
                } else {
                    MENU_ITEM_FLAGS(0)
                }
        } else {
            MF_STRING | MF_GRAYED
        };
        append(menu, flags, MENU_PROFILE_BASE + index as u32, &profile.name);
    }
    append(menu, MF_SEPARATOR, 0, "");
    append(
        menu,
        if state.restore_available {
            MF_STRING
        } else {
            MF_STRING | MF_GRAYED
        },
        MENU_RESTORE,
        "Restore",
    );
    append(menu, MF_STRING, MENU_QUIT, "Quit");
    let mut point = Default::default();
    let _ = GetCursorPos(&mut point);
    let _ = SetForegroundWindow(window);
    let _ = TrackPopupMenu(
        menu,
        TPM_RIGHTBUTTON,
        point.x,
        point.y,
        Some(0),
        window,
        None,
    );
    let _ = DestroyMenu(menu);
}

unsafe fn append(menu: HMENU, flags: MENU_ITEM_FLAGS, id: u32, label: &str) {
    let label = wide(label);
    let _ = AppendMenuW(menu, flags, id as usize, PCWSTR(label.as_ptr()));
}

unsafe fn run_menu_action(window: HWND, command: u32) {
    let (result, success) = match command {
        MENU_QUIT => {
            let _ = DestroyWindow(window);
            return;
        }
        MENU_RESTORE => (
            monitorctl_core::restore_previous_active_set(),
            "Restored previous display set".into(),
        ),
        command if command >= MENU_MONITOR_BASE && command < MENU_PROFILE_BASE => {
            let item = menu_state().and_then(|state| {
                state
                    .displays
                    .get((command - MENU_MONITOR_BASE) as usize)
                    .map(|display| (display.path.clone(), display.label.clone(), display.active))
                    .ok_or_else(|| "invalid monitor menu item".into())
            });
            match item {
                Ok((Some(path), label, active)) => (
                    monitorctl_core::toggle_display_path(&path),
                    format!("{} {label}", if active { "Disabled" } else { "Enabled" }),
                ),
                Ok((None, _, _)) => (Err("display is unavailable".into()), String::new()),
                Err(error) => (Err(error), String::new()),
            }
        }
        command if command >= MENU_PROFILE_BASE && command < MENU_RESTORE => {
            let profile = menu_state().and_then(|state| {
                state
                    .profiles
                    .get((command - MENU_PROFILE_BASE) as usize)
                    .map(|profile| profile.name.clone())
                    .ok_or_else(|| "invalid profile menu item".into())
            });
            match profile {
                Ok(name) => (
                    monitorctl_core::apply_profile(&name),
                    format!("Applied profile {name}"),
                ),
                Err(error) => (Err(error), String::new()),
            }
        }
        _ => return,
    };
    show_result(result, &success);
}

unsafe fn register_hotkeys(window: HWND) {
    let Ok(config) = load_config() else {
        return;
    };
    for (index, key) in config.hotkeys.keys().enumerate() {
        if let Some((modifiers, virtual_key)) = parse_hotkey(key) {
            if RegisterHotKey(
                Some(window),
                HOTKEY_BASE + index as i32,
                modifiers,
                virtual_key,
            )
            .is_err()
            {
                show_osd(&format!("Hotkey unavailable: {key}"), 5_000);
            }
        } else {
            show_osd(&format!("Invalid hotkey: {key}"), 5_000);
        }
    }
}

unsafe fn unregister_hotkeys(window: HWND) {
    let Ok(config) = load_config() else {
        return;
    };
    for index in 0..config.hotkeys.len() {
        let _ = UnregisterHotKey(Some(window), HOTKEY_BASE + index as i32);
    }
}

unsafe fn run_hotkey(_window: HWND, id: i32) {
    let action = load_config().and_then(|config| {
        config
            .hotkeys
            .iter()
            .nth((id - HOTKEY_BASE) as usize)
            .ok_or_else(|| "unknown hotkey".into())
            .map(|(_, action)| action.clone())
    });
    let (result, success) = match action {
        Ok(action) => (run_action(&action), action_summary(&action)),
        Err(error) => (Err(error), String::new()),
    };
    show_result(result, &success);
}

fn action_summary(action: &str) -> String {
    match action {
        "restore" => "Restored previous display set".into(),
        action if let Some(name) = action.strip_prefix("profile:") => {
            format!("Applied profile {name}")
        }
        action if let Some(display) = action.strip_prefix("toggle:") => {
            format!("Toggled {display}")
        }
        _ => "Done".into(),
    }
}

fn run_action(action: &str) -> std::result::Result<(), String> {
    match action {
        "restore" => monitorctl_core::restore_previous_active_set(),
        action if let Some(name) = action.strip_prefix("profile:") => {
            monitorctl_core::apply_profile(name)
        }
        action if let Some(display) = action.strip_prefix("toggle:") => {
            monitorctl_core::toggle_display(display)
        }
        _ => Err(format!("unknown hotkey action: {action}")),
    }
}

#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn add_icon(window: HWND) -> WindowsResult<()> {
    let mut icon = icon_data(window);
    icon.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    icon.uCallbackMessage = TRAY_MESSAGE;
    icon.hIcon =
        CreateIconFromResourceEx(&TRAY_ICON[22..], true, 0x0003_0000, 32, 32, IMAGE_FLAGS(0))?;
    copy_wide(&mut icon.szTip, "Monitorctl");
    Shell_NotifyIconW(NIM_ADD, &icon).ok()
}

#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn delete_icon(window: HWND) {
    let icon = icon_data(window);
    let _ = Shell_NotifyIconW(NIM_DELETE, &icon);
}

unsafe fn show_result(result: std::result::Result<(), String>, success: &str) {
    match result {
        Ok(()) => show_osd(success, 5_000),
        Err(error) => show_osd(&error, 5_000),
    }
}

unsafe fn show_osd(message: &str, duration: u32) {
    let opacity = load_config()
        .map(|config| config.osd.opacity)
        .unwrap_or(0.85);
    let _ = monitorctl_core::osd::show(message, duration, opacity);
}

fn icon_data(window: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: window,
        uID: 1,
        ..Default::default()
    }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn copy_wide(destination: &mut [u16], value: &str) {
    for (slot, unit) in destination.iter_mut().zip(value.encode_utf16()) {
        *slot = unit;
    }
}

fn parse_hotkey(value: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    monitorctl_core::validate_hotkey(value).ok()?;
    let mut modifiers = HOT_KEY_MODIFIERS(0);
    let mut key = None;
    for part in value
        .split('+')
        .map(|part| part.trim().to_ascii_lowercase())
    {
        match part.as_str() {
            "ctrl" | "control" => modifiers |= HOT_KEY_MODIFIERS(0x0002),
            "alt" => modifiers |= HOT_KEY_MODIFIERS(0x0001),
            "shift" => modifiers |= HOT_KEY_MODIFIERS(0x0004),
            "win" | "windows" => modifiers |= HOT_KEY_MODIFIERS(0x0008),
            part if part.len() == 1 && part.as_bytes()[0].is_ascii_alphanumeric() => {
                if key
                    .replace(part.as_bytes()[0].to_ascii_uppercase() as u32)
                    .is_some()
                {
                    return None;
                }
            }
            _ => return None,
        }
    }
    key.map(|key| (modifiers, key))
}

struct MenuState {
    displays: Vec<MenuDisplay>,
    profiles: Vec<MenuProfile>,
    active_count: usize,
    restore_available: bool,
}

struct MenuDisplay {
    label: String,
    path: Option<String>,
    active: bool,
}

struct MenuProfile {
    name: String,
    available: bool,
    active: bool,
}

fn menu_state() -> std::result::Result<MenuState, String> {
    let config = load_config()?;
    let discovered = discover_displays()?;
    let active = active_paths(&discovered);
    let present = discovered
        .iter()
        .map(|display| display.device_path.as_str())
        .collect::<BTreeSet<_>>();
    let mut displays = discovered
        .iter()
        .map(|display| {
            let alias = config
                .displays
                .iter()
                .find_map(|(alias, path)| (path == &display.device_path).then_some(alias.as_str()));
            MenuDisplay {
                label: display_label(alias, &display.friendly_name),
                path: Some(display.device_path.clone()),
                active: display.active,
            }
        })
        .collect::<Vec<_>>();
    displays.extend(
        config
            .displays
            .iter()
            .filter(|(_, path)| !present.contains(path.as_str()))
            .map(|(alias, _)| MenuDisplay {
                label: format!("{alias}  Unavailable"),
                path: None,
                active: false,
            }),
    );
    let profiles = config
        .profiles
        .iter()
        .map(|(name, paths)| MenuProfile {
            name: name.clone(),
            available: !paths.is_empty()
                && paths.iter().all(|path| present.contains(path.as_str())),
            active: paths.iter().cloned().collect::<BTreeSet<_>>() == active,
        })
        .collect();
    let restore_available = config.previous_active.as_ref().is_some_and(|paths| {
        !paths.is_empty() && paths.iter().all(|path| present.contains(path.as_str()))
    });
    Ok(MenuState {
        displays,
        profiles,
        active_count: active.len(),
        restore_available,
    })
}

fn display_label(alias: Option<&str>, friendly_name: &str) -> String {
    alias
        .filter(|alias| !alias.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            (!friendly_name.is_empty())
                .then_some(friendly_name)
                .unwrap_or("(unnamed monitor)")
                .into()
        })
}

fn active_paths(displays: &[Display]) -> BTreeSet<String> {
    displays
        .iter()
        .filter(|display| display.active)
        .map(|display| display.device_path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{HOT_KEY_MODIFIERS, action_summary, display_label, parse_hotkey};

    #[test]
    fn uses_friendly_name_without_alias() {
        assert_eq!(display_label(None, "Dell 27"), "Dell 27");
        assert_eq!(display_label(Some("desk"), "Dell 27"), "desk");
    }

    #[test]
    fn parses_configured_hotkey_format() {
        assert_eq!(
            parse_hotkey("ctrl+alt+w"),
            Some((HOT_KEY_MODIFIERS(3), 'W' as u32))
        );
    }

    #[test]
    fn rejects_two_keys() {
        assert_eq!(parse_hotkey("ctrl+a+b"), None);
    }

    #[test]
    fn summarizes_hotkey_actions() {
        assert_eq!(action_summary("profile:work"), "Applied profile work");
        assert_eq!(action_summary("toggle:desk"), "Toggled desk");
    }
}
