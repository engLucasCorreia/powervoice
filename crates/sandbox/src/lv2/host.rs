//! The host features PowerVoice gives an LV2 instance (ADR-008 Amendment 8 §2): `urid:map` /
//! `urid:unmap` (and the deprecated `uri-map`), `options:options` (block lengths, sequence size,
//! sample rate), `bufsz:boundedBlockLength`, `worker:schedule`, `state:loadDefaultState`; for
//! state save/restore also `state:mapPath` / `state:freePath` (identity mapping).
//!
//! A [`Host`] lives in a `Box` (stable addresses: the plugin keeps the feature pointers) for as
//! long as the instance it was created for; a re-instantiation (another sample rate or block
//! size) gets a new one.

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::{Mutex, PoisonError};

use rtrb::Producer;
use vox_lv2_abi::*;

use super::worker;

/// The features PowerVoice provides or honours, for a plugin's `lv2:requiredFeature` check.
/// `lv2:isLive`, `lv2:inPlaceBroken` (buffers are never shared) and `lv2:hardRTCapable` need no
/// data; the state path features are given to save/restore.
pub(crate) const SUPPORTED_FEATURES: &[&CStr] = &[
    uri::URID_MAP,
    uri::URID_UNMAP,
    uri::URI_MAP,
    uri::OPTIONS_OPTIONS,
    uri::BUF_SIZE_BOUNDED_BLOCK_LENGTH,
    uri::WORKER_SCHEDULE,
    uri::STATE_LOAD_DEFAULT_STATE,
    uri::STATE_MAP_PATH,
    uri::STATE_FREE_PATH,
    uri::STATE_THREAD_SAFE_RESTORE,
    uri::IS_LIVE,
    uri::IN_PLACE_BROKEN,
    uri::HARD_RT_CAPABLE,
];

/// The options PowerVoice passes (for a plugin's `opts:requiredOption` check).
pub(crate) const PROVIDED_OPTIONS: &[&CStr] = &[
    uri::BUF_SIZE_MAX_BLOCK_LENGTH,
    uri::BUF_SIZE_MIN_BLOCK_LENGTH,
    uri::BUF_SIZE_NOMINAL_BLOCK_LENGTH,
    uri::BUF_SIZE_SEQUENCE_SIZE,
    uri::PARAM_SAMPLE_RATE,
];

/// URI ↔ URID, shared by every caller (thread-safe, as `urid:map` requires; plugins normally
/// map at instantiation).
#[derive(Default)]
pub(crate) struct UridTable {
    inner: Mutex<Table>,
}

#[derive(Default)]
struct Table {
    ids: HashMap<Vec<u8>, u32>,
    /// `uris[id - 1]`; each `CString`'s buffer stays put when the `Vec` grows, so `unmap`'s
    /// pointers stay valid for the table's lifetime.
    uris: Vec<CString>,
}

impl UridTable {
    /// The id of `uri` (new ids from 1).
    pub(crate) fn map(&self, uri: &CStr) -> u32 {
        let mut t = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(&id) = t.ids.get(uri.to_bytes()) {
            return id;
        }
        t.uris.push(uri.to_owned());
        let id = t.uris.len() as u32;
        t.ids.insert(uri.to_bytes().to_vec(), id);
        id
    }

    /// [`Self::map`] for a Rust string (0 for an interior NUL).
    pub(crate) fn map_str(&self, uri: &str) -> u32 {
        CString::new(uri).map_or(0, |c| self.map(&c))
    }

    /// The URI of `id`.
    pub(crate) fn unmap(&self, id: u32) -> Option<String> {
        let t = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        t.uris
            .get((id as usize).checked_sub(1)?)
            .map(|c| c.to_string_lossy().into_owned())
    }

    fn unmap_ptr(&self, id: u32) -> *const c_char {
        let t = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        (id as usize)
            .checked_sub(1)
            .and_then(|i| t.uris.get(i))
            .map_or(std::ptr::null(), |c| c.as_ptr())
    }
}

unsafe extern "C" fn map_cb(handle: *mut c_void, uri: *const c_char) -> u32 {
    if handle.is_null() || uri.is_null() {
        return 0;
    }
    // SAFETY: `handle` is the host's table (it outlives the instance); `uri` is NUL-terminated.
    unsafe { (*(handle as *const UridTable)).map(CStr::from_ptr(uri)) }
}

