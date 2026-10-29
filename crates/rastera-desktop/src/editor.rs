//! Desktop Live Edit interaction. Document/rendering live in rastera-core.
#[path = "editor_mapping.rs"]
mod mapping;
#[path = "editor_probe.rs"]
mod probe;
use super::{log, Consumer};
use eframe::egui;
use mapping::Mode;
use rastera_core::{
    composition::SceneEngine,
    scene::{self, Alignment, Content, Document, History, Id, PreviewMap, Rect},
};
use std::{
    ffi::{c_char, CStr},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};
#[repr(C)]
#[derive(Default)]
struct Pointer {
    kind: u32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    scale: f32,
    modifiers: u64,
}
const _: [(); 32] = [(); std::mem::size_of::<Pointer>()];
unsafe extern "C" {
    fn rastera_editor_pointer(handle: *mut std::ffi::c_void, out: *mut Pointer) -> bool;
    fn rastera_editor_layout(
        handle: *mut std::ffi::c_void,
        rect: *const f32,
        screen: *const f32,
        selection: *const f32,
        flags: u32,
    );
    fn rastera_editor_dialog(kind: i32) -> *mut c_char;
    fn rastera_editor_free_string(value: *mut c_char);
}
fn dialog(kind: i32) -> Option<PathBuf> {
    // SAFETY: called on eframe main. Panel owns UI; the returned allocation is freed once.
    unsafe {
        let value = rastera_editor_dialog(kind);
        if value.is_null() {
            return None;
        }
        let path = PathBuf::from(CStr::from_ptr(value).to_string_lossy().into_owned());
        rastera_editor_free_string(value);
        Some(path)
    }
}
enum FileJob {
    Import(PathBuf),
    Load(PathBuf),
    Save(PathBuf, Box<Document>),
}
enum FileResult {
    Imported(scene::ImageAsset),
    Loaded(PathBuf, Document),
    Saved(PathBuf, Document),
}
struct Files {
    tx: mpsc::SyncSender<FileJob>,
    rx: mpsc::Receiver<Result<FileResult, String>>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Files {
    fn new() -> Self {
        let (tx, requests) = mpsc::sync_channel(1);
        let (responses, rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let job = match requests.recv_timeout(Duration::from_millis(50)) {
                    Ok(j) => j,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => break,
                };
                let result = match job {
                    FileJob::Import(path) => {
                        scene::inspect_decoded_image(&path).map(FileResult::Imported)
                    }
                    FileJob::Load(path) => {
                        scene::load_project(&path).map(|doc| FileResult::Loaded(path, doc))
                    }
                    FileJob::Save(path, doc) => {
                        scene::save_project(&doc, &path).map(|()| FileResult::Saved(path, *doc))
                    }
                };
                let _ = responses.try_send(result);
            }
        });
        Self {
            tx,
            rx,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
struct Gesture {
    before: Document,
    id: Id,
    start: [f32; 2],
    rect: Rect,
    corner: Option<usize>,
    aspect: Option<f32>,
}
enum Action {
    AddText,
    Import,
    Open,
    Save(bool),
    Undo,
    Redo,
    Duplicate,
    Delete,
    Up,
    Down,
    Visible(Id),
    Lock(Id),
}
pub(super) struct Editor {
    pub engine: SceneEngine,
    pub(super) capture: super::capture::Capture,
    pub doc: Document,
    pub(super) destinations: super::destinations::Destinations,
    output_project_reload: bool,
    pub(super) output_disable_requested: bool,
    selected: Option<Id>,
    history: History,
    gesture: Option<Gesture>,
    inspector_before: Option<Document>,
    revision: u64,
    files: Files,
    busy_file: bool,
    pending_load: Option<(u64, PathBuf, Document)>,
    project: Option<PathBuf>,
    saved: Option<Document>,
    pub(super) error: Option<String>,
    probe: Option<probe::Probe>,
    library: Arc<rastera_core::fonts::FontLibrary>,
    mode: Mode,
    viewport_selected: Option<u64>,
    view_gesture: Option<mapping::ViewGesture>,
    viewport_size: Option<(u64, [u32; 2], [u32; 2])>,
    settings: Option<[u32; 2]>,
    settings_requested: bool,
    font_dialog: bool,
    font_search: String,
    device_limit: u32,
    pending_before: Option<Document>,
    pending_history: Option<History>,
    pending_ready: bool,
}
impl Editor {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let device_limit = device.limits().max_texture_dimension_2d;
        let library = mapping::library();
        let engine = SceneEngine::with_fonts(device, queue, library.clone());
        let capture = super::capture::Capture::new(engine.captures());
        let doc = Document::default();
        let revision = engine.submit(doc.clone());
        let mut editor = Self {
            engine,
            capture,
            destinations: super::destinations::Destinations::new(),
            output_project_reload: false,
            output_disable_requested: false,
            library,
            mode: Mode::Canvas,
            viewport_selected: Some(1),
            view_gesture: None,
            viewport_size: None,
            settings: None,
            settings_requested: false,
            font_dialog: false,
            font_search: String::new(),
            device_limit,
            pending_before: None,
            pending_history: None,
            pending_ready: false,
            doc,
            selected: None,
            history: History::default(),
            gesture: None,
            inspector_before: None,
            revision,
            files: Files::new(),
            busy_file: false,
            pending_load: None,
            project: None,
            saved: None,
            error: None,
            probe: std::env::args().find_map(|a| {
                a.strip_prefix("--editor-probe=")
                    .map(|p| probe::Probe::new(PathBuf::from(p)))
            }),
        };
        if let Some(path) =
            std::env::args().find_map(|a| a.strip_prefix("--project=").map(PathBuf::from))
        {
            editor.file_job(FileJob::Load(path));
        }
        editor
    }
    pub(super) fn busy_loading(&self) -> bool {
        self.busy_file || self.pending_load.is_some()
    }
    fn stage_document(&mut self, doc: Document, path: Option<PathBuf>) {
        self.pending_before = Some(self.doc.clone());
        self.pending_ready = false;
        let revision = self.engine.submit(doc.clone());
        self.revision = revision;
        self.pending_load = Some((revision, path.unwrap_or_default(), doc));
    }
    fn file_job(&mut self, job: FileJob) {
        let loading = matches!(&job, FileJob::Load(_));
        match self.files.tx.try_send(job) {
            Ok(()) => {
                self.busy_file = true;
                if loading {
                    self.output_disable_requested = true;
                }
            }
            Err(_) => self.error = Some("File worker is busy; retry after it completes".into()),
        }
    }
    fn publish(&mut self) {
        self.revision = self.engine.submit(self.doc.clone());
    }
    fn finish_inspector(&mut self) {
        if let Some(before) = self.inspector_before.take() {
            self.history.record(before, &self.doc);
        }
    }
    fn finish_gesture(&mut self) {
        if let Some(g) = self.gesture.take() {
            self.history.record(g.before, &self.doc);
            log(format!(
                "editor_gesture layer={} rect={:?} complete=1\n",
                g.id,
                self.doc.layer(g.id).map(|l| l.rect)
            ));
        }
    }
    fn poll(&mut self) {
        if let Ok(result) = self.files.rx.try_recv() {
            self.busy_file = false;
            match result {
                Err(e) => self.error = Some(e),
                Ok(FileResult::Imported(asset)) => {
                    self.finish_inspector();
                    let before = self.doc.clone();
                    self.selected = Some(self.doc.add_image(asset));
                    self.history.record(before, &self.doc);
                    self.publish();
                }
                Ok(FileResult::Loaded(path, mut doc)) => {
                    doc.next_viewport_id = doc.next_viewport_id.max(self.doc.next_viewport_id);
                    self.stage_document(doc, Some(path));
                }
                Ok(FileResult::Saved(path, doc)) => {
                    log(format!("editor_saved path={}\n", path.display()));
                    self.project = Some(path);
                    self.saved = Some(doc);
                }
            }
        }
        if let Some(status) = self.engine.status() {
            if status.revision == self.revision {
                match status.result {
                    Err(e) => {
                        self.error = Some(e);
                        if self.pending_load.take().is_some() {
                            self.pending_before = None;
                            self.pending_history = None;
                            self.pending_ready = false;
                            self.publish();
                        }
                    }
                    Ok(()) => {
                        self.pending_ready = true;
                    }
                }
            }
        }
        if self.pending_ready
            && self
                .pending_load
                .as_ref()
                .is_some_and(|(r, _, _)| self.engine.applied() >= *r)
        {
            let (_, path, doc) = self.pending_load.take().unwrap();
            let before = self.pending_before.take().unwrap();
            self.doc = doc;
            if !path.as_os_str().is_empty() {
                self.selected = None;
                self.history = History::default();
                self.saved = Some(self.doc.clone());
                self.project = Some(path.clone());
                self.output_project_reload = true;
                log(format!("editor_loaded path={}\n", path.display()));
            } else if let Some(history) = self.pending_history.take() {
                self.history = history;
            } else {
                self.history.record(before, &self.doc);
            }
            self.pending_ready = false;
        }
        if self
            .viewport_selected
            .is_some_and(|id| self.doc.viewport(id).is_none())
        {
            self.viewport_selected = None;
            self.view_gesture = None;
            self.viewport_size = None;
        }
        if self.selected.is_some_and(|id| self.doc.layer(id).is_none()) {
            self.selected = None;
        }
    }
    fn action(&mut self, action: Action) {
        self.finish_gesture();
        self.finish_view_gesture();
        self.finish_inspector();
        let before = self.doc.clone();
        let old_history = self.history.clone();
        match action {
            Action::Import => {
                if let Some(path) = dialog(0) {
                    self.file_job(FileJob::Import(path));
                }
            }
            Action::Open => {
                if let Some(path) = dialog(1) {
                    self.file_job(FileJob::Load(path));
                }
            }
            Action::Save(save_as) => {
                let path = if save_as {
                    dialog(2)
                } else {
                    self.project.clone().or_else(|| dialog(2))
                };
                if let Some(path) = path {
                    self.file_job(FileJob::Save(path, Box::new(self.doc.clone())));
                }
            }
            Action::Undo => {
                if self.history.undo(&mut self.doc) {
                    if self.doc.canvas != before.canvas {
                        let candidate = self.doc.clone();
                        let next = self.history.clone();
                        self.doc = before.clone();
                        self.history = old_history;
                        self.stage_document(candidate, None);
                        self.pending_history = Some(next);
                    } else {
                        self.publish();
                    }
                }
                return;
            }
            Action::Redo => {
                if self.history.redo(&mut self.doc) {
                    if self.doc.canvas != before.canvas {
                        let candidate = self.doc.clone();
                        let next = self.history.clone();
                        self.doc = before.clone();
                        self.history = old_history;
                        self.stage_document(candidate, None);
                        self.pending_history = Some(next);
                    } else {
                        self.publish();
                    }
                }
                return;
            }
            Action::AddText => self.selected = Some(self.doc.add_text()),
            Action::Visible(id) => {
                if let Some(l) = self.doc.layer_mut(id) {
                    l.visible = !l.visible;
                }
            }
            Action::Lock(id) => {
                if let Some(l) = self.doc.layer_mut(id) {
                    l.locked = !l.locked;
                }
            }
            Action::Duplicate => {
                if let Some(id) = self.selected {
                    self.selected = self.doc.duplicate(id).or(self.selected);
                }
            }
            Action::Delete => {
                if let Some(id) = self.selected {
                    if self.doc.layer(id).is_some_and(|l| !l.locked) {
                        self.doc.layers.retain(|l| l.id != id);
                        self.doc.prune_unused();
                        self.selected = None;
                    }
                }
            }
            Action::Up | Action::Down => {
                if let Some(id) = self.selected {
                    if let Some(i) = self.doc.layers.iter().position(|l| l.id == id && !l.locked) {
                        let next = if matches!(action, Action::Up) {
                            i + 1
                        } else {
                            i.saturating_sub(1)
                        };
                        if next < self.doc.layers.len() {
                            self.doc.layers.swap(i, next);
                        }
                    }
                }
            }
        }
        if before != self.doc {
            self.history.record(before, &self.doc);
            self.publish();
        }
    }
    pub fn probe_tick(
        &mut self,
        preview: &Consumer,
        outputs: &std::collections::BTreeMap<u64, Consumer>,
        enabled: &mut std::collections::BTreeSet<u64>,
    ) {
        if let Some(mut probe) = self.probe.take() {
            if !probe.tick(self, preview, outputs, enabled) {
                self.probe = Some(probe);
            }
        }
    }
    pub fn pointers(&mut self, preview: &Consumer, ctx: &egui::Context) {
        loop {
            let mut event = Pointer::default();
            // SAFETY: main-only native consumer, valid writable POD event.
            if !unsafe { rastera_editor_pointer(preview.handle.as_ptr(), &mut event) } {
                break;
            }
            if self.pending_load.is_some() || self.settings.is_some() || self.font_dialog {
                continue;
            }
            let map = PreviewMap::for_canvas(
                self.doc.canvas,
                [event.width, event.height],
                20. * event.scale,
            );
            let Some(p) = map.canvas([event.x, event.y], true) else {
                continue;
            };
            if self.mode == Mode::Viewports {
                self.view_pointer(&event, map, p);
                continue;
            }
            match event.kind {
                1 => {
                    self.finish_gesture();
                    self.finish_inspector();
                    ctx.memory_mut(|m| {
                        if let Some(id) = m.focused() {
                            m.surrender_focus(id);
                        }
                    });
                    let handle = self
                        .selected
                        .and_then(|id| self.doc.layer(id))
                        .filter(|l| !l.locked && l.visible)
                        .and_then(|l| {
                            map.handle(
                                l.rect,
                                p,
                                8. * event.scale * self.doc.canvas[0] as f32 / map.fit[2],
                            )
                            .map(|corner| (l.id, corner))
                        });
                    let (id, corner) = if let Some((id, corner)) = handle {
                        (Some(id), Some(corner))
                    } else {
                        (
                            map.canvas([event.x, event.y], false)
                                .and_then(|p| self.doc.hit_test(p)),
                            None,
                        )
                    };
                    self.selected = id;
                    if let Some(id) = id {
                        let rect = self.doc.layer(id).unwrap().rect;
                        self.gesture = Some(Gesture {
                            before: self.doc.clone(),
                            id,
                            start: p,
                            rect,
                            corner,
                            aspect: self.doc.aspect(id),
                        });
                    }
                }
                2 | 3 => {
                    if let Some(g) = &self.gesture {
                        let rect = if let Some(corner) = g.corner {
                            scene::resize_drag(g.rect, corner, g.start, p, g.aspect)
                        } else {
                            Rect {
                                x: (g.rect.x + (p[0] - g.start[0])).clamp(-32768., 32768.),
                                y: (g.rect.y + (p[1] - g.start[1])).clamp(-32768., 32768.),
                                ..g.rect
                            }
                        };
                        if let Some(layer) = self.doc.layer_mut(g.id) {
                            if layer.rect != rect {
                                layer.rect = rect;
                                self.publish();
                            }
                        }
                    }
                    if event.kind == 3 {
                        self.finish_gesture();
                    }
                }
                4 => {
                    if let Some(g) = self.gesture.take() {
                        self.doc = g.before;
                        self.publish();
                    }
                }
                _ => {}
            }
        }
    }
    pub fn native_layout(&self, preview: &Consumer, ctx: &egui::Context, rect: egui::Rect) {
        let area = [
            rect.left(),
            rect.top(),
            rect.width().max(1.),
            rect.height().max(1.),
        ];
        let screen = ctx.screen_rect().size();
        let screen = [screen.x, screen.y];
        let (selected, flags) = self
            .selected
            .and_then(|id| self.doc.layer(id))
            .filter(|l| l.visible)
            .map(|l| {
                (
                    [l.rect.x, l.rect.y, l.rect.width, l.rect.height],
                    1 | ((l.locked as u32) << 1),
                )
            })
            .unwrap_or(([0.; 4], 0));
        // SAFETY: main-only call copies the arrays; overlay is exclusive to preview.
        unsafe {
            rastera_editor_layout(
                preview.handle.as_ptr(),
                area.as_ptr(),
                screen.as_ptr(),
                selected.as_ptr(),
                flags,
            );
        }
        self.overlays(preview);
    }
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        outputs: &mut std::collections::BTreeSet<u64>,
    ) -> egui::Rect {
        self.poll();
        self.destinations.context(
            &self.doc.project_id,
            self.project.as_deref(),
            std::mem::take(&mut self.output_project_reload),
            outputs,
        );
        self.capture.reconcile(&self.doc);
        mapping::apply_theme(ctx);
        if let Some((target, source)) = self.capture.dialog_ui(ctx) {
            self.finish_gesture();
            self.finish_inspector();
            let before = self.doc.clone();
            if let Some(id) = target {
                if let Err(error) = self.doc.assign_capture(id, source) {
                    self.error = Some(error);
                }
            } else {
                self.change_mode(Mode::Canvas);
                match self.doc.add_capture(source) {
                    Ok(id) => self.selected = Some(id),
                    Err(e) => self.error = Some(e),
                }
            }
            self.history.record(before, &self.doc);
            self.publish();
        }
        let mut action = None;
        let loading = self.busy_file
            || self.pending_load.is_some()
            || self.settings.is_some()
            || self.font_dialog;
        egui::TopBottomPanel::top("editor_toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("Live Edit — changes affect outputs");
                if ui
                    .add_enabled(!loading, egui::Button::new("Add Capture Device…"))
                    .clicked()
                {
                    self.capture.target = None;
                    self.capture.request();
                }
                if ui.button("Disable All Outputs · ⌘⇧Esc").clicked() {
                    unsafe { super::rastera_outputs_disable_all() };
                    outputs.clear();
                }
                if let Some(error) = &self.destinations.error {
                    ui.colored_label(egui::Color32::YELLOW, error);
                }
                if ui
                    .add_enabled(!loading, egui::Button::new("Settings ▾"))
                    .clicked()
                {
                    self.settings_requested = true;
                }
                let mut mode = self.mode;
                ui.add_enabled_ui(!loading, |ui| {
                    ui.selectable_value(&mut mode, Mode::Canvas, "Canvas");
                    ui.selectable_value(&mut mode, Mode::Viewports, "Viewports");
                });
                self.change_mode(mode);
                ui.add_enabled_ui(!loading, |ui| {
                    for (label, a) in [
                        ("Import image…", Action::Import),
                        ("Add text", Action::AddText),
                        ("Open…", Action::Open),
                        ("Save", Action::Save(false)),
                        ("Save as…", Action::Save(true)),
                        ("Undo", Action::Undo),
                        ("Redo", Action::Redo),
                    ] {
                        if ui.button(label).clicked() {
                            action = Some(a);
                        }
                    }
                });
            });
            ui.horizontal(|ui| {
                let mut test = self.doc.test_background;
                if ui
                    .add_enabled(
                        !loading,
                        egui::Checkbox::new(&mut test, "Diagnostic background"),
                    )
                    .changed()
                {
                    self.finish_inspector();
                    let before = self.doc.clone();
                    self.doc.test_background = test;
                    self.history.record(before, &self.doc);
                    self.publish();
                }
                let dirty = self.saved.as_ref() != Some(&self.doc);
                ui.label(format!(
                    "{}{}",
                    self.project
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .unwrap_or_default()
                        .to_string_lossy(),
                    if dirty { " *" } else { "" }
                ));
            });
        });
        if let Some(a) = action.take() {
            self.action(a);
        }
        egui::TopBottomPanel::bottom("editor_status").show(ctx,|ui|{
            if let Some(error)=self.error.clone(){ui.horizontal_wrapped(|ui|{ui.colored_label(egui::Color32::from_rgb(rastera_core::theme::HOT_MAGENTA[0],rastera_core::theme::HOT_MAGENTA[1],rastera_core::theme::HOT_MAGENTA[2]),error);if ui.small_button("Dismiss").clicked(){self.error=None;}});}
            ui.label(if loading{"Loading / saving — outputs keep the last ready composition"}else if self.engine.applied()!=self.revision{"Preparing changes — outputs keep the last ready composition"}else{"Live • canvas pixels • drag to move / corner handles resize • Cmd+D duplicate • Delete • Cmd+Z / Shift+Cmd+Z"});
        });
        if self.mode == Mode::Viewports {
            self.viewport_panel(ctx, outputs);
        } else {
            egui::SidePanel::left("editor_layers")
                .default_width(210.)
                .width_range(170.0..=350.)
                .show(ctx, |ui| {
                    ui.heading("Layers");
                    ui.label("Topmost first");
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let rows: Vec<_> = self
                            .doc
                            .layers
                            .iter()
                            .rev()
                            .map(|l| (l.id, l.name.clone(), l.visible, l.locked))
                            .collect();
                        ui.add_enabled_ui(!loading, |ui| {
                            for (id, name, visible, locked) in rows {
                                ui.push_id(id, |ui| {
                                    ui.horizontal(|ui| {
                                        if ui
                                            .small_button(if visible { "●" } else { "○" })
                                            .on_hover_text("Show / hide")
                                            .clicked()
                                        {
                                            action = Some(Action::Visible(id));
                                        }
                                        if ui
                                            .small_button(if locked { "🔒" } else { "🔓" })
                                            .on_hover_text("Lock / unlock")
                                            .clicked()
                                        {
                                            action = Some(Action::Lock(id));
                                        }
                                        if ui
                                            .selectable_label(self.selected == Some(id), name)
                                            .clicked()
                                        {
                                            self.finish_gesture();
                                            self.finish_inspector();
                                            self.selected = Some(id);
                                        }
                                    });
                                });
                            }
                        });
                    });
                    ui.separator();
                    ui.add_enabled_ui(
                        !loading
                            && self
                                .selected
                                .and_then(|id| self.doc.layer(id))
                                .is_some_and(|l| !l.locked),
                        |ui| {
                            ui.horizontal(|ui| {
                                if ui.button("Up").clicked() {
                                    action = Some(Action::Up);
                                }
                                if ui.button("Down").clicked() {
                                    action = Some(Action::Down);
                                }
                            });
                            ui.horizontal(|ui| {
                                if ui.button("Duplicate").clicked() {
                                    action = Some(Action::Duplicate);
                                }
                                if ui.button("Delete").clicked() {
                                    action = Some(Action::Delete);
                                }
                            });
                        },
                    );
                });
            if let Some(a) = action.take() {
                self.action(a);
            }
            let before = self.doc.clone();
            let mut ended = false;
            egui::SidePanel::right("editor_inspector")
            .default_width(280.)
            .width_range(240.0..=420.)
            .show(ctx, |ui| {
                ui.heading("Inspector");
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let Some(id) = self.selected else {
                        ui.label("Select a layer on the canvas or in the list.");
                        return;
                    };
                    ui.push_id(id, |ui| {
                        let aspect = self.doc.aspect(id);
                        let Some(layer) = self.doc.layer_mut(id) else {
                            return;
                        };
                        ui.label(format!("Layer #{id}"));
                        if layer.locked {
                            ui.label("Locked — unlock in the layer list to edit.");
                        }
                        ui.add_enabled_ui(!loading && !layer.locked, |ui| {
                            let r =
                                ui.add(egui::TextEdit::singleline(&mut layer.name).char_limit(128));
                            ended |= r.lost_focus();
                            egui::Grid::new("layer_geometry")
                                .num_columns(2)
                                .show(ui, |ui| {
                                    for (name, value) in
                                        [("X", &mut layer.rect.x), ("Y", &mut layer.rect.y)]
                                    {
                                        ui.label(name);
                                        let r = ui.add(
                                            egui::DragValue::new(value)
                                                .speed(2.)
                                                .range(-32768.0..=32768.),
                                        );
                                        ended |= r.drag_stopped() || r.lost_focus();
                                        ui.end_row();
                                    }
                                    ui.label("Width");
                                    let r = ui.add(
                                        egui::DragValue::new(&mut layer.rect.width)
                                            .speed(2.)
                                            .range(1.0..=8192.),
                                    );
                                    if r.changed() {
                                        if let Some(ratio) = aspect {
                                            layer.rect.height =
                                                (layer.rect.width / ratio).min(8192.);
                                            layer.rect.width = layer.rect.height * ratio;
                                        }
                                    }
                                    ended |= r.drag_stopped() || r.lost_focus();
                                    ui.end_row();
                                    ui.label("Height");
                                    let r = ui.add(
                                        egui::DragValue::new(&mut layer.rect.height)
                                            .speed(2.)
                                            .range(1.0..=8192.),
                                    );
                                    if r.changed() {
                                        if let Some(ratio) = aspect {
                                            layer.rect.width =
                                                (layer.rect.height * ratio).min(8192.);
                                            layer.rect.height = layer.rect.width / ratio;
                                        }
                                    }
                                    ended |= r.drag_stopped() || r.lost_focus();
                                    ui.end_row();
                                });
                            let r = ui.add(
                                egui::Slider::new(&mut layer.opacity, 0.0..=1.).text("Opacity"),
                            );
                            ended |= r.drag_stopped();
                            if matches!(layer.content, Content::Image(_)|Content::Capture(_)) {
                                let r =
                                    ui.checkbox(&mut layer.aspect_lock, "Lock source aspect ratio");
                                ended |= r.changed();
                            }
                        });
                        let source = layer.content.clone();
                        let locked = layer.locked;
                        if let Content::Capture(id)=source {
                            let source=&self.doc.captures[&id];let status=self.capture.status(&self.doc,id);
                            ui.separator();ui.label(format!("{} · {}",source.name,status.phase));ui.label(format!("Requested {}×{} @ {:.3} fps",source.mode.width,source.mode.height,source.mode.fps));ui.label(format!("Actual {}×{} BGRA · negotiated {:.3} fps · observed {:.2} fps · {} delivered / {} upload operations",status.actual[0],status.actual[1],status.negotiated_fps,status.cadence,status.frames,status.uploaded));
                            ui.small("Unavailable signal draws black. All layer copies share this source session.");
                            if ui.add_enabled(!loading&&!locked,egui::Button::new("Reassign device / format…")).clicked(){self.capture.edit(id,source);}
                            if ui.add_enabled(!loading&&!locked,egui::Button::new("Retry source / request permission")).clicked(){self.capture.retry(&self.doc,id);self.capture.edit(id,source);}
                        }
                        if let Content::Text(source) = source {
                            let text = self.doc.texts.get_mut(&source).unwrap();
                            ui.add_enabled_ui(!loading && !locked, |ui| {
                                ui.separator();
                                ui.label("Editable text");
                                if ui.button("Choose font family / face…").clicked(){self.font_dialog=true;}
                                if let Some(font)=&text.font{ui.label(format!("{} — {}",font.family,font.style));if self.library.ready.load(Ordering::Acquire)&&!self.library.available(font){ui.colored_label(egui::Color32::from_rgb(rastera_core::theme::ELECTRIC_YELLOW[0],rastera_core::theme::ELECTRIC_YELLOW[1],rastera_core::theme::ELECTRIC_YELLOW[2]),"Missing/unusable font — bundled Ubuntu fallback; reference retained");}}else{ui.label("Bundled Ubuntu Light");}
                                let r = ui.add(
                                    egui::TextEdit::multiline(&mut text.text)
                                        .desired_rows(6)
                                        .char_limit(4096)
                                        .desired_width(f32::INFINITY),
                                );
                                ended |= r.lost_focus();
                                let r = ui.add(
                                    egui::Slider::new(&mut text.font_size, 4.0..=512.)
                                        .text("Font px"),
                                );
                                ended |= r.drag_stopped();
                                ui.horizontal(|ui| {
                                    for (label, a) in [
                                        ("Left", Alignment::Left),
                                        ("Center", Alignment::Center),
                                        ("Right", Alignment::Right),
                                    ] {
                                        if ui
                                            .selectable_value(&mut text.alignment, a, label)
                                            .changed()
                                        {
                                            ended = true;
                                        }
                                    }
                                });
                                ui.label("Text color (straight RGBA)");
                                ui.horizontal(|ui| {
                                    for value in &mut text.color {
                                        let r = ui.add(
                                            egui::DragValue::new(value).range(0..=255).speed(1.),
                                        );
                                        ended |= r.drag_stopped() || r.lost_focus();
                                    }
                                });
                            });
                        }
                    });
                });
            });
            if self.doc != before {
                if self.inspector_before.is_none() {
                    self.inspector_before = Some(before);
                }
                self.publish();
            }
            if ended {
                self.finish_inspector();
            }
        }
        let area = egui::CentralPanel::default()
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} × {} canvas pixels • {} • overlays are operator-only",
                    self.doc.canvas[0],
                    self.doc.canvas[1],
                    if self.mode == Mode::Canvas {
                        "Canvas"
                    } else {
                        "Viewports"
                    }
                ));
                ui.available_rect_before_wrap()
            })
            .inner;
        self.dialog_ui(ctx);
        // Evaluate AFTER text widgets process this frame's focus/keyboard input.
        if !ctx.wants_keyboard_input() && !loading {
            let redo = egui::Modifiers {
                shift: true,
                ..egui::Modifiers::COMMAND
            };
            if ctx.input_mut(|i| i.consume_key(redo, egui::Key::Z)) {
                self.action(Action::Redo);
            } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
                self.action(Action::Undo);
            }
            if self.mode == Mode::Canvas
                && ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::D))
            {
                self.action(Action::Duplicate);
            }
            if self.mode == Mode::Canvas
                && ctx.input(|i| {
                    i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)
                })
            {
                self.action(Action::Delete);
            }
        }
        if !loading {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::S)) {
                self.action(Action::Save(false));
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
                self.action(Action::Open);
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::I)) {
                self.action(Action::Import);
            }
        }
        area
    }
}
