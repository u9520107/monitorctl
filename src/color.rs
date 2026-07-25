use std::{
    fs,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};
use windows::{
    Win32::{
        Devices::Display::{
            DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
            DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
            DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
            DisplayConfigGetDeviceInfo,
        },
        Foundation::{ERROR_INSUFFICIENT_BUFFER, FreeLibrary, GetLastError, HLOCAL, LocalFree},
        System::LibraryLoader::{GetProcAddress, LoadLibraryW},
        UI::ColorSystem::{
            COLORPROFILESUBTYPE, CPST_EXTENDED_DISPLAY_COLOR_MODE,
            CPST_STANDARD_DISPLAY_COLOR_MODE, CPT_ICC, GetColorDirectoryW, InstallColorProfileW,
            WCS_PROFILE_MANAGEMENT_SCOPE, WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER,
            WCS_PROFILE_MANAGEMENT_SCOPE_SYSTEM_WIDE, WcsSetUsePerUserProfiles,
        },
    },
    core::{HRESULT, PCSTR, PCWSTR, PWSTR},
};

use crate::{
    ColorChannel, discover_displays, display_config, load_config, resolve_display, target_name,
    with_monitorctl_lock,
};

struct ColorTarget {
    adapter_id: windows::Win32::Foundation::LUID,
    source_id: u32,
    gdi_device_name: String,
    channel: ColorChannel,
}

type AddDisplayAssociation = unsafe extern "system" fn(
    windows::Win32::UI::ColorSystem::WCS_PROFILE_MANAGEMENT_SCOPE,
    PCWSTR,
    windows::Win32::Foundation::LUID,
    u32,
    i32,
    i32,
) -> HRESULT;
type GetDisplayUserScope = unsafe extern "system" fn(
    windows::Win32::Foundation::LUID,
    u32,
    *mut windows::Win32::UI::ColorSystem::WCS_PROFILE_MANAGEMENT_SCOPE,
) -> HRESULT;
type GetDisplayDefault = unsafe extern "system" fn(
    windows::Win32::UI::ColorSystem::WCS_PROFILE_MANAGEMENT_SCOPE,
    windows::Win32::Foundation::LUID,
    u32,
    windows::Win32::UI::ColorSystem::COLORPROFILETYPE,
    COLORPROFILESUBTYPE,
    *mut PWSTR,
) -> HRESULT;
type GetDisplayList = unsafe extern "system" fn(
    WCS_PROFILE_MANAGEMENT_SCOPE,
    windows::Win32::Foundation::LUID,
    u32,
    *mut *mut PWSTR,
    *mut u32,
) -> HRESULT;
type SetDisplayDefault = unsafe extern "system" fn(
    WCS_PROFILE_MANAGEMENT_SCOPE,
    PCWSTR,
    windows::Win32::UI::ColorSystem::COLORPROFILETYPE,
    COLORPROFILESUBTYPE,
    windows::Win32::Foundation::LUID,
    u32,
) -> HRESULT;

struct ColorApi {
    module: windows::Win32::Foundation::HMODULE,
    add_display_association: AddDisplayAssociation,
    get_display_user_scope: GetDisplayUserScope,
    get_display_default: GetDisplayDefault,
    get_display_list: GetDisplayList,
    set_display_default: SetDisplayDefault,
}

impl Drop for ColorApi {
    fn drop(&mut self) {
        unsafe { FreeLibrary(self.module) }.ok();
    }
}

fn color_api() -> Result<ColorApi, String> {
    let module = unsafe { LoadLibraryW(PCWSTR(wide("mscms.dll").as_ptr())) }
        .map_err(|_| "color profiles require Windows 11".to_string())?;
    unsafe fn procedure<T>(
        module: windows::Win32::Foundation::HMODULE,
        name: &'static [u8],
    ) -> Result<T, String> {
        let procedure = unsafe { GetProcAddress(module, PCSTR(name.as_ptr())) }
            .ok_or("color profiles require Windows 11")?;
        Ok(unsafe { std::mem::transmute_copy(&procedure) })
    }
    let result = unsafe {
        Ok(ColorApi {
            module,
            add_display_association: procedure(module, b"ColorProfileAddDisplayAssociation\0")?,
            get_display_user_scope: procedure(module, b"ColorProfileGetDisplayUserScope\0")?,
            get_display_default: procedure(module, b"ColorProfileGetDisplayDefault\0")?,
            get_display_list: procedure(module, b"ColorProfileGetDisplayList\0")?,
            set_display_default: procedure(module, b"ColorProfileSetDisplayDefaultAssociation\0")?,
        })
    };
    if result.is_err() {
        unsafe { FreeLibrary(module) }.ok();
    }
    result
}