unsafe extern "C" fn unmap_cb(handle: *mut c_void, id: u32) -> *const c_char {
    if handle.is_null() {
        return std::ptr::null();
    }
    // SAFETY: `handle` is the host's table.
    unsafe { (*(handle as *const UridTable)).unmap_ptr(id) }
}

unsafe extern "C" fn uri_to_id_cb(
    data: *mut c_void,
    _map: *const c_char,
    uri: *const c_char,
) -> u32 {
    // SAFETY: same contract as `map_cb`.
    unsafe { map_cb(data, uri) }
}

/// The URIDs the host itself uses.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Urids {
    pub(crate) atom_sequence: u32,
    pub(crate) atom_chunk: u32,
    pub(crate) atom_int: u32,
    pub(crate) atom_float: u32,
    pub(crate) max_block: u32,
    pub(crate) min_block: u32,
    pub(crate) nominal_block: u32,
    pub(crate) sequence_size: u32,
    pub(crate) sample_rate: u32,
}

/// Where `worker:schedule` puts requests: the ring of the current activation (set and cleared
/// by the main thread while no `run` can happen; used by whichever thread runs the plugin — the
/// audio thread, or the main thread during the activation's latency probe; never two at once).
pub(crate) struct WorkerSlot {
    tx: UnsafeCell<Option<Producer<u8>>>,
}

// SAFETY: see the type's docs — accesses never overlap (activation/deactivation on the main
// thread happen while the audio thread doesn't run the plugin).
unsafe impl Sync for WorkerSlot {}

impl WorkerSlot {
    /// Installs (or removes) the request ring.
    ///
    /// # Safety
    /// No `run` of the instance may be in progress (the plugin is inactive, or the caller is the
    /// thread that runs it).
    pub(crate) unsafe fn set(&self, tx: Option<Producer<u8>>) {
        // SAFETY: caller's contract: no concurrent access.
        unsafe { *self.tx.get() = tx };
    }
}

unsafe extern "C" fn schedule_cb(handle: *mut c_void, size: u32, data: *const c_void) -> u32 {
    if handle.is_null() {
        return LV2_WORKER_ERR_UNKNOWN;
    }
    // SAFETY: `handle` is the host's slot; only the thread running the plugin gets here.
    let slot = unsafe { &mut *(*(handle as *const WorkerSlot)).tx.get() };
    match slot {
        Some(tx) => worker::write_frame(tx, data, size),
        None => LV2_WORKER_ERR_UNKNOWN,
    }
}

/// `state:mapPath`: paths are kept as they are (absolute).
unsafe extern "C" fn map_path_cb(_handle: *mut c_void, path: *const c_char) -> *mut c_char {
    if path.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: a NUL-terminated path; the plugin frees the copy with `free`/`freePath`.
    unsafe { libc::strdup(path) }
}

unsafe extern "C" fn free_path_cb(_handle: *mut c_void, path: *mut c_char) {
    // SAFETY: a path `map_path_cb` returned (`strdup` → `free`).
    unsafe { libc::free(path.cast()) };
}

/// The option values (`options` points at these).
struct OptionValues {
    max_block: i32,
    min_block: i32,
    nominal_block: i32,
    sequence_size: i32,
    sample_rate: f32,
}

const FEATURES: usize = 7;

/// Every feature of one instance, at stable addresses.
pub(crate) struct Host {
    pub(crate) urid: UridTable,
    pub(crate) u: Urids,
    pub(crate) worker: WorkerSlot,
    map: LV2_URID_Map,
    unmap: LV2_URID_Unmap,
    uri_map: LV2_URI_Map_Feature,
    schedule: LV2_Worker_Schedule,
    values: OptionValues,
    options: [LV2_Options_Option; 6],
    map_path: LV2_State_Map_Path,
    free_path: LV2_State_Free_Path,
    features: [LV2_Feature; FEATURES],
    feature_ptrs: [*const LV2_Feature; FEATURES + 1],
    state_features: [LV2_Feature; 2],
    state_feature_ptrs: [*const LV2_Feature; 3],
}

// SAFETY: after construction the host is immutable except `worker` (see `WorkerSlot`) and the
// URID table (a mutex); its raw pointers point into itself.
unsafe impl Send for Host {}
// SAFETY: see `Send`.
unsafe impl Sync for Host {}

const NULL_FEATURE: LV2_Feature = LV2_Feature {
    URI: std::ptr::null(),
    data: std::ptr::null_mut(),
};

const NULL_OPTION: LV2_Options_Option = LV2_Options_Option {
    context: 0,
    subject: 0,
    key: 0,
    size: 0,
    type_: 0,
    value: std::ptr::null(),
};

