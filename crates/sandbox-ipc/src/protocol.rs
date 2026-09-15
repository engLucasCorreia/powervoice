//! Control-channel messages, protocol version 2 (T-802, ADR-008 §4 + Amendments 2–3; v2 = T-803:
//! parameter text requests, `PluginInfo::param_text`, the scan report of `--scan`).
//!
//! Every host → sandbox [`Request`] gets exactly one [`Response`] with the same `id`, in order.
//! Messages are JSON (`serde`; the parameter schema is the module API's own types, as in the
//! sidecar); plugin state rides in the frame's binary payload ([`crate::control`]), never inside
//! the JSON. The sandbox never sends anything unsolicited in v1 (its log goes to stderr).
//!
//! Lifecycle (one sandbox process per plugin instance):
//! `Hello` → `Load` → (`SetParams` | `LoadState` | `SaveState`)* → `Activate` → … →
//! `Deactivate` → … → `Shutdown`. The shared-memory segment is handed over at spawn
//! (`--shm <handle>`) and re-initialised by the host before each `Activate`.

use std::io::{self, Read, Write};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use vox_module_api::{ParamGroup, ParamId, ParamInfo, ProcessMode};

use crate::control::{Frame, read_frame, write_frame};

/// Bumped on any incompatible message change; checked by `Hello`.
pub const PROTOCOL_VERSION: u32 = 2;

/// A host → sandbox request.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// Correlates the response.
    pub id: u64,
    /// What to do.
    pub body: RequestBody,
}

/// One parameter value (plain).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamValue {
    /// Parameter id.
    pub id: ParamId,
    /// Plain value.
    pub value: f64,
}

/// Request kinds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum RequestBody {
    /// Handshake: the host's protocol version. → [`ResponseBody::Hello`].
    Hello {
        /// [`PROTOCOL_VERSION`] of the host.
        protocol: u32,
    },
    /// Instantiate `plugin` through `backend` (`"test"`; T-803+: `"clap"`, `"vst3"`, …).
    /// → [`ResponseBody::Loaded`].
    Load {
        /// Backend name.
        backend: String,
        /// Backend-specific plugin reference (path, id, options).
        plugin: String,
    },
    /// Attach to the segment (initialised for `max_block`), activate the plugin and start the
    /// sandbox audio thread. → [`ResponseBody::Activated`].
    Activate {
        /// Sample rate in Hz.
        sample_rate: f64,
        /// The segment's `max_block` (ADR-008 `B`): the largest chunk the plugin gets.
        max_block: u32,
        /// Realtime or offline.
        mode: ProcessMode,
    },
    /// Stop the audio thread (after the host's `host_command` shutdown) and deactivate.
    /// → [`ResponseBody::Ok`].
    Deactivate,
    /// Set parameters (inactive only; while active, values travel through the event ring).
    /// → [`ResponseBody::Ok`].
    SetParams {
        /// Values to set.
        values: Vec<ParamValue>,
    },
    /// The plugin's state. → [`ResponseBody::State`] (payload = state bytes).
    SaveState,
    /// Load the payload as the plugin's state (inactive only). → [`ResponseBody::Params`]
    /// (the values the state set).
    LoadState,
    /// Stop everything and exit. → [`ResponseBody::Ok`], then the process exits.
    Shutdown,
    /// The plugin's display text for each value (T-803). → [`ResponseBody::Texts`] (one entry
    /// per value, `None` where the plugin has no text).
    ParamTexts {
        /// Values to format.
        values: Vec<ParamValue>,
    },
    /// Parses `text` with the plugin's own parser (T-803). → [`ResponseBody::Value`].
    TextToParam {
        /// Parameter.
        id: ParamId,
        /// Typed text.
        text: String,
    },
}

/// A sandbox → host response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// The request's id.
    pub id: u64,
    /// The answer.
    pub body: ResponseBody,
}

