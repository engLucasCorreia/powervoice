//! Persisted device preferences (SPEC-001 §2.5).
//!
//! [`DevicePrefs`] is the user's **intent**. Applying it to the current hardware
//! ([`crate::devices::resolve_devices`]) never modifies it: a fallback (missing device,
//! unsupported rate, out-of-range buffer) changes only the *applied* values, so the saved
//! preference is retried the next time the device or its capabilities come back. The settings
//! file itself (versioning, atomic writes) belongs to the app's settings service (T-104).
//!
//! Save rule for callers: write a user's change to the settings file only after the stream it
//! affects opened successfully (with or without fallback), never merely on selection.

use serde::{Deserialize, Deserializer, Serialize};

use crate::backend::{BufferRequest, HostId};

/// Device preferences: the six persisted values of SPEC-001 §3.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DevicePrefs {
    /// Audio host; `None` = the platform default. An unknown host string (e.g. a settings file
    /// copied from another OS) reads as `None`.
    #[serde(deserialize_with = "lenient_host")]
    pub host: Option<HostId>,
    /// Input device name on `host`; `None` = "None" (input disarmed, factory default).
    pub input_device: Option<String>,
    /// Input channel, 1-based (factory default 1).
    pub input_channel: u16,
    /// Output device name on `host`; `None` = the host's default output device.
    pub output_device: Option<String>,
    /// Sample rate; `None` = the device default.
    pub sample_rate_hz: Option<u32>,
    /// Buffer size; `Auto` (factory default) = device default.
    pub buffer_size: BufferRequest,
}

impl Default for DevicePrefs {
    fn default() -> Self {
        Self {
            host: None,
            input_device: None,
            input_channel: 1,
            output_device: None,
            sample_rate_hz: None,
            buffer_size: BufferRequest::Auto,
        }
    }
}

fn lenient_host<'de, D: Deserializer<'de>>(de: D) -> Result<Option<HostId>, D::Error> {
    let raw: Option<String> = Option::deserialize(de)?;
    Ok(raw.and_then(|s| s.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_defaults() {
        let p = DevicePrefs::default();
        assert_eq!(p.host, None);
        assert_eq!(p.input_device, None);
        assert_eq!(p.input_channel, 1);
        assert_eq!(p.output_device, None);
        assert_eq!(p.sample_rate_hz, None);
        assert_eq!(p.buffer_size, BufferRequest::Auto);
    }

    #[test]
    fn round_trips_all_six_values() {
        let p = DevicePrefs {
            host: Some(HostId::PipeWire),
            input_device: Some("Scarlett 2i2 USB".into()),
            input_channel: 2,
            output_device: Some("Built-in Audio Analog Stereo".into()),
            sample_rate_hz: Some(96_000),
            buffer_size: BufferRequest::Frames(256),
        };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<DevicePrefs>(&json).unwrap(), p);
        assert!(json.contains("\"host\":\"pipewire\""), "{json}");
        assert!(json.contains("\"buffer_size\":{\"frames\":256}"), "{json}");

        let auto = DevicePrefs::default();
        let json = serde_json::to_string(&auto).unwrap();
        assert!(json.contains("\"buffer_size\":\"auto\""), "{json}");
        assert_eq!(serde_json::from_str::<DevicePrefs>(&json).unwrap(), auto);
    }

    #[test]
    fn missing_fields_take_defaults_and_unknown_host_is_ignored() {
        let p: DevicePrefs = serde_json::from_str("{}").unwrap();
        assert_eq!(p, DevicePrefs::default());
        let p: DevicePrefs =
            serde_json::from_str(r#"{"host":"asio","output_device":"X","future":1}"#).unwrap();
        assert_eq!(p.host, None);
        assert_eq!(p.output_device.as_deref(), Some("X"));
    }
}