pub fn list() -> Result<(), String> {
    let installed = installed_profiles()?;
    println!("Profiles:");
    if installed.is_empty() {
        println!("  (none)");
    }
    for (file, path) in installed {
        let detail = match profile_bytes(&path) {
            Ok((_, channel)) => format!("{channel:?}"),
            Err(_) => "unsupported".into(),
        };
        println!("  {file}  {detail}");
    }
    Ok(())
}

pub fn import(source: &str) -> Result<(), String> {
    with_monitorctl_lock(|| {
        let source = Path::new(source);
        let file = source
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| is_icc_filename(name))
            .ok_or("color profile import requires an .icc or .icm filename")?;
        let (sha256, _) = profile_bytes(source)?;
        let installed = installed_profiles()?;
        let known = installed
            .iter()
            .map(|(file, path)| (file.clone(), profile_bytes(path).ok().map(|(hash, _)| hash)))
            .collect::<Vec<_>>();
        if matches!(
            import_decision(file, &sha256, &known)?,
            ImportDecision::Install
        ) {
            let source = source
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            unsafe { InstallColorProfileW(None, PCWSTR(source.as_ptr())) }
                .ok()
                .map_err(|error| format!("cannot install color profile: {error}"))?;
        }
        Ok(())
    })
}

enum ImportDecision {
    Reuse,
    Install,
}

fn import_decision(
    source_file: &str,
    source_hash: &str,
    installed: &[(String, Option<String>)],
) -> Result<ImportDecision, String> {
    if let Some((_, hash)) = installed.iter().find(|(file, _)| file == source_file) {
        return (hash.as_deref() == Some(source_hash))
            .then_some(ImportDecision::Reuse)
            .ok_or_else(|| {
                format!(
                    "installed color profile {source_file:?} has different contents; rename source file"
                )
            });
    }
    if installed
        .iter()
        .any(|(_, hash)| hash.as_deref() == Some(source_hash))
    {
        Ok(ImportDecision::Reuse)
    } else {
        Ok(ImportDecision::Install)
    }
}

pub fn current(selector: &str) -> Result<(), String> {
    let config = load_config()?;
    let displays = discover_displays()?;
    let display = resolve_display(&displays, &config.displays, selector)?;
    let target = color_target(&display.device_path)?;
    let file = current_for_path(&display.device_path)?;
    println!(
        "{}\n  channel: {:?}\n  default: {}",
        display.friendly_name,
        target.channel,
        file.unwrap_or_else(|| "Windows default".into())
    );
    Ok(())
}

pub fn current_for_path(device_path: &str) -> Result<Option<String>, String> {
    let target = color_target(device_path)?;
    default_profile(&target, target.channel, display_scope(&target)?)
}

pub fn uses_current_user_settings_for_path(device_path: &str) -> Result<bool, String> {
    Ok(display_scope(&color_target(device_path)?)? == WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER)
}

pub fn use_system_settings_for_path(device_path: &str) -> Result<(), String> {
    with_monitorctl_lock(|| set_profile_scope(&color_target(device_path)?, false))
}

pub fn set(selector: &str, file: &str) -> Result<(), String> {
    with_monitorctl_lock(|| {
        let config = load_config()?;
        let displays = discover_displays()?;
        let display = resolve_display(&displays, &config.displays, selector)?;
        set_for_path(&display.device_path, file)
    })
}

pub fn set_path(device_path: &str, file: &str) -> Result<(), String> {
    with_monitorctl_lock(|| set_for_path(device_path, file))
}

pub fn safe_profiles_for_path(device_path: &str) -> Result<Vec<String>, String> {
    let channel = color_target(device_path)?.channel;
    Ok(installed_profiles()?
        .into_iter()
        .filter(|(_, path)| profile_bytes(path).is_ok_and(|(_, detected)| detected == channel))
        .map(|(file, _)| file)
        .collect())
}