/// What a loaded plugin looks like.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginInfo {
    /// Display name.
    pub name: String,
    /// Vendor.
    pub vendor: String,
    /// Plugin version (free text; the host parses it leniently).
    pub version: String,
    /// Parameter schema (mirrored by the host's proxy module).
    pub params: Vec<ParamInfo>,
    /// Parameter groups.
    pub groups: Vec<ParamGroup>,
    /// Current value of every parameter in `params`.
    pub values: Vec<ParamValue>,
    /// The plugin formats and parses its own parameter text (`ParamTexts`/`TextToParam`
    /// answer it; T-803). `false`: the host formats with the module API's text rules.
    #[serde(default)]
    pub param_text: bool,
}

/// Response kinds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub enum ResponseBody {
    /// Handshake answer.
    Hello {
        /// [`PROTOCOL_VERSION`] of the sandbox.
        protocol: u32,
        /// The sandbox's process id.
        pid: u32,
    },
    /// The plugin loaded.
    Loaded(PluginInfo),
    /// The plugin is active and served.
    Activated {
        /// The plugin's own latency (the proxy adds the transport's `B`).
        latency_samples: u32,
        /// Tail in samples; `None` = infinite.
        tail_samples: Option<u64>,
    },
    /// Done.
    Ok,
    /// The plugin state is the frame's payload.
    State,
    /// Current parameter values (after `LoadState`).
    Params {
        /// Every parameter's value.
        values: Vec<ParamValue>,
    },
    /// Display texts (`ParamTexts`).
    Texts {
        /// One per requested value.
        texts: Vec<Option<String>>,
    },
    /// A parsed value (`TextToParam`); `None` if the plugin couldn't parse the text.
    Value {
        /// Plain value.
        value: Option<f64>,
    },
    /// The request failed (the plugin or backend's message).
    Error {
        /// What went wrong.
        message: String,
    },
}

/// What the CLAP backend loads (T-803): the `.clap` file (bundle on macOS) and the plugin id
/// inside it. Travels as the `Load { plugin }` reference, JSON-encoded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClapPluginRef {
    /// Path of the `.clap` file or bundle.
    pub path: String,
    /// The CLAP plugin id.
    pub id: String,
}

impl ClapPluginRef {
    /// The `Load { plugin }` string.
    pub fn to_reference(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Parses a `Load { plugin }` string.
    pub fn parse(reference: &str) -> Result<Self, String> {
        serde_json::from_str(reference).map_err(|e| format!("bad CLAP plugin reference: {e}"))
    }
}

/// What the VST3 backend loads (T-806, ADR-008 Amendment 6): the `.vst3` bundle (a regular file
/// is loaded as the module binary itself) and the class id of the audio processor inside it —
/// 32 upper-case hex digits in the SDK's FUID string order, as `moduleinfo.json`'s `CID`.
/// Travels as the `Load { plugin }` reference, JSON-encoded like [`ClapPluginRef`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vst3PluginRef {
    /// Path of the `.vst3` bundle (or single file).
    pub path: String,
    /// The processor's class id (32 hex digits).
    pub cid: String,
}

impl Vst3PluginRef {
    /// The `Load { plugin }` string.
    pub fn to_reference(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Parses a `Load { plugin }` string.
    pub fn parse(reference: &str) -> Result<Self, String> {
        serde_json::from_str(reference).map_err(|e| format!("bad VST3 plugin reference: {e}"))
    }
}

/// What the LV2 backend loads (T-807, ADR-008 Amendment 8): the `.lv2` bundle directory and the
/// plugin's URI inside it. Travels as the `Load { plugin }` reference, JSON-encoded like
/// [`ClapPluginRef`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lv2PluginRef {
    /// Path of the `.lv2` bundle directory.
    pub path: String,
    /// The plugin's URI.
    pub uri: String,
}

