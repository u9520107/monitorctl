use std::fmt;

use serde::{Deserialize, Serialize};

use windows::{
    Win32::{
        Devices::FunctionDiscovery::{
            PKEY_Device_DeviceDesc, PKEY_Device_FriendlyName, PKEY_Device_Manufacturer,
            PKEY_DeviceInterface_FriendlyName,
        },
        Foundation::{ERROR_NOT_FOUND, PROPERTYKEY},
        Media::Audio::{
            DEVICE_STATE, DEVICE_STATE_ACTIVE, DEVICE_STATE_DISABLED, DEVICE_STATE_NOTPRESENT,
            DEVICE_STATE_UNPLUGGED, DEVICE_STATEMASK_ALL, IMMDevice, IMMDeviceEnumerator,
            MMDeviceEnumerator, eCommunications, eConsole, eMultimedia, eRender,
        },
        System::Com::{
            CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
            CoUninitialize, STGM_READ,
            StructuredStorage::{PROPVARIANT, PropVariantToString},
        },
        UI::Shell::PropertiesSystem::IPropertyStore,
    },
    core::{GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface, PCWSTR, PWSTR},
};

const CLSID_POLICY_CONFIG_CLIENT: GUID = GUID::from_u128(0x870af99c171d4f9eaf0de63df40c2bc9);
const IID_POLICY_CONFIG: GUID = GUID::from_u128(0xf8679f50850a41cf9c72430f290290c8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvidiaClassification {
    Nvidia,
    NonNvidia,
    Unknown,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct AudioConfig {
    #[serde(default)]
    pub suppress_nvidia: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
}

impl AudioConfig {
    pub(crate) fn is_default(&self) -> bool {
        !self.suppress_nvidia && self.order.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioDecision {
    Noop,
    Promote(String),
    Restore(String),
    NoEligibleFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioReconciliation {
    pub order: Vec<String>,
    pub decision: AudioDecision,
    pub enumeration_failed: bool,
}

impl fmt::Display for NvidiaClassification {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Nvidia => "NVIDIA",
            Self::NonNvidia => "non-NVIDIA",
            Self::Unknown => "unknown",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub id: String,
    pub name: String,
    pub state: EndpointState,
    pub active: bool,
    pub adapter_name: Option<String>,
    pub manufacturer: Option<String>,
    pub classification: NvidiaClassification,
    pub classification_evidence: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointState {
    Active,
    Disabled,
    NotPresent,
    Unplugged,
    Unknown(u32),
}

impl EndpointState {
    fn from_win32(state: DEVICE_STATE) -> Self {
        match state {
            DEVICE_STATE_ACTIVE => Self::Active,
            DEVICE_STATE_DISABLED => Self::Disabled,
            DEVICE_STATE_NOTPRESENT => Self::NotPresent,
            DEVICE_STATE_UNPLUGGED => Self::Unplugged,
            value => Self::Unknown(value.0),
        }
    }
}

impl fmt::Display for EndpointState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Active => formatter.write_str("active"),
            Self::Disabled => formatter.write_str("disabled"),
            Self::NotPresent => formatter.write_str("not-present"),
            Self::Unplugged => formatter.write_str("unplugged"),
            Self::Unknown(value) => write!(formatter, "unknown({value})"),
        }
    }
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, String> {
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result.0 < 0 {
            return Err(format!("cannot initialize Core Audio COM: {result:?}"));
        }
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

pub fn list(include_all: bool) -> Result<(), String> {
    let snapshot = snapshot(include_all)?;
    let defaults = defaults()?;
    for endpoint in &snapshot {
        let roles = defaults
            .iter()
            .filter_map(|(role, id)| (id.as_deref() == Some(endpoint.id.as_str())).then_some(*role))
            .collect::<Vec<_>>();
        println!(
            "{}\n  id: {}\n  state: {}\n  active: {}\n  adapter: {}\n  manufacturer: {}\n  NVIDIA classification: {}\n  classification evidence: {}\n  default roles: {}",
            endpoint.name,
            endpoint.id,
            endpoint.state,
            endpoint.active,
            endpoint.adapter_name.as_deref().unwrap_or("-"),
            endpoint.manufacturer.as_deref().unwrap_or("-"),
            endpoint.classification,
            if endpoint.classification_evidence.is_empty() {
                "none"
            } else {
                &endpoint.classification_evidence
            },
            if roles.is_empty() {
                "-".into()
            } else {
                roles.join(", ")
            },
        );
    }
    Ok(())
}

pub fn default() -> Result<(), String> {
    for (role, id) in defaults()? {
        println!("{role}: {}", id.as_deref().unwrap_or("unavailable"));
    }
    Ok(())
}

pub fn set_default(selector: &str) -> Result<(), String> {
    let endpoints = snapshot(false)?;
    let endpoint = resolve_endpoint(&endpoints, selector)?;
    let endpoint_name = endpoint.name.clone();
    let endpoint_id = endpoint.id.clone();

    let setter_error = set_default_roles(&endpoint_id).err();
    let defaults = match defaults() {
        Ok(defaults) => defaults,
        Err(error) => {
            return Err(match setter_error {
                Some(setter_error) => format!(
                    "selected {endpoint_name:?} ({endpoint_id}), but {setter_error}; cannot verify defaults: {error}"
                ),
                None => format!(
                    "selected {endpoint_name:?} ({endpoint_id}), but cannot verify defaults: {error}"
                ),
            });
        }
    };
    let mut failures = Vec::new();
    if let Some(error) = setter_error {
        failures.push(error);
    }
    for role in ["Console", "Multimedia"] {
        let selected = defaults
            .iter()
            .find(|(current_role, _)| *current_role == role)
            .and_then(|(_, id)| id.as_deref());
        if selected != Some(endpoint_id.as_str()) {
            failures.push(format!(
                "{role} verification failed: expected {endpoint_id}, got {}",
                selected.unwrap_or("unavailable")
            ));
        }
    }
    if !failures.is_empty() {
        return Err(format!(
            "selected {endpoint_name:?} ({endpoint_id}), but {}",
            failures.join("; ")
        ));
    }

    println!("selected {endpoint_name:?} ({endpoint_id})");
    println!("Console: verified");
    println!("Multimedia: verified");
    Ok(())
}

pub fn tray_snapshot() -> Result<(Vec<Endpoint>, Option<String>), String> {
    let endpoints = snapshot(false)?;
    let multimedia = defaults()?
        .into_iter()
        .find(|(role, _)| *role == "Multimedia")
        .and_then(|(_, id)| id);
    Ok((endpoints, multimedia))
}

pub fn reconcile(
    saved_order: &[String],
    endpoints: Result<&[Endpoint], ()>,
    current_default: Option<&str>,
    suppress_nvidia: bool,
) -> AudioReconciliation {
    let Ok(endpoints) = endpoints else {
        return AudioReconciliation {
            order: saved_order.to_vec(),
            decision: AudioDecision::Noop,
            enumeration_failed: true,
        };
    };

    let active = endpoints
        .iter()
        .filter(|endpoint| endpoint.active && endpoint.state == EndpointState::Active)
        .collect::<Vec<_>>();
    let active_ids = active
        .iter()
        .map(|endpoint| endpoint.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut order = Vec::with_capacity(active.len());
    for id in saved_order {
        if active_ids.contains(id.as_str()) && !order.iter().any(|existing| existing == id) {
            order.push(id.clone());
        }
    }
    let mut new_endpoints = active
        .iter()
        .filter(|endpoint| !order.iter().any(|id| id == &endpoint.id))
        .collect::<Vec<_>>();
    new_endpoints.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
            .then(left.id.cmp(&right.id))
    });
    order.extend(
        new_endpoints
            .into_iter()
            .map(|endpoint| endpoint.id.clone()),
    );

    let decision = current_default
        .and_then(|id| active.iter().find(|endpoint| endpoint.id == id))
        .map_or(AudioDecision::Noop, |current| {
            if !suppress_nvidia {
                return AudioDecision::Promote(current.id.clone());
            }
            match current.classification {
                NvidiaClassification::NonNvidia => AudioDecision::Promote(current.id.clone()),
                NvidiaClassification::Nvidia => order
                    .iter()
                    .filter_map(|id| active.iter().find(|endpoint| endpoint.id == *id))
                    .find(|endpoint| endpoint.classification == NvidiaClassification::NonNvidia)
                    .map(|endpoint| AudioDecision::Restore(endpoint.id.clone()))
                    .unwrap_or(AudioDecision::NoEligibleFallback),
                NvidiaClassification::Unknown => AudioDecision::Noop,
            }
        });

    if let AudioDecision::Promote(id) = &decision {
        if let Some(position) = order.iter().position(|candidate| candidate == id) {
            let id = order.remove(position);
            order.insert(0, id);
        }
    }

    AudioReconciliation {
        order,
        decision,
        enumeration_failed: false,
    }
}

fn resolve_endpoint<'a>(endpoints: &'a [Endpoint], selector: &str) -> Result<&'a Endpoint, String> {
    if let Some(endpoint) = endpoints.iter().find(|endpoint| endpoint.id == selector) {
        return Ok(endpoint);
    }

    let exact = endpoints
        .iter()
        .filter(|endpoint| endpoint.name == selector)
        .collect::<Vec<_>>();
    match exact.as_slice() {
        [endpoint] => return Ok(endpoint),
        [] => {}
        _ => return Err(format!("audio selector {selector:?} is ambiguous")),
    }

    let selector = selector.to_lowercase();
    let matches = endpoints
        .iter()
        .filter(|endpoint| endpoint.name.to_lowercase().contains(&selector))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [endpoint] => Ok(endpoint),
        [] => Err(format!("no audio endpoint matches {selector:?}")),
        _ => Err(format!(
            "audio selector {selector:?} is ambiguous: {}",
            matches
                .iter()
                .map(|endpoint| endpoint.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn set_default_roles(endpoint_id: &str) -> Result<(), String> {
    let _com = ComApartment::initialize()?;
    let policy: IPolicyConfig =
        unsafe { CoCreateInstance(&CLSID_POLICY_CONFIG_CLIENT, None, CLSCTX_ALL) }
            .map_err(|error| format!("cannot create audio default setter: {error}"))?;
    let endpoint_id = endpoint_id
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut failures = Vec::new();
    for (role_name, role) in [("Console", eConsole), ("Multimedia", eMultimedia)] {
        if let Err(error) =
            unsafe { policy.set_default_endpoint(PCWSTR(endpoint_id.as_ptr()), role) }
        {
            failures.push(format!(
                "cannot set {role_name} default render endpoint: {error}"
            ));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

#[repr(transparent)]
#[derive(Clone)]
struct IPolicyConfig(IUnknown);

unsafe impl Interface for IPolicyConfig {
    type Vtable = IPolicyConfigVtbl;
    const IID: GUID = IID_POLICY_CONFIG;
}

type UnusedPolicyMethod = unsafe extern "system" fn();

#[repr(C)]
struct IPolicyConfigVtbl {
    base__: IUnknown_Vtbl,
    get_mix_format: UnusedPolicyMethod,
    get_device_format: UnusedPolicyMethod,
    reset_device_format: UnusedPolicyMethod,
    set_device_format: UnusedPolicyMethod,
    get_processing_period: UnusedPolicyMethod,
    set_processing_period: UnusedPolicyMethod,
    get_share_mode: UnusedPolicyMethod,
    set_share_mode: UnusedPolicyMethod,
    get_property_value: UnusedPolicyMethod,
    set_property_value: UnusedPolicyMethod,
    set_default_endpoint: unsafe extern "system" fn(
        *mut core::ffi::c_void,
        PCWSTR,
        windows::Win32::Media::Audio::ERole,
    ) -> HRESULT,
    set_endpoint_visibility: UnusedPolicyMethod,
}

impl IPolicyConfig {
    unsafe fn set_default_endpoint(
        &self,
        endpoint_id: PCWSTR,
        role: windows::Win32::Media::Audio::ERole,
    ) -> windows::core::Result<()> {
        unsafe { (self.vtable().set_default_endpoint)(self.as_raw(), endpoint_id, role).ok() }
    }
}

fn create_enumerator() -> Result<IMMDeviceEnumerator, String> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
        .map_err(|error| format!("cannot create Core Audio enumerator: {error}"))
}

fn snapshot(include_all: bool) -> Result<Vec<Endpoint>, String> {
    let _com = ComApartment::initialize()?;
    let enumerator = create_enumerator()?;
    let state_mask = if include_all {
        DEVICE_STATE(DEVICE_STATEMASK_ALL)
    } else {
        DEVICE_STATE_ACTIVE
    };
    let collection = unsafe { enumerator.EnumAudioEndpoints(eRender, state_mask) }
        .map_err(|error| format!("cannot enumerate render endpoints: {error}"))?;
    let count = unsafe { collection.GetCount() }
        .map_err(|error| format!("cannot count render endpoints: {error}"))?;
    let mut endpoints = Vec::with_capacity(count as usize);
    for index in 0..count {
        let device = unsafe { collection.Item(index) }
            .map_err(|error| format!("cannot read render endpoint {index}: {error}"))?;
        endpoints.push(endpoint(&device)?);
    }
    endpoints.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
            .then(left.id.cmp(&right.id))
    });
    Ok(endpoints)
}

fn defaults() -> Result<Vec<(&'static str, Option<String>)>, String> {
    let _com = ComApartment::initialize()?;
    let enumerator = create_enumerator()?;
    [
        ("Console", eConsole),
        ("Multimedia", eMultimedia),
        ("Communications", eCommunications),
    ]
    .into_iter()
    .map(|(name, role)| Ok((name, default_for_role(&enumerator, name, role)?)))
    .collect()
}

fn default_for_role(
    enumerator: &IMMDeviceEnumerator,
    role_name: &str,
    role: windows::Win32::Media::Audio::ERole,
) -> Result<Option<String>, String> {
    let device = match unsafe { enumerator.GetDefaultAudioEndpoint(eRender, role) } {
        Ok(device) => device,
        Err(error) if is_no_default_error(&error) => return Ok(None),
        Err(error) => {
            return Err(format!(
                "cannot query {role_name} default render endpoint: {error}"
            ));
        }
    };
    let id = unsafe { device.GetId() }
        .map_err(|error| format!("cannot read {role_name} default endpoint ID: {error}"))?;
    Ok(Some(unsafe { take_pwstr(id) }))
}

fn endpoint(device: &IMMDevice) -> Result<Endpoint, String> {
    let id =
        unsafe { device.GetId() }.map_err(|error| format!("cannot read endpoint ID: {error}"))?;
    let id = unsafe { take_pwstr(id) };
    let state = EndpointState::from_win32(
        unsafe { device.GetState() }
            .map_err(|error| format!("cannot read endpoint state: {error}"))?,
    );
    let store = unsafe { device.OpenPropertyStore(STGM_READ) }
        .map_err(|error| format!("cannot read properties for {id}: {error}"))?;
    let friendly_name = property_string(&store, &PKEY_Device_FriendlyName);
    let name = friendly_name.clone().unwrap_or_else(|| id.clone());
    let adapter_name = property_string(&store, &PKEY_DeviceInterface_FriendlyName);
    let manufacturer = property_string(&store, &PKEY_Device_Manufacturer);
    let description = property_string(&store, &PKEY_Device_DeviceDesc);
    let evidence = [
        ("friendly_name", friendly_name.as_deref()),
        ("adapter", adapter_name.as_deref()),
        ("manufacturer", manufacturer.as_deref()),
        ("description", description.as_deref()),
    ];
    let evidence_text = evidence
        .iter()
        .filter_map(|(source, value)| {
            value
                .filter(|value| !value.is_empty())
                .map(|value| format!("{source}={value}"))
        })
        .collect::<Vec<_>>()
        .join(" | ");
    let classification = classify(&evidence);
    Ok(Endpoint {
        id,
        name,
        state,
        active: state == EndpointState::Active,
        adapter_name,
        manufacturer,
        classification,
        classification_evidence: evidence_text,
    })
}

fn property_string(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let value: PROPVARIANT = unsafe { store.GetValue(key).ok()? };
    let mut buffer = [0u16; 512];
    unsafe { PropVariantToString(&value, &mut buffer).ok()? };
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    let value = String::from_utf16_lossy(&buffer[..length]);
    (!value.is_empty()).then_some(value)
}

fn classify(metadata: &[(&str, Option<&str>)]) -> NvidiaClassification {
    let trusted_values = metadata
        .iter()
        .filter(|(source, _)| matches!(*source, "adapter" | "manufacturer"))
        .filter_map(|(_, value)| *value);
    let values = trusted_values.collect::<Vec<_>>();
    if values
        .iter()
        .any(|value| value.to_ascii_lowercase().contains("nvidia"))
    {
        NvidiaClassification::Nvidia
    } else if values.iter().any(|value| !is_generic_metadata(value)) {
        NvidiaClassification::NonNvidia
    } else {
        NvidiaClassification::Unknown
    }
}

fn is_no_default_error(error: &windows::core::Error) -> bool {
    error.code() == ERROR_NOT_FOUND.to_hresult()
}

fn is_generic_metadata(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "audio device"
            | "high definition audio"
            | "high definition audio device"
            | "usb audio device"
    )
}

unsafe fn take_pwstr(value: PWSTR) -> String {
    unsafe {
        if value.0.is_null() {
            return String::new();
        }
        let mut length = 0;
        while *value.0.add(length) != 0 {
            length += 1;
        }
        let result = String::from_utf16_lossy(std::slice::from_raw_parts(value.0, length));
        CoTaskMemFree(Some(value.0.cast()));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AudioDecision, Endpoint, EndpointState, NvidiaClassification, classify,
        is_no_default_error, reconcile, resolve_endpoint,
    };
    use windows::core::{Error, HRESULT};

    #[test]
    fn classification_uses_metadata() {
        assert_eq!(
            classify(&[
                ("friendly_name", Some("Speakers")),
                ("adapter", Some("NVIDIA High Definition Audio"),)
            ]),
            NvidiaClassification::Nvidia
        );
        assert_eq!(
            classify(&[("adapter", Some("Focusrite USB"))]),
            NvidiaClassification::NonNvidia
        );
        assert_eq!(
            classify(&[("friendly_name", Some("HDMI"))]),
            NvidiaClassification::Unknown
        );
        assert_eq!(
            classify(&[("adapter", Some("High Definition Audio Device"))]),
            NvidiaClassification::Unknown
        );
        assert_eq!(
            classify(&[
                ("adapter", Some("High Definition Audio Device")),
                ("description", Some("Gigabyte M32Q")),
            ]),
            NvidiaClassification::Unknown
        );
        assert_eq!(classify(&[]), NvidiaClassification::Unknown);
    }

    #[test]
    fn only_no_default_is_treated_as_unavailable() {
        assert!(is_no_default_error(&Error::from_hresult(
            super::ERROR_NOT_FOUND.to_hresult()
        )));
        assert!(!is_no_default_error(&Error::from_hresult(HRESULT(
            0x8000_4005_u32 as i32
        ))));
    }

    fn endpoint(id: &str, name: &str) -> Endpoint {
        Endpoint {
            id: id.into(),
            name: name.into(),
            state: EndpointState::Active,
            active: true,
            adapter_name: None,
            manufacturer: None,
            classification: NvidiaClassification::NonNvidia,
            classification_evidence: String::new(),
        }
    }

    fn classified_endpoint(id: &str, name: &str, classification: NvidiaClassification) -> Endpoint {
        Endpoint {
            classification,
            ..endpoint(id, name)
        }
    }

    #[test]
    fn reconciles_active_ids_and_appends_new_outputs_deterministically() {
        let endpoints = [
            endpoint("id-b", "Same name"),
            endpoint("id-a", "Same name"),
            endpoint("id-c", "Other name"),
        ];
        let result = reconcile(
            &["removed".into(), "id-b".into(), "id-b".into()],
            Ok(&endpoints),
            None,
            false,
        );

        assert_eq!(result.order, ["id-b", "id-c", "id-a"]);
        assert_eq!(result.decision, AudioDecision::Noop);
        assert!(!result.enumeration_failed);
    }

    #[test]
    fn reconnected_output_is_appended_after_live_removal() {
        let endpoints = [endpoint("id-a", "A"), endpoint("id-b", "B")];
        let present = reconcile(
            &["id-a".into(), "id-b".into()],
            Ok(&endpoints[..1]),
            None,
            false,
        );
        let returned = reconcile(&present.order, Ok(&endpoints), None, false);

        assert_eq!(present.order, ["id-a"]);
        assert_eq!(returned.order, ["id-a", "id-b"]);
    }

    #[test]
    fn promotes_selected_output_by_exact_id() {
        let endpoints = [endpoint("id-a", "Speakers"), endpoint("id-b", "Headset")];
        let result = reconcile(
            &["id-a".into(), "id-b".into()],
            Ok(&endpoints),
            Some("id-b"),
            false,
        );

        assert_eq!(result.order, ["id-b", "id-a"]);
        assert_eq!(result.decision, AudioDecision::Promote("id-b".into()));
    }

    #[test]
    fn suppression_restores_first_known_non_nvidia_output() {
        let endpoints = [
            classified_endpoint("nvidia", "Monitor", NvidiaClassification::Nvidia),
            classified_endpoint("dac", "DAC", NvidiaClassification::NonNvidia),
            classified_endpoint("unknown", "HDMI", NvidiaClassification::Unknown),
        ];
        let result = reconcile(
            &["nvidia".into(), "unknown".into(), "dac".into()],
            Ok(&endpoints),
            Some("nvidia"),
            true,
        );

        assert_eq!(result.order, ["nvidia", "unknown", "dac"]);
        assert_eq!(result.decision, AudioDecision::Restore("dac".into()));
    }

    #[test]
    fn suppression_accepts_non_nvidia_and_nvidia_when_disabled() {
        let endpoints = [
            classified_endpoint("nvidia", "Monitor", NvidiaClassification::Nvidia),
            classified_endpoint("dac", "DAC", NvidiaClassification::NonNvidia),
        ];
        let accepted = reconcile(
            &["dac".into(), "nvidia".into()],
            Ok(&endpoints),
            Some("dac"),
            true,
        );
        let allowed = reconcile(
            &["dac".into(), "nvidia".into()],
            Ok(&endpoints),
            Some("nvidia"),
            false,
        );

        assert_eq!(accepted.decision, AudioDecision::Promote("dac".into()));
        assert_eq!(allowed.decision, AudioDecision::Promote("nvidia".into()));
        assert_eq!(allowed.order, ["nvidia", "dac"]);
    }

    #[test]
    fn suppression_reports_missing_fallback_and_skips_unknown_classification() {
        let nvidia = classified_endpoint("nvidia", "Monitor", NvidiaClassification::Nvidia);
        let unknown = classified_endpoint("unknown", "HDMI", NvidiaClassification::Unknown);
        let no_fallback = reconcile(
            &["nvidia".into(), "unknown".into()],
            Ok(&[nvidia, unknown]),
            Some("nvidia"),
            true,
        );

        assert_eq!(no_fallback.decision, AudioDecision::NoEligibleFallback);
        assert_eq!(
            reconcile(
                &["unknown".into()],
                Ok(&[classified_endpoint(
                    "unknown",
                    "HDMI",
                    NvidiaClassification::Unknown,
                )]),
                Some("unknown"),
                true,
            )
            .decision,
            AudioDecision::Noop
        );
    }

    #[test]
    fn failed_enumeration_keeps_seed_but_empty_success_clears_it() {
        let seed = ["id-a".into()];
        let failed = reconcile(&seed, Err(()), Some("id-a"), true);
        let empty = reconcile(&seed, Ok(&[]), Some("id-a"), true);

        assert_eq!(failed.order, seed);
        assert!(failed.enumeration_failed);
        assert!(empty.order.is_empty());
        assert!(!empty.enumeration_failed);
        assert_eq!(empty.decision, AudioDecision::Noop);
    }

    #[test]
    fn resolves_audio_selectors_by_id_name_then_unique_substring() {
        let endpoints = [
            endpoint("id-1", "Desk Speakers"),
            endpoint("id-2", "USB DAC"),
        ];
        assert_eq!(resolve_endpoint(&endpoints, "id-1").unwrap().id, "id-1");
        assert_eq!(resolve_endpoint(&endpoints, "USB DAC").unwrap().id, "id-2");
        assert_eq!(resolve_endpoint(&endpoints, "desk").unwrap().id, "id-1");
    }

    #[test]
    fn rejects_ambiguous_audio_substrings() {
        let endpoints = [
            endpoint("id-1", "Desk Speakers"),
            endpoint("id-2", "Desk Headset"),
        ];
        assert!(resolve_endpoint(&endpoints, "desk").is_err());
    }

    #[test]
    fn resolves_unicode_case_insensitive_substrings() {
        let endpoints = [endpoint("id-1", "Écran USB")];
        assert_eq!(resolve_endpoint(&endpoints, "éCRAN").unwrap().id, "id-1");
    }
}