fn set_for_path(device_path: &str, selector: &str) -> Result<(), String> {
    let (file, installed) = resolve_installed_file(selector)?;
    let (_, channel) = profile_bytes(&installed)?;
    let target = color_target(device_path)?;
    if channel != target.channel {
        return Err(format!(
            "color profile {file:?} has {:?} channel but display requires {:?}",
            channel, target.channel
        ));
    }
    let wide = wide(&file);
    let api = color_api()?;
    ensure_current_user_scope(&target)?;
    let associated = display_profiles(&api, &target, WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER)?;
    if !associated
        .iter()
        .any(|profile| profile.eq_ignore_ascii_case(&file))
    {
        unsafe {
            (api.add_display_association)(
                WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER,
                PCWSTR(wide.as_ptr()),
                target.adapter_id,
                target.source_id,
                0,
                (channel == ColorChannel::Advanced) as i32,
            )
            .ok()
            .map_err(|error| format!("cannot associate Windows color profile: {error}"))?;
        }
    }
    unsafe {
        (api.set_display_default)(
            WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER,
            PCWSTR(wide.as_ptr()),
            CPT_ICC,
            color_subtype(channel),
            target.adapter_id,
            target.source_id,
        )
        .ok()
        .map_err(|error| format!("cannot set Windows default color profile: {error}"))?;
    }
    let actual = default_profile(&target, channel, WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER)?;
    (actual.as_deref() == Some(file.as_str()))
        .then_some(())
        .ok_or_else(|| "Windows did not report requested color profile as default".into())
}

