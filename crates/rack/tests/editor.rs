//! T-901 (rack side): a module whose plugin has its own editor window — one answering the
//! host-internal `PluginEditor` extension — shows it on its slot; what the window changes
//! reaches the mirror (parameters) and the committed blob (GUI-only state); a save can capture
//! the plugin's state first; the window closes with its slot and the rack, and follows the slot
//! through a plugin-requested restart (not through a crash, whose window died with its process).
//! In-process stand-in for the sandbox proxy (the real one is exercised by `powervoice-sandbox`'s
//! process tests).

vox_module_api::install_test_allocator!();

mod common;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Driver, SR};
use vox_module_api::{
    ActivateConfig, EditorRequest, EditorUpdate, Extension, ExtensionId, HostRequest,
    LocalizedText, MODULE_API_VERSION, Module, ModuleDescriptor, ModuleError, ModuleFactory,
    ModuleRef, ModuleState, ParamFlags, ParamId, ParamInfo, PluginEditor, ProcessContext,
    ProcessStatus, StateError, Tail, Taper, Unit, Version,
};
use vox_rack::{RackError, RackModel, RackNotice, Registry, SlotModel};

const LEVEL: ParamId = ParamId(0);
const ID: &str = "test:windowed";

/// One instance's window, as the test sees it.
#[derive(Default)]
struct Window {
    open: AtomicBool,
    opens: AtomicU32,
    closes: AtomicU32,
    request: Mutex<Option<EditorRequest>>,
    /// What the next `poll` reports.
    params: Mutex<Vec<(ParamId, f64)>>,
    state: Mutex<Option<Vec<u8>>>,
    /// What `capture_state` answers.
    captured: Mutex<Option<Vec<u8>>>,
    /// The instance's process "died": the window is gone and can't reopen.
    dead: AtomicBool,
}

struct Editor(Arc<Window>);

impl PluginEditor for Editor {
    fn available(&self) -> bool {
        true
    }
    fn open(&self, request: &EditorRequest) -> Result<(), String> {
        if self.0.dead.load(Ordering::Acquire) {
            return Err("the plugin isn't running".into());
        }
        *self.0.request.lock().unwrap() = Some(request.clone());
        self.0.opens.fetch_add(1, Ordering::AcqRel);
        self.0.open.store(true, Ordering::Release);
        Ok(())
    }
    fn close(&self) {
        if self.0.open.swap(false, Ordering::AcqRel) {
            self.0.closes.fetch_add(1, Ordering::AcqRel);
        }
    }
    fn is_open(&self) -> bool {
        self.0.open.load(Ordering::Acquire) && !self.0.dead.load(Ordering::Acquire)
    }
    fn poll(&self) -> EditorUpdate {
        EditorUpdate {
            open: self.is_open(),
            params: std::mem::take(&mut *self.0.params.lock().unwrap()),
            state: self.0.state.lock().unwrap().take(),
        }
    }
    fn capture_state(&self) -> Option<Vec<u8>> {
        self.0.captured.lock().unwrap().clone()
    }
}

/// Shared by the factory and the test: every instance's window, and a restart request switch.
#[derive(Default)]
struct Shared {
    windows: Mutex<Vec<Arc<Window>>>,
    request_restart: AtomicBool,
    /// Instances with an editor (else: a plain module, `has_editor` false).
    with_editor: bool,
}

impl Shared {
    fn window(&self, generation: usize) -> Arc<Window> {
        self.windows.lock().unwrap()[generation].clone()
    }
    fn latest(&self) -> Arc<Window> {
        self.windows.lock().unwrap().last().unwrap().clone()
    }
    fn count(&self) -> usize {
        self.windows.lock().unwrap().len()
    }
}

struct Windowed {
    desc: ModuleDescriptor,
    params: Vec<ParamInfo>,
    level: f64,
    blob: Option<Vec<u8>>,
    window: Arc<Window>,
    shared: Arc<Shared>,
}