impl Host {
    /// The features for an instance at `sample_rate` that is run with at most `max_block` frames
    /// (and at least one), with atom buffers of `sequence_size` bytes.
    pub(crate) fn new(sample_rate: f64, max_block: u32, sequence_size: u32) -> Box<Self> {
        let urid = UridTable::default();
        let u = Urids {
            atom_sequence: urid.map(uri::ATOM_SEQUENCE),
            atom_chunk: urid.map(uri::ATOM_CHUNK),
            atom_int: urid.map(uri::ATOM_INT),
            atom_float: urid.map(uri::ATOM_FLOAT),
            max_block: urid.map(uri::BUF_SIZE_MAX_BLOCK_LENGTH),
            min_block: urid.map(uri::BUF_SIZE_MIN_BLOCK_LENGTH),
            nominal_block: urid.map(uri::BUF_SIZE_NOMINAL_BLOCK_LENGTH),
            sequence_size: urid.map(uri::BUF_SIZE_SEQUENCE_SIZE),
            sample_rate: urid.map(uri::PARAM_SAMPLE_RATE),
        };
        let max = i32::try_from(max_block.max(1)).unwrap_or(i32::MAX);
        let mut h = Box::new(Self {
            urid,
            u,
            worker: WorkerSlot {
                tx: UnsafeCell::new(None),
            },
            map: LV2_URID_Map {
                handle: std::ptr::null_mut(),
                map: Some(map_cb),
            },
            unmap: LV2_URID_Unmap {
                handle: std::ptr::null_mut(),
                unmap: Some(unmap_cb),
            },
            uri_map: LV2_URI_Map_Feature {
                callback_data: std::ptr::null_mut(),
                uri_to_id: Some(uri_to_id_cb),
            },
            schedule: LV2_Worker_Schedule {
                handle: std::ptr::null_mut(),
                schedule_work: Some(schedule_cb),
            },
            values: OptionValues {
                max_block: max,
                // Blocks are split at parameter events (sample-accurate control), down to one
                // frame.
                min_block: 1,
                nominal_block: max,
                sequence_size: i32::try_from(sequence_size).unwrap_or(i32::MAX),
                sample_rate: sample_rate as f32,
            },
            options: [NULL_OPTION; 6],
            map_path: LV2_State_Map_Path {
                handle: std::ptr::null_mut(),
                abstract_path: Some(map_path_cb),
                absolute_path: Some(map_path_cb),
            },
            free_path: LV2_State_Free_Path {
                handle: std::ptr::null_mut(),
                free_path: Some(free_path_cb),
            },
            features: [NULL_FEATURE; FEATURES],
            feature_ptrs: [std::ptr::null(); FEATURES + 1],
            state_features: [NULL_FEATURE; 2],
            state_feature_ptrs: [std::ptr::null(); 3],
        });
        let table: *mut c_void = (&raw const h.urid).cast_mut().cast();
        h.map.handle = table;
        h.unmap.handle = table;
        h.uri_map.callback_data = table;
        h.schedule.handle = (&raw const h.worker).cast_mut().cast();
        let option = |key: u32, type_: u32, value: *const c_void, size: u32| LV2_Options_Option {
            context: LV2_OPTIONS_INSTANCE,
            subject: 0,
            key,
            size,
            type_,
            value,
        };
        let (int, float) = (h.u.atom_int, h.u.atom_float);
        h.options = [
            option(
                h.u.max_block,
                int,
                (&raw const h.values.max_block).cast(),
                4,
            ),
            option(
                h.u.min_block,
                int,
                (&raw const h.values.min_block).cast(),
                4,
            ),
            option(
                h.u.nominal_block,
                int,
                (&raw const h.values.nominal_block).cast(),
                4,
            ),
            option(
                h.u.sequence_size,
                int,
                (&raw const h.values.sequence_size).cast(),
                4,
            ),
            option(
                h.u.sample_rate,
                float,
                (&raw const h.values.sample_rate).cast(),
                4,
            ),
            NULL_OPTION,
        ];
        let feature = |uri: &CStr, data: *mut c_void| LV2_Feature {
            URI: uri.as_ptr(),
            data,
        };
        h.features = [
            feature(uri::URID_MAP, (&raw mut h.map).cast()),
            feature(uri::URID_UNMAP, (&raw mut h.unmap).cast()),
            feature(uri::URI_MAP, (&raw mut h.uri_map).cast()),
            feature(uri::OPTIONS_OPTIONS, h.options.as_mut_ptr().cast()),
            feature(uri::BUF_SIZE_BOUNDED_BLOCK_LENGTH, std::ptr::null_mut()),
            feature(uri::WORKER_SCHEDULE, (&raw mut h.schedule).cast()),
            feature(uri::STATE_LOAD_DEFAULT_STATE, std::ptr::null_mut()),
        ];
        for i in 0..FEATURES {
            h.feature_ptrs[i] = &raw const h.features[i];
        }
        h.state_features = [
            feature(uri::STATE_MAP_PATH, (&raw mut h.map_path).cast()),
            feature(uri::STATE_FREE_PATH, (&raw mut h.free_path).cast()),
        ];
        h.state_feature_ptrs = [
            &raw const h.state_features[0],
            &raw const h.state_features[1],
            std::ptr::null(),
        ];
        h
    }