fn color_target(device_path: &str) -> Result<ColorTarget, String> {
    let topology = display_config(crate::QDC_ALL_PATHS)?;
    let matches = topology
        .paths
        .iter()
        .filter(|path| {
            path.flags & 1 != 0
                && target_name(path)
                    .map(|target| crate::utf16_string(&target.monitorDevicePath) == device_path)
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let [path] = matches.as_slice() else {
        return Err(
            "display topology changed or monitor is not active; no color change made".into(),
        );
    };
    let mut info = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO::default();
    info.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
        size: std::mem::size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32,
        adapterId: path.targetInfo.adapterId,
        id: path.targetInfo.id,
    };
    let result = unsafe { DisplayConfigGetDeviceInfo(&mut info.header) };
    if result != 0 {
        return Err(format!("cannot query display color state: {result}"));
    }
    Ok(ColorTarget {
        adapter_id: path.targetInfo.adapterId,
        source_id: path.sourceInfo.id,
        gdi_device_name: source_device_name(path)?,
        channel: if unsafe { info.Anonymous.value & 0x2 != 0 } {
            ColorChannel::Advanced
        } else {
            ColorChannel::Normal
        },
    })
}

fn source_device_name(
    path: &windows::Win32::Devices::Display::DISPLAYCONFIG_PATH_INFO,
) -> Result<String, String> {
    let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
    source.header = DISPLAYCONFIG_DEVICE_INFO_HEADER {
        r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
        size: std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
        adapterId: path.sourceInfo.adapterId,
        id: path.sourceInfo.id,
    };
    let result = unsafe { DisplayConfigGetDeviceInfo(&mut source.header) };
    if result != 0 {
        return Err(format!("cannot resolve display device name: {result}"));
    }
    Ok(crate::utf16_string(&source.viewGdiDeviceName))
}

fn display_scope(target: &ColorTarget) -> Result<WCS_PROFILE_MANAGEMENT_SCOPE, String> {
    let api = color_api()?;
    let mut scope = Default::default();
    unsafe { (api.get_display_user_scope)(target.adapter_id, target.source_id, &mut scope) }
        .ok()
        .map_err(|error| format!("cannot query color profile scope: {error}"))?;
    Ok(scope)
}

fn ensure_current_user_scope(target: &ColorTarget) -> Result<(), String> {
    set_profile_scope(target, true)
}

fn set_profile_scope(target: &ColorTarget, current_user: bool) -> Result<(), String> {
    let wanted = if current_user {
        WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER
    } else {
        WCS_PROFILE_MANAGEMENT_SCOPE_SYSTEM_WIDE
    };
    if display_scope(target)? == wanted {
        return Ok(());
    }
    let device_name = wide(&target.gdi_device_name);
    let class_monitor = u32::from_be_bytes(*b"mntr");
    if !unsafe {
        WcsSetUsePerUserProfiles(PCWSTR(device_name.as_ptr()), class_monitor, current_user)
    }
    .as_bool()
    {
        return Err(format!(
            "cannot change color settings scope: {:?}",
            unsafe { GetLastError() }
        ));
    }
    (display_scope(target)? == wanted)
        .then_some(())
        .ok_or("Windows did not change color settings scope".into())
}

fn color_subtype(channel: ColorChannel) -> COLORPROFILESUBTYPE {
    if channel == ColorChannel::Advanced {
        CPST_EXTENDED_DISPLAY_COLOR_MODE
    } else {
        CPST_STANDARD_DISPLAY_COLOR_MODE
    }
}

fn display_profiles(
    api: &ColorApi,
    target: &ColorTarget,
    scope: WCS_PROFILE_MANAGEMENT_SCOPE,
) -> Result<Vec<String>, String> {
    let mut profiles = std::ptr::null_mut();
    let mut count = 0;
    unsafe {
        (api.get_display_list)(
            scope,
            target.adapter_id,
            target.source_id,
            &mut profiles,
            &mut count,
        )
    }
    .ok()
    .map_err(|error| format!("cannot query display color profiles: {error}"))?;
    if count == 0 {
        return Ok(Vec::new());
    }
    if profiles.is_null() {
        return Err("Windows returned a null display color profile list".into());
    }
    let result: Result<Vec<_>, String> = unsafe {
        std::slice::from_raw_parts(profiles, count as usize)
            .iter()
            .map(|profile| {
                profile
                    .to_string()
                    .map_err(|error| format!("cannot read display color profiles: {error}"))
            })
            .collect()
    };
    if !profiles.is_null() {
        unsafe { LocalFree(Some(HLOCAL(profiles.cast()))) };
    }
    result
}

fn default_profile(
    target: &ColorTarget,
    channel: ColorChannel,
    scope: WCS_PROFILE_MANAGEMENT_SCOPE,
) -> Result<Option<String>, String> {
    let api = color_api()?;
    let subtype = color_subtype(channel);
    let mut profile = PWSTR::null();
    let result = unsafe {
        (api.get_display_default)(
            scope,
            target.adapter_id,
            target.source_id,
            CPT_ICC,
            subtype,
            &mut profile,
        )
    };
    match result.ok() {
        Ok(()) => {
            let value = unsafe { profile.to_string() }
                .map_err(|error| format!("cannot read default color profile: {error}"));
            unsafe { LocalFree(Some(HLOCAL(profile.0.cast()))) };
            value.map(Some)
        }
        Err(error) if matches!(error.code().0 as u32, 0x8007_0490 | 0x8007_0002) => Ok(None),
        Err(error) => Err(format!("cannot query default color profile: {error}")),
    }
}

fn resolve_installed_file(selector: &str) -> Result<(String, PathBuf), String> {
    if Path::new(selector)
        .file_name()
        .and_then(|name| name.to_str())
        != Some(selector)
    {
        return Err("installed color profile must be a filename, not a path".into());
    }
    let profiles = installed_profiles()?;
    let file = resolve_filename(
        &profiles
            .iter()
            .map(|(file, _)| file.clone())
            .collect::<Vec<_>>(),
        selector,
    )?;
    let path = profiles
        .iter()
        .find_map(|(candidate, path)| (candidate == &file).then_some(path.clone()))
        .expect("resolved color profile came from installed profiles");
    Ok((file, path))
}

fn installed_profiles() -> Result<Vec<(String, PathBuf)>, String> {
    let mut profiles = fs::read_dir(color_directory()?)
        .map_err(|error| format!("cannot read Windows color profile store: {error}"))?
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            is_icc_filename(&file).then_some((file, entry.path()))
        })
        .collect::<Vec<_>>();
    profiles.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(profiles)
}