impl Module for Windowed {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.desc
    }
    fn params(&self) -> &[ParamInfo] {
        &self.params
    }
    fn activate(&mut self, _: &ActivateConfig) -> Result<(), ModuleError> {
        Ok(())
    }
    fn deactivate(&mut self) {}
    fn latency_samples(&self) -> u32 {
        0
    }
    fn tail(&self) -> Tail {
        Tail::Samples(0)
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus {
        for e in ctx.events {
            if e.id == LEVEL {
                self.level = e.value;
            }
        }
        for (o, i) in outputs[0].iter_mut().zip(inputs[0]) {
            *o = *i * self.level as f32;
        }
        if self.shared.request_restart.swap(false, Ordering::AcqRel) {
            ctx.request(HostRequest::Restart);
        }
        ProcessStatus::Continue
    }
    fn reset(&mut self) {}
    fn param_value(&self, id: ParamId) -> Option<f64> {
        (id == LEVEL).then_some(self.level)
    }
    fn save_state(&self) -> Result<ModuleState, StateError> {
        let mut s = ModuleState::new(1);
        s.params.insert("level".into(), self.level);
        s.blob = self.blob.clone();
        Ok(s)
    }
    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError> {
        self.level = state.params.get("level").copied().unwrap_or(1.0);
        if state.blob.is_some() {
            self.blob = state.blob.clone();
        }
        Ok(())
    }
    fn extension(&self, id: ExtensionId) -> Option<Extension> {
        match id {
            ExtensionId::PluginEditor if self.shared.with_editor => Some(Extension::PluginEditor(
                Arc::new(Editor(self.window.clone())),
            )),
            _ => None,
        }
    }
}

struct Factory {
    desc: ModuleDescriptor,
    shared: Arc<Shared>,
}

impl ModuleFactory for Factory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.desc
    }
    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        let window = Arc::new(Window::default());
        self.shared.windows.lock().unwrap().push(window.clone());
        Ok(Box::new(Windowed {
            desc: self.desc.clone(),
            params: vec![ParamInfo {
                id: LEVEL,
                key: "level".into(),
                name: LocalizedText::plain("Level"),
                group: None,
                unit: Unit::None,
                min: 0.0,
                max: 2.0,
                default: 1.0,
                taper: Taper::Linear,
                step: None,
                enum_labels: Vec::new(),
                decimals: 2,
                smoothing_ms: 0.0,
                flags: ParamFlags::AUTOMATABLE,
            }],
            level: 1.0,
            blob: Some(b"initial".to_vec()),
            window,
            shared: self.shared.clone(),
        }))
    }
}

fn descriptor() -> ModuleDescriptor {
    ModuleDescriptor {
        id: ID.into(),
        version: Version::new(1, 0, 0),
        name: LocalizedText::plain("Windowed"),
        vendor: "test".into(),
        description: LocalizedText::plain(""),
        url: None,
        features: Vec::new(),
        state_format_version: 1,
        api_version: MODULE_API_VERSION,
    }
}

fn slot() -> SlotModel {
    let mut state = ModuleState::new(1);
    state.params.insert("level".into(), 0.5);
    let r: ModuleRef = format!("{ID}@1.0.0").parse().unwrap();
    SlotModel::new(&r, false, &state)
}

fn rig_with(with_editor: bool, slots: usize) -> (Driver, Arc<Shared>, Vec<f32>) {
    let shared = Arc::new(Shared {
        with_editor,
        ..Shared::default()
    });
    let registry = Registry::with_factories([Arc::new(Factory {
        desc: descriptor(),
        shared: shared.clone(),
    }) as Arc<dyn ModuleFactory>])
    .unwrap();
    let model = RackModel {
        slots: (0..slots).map(|_| slot()).collect(),
    };
    let len = (SR * 2.0) as usize;
    let input: Vec<f32> = (0..len).map(|i| ((i % 97) as f32 / 97.0) - 0.5).collect();
    (
        Driver::with_registry(Arc::new(registry), &model, 11, len),
        shared,
        input,
    )
}

fn state_of(s: &SlotModel) -> ModuleState {
    serde_json::from_value(s.state.clone()).unwrap()
}

fn rig() -> (Driver, Arc<Shared>, Vec<f32>) {
    rig_with(true, 1)
}

fn request() -> EditorRequest {
    EditorRequest {
        title: "Windowed — PowerVoice".into(),
        parent: Some(42),
    }
}

/// Opens slot `index`'s window the way the engine does: take the handle on the control thread,
/// open it off it.
fn open(d: &mut Driver, index: usize) {
    let editor = d.host.editor_for_open(index, request()).unwrap();
    editor.open(&request()).unwrap();
    d.tick();
}

fn editor_notices(d: &Driver) -> Vec<(usize, bool)> {
    d.notices
        .iter()
        .filter_map(|(_, n)| match n {
            RackNotice::EditorChanged { index, open, .. } => Some((*index, *open)),
            _ => None,
        })
        .collect()
}