    /// The null-terminated feature array for `instantiate`.
    pub(crate) fn features(&self) -> *const *const LV2_Feature {
        self.feature_ptrs.as_ptr()
    }

    /// The null-terminated feature array for state save/restore.
    pub(crate) fn state_features(&self) -> *const *const LV2_Feature {
        self.state_feature_ptrs.as_ptr()
    }

    /// `urid:map` (for lilv's default-state loading).
    pub(crate) fn map(&self) -> *const LV2_URID_Map {
        &raw const self.map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// # Safety
    /// `features` is a valid null-terminated feature array.
    unsafe fn find(features: *const *const LV2_Feature, uri: &CStr) -> Option<*mut c_void> {
        let mut k = 0;
        loop {
            // SAFETY: caller's contract.
            let f = unsafe { *features.add(k) };
            if f.is_null() {
                return None;
            }
            // SAFETY: a valid feature.
            if unsafe { CStr::from_ptr((*f).URI) } == uri {
                // SAFETY: as above.
                return Some(unsafe { (*f).data });
            }
            k += 1;
        }
    }

    #[test]
    fn features_map_urids_and_carry_the_options() {
        let h = Host::new(44_100.0, 1024, 8192);
        // SAFETY: the host's own arrays; the callbacks get the host's handles.
        unsafe {
            let map = find(h.features(), uri::URID_MAP).unwrap() as *const LV2_URID_Map;
            let id = ((*map).map.unwrap())((*map).handle, c"urn:x:a".as_ptr());
            assert_eq!(
                ((*map).map.unwrap())((*map).handle, c"urn:x:a".as_ptr()),
                id
            );
            let unmap = find(h.features(), uri::URID_UNMAP).unwrap() as *const LV2_URID_Unmap;
            let back = ((*unmap).unmap.unwrap())((*unmap).handle, id);
            assert_eq!(CStr::from_ptr(back), c"urn:x:a");
            assert!(((*unmap).unmap.unwrap())((*unmap).handle, 9999).is_null());
            let old = find(h.features(), uri::URI_MAP).unwrap() as *const LV2_URI_Map_Feature;
            assert_eq!(
                ((*old).uri_to_id.unwrap())(
                    (*old).callback_data,
                    std::ptr::null(),
                    c"urn:x:a".as_ptr()
                ),
                id
            );
            let mut o =
                find(h.features(), uri::OPTIONS_OPTIONS).unwrap() as *const LV2_Options_Option;
            let (mut max, mut rate) = (None, None);
            while (*o).key != 0 {
                if (*o).key == h.u.max_block {
                    max = Some(*((*o).value as *const i32));
                }
                if (*o).key == h.u.sample_rate {
                    rate = Some(*((*o).value as *const f32));
                }
                o = o.add(1);
            }
            assert_eq!((max, rate), (Some(1024), Some(44_100.0)));
            assert!(find(h.features(), uri::BUF_SIZE_BOUNDED_BLOCK_LENGTH).is_some());
            // No ring installed: scheduling fails cleanly.
            let s = find(h.features(), uri::WORKER_SCHEDULE).unwrap() as *const LV2_Worker_Schedule;
            let data = 7u32;
            assert_eq!(
                ((*s).schedule_work.unwrap())((*s).handle, 4, (&raw const data).cast()),
                LV2_WORKER_ERR_UNKNOWN
            );
        }
        assert_eq!(
            h.urid.unmap(h.u.atom_int).as_deref(),
            Some("http://lv2plug.in/ns/ext/atom#Int")
        );
    }
}
