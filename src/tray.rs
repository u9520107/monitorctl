#![allow(unsafe_op_in_unsafe_fn)]
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::{
    collections::{BTreeMap, BTreeSet},
    mem::size_of,
    sync::{Mutex, OnceLock},
};

use monitorctl_core::{
    Display, HotkeyAction, discover_displays, load_config, resolve_identities, resolve_identity,
};
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
                MF_DISABLED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG, PostQuitMessage,
                RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, TPM_RIGHTBUTTON,
                TrackPopupMenu, TranslateMessage, WM_APP, WM_COMMAND, WM_CONTEXTMENU, WM_DESTROY,
                WM_HOTKEY, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
            },
        },
    },
    core::{PCWSTR, Result as WindowsResult},
};

const CLASS_NAME: &str = "monitorctl-tray";
const TRAY_MESSAGE: u32 = WM_APP + 1;
const MENU_MONITOR_BASE: u32 = 1_000;
const MENU_PROFILE_BASE: u32 = 2_000;
const MENU_COLOR_BASE: u32 = 2_500;
const MENU_RESTORE: u32 = 3_000;
const MENU_QUIT: u32 = 3_001;
const HOTKEY_BASE: i32 = 4_000;
const MOD_NOREPEAT: HOT_KEY_MODIFIERS = HOT_KEY_MODIFIERS(0x4000);
const TRAY_ICON: &[u8] = include_bytes!("../assets/monitorctl.ico");

static TRAY_STATE: OnceLock<Mutex<TrayState>> = OnceLock::new();
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();

#[derive(Default)]
struct TrayState {
    hotkeys: BTreeMap<i32, HotkeyAction>,
    menu_actions: BTreeMap<u32, MenuAction>,
}

#[derive(Clone)]
enum MenuAction {
    Toggle {
        path: Option<String>,
        label: String,
        active: bool,
    },
    Profile(String),
    Color {
        path: String,
        file: String,
    },
    SystemColor {
        path: String,
        label: String,
    },
    Restore,
}

fn tray_state() -> &'static Mutex<TrayState> {
    TRAY_STATE.get_or_init(|| Mutex::new(TrayState::default()))
}