impl Lv2PluginRef {
    /// The `Load { plugin }` string.
    pub fn to_reference(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Parses a `Load { plugin }` string.
    pub fn parse(reference: &str) -> Result<Self, String> {
        serde_json::from_str(reference).map_err(|e| format!("bad LV2 plugin reference: {e}"))
    }
}

/// One plugin a scanned file offers (`powervoice-sandbox --scan`, ADR-008 §6; T-803, richer
/// fields T-804 Amendment 4).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScannedPlugin {
    /// The format's plugin id (CLAP: reverse DNS).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Vendor.
    pub vendor: String,
    /// Version (free text).
    pub version: String,
    /// One-line description.
    pub description: String,
    /// Homepage.
    pub url: Option<String>,
    /// Feature strings (CLAP features: `"audio-effect"`, `"compressor"`, `"stereo"`, …).
    pub features: Vec<String>,
    /// Parameter count (T-804, ADR-008 §6 "richer scan data"): known only for a plugin the scan
    /// instantiated (an audio effect); `0` otherwise (never instantiated).
    #[serde(default)]
    pub param_count: u32,
    /// Main input audio port channel count (`0`: no input port, or never instantiated). `1` =
    /// mono, `2` = stereo (hosted through the mono shim, ADR-008 Amendment 3 §2).
    #[serde(default)]
    pub main_input_channels: u32,
    /// Main output audio port channel count, same convention as
    /// [`main_input_channels`](Self::main_input_channels).
    #[serde(default)]
    pub main_output_channels: u32,
}

/// What `powervoice-sandbox --scan <file> --format <format>` prints on its protocol output
/// (one JSON document) before exiting 0.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanReport {
    /// The scanned file.
    pub path: String,
    /// Its plugins.
    pub plugins: Vec<ScannedPlugin>,
}

/// What `powervoice-sandbox --scan` prints (one JSON document): the report, or why the file
/// couldn't be scanned (not a plugin, no factory, …). A crash or hang prints nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanReply {
    /// Scanned.
    Ok(ScanReport),
    /// Couldn't be scanned.
    Error(String),
}

