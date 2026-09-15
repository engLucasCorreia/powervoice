//! The COM objects PowerVoice presents to a VST3 plugin, and the state it shares between the
//! sandbox's main thread (controller calls) and audio thread (processor calls).

use std::cell::UnsafeCell;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use vst3::Steinberg::Vst::RestartFlags_::{
    kIoChanged, kLatencyChanged, kParamValuesChanged, kReloadComponent,
};
use vst3::Steinberg::Vst::{
    IAttributeList, IAttributeListTrait, IComponentHandler, IComponentHandlerTrait,
    IHostApplication, IHostApplicationTrait, IMessage, IMessageTrait, IParamValueQueue,
    IParamValueQueueTrait, IParameterChanges, IParameterChangesTrait, ParamID, ParamValue,
    String128, TChar,
};
use vst3::Steinberg::{
    FIDString, TUID, int32, int64, kInvalidArgument, kNoInterface, kResultFalse, kResultOk,
    tresult, uint32,
};
use vst3::{Class, ComWrapper, Interface};

use super::params::Domain;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Copies `src` into a UTF-16 string buffer (NUL-terminated, truncated to fit).
pub(crate) fn write_wstring(src: &str, dst: &mut [TChar]) {
    let mut n = 0;
    let room = dst.len().saturating_sub(1);
    for (d, s) in dst.iter_mut().take(room).zip(src.encode_utf16()) {
        *d = s as TChar;
        n += 1;
    }
    if let Some(end) = dst.get_mut(n) {
        *end = 0;
    }
}

/// A UTF-16 string buffer up to its NUL.
pub(crate) fn read_wstring(src: &[TChar]) -> String {
    let len = src.iter().position(|c| *c == 0).unwrap_or(src.len());
    let units: Vec<u16> = src[..len].iter().map(|c| *c as u16).collect();
    String::from_utf16_lossy(&units)
}

// --- Shared main ↔ audio state ---------------------------------------------------------------

/// One parameter's crossing points between the threads (lock-free).
pub(crate) struct Slot {
    pub(crate) id: u32,
    pub(crate) domain: Domain,
    /// Audio → main: the last normalized value the processor got or reported, for
    /// `IEditController::setParamNormalized` (keeps the controller in sync).
    to_ctrl: AtomicU64,
    to_ctrl_dirty: AtomicBool,
    /// Main → audio: a plugin-originated value (normalized) and its kind (0 none, 1 mirror
    /// only — `kParamValuesChanged`, 2 an edit the processor must also get — `performEdit`).
    edit: AtomicU64,
    edit_kind: AtomicU8,
    begin: AtomicBool,
    end: AtomicBool,
}

/// A plugin-originated change, as the audio thread forwards it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PluginEdit {
    Begin(u32),
    Value {
        id: u32,
        normalized: f64,
        /// `performEdit`: the processor gets it too (not just the host's mirror).
        deliver: bool,
    },
    End(u32),
}

/// What the plugin's controller and processor sides share with the host.
pub(crate) struct Shared {
    /// Sorted by id.
    slots: Vec<Slot>,
    to_ctrl_any: AtomicBool,
    edits_any: AtomicBool,
    /// `restartComponent` with latency / I/O / reload flags (→ `RESTART_REQUEST`).
    pub(crate) restart_requested: AtomicBool,
    /// `restartComponent(kParamValuesChanged)`: the main thread re-reads every value.
    pub(crate) values_changed: AtomicBool,
}

impl Shared {
    pub(crate) fn new(mut params: Vec<(u32, Domain)>) -> Self {
        params.sort_unstable_by_key(|p| p.0);
        params.dedup_by_key(|p| p.0);
        Self {
            slots: params
                .into_iter()
                .map(|(id, domain)| Slot {
                    id,
                    domain,
                    to_ctrl: AtomicU64::new(0),
                    to_ctrl_dirty: AtomicBool::new(false),
                    edit: AtomicU64::new(0),
                    edit_kind: AtomicU8::new(0),
                    begin: AtomicBool::new(false),
                    end: AtomicBool::new(false),
                })
                .collect(),
            to_ctrl_any: AtomicBool::new(false),
            edits_any: AtomicBool::new(false),
            restart_requested: AtomicBool::new(false),
            values_changed: AtomicBool::new(false),
        }
    }