fn wait_until(mut f: impl FnMut() -> bool) -> bool {
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_secs(5) {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    f()
}

#[test]
fn a_slot_shows_whether_its_plugin_has_a_window_and_whether_it_is_open() {
    let (mut d, shared, _) = rig();
    let info = d.host.slot_info(0).unwrap();
    assert!(info.has_editor && !info.editor_open);
    open(&mut d, 0);
    assert!(d.host.slot_info(0).unwrap().editor_open);
    assert_eq!(editor_notices(&d), vec![(0, true)]);
    assert_eq!(
        shared.latest().request.lock().unwrap().as_ref(),
        Some(&request())
    );

    // The user closes the window: the next tick reports it.
    shared.latest().open.store(false, Ordering::Release);
    d.tick();
    assert!(!d.host.slot_info(0).unwrap().editor_open);
    assert_eq!(editor_notices(&d), vec![(0, true), (0, false)]);

    // Closing from the host.
    open(&mut d, 0);
    d.host.close_editor(0).unwrap();
    assert!(!d.host.slot_info(0).unwrap().editor_open);
    assert_eq!(shared.latest().closes.load(Ordering::Acquire), 1);
    assert_eq!(editor_notices(&d).last(), Some(&(0, true)));
    assert!(
        d.host
            .take_notices()
            .iter()
            .any(|n| matches!(n, RackNotice::EditorChanged { open: false, .. }))
    );
}

#[test]
fn a_module_without_a_window_offers_none() {
    let (mut d, _, _) = rig_with(false, 1);
    let info = d.host.slot_info(0).unwrap();
    assert!(!info.has_editor && !info.editor_open);
    match d.host.editor_for_open(0, request()) {
        Err(RackError::NoEditor { name }) => assert_eq!(name, "Windowed"),
        Err(other) => panic!("expected NoEditor, got {other:?}"),
        Ok(_) => panic!("expected NoEditor, got a handle"),
    }
    assert!(matches!(
        d.host.editor_for_open(3, request()),
        Err(RackError::IndexOutOfRange { index: 3, len: 1 })
    ));
    assert!(d.host.editors_to_capture().is_empty());
}

#[test]
fn window_parameter_changes_reach_the_mirror_and_the_model() {
    let (mut d, shared, _) = rig();
    open(&mut d, 0);
    // Out of range values are clamped, like any other edit.
    *shared.latest().params.lock().unwrap() = vec![(LEVEL, 1.25), (LEVEL, 7.0)];
    d.tick();
    assert_eq!(d.host.param_value(0, LEVEL), Some(2.0));
    let texts: Vec<f64> = d
        .notices
        .iter()
        .filter_map(|(_, n)| match n {
            RackNotice::ParamChanged { id, value, .. } if *id == LEVEL => Some(*value),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec![2.0], "coalesced: the newest value wins");
    assert_eq!(
        state_of(&d.host.model().slots[0]).params["level"].to_bits(),
        2.0f64.to_bits()
    );
}

#[test]
fn a_gui_only_state_change_refreshes_the_committed_blob() {
    let (mut d, shared, _) = rig();
    assert_eq!(
        state_of(&d.host.model().slots[0]).blob.as_deref(),
        Some(&b"initial"[..])
    );
    open(&mut d, 0);
    *shared.latest().state.lock().unwrap() = Some(b"gui-only".to_vec());
    d.tick();
    assert_eq!(
        state_of(&d.host.model().slots[0]).blob.as_deref(),
        Some(&b"gui-only"[..])
    );
    assert!(
        d.notices
            .iter()
            .any(|(_, n)| matches!(n, RackNotice::PluginStateChanged { index: 0, .. }))
    );
    // The same blob again changes nothing.
    let before = d.notices.len();
    *shared.latest().state.lock().unwrap() = Some(b"gui-only".to_vec());
    d.tick();
    assert!(
        !d.notices[before..]
            .iter()
            .any(|(_, n)| matches!(n, RackNotice::PluginStateChanged { .. }))
    );
}

#[test]
fn a_save_captures_the_state_of_every_plugin_whose_window_was_used() {
    let (mut d, shared, _) = rig_with(true, 2);
    assert!(
        d.host.editors_to_capture().is_empty(),
        "no window opened yet: nothing a GUI could have changed"
    );
    open(&mut d, 1);
    d.host.close_editor(1).unwrap();
    let to_capture = d.host.editors_to_capture();
    assert_eq!(
        to_capture.len(),
        1,
        "still captured after the window closed"
    );
    let (uid, editor) = &to_capture[0];
    assert_eq!(Some(*uid), d.host.slot_uid(1));
    *shared.window(1).captured.lock().unwrap() = Some(b"captured".to_vec());
    let blob = editor.capture_state().unwrap();
    d.host.apply_plugin_state(*uid, blob);
    let model = d.host.model();
    assert_eq!(
        state_of(&model.slots[1]).blob.as_deref(),
        Some(&b"captured"[..])
    );
    assert_eq!(
        state_of(&model.slots[0]).blob.as_deref(),
        Some(&b"initial"[..])
    );
    assert!(
        d.host
            .take_notices()
            .iter()
            .any(|n| matches!(n, RackNotice::PluginStateChanged { index: 1, .. }))
    );
    // A removed slot's late capture is ignored.
    d.host.remove(1).unwrap();
    d.host.apply_plugin_state(*uid, b"late".to_vec());
    assert_eq!(d.host.len(), 1);
}

#[test]
fn windows_close_with_their_slot_and_with_the_rack() {
    let (mut d, shared, _) = rig_with(true, 3);
    open(&mut d, 0);
    open(&mut d, 1);
    open(&mut d, 2);
    d.host.remove(1).unwrap();
    assert!(
        !shared.window(1).open.load(Ordering::Acquire),
        "removed slot"
    );
    assert!(shared.window(0).open.load(Ordering::Acquire));
    d.host.close_all_editors();
    assert!(!shared.window(0).open.load(Ordering::Acquire));
    assert!(!shared.window(2).open.load(Ordering::Acquire));

    open(&mut d, 0);
    // Loading another rack (document open) removes every slot, and every window with it.
    d.host.load_model(&RackModel::default()).unwrap();
    assert!(!shared.window(0).open.load(Ordering::Acquire));

    let (mut d, shared, _) = rig();
    open(&mut d, 0);
    let Driver { host, live, .. } = d;
    host.teardown(live);
    assert!(
        !shared.latest().open.load(Ordering::Acquire),
        "rack teardown"
    );
}

#[test]
fn a_plugin_requested_restart_reopens_the_window_on_the_new_instance() {
    let (mut d, shared, input) = rig();
    open(&mut d, 0);
    assert_eq!(shared.count(), 1);
    shared.request_restart.store(true, Ordering::Release);
    d.run_until(&input, 9600);
    d.tick();
    assert_eq!(shared.count(), 2, "restarted");
    let new = shared.window(1);
    assert!(
        wait_until(|| new.opens.load(Ordering::Acquire) == 1),
        "the new instance's window opened"
    );
    assert_eq!(new.request.lock().unwrap().as_ref(), Some(&request()));
    assert!(
        !shared.window(0).open.load(Ordering::Acquire),
        "old window closed"
    );
    d.tick();
    assert!(d.host.slot_info(0).unwrap().editor_open);
}

#[test]
fn a_moved_slot_keeps_its_window() {
    let (mut d, shared, _) = rig_with(true, 2);
    open(&mut d, 0);
    d.host.move_slot(0, 1).unwrap();
    assert_eq!(shared.count(), 3, "the moved module restarted");
    assert!(!shared.window(0).open.load(Ordering::Acquire));
    let moved = shared.window(2);
    assert!(wait_until(|| moved.opens.load(Ordering::Acquire) == 1));
    d.tick();
    assert!(d.host.slot_info(1).unwrap().editor_open);
    assert!(!d.host.slot_info(0).unwrap().editor_open);
}

#[test]
fn a_crashed_instance_is_restarted_without_its_window() {
    let (mut d, shared, input) = rig();
    open(&mut d, 0);
    // The process died (window gone), then the plugin is replaced (here: a restart request
    // stands in for the rack's restart policy, which only the sandbox proxy triggers).
    shared.latest().dead.store(true, Ordering::Release);
    d.tick();
    assert!(!d.host.slot_info(0).unwrap().editor_open);
    shared.request_restart.store(true, Ordering::Release);
    d.run_until(&input, 9600);
    d.tick();
    assert_eq!(shared.count(), 2);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        shared.window(1).opens.load(Ordering::Acquire),
        0,
        "a crash's window isn't reopened"
    );
}