fn main() -> WindowsResult<()> {
    unsafe {
        let taskbar_created = wide("TaskbarCreated");
        let _ = TASKBAR_CREATED.set(RegisterWindowMessageW(PCWSTR(taskbar_created.as_ptr())));
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
        message
            if TASKBAR_CREATED
                .get()
                .is_some_and(|taskbar| *taskbar == message) =>
        {
            let _ = add_icon(window);
            LRESULT(0)
        }
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
    let mut menu_actions = BTreeMap::new();
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
        menu_actions.insert(
            MENU_MONITOR_BASE + index as u32,
            MenuAction::Toggle {
                path: display.path.clone(),
                label: display.label.clone(),
                active: display.active,
            },
        );
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
        for display in &profile.displays {
            append(
                menu,
                MF_STRING | MF_GRAYED,
                0,
                &format!("    - {}", display.label),
            );
        }
        menu_actions.insert(
            MENU_PROFILE_BASE + index as u32,
            MenuAction::Profile(profile.name.clone()),
        );
    }
    if !state.colors.is_empty() {
        append(menu, MF_SEPARATOR, 0, "");
        append(menu, MF_STRING | MF_DISABLED, 0, "Color profiles");
        let mut color_id = MENU_COLOR_BASE;
        for color in &state.colors {
            let Ok(submenu) = CreatePopupMenu() else {
                continue;
            };
            append_submenu(menu, submenu, &color.label);
            append(
                submenu,
                MF_STRING
                    | if color.uses_current_user_settings {
                        MENU_ITEM_FLAGS(0)
                    } else {
                        MF_CHECKED
                    },
                color_id,
                "Use system color settings",
            );
            menu_actions.insert(
                color_id,
                MenuAction::SystemColor {
                    path: color.path.clone(),
                    label: color.label.clone(),
                },
            );
            color_id += 1;
            append(submenu, MF_SEPARATOR, 0, "");
            for file in &color.profiles {
                let active = color.uses_current_user_settings
                    && color
                        .current
                        .as_ref()
                        .is_some_and(|current| file.eq_ignore_ascii_case(current));
                append(
                    submenu,
                    MF_STRING
                        | if active {
                            MF_CHECKED
                        } else {
                            MENU_ITEM_FLAGS(0)
                        },
                    color_id,
                    file,
                );
                menu_actions.insert(
                    color_id,
                    MenuAction::Color {
                        path: color.path.clone(),
                        file: file.clone(),
                    },
                );
                color_id += 1;
            }
        }
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
        "Restore displays",
    );
    menu_actions.insert(MENU_RESTORE, MenuAction::Restore);
    append(menu, MF_STRING, MENU_QUIT, "Quit");
    tray_state().lock().unwrap().menu_actions = menu_actions;
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

unsafe fn append_submenu(menu: HMENU, submenu: HMENU, label: &str) {
    let label = wide(label);
    let _ = AppendMenuW(
        menu,
        MF_POPUP | MF_STRING,
        submenu.0 as usize,
        PCWSTR(label.as_ptr()),
    );
}

unsafe fn run_menu_action(window: HWND, command: u32) {
    if command == MENU_QUIT {
        let _ = DestroyWindow(window);
        return;
    }
    let action = tray_state()
        .lock()
        .unwrap()
        .menu_actions
        .get(&command)
        .cloned();
    let (result, success) = match action {
        Some(MenuAction::Restore) => (
            monitorctl_core::restore_previous_active_set(),
            "Restored previous display set".into(),
        ),
        Some(MenuAction::Toggle {
            path,
            label,
            active,
        }) => match path {
            Some(path) => (
                monitorctl_core::toggle_display_path(&path),
                format!("{} {label}", if active { "Disabled" } else { "Enabled" }),
            ),
            None => (Err("display is unavailable".into()), String::new()),
        },
        Some(MenuAction::Profile(name)) => (
            monitorctl_core::apply_profile(&name),
            format!("Applied profile {name}"),
        ),
        Some(MenuAction::Color { path, file }) => (
            monitorctl_core::color::set_path(&path, &file),
            format!("Applied color profile {file}"),
        ),
        Some(MenuAction::SystemColor { path, label }) => (
            monitorctl_core::color::use_system_settings_for_path(&path),
            format!("Using system color settings for {label}"),
        ),
        None => return,
    };
    show_result(result, &success);
}

unsafe fn register_hotkeys(window: HWND) {
    let Ok(config) = load_config() else {
        return;
    };
    let mut hotkeys = tray_state().lock().unwrap();
    for (index, (key, action)) in config.hotkeys.iter().enumerate() {
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
            } else {
                hotkeys
                    .hotkeys
                    .insert(HOTKEY_BASE + index as i32, action.clone());
            }
        } else {
            show_osd(&format!("Invalid hotkey: {key}"), 5_000);
        }
    }
}

unsafe fn unregister_hotkeys(window: HWND) {
    let ids = {
        let mut state = tray_state().lock().unwrap();
        std::mem::take(&mut state.hotkeys)
            .into_keys()
            .collect::<Vec<_>>()
    };
    for id in ids {
        let _ = UnregisterHotKey(Some(window), id);
    }
}

unsafe fn run_hotkey(_window: HWND, id: i32) {
    let action = tray_state().lock().unwrap().hotkeys.get(&id).cloned();
    let (result, success) = match action {
        Some(action) => (run_action(&action), action_summary(&action)),
        None => (Err("unknown hotkey".into()), String::new()),
    };
    show_result(result, &success);
}

fn action_summary(action: &HotkeyAction) -> String {
    match action {
        HotkeyAction::Restore => "Restored previous display set".into(),
        HotkeyAction::Profile { name } => {
            format!("Applied profile {name}")
        }
        HotkeyAction::Toggle { display } => format!(
            "Toggled {}",
            display
                .friendly_name
                .as_deref()
                .unwrap_or(&display.device_path)
        ),
        HotkeyAction::Color { file, .. } => format!("Applied color profile {file}"),
        HotkeyAction::Legacy { action } => format!("Legacy hotkey: {action}"),
    }
}