    /// The slot of parameter `id` (RT-safe: a binary search).
    pub(crate) fn slot(&self, id: u32) -> Option<&Slot> {
        self.slots
            .binary_search_by_key(&id, |s| s.id)
            .ok()
            .map(|i| &self.slots[i])
    }

    /// \[audio\] The processor got or reported `normalized` for `slot`.
    pub(crate) fn note_to_controller(&self, slot: &Slot, normalized: f64) {
        slot.to_ctrl.store(normalized.to_bits(), Ordering::Release);
        slot.to_ctrl_dirty.store(true, Ordering::Release);
        self.to_ctrl_any.store(true, Ordering::Release);
    }

    /// \[main\] Every value noted since the last call.
    pub(crate) fn take_to_controller(&self, mut f: impl FnMut(u32, f64)) {
        if !self.to_ctrl_any.swap(false, Ordering::AcqRel) {
            return;
        }
        for s in &self.slots {
            if s.to_ctrl_dirty.swap(false, Ordering::AcqRel) {
                f(s.id, f64::from_bits(s.to_ctrl.load(Ordering::Acquire)));
            }
        }
    }

    /// \[main\] A plugin-originated value (`deliver`: `performEdit`, the processor gets it too).
    pub(crate) fn plugin_edit(&self, id: u32, normalized: f64, deliver: bool) {
        let Some(s) = self.slot(id) else {
            return;
        };
        s.edit.store(normalized.to_bits(), Ordering::Release);
        s.edit_kind
            .fetch_max(if deliver { 2 } else { 1 }, Ordering::AcqRel);
        self.edits_any.store(true, Ordering::Release);
    }

    /// \[main\] `beginEdit` / `endEdit`.
    pub(crate) fn gesture(&self, id: u32, begin: bool) {
        let Some(s) = self.slot(id) else {
            return;
        };
        if begin { &s.begin } else { &s.end }.store(true, Ordering::Release);
        self.edits_any.store(true, Ordering::Release);
    }

    /// \[audio\] Every plugin-originated change since the last call, per parameter in the order
    /// begin, value, end (no allocation).
    pub(crate) fn take_plugin_edits(&self, mut f: impl FnMut(PluginEdit)) {
        if !self.edits_any.swap(false, Ordering::AcqRel) {
            return;
        }
        for s in &self.slots {
            if s.begin.swap(false, Ordering::AcqRel) {
                f(PluginEdit::Begin(s.id));
            }
            match s.edit_kind.swap(0, Ordering::AcqRel) {
                0 => {}
                kind => f(PluginEdit::Value {
                    id: s.id,
                    normalized: f64::from_bits(s.edit.load(Ordering::Acquire)),
                    deliver: kind == 2,
                }),
            }
            if s.end.swap(false, Ordering::AcqRel) {
                f(PluginEdit::End(s.id));
            }
        }
    }
}

// --- IComponentHandler -----------------------------------------------------------------------

/// The controller's `IComponentHandler`: edits and restart requests go to [`Shared`].
pub(crate) struct ComponentHandler {
    pub(crate) shared: Arc<Shared>,
}

impl Class for ComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for ComponentHandler {
    unsafe fn beginEdit(&self, id: ParamID) -> tresult {
        self.shared.gesture(id, true);
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, value: ParamValue) -> tresult {
        if !value.is_finite() {
            return kInvalidArgument;
        }
        self.shared.plugin_edit(id, value.clamp(0.0, 1.0), true);
        kResultOk
    }

    unsafe fn endEdit(&self, id: ParamID) -> tresult {
        self.shared.gesture(id, false);
        kResultOk
    }

