//! Desktop Live Edit interaction. Document/rendering live in rastera-core.
#[path = "editor_probe.rs"]
mod probe;
use super::{log, Consumer};
use eframe::egui;
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
}
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
    Save(PathBuf, Document),
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
                        scene::save_project(&doc, &path).map(|()| FileResult::Saved(path, doc))
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
    pub doc: Document,
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
    error: Option<String>,
    probe: Option<probe::Probe>,
}
impl Editor {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let engine = SceneEngine::new(device, queue);
        let doc = Document::default();
        let revision = engine.submit(doc.clone());
        let mut editor = Self {
            engine,
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
    fn file_job(&mut self, job: FileJob) {
        match self.files.tx.try_send(job) {
            Ok(()) => self.busy_file = true,
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
                Ok(FileResult::Loaded(path, doc)) => {
                    let revision = self.engine.submit(doc.clone());
                    self.revision = revision;
                    self.pending_load = Some((revision, path, doc));
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
                            self.publish();
                        }
                    }
                    Ok(()) => {
                        if let Some((revision, path, doc)) = self.pending_load.take() {
                            if revision == status.revision {
                                self.doc = doc;
                                self.selected = None;
                                self.history = History::default();
                                self.inspector_before = None;
                                self.gesture = None;
                                self.saved = Some(self.doc.clone());
                                self.project = Some(path.clone());
                                log(format!(
                                    "editor_loaded path={} prepared_revision={}\n",
                                    path.display(),
                                    revision
                                ));
                            }
                        }
                    }
                }
            }
        }
        if self.selected.is_some_and(|id| self.doc.layer(id).is_none()) {
            self.selected = None;
        }
    }
    fn action(&mut self, action: Action) {
        self.finish_gesture();
        self.finish_inspector();
        let before = self.doc.clone();
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
                    self.file_job(FileJob::Save(path, self.doc.clone()));
                }
            }
            Action::Undo => {
                if self.history.undo(&mut self.doc) {
                    self.publish();
                }
                return;
            }
            Action::Redo => {
                if self.history.redo(&mut self.doc) {
                    self.publish();
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
        outputs: &[Option<Consumer>; 2],
        enabled: &mut [bool; 2],
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
            if self.pending_load.is_some() {
                continue;
            }
            let map = PreviewMap::new([event.width, event.height]);
            let Some(p) = map.canvas([event.x, event.y], true) else {
                continue;
            };
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
                            map.handle(l.rect, p, 8. * event.scale * 3840. / map.fit[2])
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
    }
    pub fn ui(&mut self, ctx: &egui::Context, outputs: &mut [bool; 2]) -> egui::Rect {
        self.poll();
        let mut action = None;
        let loading = self.busy_file || self.pending_load.is_some();
        egui::TopBottomPanel::top("editor_toolbar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("Live Edit — changes affect outputs");
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
                ui.checkbox(&mut outputs[0], "Left output");
                ui.checkbox(&mut outputs[1], "Right output");
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
            if let Some(error)=self.error.clone(){ui.horizontal_wrapped(|ui|{ui.colored_label(egui::Color32::LIGHT_RED,error);if ui.small_button("Dismiss").clicked(){self.error=None;}});}
            ui.label(if loading{"Loading / saving — outputs keep the last ready composition"}else if self.engine.applied()!=self.revision{"Preparing changes — outputs keep the last ready composition"}else{"Live • 3840 × 1080 • drag to move / corner handles resize • Cmd+D duplicate • Delete • Cmd+Z / Shift+Cmd+Z"});
        });
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
                            if matches!(layer.content, Content::Image(_)) {
                                let r =
                                    ui.checkbox(&mut layer.aspect_lock, "Lock image aspect ratio");
                                ended |= r.changed();
                            }
                        });
                        let source = layer.content.clone();
                        let locked = layer.locked;
                        if let Content::Text(source) = source {
                            let text = self.doc.texts.get_mut(&source).unwrap();
                            ui.add_enabled_ui(!loading && !locked, |ui| {
                                ui.separator();
                                ui.label("Editable text");
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
        let area = egui::CentralPanel::default()
            .show(ctx, |ui| {
                ui.label("Canvas preview • canvas pixels • selection is operator-only");
                ui.available_rect_before_wrap()
            })
            .inner;
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
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::D)) {
                self.action(Action::Duplicate);
            }
            if ctx
                .input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
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