fn json_err(e: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// Sends `msg` (+ `payload`) as one frame.
pub fn send<T: Serialize>(w: &mut impl Write, msg: &T, payload: &[u8]) -> io::Result<()> {
    let json = serde_json::to_vec(msg).map_err(json_err)?;
    write_frame(w, &json, payload)
}

/// Receives one message and its payload; `Ok(None)` when the peer closed the stream.
pub fn recv<T: DeserializeOwned>(r: &mut impl Read) -> io::Result<Option<(T, Vec<u8>)>> {
    let Some(Frame { json, payload }) = read_frame(r)? else {
        return Ok(None);
    };
    let msg = serde_json::from_slice(&json).map_err(json_err)?;
    Ok(Some((msg, payload)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_module_api::{LocalizedText, ParamFlags, Taper, Unit};

    fn param() -> ParamInfo {
        ParamInfo {
            id: ParamId(7),
            key: "gain_db".into(),
            name: LocalizedText::plain("Gain"),
            group: None,
            unit: Unit::Db,
            min: -60.0,
            max: 24.0,
            default: 0.1,
            taper: Taper::Db {
                neg_inf_at_min: true,
            },
            step: None,
            enum_labels: Vec::new(),
            decimals: 1,
            smoothing_ms: 20.0,
            flags: ParamFlags::AUTOMATABLE,
        }
    }

    #[test]
    fn messages_round_trip_with_payloads() {
        let reqs = [
            RequestBody::Hello {
                protocol: PROTOCOL_VERSION,
            },
            RequestBody::Load {
                backend: "test".into(),
                plugin: "crash?after=3".into(),
            },
            RequestBody::Activate {
                sample_rate: 44_100.0,
                max_block: 256,
                mode: ProcessMode::Offline,
            },
            RequestBody::Deactivate,
            RequestBody::SetParams {
                values: vec![ParamValue {
                    id: ParamId(0),
                    value: -59.999_999_999_999_99,
                }],
            },
            RequestBody::SaveState,
            RequestBody::LoadState,
            RequestBody::Shutdown,
            RequestBody::ParamTexts {
                values: vec![ParamValue {
                    id: ParamId(3),
                    value: 0.5,
                }],
            },
            RequestBody::TextToParam {
                id: ParamId(3),
                text: "-6 dB".into(),
            },
        ];
        let mut buf = Vec::new();
        for (i, body) in reqs.iter().enumerate() {
            let payload = vec![i as u8; i];
            send(
                &mut buf,
                &Request {
                    id: i as u64,
                    body: body.clone(),
                },
                &payload,
            )
            .unwrap();
        }
        let mut r = buf.as_slice();
        for (i, body) in reqs.iter().enumerate() {
            let (got, payload): (Request, _) = recv(&mut r).unwrap().unwrap();
            assert_eq!(got.id, i as u64);
            assert_eq!(&got.body, body);
            assert_eq!(payload, vec![i as u8; i]);
        }
        assert!(recv::<Request>(&mut r).unwrap().is_none());

        let resps = [
            ResponseBody::Hello {
                protocol: 1,
                pid: 42,
            },
            ResponseBody::Loaded(PluginInfo {
                name: "Gain".into(),
                vendor: "PowerVoice".into(),
                version: "1.0.0".into(),
                params: vec![param()],
                groups: Vec::new(),
                values: vec![ParamValue {
                    id: ParamId(7),
                    value: 0.1,
                }],
                param_text: true,
            }),
            ResponseBody::Activated {
                latency_samples: 3,
                tail_samples: None,
            },
            ResponseBody::Ok,
            ResponseBody::State,
            ResponseBody::Params {
                values: vec![ParamValue {
                    id: ParamId(1),
                    value: 0.25,
                }],
            },
            ResponseBody::Texts {
                texts: vec![Some("-6.0 dB".into()), None],
            },
            ResponseBody::Value { value: Some(0.25) },
            ResponseBody::Error {
                message: "nope".into(),
            },
        ];
        let mut buf = Vec::new();
        for (i, body) in resps.iter().enumerate() {
            send(
                &mut buf,
                &Response {
                    id: i as u64,
                    body: body.clone(),
                },
                b"state",
            )
            .unwrap();
        }
        let mut r = buf.as_slice();
        for body in &resps {
            let (got, payload): (Response, _) = recv(&mut r).unwrap().unwrap();
            assert_eq!(&got.body, body);
            assert_eq!(payload, b"state");
        }
    }

    #[test]
    fn clap_references_and_scan_reports_round_trip() {
        let r = ClapPluginRef {
            path: "/home/u/.clap/a|b@c.clap".into(),
            id: "com.acme.deesser".into(),
        };
        assert_eq!(ClapPluginRef::parse(&r.to_reference()).unwrap(), r);
        assert!(ClapPluginRef::parse("com.acme.deesser").is_err());
        let v = Vst3PluginRef {
            path: "/home/u/.vst3/Acme Gain.vst3".into(),
            cid: "50565633544553544741494E00000001".into(),
        };
        assert_eq!(Vst3PluginRef::parse(&v.to_reference()).unwrap(), v);
        assert!(Vst3PluginRef::parse(&r.to_reference()).is_err());
        let l = Lv2PluginRef {
            path: "/usr/lib/lv2/acme.lv2".into(),
            uri: "http://acme.example/plugins/eq".into(),
        };
        assert_eq!(Lv2PluginRef::parse(&l.to_reference()).unwrap(), l);
        assert!(Lv2PluginRef::parse(&r.to_reference()).is_err());
        let report = ScanReport {
            path: "/x.clap".into(),
            plugins: vec![ScannedPlugin {
                id: "com.acme.deesser".into(),
                name: "De-esser".into(),
                vendor: "Acme".into(),
                version: "1.2".into(),
                description: String::new(),
                url: None,
                features: vec!["audio-effect".into(), "stereo".into()],
                param_count: 3,
                main_input_channels: 2,
                main_output_channels: 2,
            }],
        };
        let json = serde_json::to_string(&ScanReply::Ok(report.clone())).unwrap();
        assert_eq!(
            serde_json::from_str::<ScanReply>(&json).unwrap(),
            ScanReply::Ok(report)
        );
        let json = serde_json::to_string(&ScanReply::Error("no".into())).unwrap();
        assert_eq!(json, r#"{"error":"no"}"#);
        // A v1 `Loaded` (no `param_text`) still parses.
        let info: PluginInfo = serde_json::from_str(
            r#"{"name":"G","vendor":"V","version":"1","params":[],"groups":[],"values":[]}"#,
        )
        .unwrap();
        assert!(!info.param_text);
    }

    #[test]
    fn json_shape_is_tagged() {
        let json = serde_json::to_string(&Request {
            id: 3,
            body: RequestBody::Deactivate,
        })
        .unwrap();
        assert_eq!(json, r#"{"id":3,"body":{"op":"deactivate"}}"#);
    }
}