    unsafe fn restartComponent(&self, flags: int32) -> tresult {
        let restart = (kLatencyChanged | kIoChanged | kReloadComponent) as int32;
        if flags & restart != 0 {
            self.shared.restart_requested.store(true, Ordering::Release);
        }
        if flags & kParamValuesChanged as int32 != 0 {
            self.shared.values_changed.store(true, Ordering::Release);
        }
        kResultOk
    }
}

// --- IHostApplication, IMessage, IAttributeList ----------------------------------------------

/// The host context every component and controller is initialised with.
pub(crate) struct HostApplication;

impl Class for HostApplication {
    type Interfaces = (IHostApplication,);
}

/// # Safety
/// `p` is null or points at 16 bytes.
unsafe fn is_iid<I: Interface>(p: *const TUID) -> bool {
    // SAFETY: caller's contract.
    !p.is_null() && unsafe { std::slice::from_raw_parts(p.cast::<u8>(), 16) } == I::IID
}

impl IHostApplicationTrait for HostApplication {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: the plugin's writable String128.
        write_wstring("PowerVoice", unsafe { &mut *name });
        kResultOk
    }

    unsafe fn createInstance(
        &self,
        cid: *mut TUID,
        iid: *mut TUID,
        obj: *mut *mut c_void,
    ) -> tresult {
        if obj.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: class and interface ids are 16 bytes (or null).
        let created = unsafe {
            if is_iid::<IMessage>(cid) && is_iid::<IMessage>(iid) {
                ComWrapper::new(HostMessage::default())
                    .to_com_ptr::<IMessage>()
                    .map(|p| p.into_raw().cast::<c_void>())
            } else if is_iid::<IAttributeList>(cid) && is_iid::<IAttributeList>(iid) {
                ComWrapper::new(AttributeList::default())
                    .to_com_ptr::<IAttributeList>()
                    .map(|p| p.into_raw().cast::<c_void>())
            } else {
                None
            }
        };
        // SAFETY: writable per the contract; the reference goes to the caller.
        unsafe { *obj = created.unwrap_or(std::ptr::null_mut()) };
        if created.is_some() {
            kResultOk
        } else {
            kNoInterface
        }
    }
}

/// A message a component and its controller exchange (allocated by the plugin through
/// [`HostApplication`]).
#[derive(Default)]
pub(crate) struct HostMessage {
    id: Mutex<Option<CString>>,
    attributes: Mutex<Option<ComWrapper<AttributeList>>>,
}

impl Class for HostMessage {
    type Interfaces = (IMessage,);
}

impl IMessageTrait for HostMessage {
    unsafe fn getMessageID(&self) -> FIDString {
        // The pointer stays valid until the next `setMessageID` (the string's heap buffer).
        lock(&self.id)
            .as_ref()
            .map_or(std::ptr::null(), |s| s.as_ptr())
    }

    unsafe fn setMessageID(&self, id: FIDString) {
        // SAFETY: a NUL-terminated id (or null) from the plugin.
        *lock(&self.id) = (!id.is_null()).then(|| unsafe { CStr::from_ptr(id) }.to_owned());
    }

    unsafe fn getAttributes(&self) -> *mut IAttributeList {
        // Owned by the message (no reference added, as in the SDK's own host classes).
        lock(&self.attributes)
            .get_or_insert_with(|| ComWrapper::new(AttributeList::default()))
            .as_com_ref::<IAttributeList>()
            .map_or(std::ptr::null_mut(), |r| r.as_ptr())
    }
}

enum Attr {
    Int(i64),
    Float(f64),
    Str(Vec<TChar>),
    Bin(Vec<u8>),
}

/// A message's attributes.
#[derive(Default)]
pub(crate) struct AttributeList {
    map: Mutex<Vec<(CString, Attr)>>,
}

impl Class for AttributeList {
    type Interfaces = (IAttributeList,);
}

