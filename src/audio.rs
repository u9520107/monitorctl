use std::fmt;

use windows::{
    Win32::{
        Devices::FunctionDiscovery::{
            PKEY_Device_DeviceDesc, PKEY_Device_FriendlyName, PKEY_Device_Manufacturer,
            PKEY_DeviceInterface_FriendlyName,
        },
        Foundation::PROPERTYKEY,
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
    core::PWSTR,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NvidiaClassification {
    Nvidia,
    NonNvidia,
    Unknown,
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
    Ok([
        ("Console", eConsole),
        ("Multimedia", eMultimedia),
        ("Communications", eCommunications),
    ]
    .into_iter()
    .map(|(name, role)| {
        let id = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, role) }
            .ok()
            .and_then(|device| unsafe { device.GetId().ok() })
            .map(|id| unsafe { take_pwstr(id) });
        (name, id)
    })
    .collect())
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
        friendly_name,
        adapter_name.clone(),
        manufacturer.clone(),
        description,
    ]
    .into_iter()
    .flatten()
    .filter(|value| !value.is_empty())
    .collect::<Vec<_>>();
    let evidence_text = evidence.join(" | ");
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

fn classify(values: &[String]) -> NvidiaClassification {
    if values.is_empty() {
        return NvidiaClassification::Unknown;
    }
    if values
        .iter()
        .any(|value| value.to_ascii_lowercase().contains("nvidia"))
    {
        NvidiaClassification::Nvidia
    } else {
        NvidiaClassification::NonNvidia
    }
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
    use super::{NvidiaClassification, classify};

    #[test]
    fn classification_uses_metadata() {
        assert_eq!(
            classify(&["Speakers".into(), "NVIDIA High Definition Audio".into()]),
            NvidiaClassification::Nvidia
        );
        assert_eq!(
            classify(&["Focusrite USB".into()]),
            NvidiaClassification::NonNvidia
        );
        assert_eq!(classify(&[]), NvidiaClassification::Unknown);
    }
}