fn resolve_filename(candidates: &[String], selector: &str) -> Result<String, String> {
    if let Some(found) = candidates.iter().find(|file| *file == selector) {
        return Ok(found.clone());
    }
    let selector = selector.to_lowercase();
    let matches = candidates
        .iter()
        .filter(|file| file.to_lowercase().contains(&selector))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [found] => Ok((*found).clone()),
        [] => Err(format!("no installed color profile matches {selector:?}")),
        matches => Err(format!(
            "color profile selector {selector:?} is ambiguous: {}",
            matches
                .iter()
                .map(|file| file.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn color_directory() -> Result<PathBuf, String> {
    let mut size = 0;
    let first = unsafe { GetColorDirectoryW(None, None, &mut size) };
    if !first.as_bool() && unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
        return Err("cannot locate Windows color profile store".into());
    }
    let mut directory = vec![0; size as usize];
    unsafe { GetColorDirectoryW(None, Some(PWSTR(directory.as_mut_ptr())), &mut size) }
        .ok()
        .map_err(|error| format!("cannot locate Windows color profile store: {error}"))?;
    Ok(PathBuf::from(crate::utf16_string(&directory)))
}

fn is_icc_filename(file: &str) -> bool {
    Path::new(file)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("icc") || extension.eq_ignore_ascii_case("icm")
        })
}

fn profile_bytes(path: &Path) -> Result<(String, ColorChannel), String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if bytes.len() < 132
        || &bytes[12..16] != b"mntr"
        || &bytes[16..20] != b"RGB "
        || &bytes[36..40] != b"acsp"
    {
        return Err(format!(
            "{} is not a supported RGB display ICC profile",
            path.display()
        ));
    }
    let profile_size = u32::from_be_bytes(bytes[0..4].try_into().expect("four bytes")) as usize;
    let tag_count = u32::from_be_bytes(bytes[128..132].try_into().expect("four bytes")) as usize;
    let tag_table_end = 132usize
        .checked_add(
            tag_count
                .checked_mul(12)
                .ok_or_else(|| format!("{} has an invalid ICC tag table", path.display()))?,
        )
        .ok_or_else(|| format!("{} has an invalid ICC tag table", path.display()))?;
    if profile_size < tag_table_end || profile_size > bytes.len() || tag_table_end > bytes.len() {
        return Err(format!("{} has an invalid ICC tag table", path.display()));
    }
    let channel = if bytes[132..tag_table_end]
        .chunks_exact(12)
        .any(|tag| &tag[..4] == b"MHC2")
    {
        ColorChannel::Advanced
    } else {
        ColorChannel::Normal
    };
    Ok((format!("{:x}", Sha256::digest(bytes)), channel))
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        ImportDecision, import_decision, is_icc_filename, profile_bytes, resolve_filename,
    };
    use crate::ColorChannel;

    fn icc(tags: &[&[u8; 4]]) -> Vec<u8> {
        let mut bytes = vec![0; 132 + tags.len() * 12];
        let size = bytes.len() as u32;
        bytes[0..4].copy_from_slice(&size.to_be_bytes());
        bytes[12..16].copy_from_slice(b"mntr");
        bytes[16..20].copy_from_slice(b"RGB ");
        bytes[36..40].copy_from_slice(b"acsp");
        bytes[128..132].copy_from_slice(&(tags.len() as u32).to_be_bytes());
        for (index, tag) in tags.iter().enumerate() {
            let offset = 132 + index * 12;
            bytes[offset..offset + 4].copy_from_slice(*tag);
        }
        bytes
    }

    #[test]
    fn recognizes_rgb_display_icc_and_mhc_channel() {
        let path =
            std::env::temp_dir().join(format!("monitorctl-color-{}.icc", std::process::id()));
        let mut bytes = icc(&[]);
        bytes[64..68].copy_from_slice(b"MHC2");
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(profile_bytes(&path).unwrap().1, ColorChannel::Normal);
        bytes = icc(&[b"MHC2"]);
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(profile_bytes(&path).unwrap().1, ColorChannel::Advanced);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn recognizes_icc_filenames_case_insensitively() {
        assert!(is_icc_filename("desk.ICC"));
        assert!(is_icc_filename("desk.icm"));
        assert!(!is_icc_filename("desk.icc.bak"));
    }

    #[test]
    fn resolves_unique_profile_filename_substrings() {
        let files = vec![
            "HDR Calibrated.icc".into(),
            "sRGB Color Space Profile.icm".into(),
            "Default Profile.icm".into(),
        ];
        assert_eq!(
            resolve_filename(&files, "hdr cali").unwrap(),
            "HDR Calibrated.icc"
        );
        assert!(resolve_filename(&files, "profile").is_err());
    }

    #[test]
    fn rejects_same_name_with_different_profile_bytes() {
        let installed = vec![
            ("Calibrated.icc".into(), Some("old".into())),
            ("Copy.icc".into(), Some("new".into())),
        ];
        assert!(import_decision("Calibrated.icc", "new", &installed).is_err());
        assert!(matches!(
            import_decision("New.icc", "new", &installed).unwrap(),
            ImportDecision::Reuse
        ));
    }
}
