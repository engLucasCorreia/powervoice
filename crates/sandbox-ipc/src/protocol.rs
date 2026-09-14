//! Control-channel messages, protocol version 1 (T-802, ADR-008 §4 + Amendment 2).
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
pub const PROTOCOL_VERSION: u32 = 1;

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
    /// The request failed (the plugin or backend's message).
    Error {
        /// What went wrong.
        message: String,
    },
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
    fn json_shape_is_tagged() {
        let json = serde_json::to_string(&Request {
            id: 3,
            body: RequestBody::Deactivate,
        })
        .unwrap();
        assert_eq!(json, r#"{"id":3,"body":{"op":"deactivate"}}"#);
    }
}