impl AttributeList {
    /// # Safety
    /// `id` is a NUL-terminated string or null.
    unsafe fn set(&self, id: *const c_char, value: Attr) -> tresult {
        if id.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: caller's contract.
        let key = unsafe { CStr::from_ptr(id) }.to_owned();
        let mut map = lock(&self.map);
        match map.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => map.push((key, value)),
        }
        kResultOk
    }

    /// # Safety
    /// `id` is a NUL-terminated string or null.
    unsafe fn with<R>(&self, id: *const c_char, f: impl FnOnce(&Attr) -> Option<R>) -> Option<R> {
        if id.is_null() {
            return None;
        }
        // SAFETY: caller's contract.
        let key = unsafe { CStr::from_ptr(id) };
        let map = lock(&self.map);
        map.iter()
            .find(|(k, _)| k.as_c_str() == key)
            .and_then(|(_, v)| f(v))
    }
}

impl IAttributeListTrait for AttributeList {
    unsafe fn setInt(&self, id: *const c_char, value: int64) -> tresult {
        // SAFETY: forwarded contract.
        unsafe { self.set(id, Attr::Int(value)) }
    }

    unsafe fn getInt(&self, id: *const c_char, value: *mut int64) -> tresult {
        // SAFETY: forwarded contract.
        match unsafe { self.with(id, |a| if let Attr::Int(v) = a { Some(*v) } else { None }) } {
            Some(v) if !value.is_null() => {
                // SAFETY: writable per the contract.
                unsafe { *value = v };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setFloat(&self, id: *const c_char, value: f64) -> tresult {
        // SAFETY: forwarded contract.
        unsafe { self.set(id, Attr::Float(value)) }
    }

    unsafe fn getFloat(&self, id: *const c_char, value: *mut f64) -> tresult {
        // SAFETY: forwarded contract.
        match unsafe {
            self.with(id, |a| {
                if let Attr::Float(v) = a {
                    Some(*v)
                } else {
                    None
                }
            })
        } {
            Some(v) if !value.is_null() => {
                // SAFETY: writable per the contract.
                unsafe { *value = v };
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn setString(&self, id: *const c_char, string: *const TChar) -> tresult {
        if string.is_null() {
            return kInvalidArgument;
        }
        let mut len = 0;
        // SAFETY: a NUL-terminated UTF-16 string from the plugin.
        while unsafe { *string.add(len) } != 0 {
            len += 1;
        }
        // SAFETY: `len` characters were just read.
        let s = unsafe { std::slice::from_raw_parts(string, len) }.to_vec();
        // SAFETY: forwarded contract.
        unsafe { self.set(id, Attr::Str(s)) }
    }

    unsafe fn getString(
        &self,
        id: *const c_char,
        string: *mut TChar,
        size_in_bytes: uint32,
    ) -> tresult {
        let capacity = size_in_bytes as usize / std::mem::size_of::<TChar>();
        if string.is_null() || capacity == 0 {
            return kInvalidArgument;
        }
        // SAFETY: forwarded contract.
        let copied = unsafe {
            self.with(id, |a| {
                let Attr::Str(s) = a else {
                    return None;
                };
                let n = s.len().min(capacity - 1);
                // SAFETY: the plugin's buffer holds `capacity` characters.
                std::ptr::copy_nonoverlapping(s.as_ptr(), string, n);
                *string.add(n) = 0;
                Some(())
            })
        };
        if copied.is_some() {
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn setBinary(
        &self,
        id: *const c_char,
        data: *const c_void,
        size_in_bytes: uint32,
    ) -> tresult {
        if data.is_null() && size_in_bytes > 0 {
            return kInvalidArgument;
        }
        let bytes = if size_in_bytes == 0 {
            Vec::new()
        } else {
            // SAFETY: the plugin's buffer of `size_in_bytes` bytes.
            unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size_in_bytes as usize) }
                .to_vec()
        };
        // SAFETY: forwarded contract.
        unsafe { self.set(id, Attr::Bin(bytes)) }
    }

    unsafe fn getBinary(
        &self,
        id: *const c_char,
        data: *mut *const c_void,
        size_in_bytes: *mut uint32,
    ) -> tresult {
        if data.is_null() || size_in_bytes.is_null() {
            return kInvalidArgument;
        }
        // The pointer stays valid while the attribute isn't replaced (it points into the
        // stored buffer, as in the SDK's own host classes).
        // SAFETY: forwarded contract.
        let found = unsafe {
            self.with(id, |a| match a {
                Attr::Bin(b) => Some((b.as_ptr().cast::<c_void>(), b.len() as uint32)),
                _ => None,
            })
        };
        match found {
            Some((p, n)) => {
                // SAFETY: writable per the contract.
                unsafe {
                    *data = p;
                    *size_in_bytes = n;
                }
                kResultOk
            }
            None => kResultFalse,
        }
    }
}

// --- IParameterChanges -----------------------------------------------------------------------

struct QueueData {
    id: u32,
    /// `(sample offset, normalized value)`, in offset order; capacity fixed at creation.
    points: Vec<(i32, f64)>,
}

/// One parameter's points of a block.
pub(crate) struct ParamQueue {
    data: UnsafeCell<QueueData>,
}

// SAFETY: a queue is touched by one thread at a time — the sandbox audio thread while it builds
// or reads a block's changes, and the plugin inside that thread's `process` call; the main
// thread only while the audio thread is stopped (the parameter flush at activation).
unsafe impl Sync for ParamQueue {}
// SAFETY: see `Sync`.
unsafe impl Send for ParamQueue {}

impl Class for ParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl ParamQueue {
    #[allow(clippy::mut_from_ref)]
    fn data(&self) -> &mut QueueData {
        // SAFETY: single-threaded use (see `Sync`), and no reference outlives a call.
        unsafe { &mut *self.data.get() }
    }
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.data().id
    }

    unsafe fn getPointCount(&self) -> int32 {
        self.data().points.len() as int32
    }

    unsafe fn getPoint(&self, index: int32, offset: *mut int32, value: *mut ParamValue) -> tresult {
        match usize::try_from(index)
            .ok()
            .and_then(|i| self.data().points.get(i))
        {
            Some(&(o, v)) if !offset.is_null() && !value.is_null() => {
                // SAFETY: writable per the contract.
                unsafe {
                    *offset = o;
                    *value = v;
                }
                kResultOk
            }
            _ => kResultFalse,
        }
    }

    unsafe fn addPoint(&self, offset: int32, value: ParamValue, index: *mut int32) -> tresult {
        let d = self.data();
        let i = d.points.partition_point(|p| p.0 < offset);
        if d.points.get(i).is_some_and(|p| p.0 == offset) {
            d.points[i].1 = value;
        } else if d.points.len() < d.points.capacity() {
            d.points.insert(i, (offset, value));
        } else {
            return kResultFalse;
        }
        if !index.is_null() {
            // SAFETY: writable per the contract.
            unsafe { *index = i as int32 };
        }
        kResultOk
    }
}

/// A block's parameter changes (input or output), preallocated: no allocation while in use.
pub(crate) struct ParamChanges {
    queues: Vec<ComWrapper<ParamQueue>>,
    ptrs: Vec<*mut IParamValueQueue>,
    used: UnsafeCell<usize>,
}

// SAFETY: as for `ParamQueue`: one thread at a time; the raw pointers point at the queues this
// value owns.
unsafe impl Sync for ParamChanges {}
// SAFETY: see `Sync`.
unsafe impl Send for ParamChanges {}

impl Class for ParamChanges {
    type Interfaces = (IParameterChanges,);
}

impl ParamChanges {
    /// `queues` parameters of up to `points` points each.
    pub(crate) fn new(queues: usize, points: usize) -> ComWrapper<Self> {
        let queues: Vec<ComWrapper<ParamQueue>> = (0..queues.max(1))
            .map(|_| {
                ComWrapper::new(ParamQueue {
                    data: UnsafeCell::new(QueueData {
                        id: 0,
                        points: Vec::with_capacity(points.max(1)),
                    }),
                })
            })
            .collect();
        let ptrs = queues
            .iter()
            .map(|q| {
                q.as_com_ref::<IParamValueQueue>()
                    .map_or(std::ptr::null_mut(), |r| r.as_ptr())
            })
            .collect();
        ComWrapper::new(Self {
            queues,
            ptrs,
            used: UnsafeCell::new(0),
        })
    }

    #[allow(clippy::mut_from_ref)]
    fn used(&self) -> &mut usize {
        // SAFETY: single-threaded use (see `Sync`), and no reference outlives a call.
        unsafe { &mut *self.used.get() }
    }

    /// The queue for `id`, starting one if needed (`None`: every queue is in use).
    fn queue_for(&self, id: u32) -> Option<usize> {
        let used = self.used();
        if let Some(i) = (0..*used).find(|&i| self.queues[i].data().id == id) {
            return Some(i);
        }
        if *used == self.queues.len() {
            return None;
        }
        let i = *used;
        let d = self.queues[i].data();
        d.id = id;
        d.points.clear();
        *used += 1;
        Some(i)
    }

    /// Empties the list (RT-safe).
    pub(crate) fn clear(&self) {
        *self.used() = 0;
    }

    /// Appends a point (offsets non-decreasing per parameter; an equal offset replaces the
    /// value). `false` when full (RT-safe, no allocation).
    pub(crate) fn push(&self, id: u32, offset: i32, value: f64) -> bool {
        let Some(q) = self.queue_for(id) else {
            return false;
        };
        let d = self.queues[q].data();
        if let Some(last) = d.points.last_mut()
            && last.0 == offset
        {
            last.1 = value;
            return true;
        }
        if d.points.len() < d.points.capacity() {
            d.points.push((offset, value));
            true
        } else {
            false
        }
    }

    /// Every point, queue by queue (RT-safe).
    pub(crate) fn for_each(&self, mut f: impl FnMut(u32, i32, f64)) {
        for q in &self.queues[..*self.used()] {
            let d = q.data();
            for &(offset, value) in &d.points {
                f(d.id, offset, value);
            }
        }
    }
}

/// The `IParameterChanges` pointer of `changes` (owned by it).
pub(crate) fn changes_ptr(changes: &ComWrapper<ParamChanges>) -> *mut IParameterChanges {
    changes
        .as_com_ref::<IParameterChanges>()
        .map_or(std::ptr::null_mut(), |r| r.as_ptr())
}

impl IParameterChangesTrait for ParamChanges {
    unsafe fn getParameterCount(&self) -> int32 {
        *self.used() as int32
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        match usize::try_from(index) {
            Ok(i) if i < *self.used() => self.ptrs[i],
            _ => std::ptr::null_mut(),
        }
    }

    unsafe fn addParameterData(
        &self,
        id: *const ParamID,
        index: *mut int32,
    ) -> *mut IParamValueQueue {
        if id.is_null() {
            return std::ptr::null_mut();
        }
        // SAFETY: readable per the contract.
        let id = unsafe { *id };
        let Some(q) = self.queue_for(id) else {
            return std::ptr::null_mut();
        };
        if !index.is_null() {
            // SAFETY: writable per the contract.
            unsafe { *index = q as int32 };
        }
        self.ptrs[q]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vst3::ComRef;

    #[test]
    fn parameter_changes_are_bounded_and_ordered() {
        let c = ParamChanges::new(2, 3);
        assert!(c.push(7, 0, 0.1));
        assert!(c.push(7, 5, 0.2));
        assert!(c.push(7, 5, 0.3), "same offset replaces");
        assert!(c.push(9, 1, 0.5));
        assert!(!c.push(11, 0, 0.5), "every queue in use");
        let mut seen = Vec::new();
        c.for_each(|id, o, v| seen.push((id, o, v)));
        assert_eq!(seen, vec![(7, 0, 0.1), (7, 5, 0.3), (9, 1, 0.5)]);
        // The plugin's view.
        // SAFETY: our own live object.
        let r = unsafe { ComRef::from_raw(changes_ptr(&c)) }.unwrap();
        // SAFETY: our own queues.
        unsafe {
            assert_eq!(r.getParameterCount(), 2);
            let q = ComRef::from_raw(r.getParameterData(1)).unwrap();
            assert_eq!((q.getParameterId(), q.getPointCount()), (9, 1));
            let mut idx = -1;
            assert_eq!(q.addPoint(0, 0.7, &mut idx), kResultOk);
            assert_eq!(idx, 0, "inserted before offset 1");
            assert_eq!(q.addPoint(2, 0.8, &mut idx), kResultOk);
            assert_eq!(q.addPoint(3, 0.9, &mut idx), kResultFalse, "full");
            assert!(r.getParameterData(2).is_null());
        }
        c.clear();
        let mut n = 0;
        c.for_each(|_, _, _| n += 1);
        assert_eq!(n, 0);
    }

    #[test]
    fn plugin_edits_cross_in_begin_value_end_order() {
        let s = Shared::new(vec![(3, Domain::Continuous), (1, Domain::Discrete(4))]);
        s.gesture(3, true);
        s.plugin_edit(3, 0.25, true);
        s.gesture(3, false);
        s.plugin_edit(1, 0.5, false);
        s.plugin_edit(99, 0.5, true);
        let mut got = Vec::new();
        s.take_plugin_edits(|e| got.push(e));
        assert_eq!(
            got,
            vec![
                PluginEdit::Value {
                    id: 1,
                    normalized: 0.5,
                    deliver: false
                },
                PluginEdit::Begin(3),
                PluginEdit::Value {
                    id: 3,
                    normalized: 0.25,
                    deliver: true
                },
                PluginEdit::End(3),
            ]
        );
        got.clear();
        s.take_plugin_edits(|e| got.push(e));
        assert!(got.is_empty());
        let slot = s.slot(3).unwrap();
        s.note_to_controller(slot, 0.75);
        let mut sync = Vec::new();
        s.take_to_controller(|id, v| sync.push((id, v)));
        assert_eq!(sync, vec![(3, 0.75)]);
    }

    #[test]
    fn messages_carry_attributes() {
        let host = ComWrapper::new(HostApplication);
        let app = host.as_com_ref::<IHostApplication>().unwrap();
        let mut obj: *mut c_void = std::ptr::null_mut();
        let iid = IMessage::IID;
        // SAFETY: our own objects; ids are 16 bytes; strings NUL-terminated.
        unsafe {
            let r = app.createInstance(
                iid.as_ptr() as *mut TUID,
                iid.as_ptr() as *mut TUID,
                &mut obj,
            );
            assert_eq!(r, kResultOk);
            let msg = vst3::ComPtr::from_raw(obj.cast::<IMessage>()).unwrap();
            msg.setMessageID(c"hello".as_ptr());
            assert_eq!(CStr::from_ptr(msg.getMessageID()), c"hello");
            let attrs = ComRef::from_raw(msg.getAttributes()).unwrap();
            attrs.setInt(c"n".as_ptr(), 42);
            let text: Vec<TChar> = "hé".encode_utf16().chain([0]).map(|c| c as TChar).collect();
            attrs.setString(c"s".as_ptr(), text.as_ptr());
            attrs.setBinary(c"b".as_ptr(), b"xyz".as_ptr().cast(), 3);
            let mut n = 0;
            assert_eq!(attrs.getInt(c"n".as_ptr(), &mut n), kResultOk);
            assert_eq!(n, 42);
            assert_eq!(attrs.getInt(c"s".as_ptr(), &mut n), kResultFalse);
            let mut buf = [0 as TChar; 8];
            assert_eq!(
                attrs.getString(c"s".as_ptr(), buf.as_mut_ptr(), 16),
                kResultOk
            );
            assert_eq!(read_wstring(&buf), "hé");
            let (mut p, mut len) = (std::ptr::null(), 0u32);
            assert_eq!(attrs.getBinary(c"b".as_ptr(), &mut p, &mut len), kResultOk);
            assert_eq!(
                std::slice::from_raw_parts(p.cast::<u8>(), len as usize),
                b"xyz"
            );
            let other = [0u8; 16];
            assert_eq!(
                app.createInstance(
                    other.as_ptr() as *mut TUID,
                    other.as_ptr() as *mut TUID,
                    &mut obj
                ),
                kNoInterface
            );
            assert!(obj.is_null());
        }
    }
}