fn run_action(action: &HotkeyAction) -> std::result::Result<(), String> {
    match action {
        HotkeyAction::Restore => monitorctl_core::restore_previous_active_set(),
        HotkeyAction::Profile { name } => monitorctl_core::apply_profile(name),
        HotkeyAction::Toggle { display } => monitorctl_core::toggle_display_identity(display),
        HotkeyAction::Color { display, file } => {
            monitorctl_core::color::set_identity(display, file)
        }
        HotkeyAction::Legacy { action } if action == "restore" => {
            monitorctl_core::restore_previous_active_set()
        }
        HotkeyAction::Legacy { action } if let Some(name) = action.strip_prefix("profile:") => {
            monitorctl_core::apply_profile(name)
        }
        HotkeyAction::Legacy { action } if let Some(display) = action.strip_prefix("toggle:") => {
            monitorctl_core::toggle_display(display)
        }
        HotkeyAction::Legacy { action } => Err(format!("unknown hotkey action: {action}")),
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
    key.map(|key| (modifiers | MOD_NOREPEAT, key))
}

struct MenuState {
    displays: Vec<MenuDisplay>,
    profiles: Vec<MenuProfile>,
    colors: Vec<MenuColor>,
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
    displays: Vec<MenuProfileDisplay>,
    available: bool,
    active: bool,
}

struct MenuProfileDisplay {
    label: String,
}

struct MenuColor {
    label: String,
    path: String,
    uses_current_user_settings: bool,
    current: Option<String>,
    profiles: Vec<String>,
}

fn menu_state() -> std::result::Result<MenuState, String> {
    let config = load_config()?;
    let discovered = discover_displays()?;
    let active = active_paths(&discovered);
    let mut displays = discovered
        .iter()
        .map(|display| {
            let alias = alias_for_display(&config, &discovered, display);
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
            .filter(|(_, identity)| resolve_identity(&discovered, identity).is_err())
            .map(|(alias, _)| MenuDisplay {
                label: format!("{alias}  Unavailable"),
                path: None,
                active: false,
            }),
    );
    let profiles = config
        .profiles
        .iter()
        .map(|(name, identities)| {
            let displays = identities
                .iter()
                .map(|identity| {
                    let label = resolve_identity(&discovered, identity)
                        .map(|display| {
                            display_label(
                                alias_for_display(&config, &discovered, display),
                                &display.friendly_name,
                            )
                        })
                        .unwrap_or_else(|_| {
                            identity
                                .friendly_name
                                .clone()
                                .unwrap_or_else(|| "Unavailable display".into())
                        });
                    MenuProfileDisplay { label }
                })
                .collect();
            let resolved = resolve_identities(&discovered, identities);
            MenuProfile {
                name: name.clone(),
                displays,
                available: !identities.is_empty() && resolved.is_ok(),
                active: resolved.is_ok_and(|paths| paths == active),
            }
        })
        .collect();
    let colors = discovered
        .iter()
        .filter(|display| display.active)
        .filter_map(|display| {
            let alias = alias_for_display(&config, &discovered, display);
            let label = display_label(alias, &display.friendly_name);
            let profiles = monitorctl_core::color::safe_profiles_for_path(&display.device_path)
                .unwrap_or_default();
            (!profiles.is_empty()).then(|| MenuColor {
                label,
                path: display.device_path.clone(),
                uses_current_user_settings:
                    monitorctl_core::color::uses_current_user_settings_for_path(
                        &display.device_path,
                    )
                    .unwrap_or(false),
                current: monitorctl_core::color::current_for_path(&display.device_path)
                    .ok()
                    .flatten(),
                profiles,
            })
        })
        .collect();
    let restore_available = config.previous_active.as_ref().is_some_and(|identities| {
        !identities.is_empty() && resolve_identities(&discovered, identities).is_ok()
    });
    Ok(MenuState {
        displays,
        profiles,
        colors,
        active_count: active.len(),
        restore_available,
    })
}

fn alias_for_display<'a>(
    config: &'a monitorctl_core::Config,
    displays: &[Display],
    display: &Display,
) -> Option<&'a str> {
    config.displays.iter().find_map(|(alias, identity)| {
        resolve_identity(displays, identity)
            .ok()
            .is_some_and(|candidate| candidate.device_path == display.device_path)
            .then_some(alias.as_str())
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
            Some((HOT_KEY_MODIFIERS(0x4003), 'W' as u32))
        );
    }

    #[test]
    fn rejects_two_keys() {
        assert_eq!(parse_hotkey("ctrl+a+b"), None);
    }

    #[test]
    fn summarizes_hotkey_actions() {
        assert_eq!(
            action_summary(&monitorctl_core::HotkeyAction::Profile {
                name: "work".into()
            }),
            "Applied profile work"
        );
        assert_eq!(
            action_summary(&monitorctl_core::HotkeyAction::Toggle {
                display: monitorctl_core::DisplayIdentity {
                    device_path: "desk".into(),
                    friendly_name: Some("desk".into()),
                    serial: None,
                },
            }),
            "Toggled desk"
        );
    }
}
