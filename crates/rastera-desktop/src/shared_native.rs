#[path = "outputs.rs"]
mod destinations;
#[path = "editor.rs"]
mod editor;
use eframe::egui;
use foreign_types::ForeignType;
use rastera_core::{
    mapping::FrameViewport,
    shared_frames::{FrameLease, FramePool, SharedCanvasProducer},
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{c_char, c_void, CString},
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
    sync::{atomic::Ordering, mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

type Pool = FramePool<wgpu::Texture>;
#[repr(C)]
struct NativeFrame {
    texture: *mut c_void,
    lease: *mut c_void,
    frame: u64,
    width: u32,
    height: u32,
    viewports: *const FrameViewport,
    viewport_count: u32,
    reserved: u32,
}
const _: [(); 48] = [(); std::mem::size_of::<NativeFrame>()];
const _: () = assert!(std::mem::offset_of!(NativeFrame, viewports) == 32);
const _: () = assert!(std::mem::offset_of!(FrameViewport, matrix) == 16);
unsafe extern "C" {
    fn rastera_shared_create(
        source: *mut c_void,
        host: *mut c_void,
        viewport_id: u64,
        defer_show: bool,
    ) -> *mut c_void;
    fn rastera_shared_destroy(handle: *mut c_void);
    fn rastera_shared_errors(handle: *mut c_void) -> u64;
    fn rastera_shared_is_open(handle: *mut c_void) -> bool;
    fn rastera_shared_report();
    fn rastera_displays_json() -> *mut c_char;
    fn rastera_editor_free_string(value: *mut c_char);
    fn rastera_identify_displays();
    fn rastera_outputs_disable_all();
    fn rastera_outputs_hide_all();
    fn rastera_outputs_take_disable() -> bool;
    fn rastera_shared_destination(handle: *mut c_void, display: *const c_char) -> bool;
    fn rastera_shared_sync_window_size(handle: *mut c_void);
    fn rastera_shared_title(handle: *mut c_void, name: *const c_char);
    fn rastera_shared_host_time() -> f64;
    fn rastera_shared_log(line: *const c_char);
}
fn host_time() -> f64 {
    // SAFETY: monotonic host clock, no AppKit access.
    unsafe { rastera_shared_host_time() }
}
fn log(line: String) {
    let line = CString::new(line).unwrap();
    // SAFETY: synchronous logging; string remains valid for the call.
    unsafe {
        rastera_shared_log(line.as_ptr());
    }
}
#[unsafe(no_mangle)]
unsafe extern "C" fn rastera_shared_acquire(source: *mut c_void, out: *mut NativeFrame) -> bool {
    // SAFETY: native owner borrows this boxed Arc until its render thread joins.
    let pool = unsafe { &*source.cast::<Arc<Pool>>() };
    let Some(lease) = pool.acquire() else {
        return false;
    };
    // SAFETY: Metal-only texture. The lease owns the wgpu texture throughout all
    // native reads; neither bridge nor consumer destroys or mutates the raw texture.
    let texture = unsafe {
        lease
            .resource()
            .as_hal::<wgpu::hal::api::Metal, _, _>(|hal| {
                hal.map(|t| t.raw_handle().as_ptr().cast::<c_void>())
            })
    };
    let Some(texture) = texture else {
        return false;
    };
    let frame = lease.frame_id;
    let canvas = lease.meta.canvas;
    let viewports = lease.meta.viewports.as_ptr();
    let viewport_count = lease.meta.viewports.len() as u32;
    let lease = Box::into_raw(Box::new(lease)).cast();
    // SAFETY: native caller passes a valid out pointer, and must release once.
    unsafe {
        out.write(NativeFrame {
            texture,
            lease,
            frame,
            width: canvas[0],
            height: canvas[1],
            viewports,
            viewport_count,
            reserved: 0,
        });
    }
    true
}
#[unsafe(no_mangle)]
unsafe extern "C" fn rastera_shared_release(lease: *mut c_void) {
    // SAFETY: exactly once, on encoding failure or AFTER consumer GPU completion.
    // No source context, window or owner is accessed by a late completion.
    unsafe {
        drop(Box::from_raw(lease.cast::<FrameLease<wgpu::Texture>>()));
    }
}
#[unsafe(no_mangle)]
unsafe extern "C" fn rastera_shared_fit(w: u32, h: u32, dw: f32, dh: f32, out: *mut f32) {
    let fit = rastera_core::fit([w, h], [dw, dh]);
    // SAFETY: native mapping struct supplies four writable floats.
    unsafe {
        std::ptr::copy_nonoverlapping(fit.as_ptr(), out, 4);
    }
}
#[unsafe(no_mangle)]
unsafe extern "C" fn rastera_editor_fit(
    w: u32,
    h: u32,
    dw: f32,
    dh: f32,
    margin: f32,
    out: *mut f32,
) {
    let fit = rastera_core::scene::PreviewMap::for_canvas([w, h], [dw, dh], margin).fit;
    unsafe {
        std::ptr::copy_nonoverlapping(fit.as_ptr(), out, 4);
    }
}
#[unsafe(no_mangle)]
unsafe extern "C" fn rastera_theme_workspace(out: *mut f32) {
    let c = rastera_core::theme::floats(rastera_core::theme::WORKSPACE);
    unsafe {
        std::ptr::copy_nonoverlapping(c.as_ptr(), out, 4);
    }
}
struct Consumer {
    handle: NonNull<c_void>,
    source: *mut Arc<Pool>,
    _main: PhantomData<Rc<()>>,
}
impl Consumer {
    fn new(pool: &Arc<Pool>, host: *mut c_void, viewport_id: u64) -> std::io::Result<Self> {
        Self::new_deferred(pool, host, viewport_id, false)
    }
    fn new_deferred(
        pool: &Arc<Pool>,
        host: *mut c_void,
        viewport_id: u64,
        defer_show: bool,
    ) -> std::io::Result<Self> {
        let source = Box::into_raw(Box::new(pool.clone()));
        // SAFETY: called on eframe main; host is either the live AppKit view or nil.
        let handle = unsafe { rastera_shared_create(source.cast(), host, viewport_id, defer_show) };
        match NonNull::new(handle) {
            Some(handle) => Ok(Self {
                handle,
                source,
                _main: PhantomData,
            }),
            None => {
                // No worker survives failed native initialization.
                unsafe {
                    drop(Box::from_raw(source));
                }
                Err(std::io::Error::other("native consumer initialization failed (requires macOS 14+ and compatible Metal texture)"))
            }
        }
    }
    fn open(&self) -> bool {
        // SAFETY: live main-thread-only owner.
        unsafe { rastera_shared_is_open(self.handle.as_ptr()) }
    }
}
impl Drop for Consumer {
    fn drop(&mut self) {
        // SAFETY: main-only type. Stop/join/drain before freeing source context.
        // Late GPU handlers retain leases, not this context.
        unsafe {
            rastera_shared_destroy(self.handle.as_ptr());
            drop(Box::from_raw(self.source));
        }
    }
}
struct App {
    preview: Option<Consumer>,
    outputs: BTreeMap<u64, Consumer>,
    open_outputs: BTreeSet<u64>,
    initial_output_count: Option<usize>,
    active_destinations: BTreeMap<u64, destinations::Destination>,
    display_refresh: Instant,
    producer: Option<SharedCanvasProducer>,
    stop: mpsc::Sender<()>,
    monitor: Option<thread::JoinHandle<()>>,
    origin: Instant,
    previous: Instant,
    callbacks: u64,
    editor: Option<editor::Editor>,
}
impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        // Native recovery hides immediately even before the next eframe callback.
        if unsafe { rastera_outputs_take_disable() } {
            self.open_outputs.clear();
            self.initial_output_count = None;
        }
        if let Some(editor) = &mut self.editor {
            if self.display_refresh.elapsed() > Duration::from_secs(1)
                || editor.destinations.refresh
            {
                editor.destinations.refresh = false;
                self.display_refresh = Instant::now();
                let raw = unsafe { rastera_displays_json() };
                if !raw.is_null() {
                    let parsed = unsafe {
                        serde_json::from_slice::<destinations::Discovery>(
                            std::ffi::CStr::from_ptr(raw).to_bytes(),
                        )
                    };
                    unsafe { rastera_editor_free_string(raw) };
                    match parsed {
                        Ok(discovery) => {
                            editor.destinations.discovery = discovery;
                            let recovered: Vec<_> = editor
                                .destinations
                                .status
                                .iter()
                                .filter(|(id, status)| {
                                    status.starts_with("Missing")
                                        && editor.destinations.preferences.get(id).is_some_and(
                                            |d| {
                                                destinations::resolve(
                                                    d,
                                                    &editor.destinations.discovery.displays,
                                                )
                                                .is_ok()
                                            },
                                        )
                                })
                                .map(|(&id, _)| id)
                                .collect();
                            for id in recovered {
                                editor.destinations.status.remove(&id);
                            }
                            if editor.destinations.discovery.machine.is_empty() {
                                editor.destinations.error=Some("Machine identity unavailable; destination preferences cannot be persisted".into());
                            }
                        }
                        Err(e) => {
                            log(format!("display_discovery error={e}\n"));
                            editor.destinations.error = Some(format!("Display discovery: {e}"))
                        }
                    }
                }
            }
        }
        let now = Instant::now();
        let gap = now.duration_since(self.previous);
        self.previous = now;
        self.callbacks += 1;
        if gap > Duration::from_millis(100) {
            log(format!(
                "shared_ui_gap t_us={} gap_us={} callbacks={} host_s={}\n",
                self.origin.elapsed().as_micros(),
                gap.as_micros(),
                self.callbacks,
                host_time()
            ));
        }
        ctx.request_repaint_after(Duration::from_secs_f64(1.0 / 60.0));
        // Native close retires only that ID, before editor controls can explicitly reopen it.
        let closed: Vec<_> = self
            .outputs
            .iter()
            .filter(|(_, c)| !c.open())
            .map(|(&id, _)| id)
            .collect();
        for id in closed {
            let failed = self
                .outputs
                .get(&id)
                .is_some_and(|c| unsafe { rastera_shared_errors(c.handle.as_ptr()) } > 0);
            if failed {
                if let Some(editor) = &mut self.editor {
                    editor.destinations.status.insert(
                        id,
                        "Error: native GPU output stopped; re-enable explicitly".into(),
                    );
                }
            }
            self.outputs.remove(&id);
            self.open_outputs.remove(&id);
            self.active_destinations.remove(&id);
            if let Some(editor) = &mut self.editor {
                if !failed {
                    editor.destinations.status.remove(&id);
                }
            }
        }
        let definitions = if let Some(editor) = &mut self.editor {
            if let Some(preview) = &self.preview {
                editor.pointers(preview, ctx);
            }
            let area = editor.ui(ctx, &mut self.open_outputs);
            if let Some(preview) = &self.preview {
                editor.native_layout(preview, ctx, area);
                if editor.settings_requested() {
                    editor.settings_menu(preview);
                }
                editor.probe_tick(preview, &self.outputs, &mut self.open_outputs);
            }
            if std::mem::take(&mut editor.output_disable_requested) {
                self.open_outputs.clear();
                unsafe { rastera_outputs_hide_all() };
            }
            if unsafe { rastera_outputs_take_disable() } {
                self.open_outputs.clear();
                self.initial_output_count = None;
            }
            editor
                .doc
                .viewports
                .iter()
                .map(|v| (v.id, v.name.clone()))
                .collect::<BTreeMap<_, _>>()
        } else {
            egui::TopBottomPanel::top("controls").exact_height(170.).show(ctx,|ui|{
                ui.heading("Operator / UI preview — shared canvas integration");
                ui.monospace(format!("UI elapsed {:.3}s · callback {}",self.origin.elapsed().as_secs_f64(),self.callbacks));
                for (id,name) in [(1,"Left"),(2,"Right")] {
                    let mut open=self.open_outputs.contains(&id);
                    if ui.checkbox(&mut open,format!("{name} native clean output")).changed(){if open{self.open_outputs.insert(id);}else{self.open_outputs.remove(&id);}}
                }
                ui.label("Native canvas preview: full 3840 × 1080. Source dimensions never follow window size.");
            });
            egui::CentralPanel::default().show(ctx, |_| {});
            [(1, "Left".to_owned()), (2, "Right".to_owned())]
                .into_iter()
                .collect()
        };
        if let Some(editor) = &mut self.editor {
            if editor.destinations.identify {
                editor.destinations.identify = false;
                unsafe { rastera_identify_displays() };
            }
            let requests = destinations::plan(
                &editor.destinations.preferences,
                &self.open_outputs,
                &self.active_destinations,
                &editor.destinations.discovery.displays,
            );
            for (id, result) in requests {
                if let Err(error) = result {
                    self.open_outputs.remove(&id);
                    editor.destinations.status.insert(id, error);
                }
            }
            editor
                .destinations
                .status
                .retain(|id, status| self.open_outputs.contains(id) || status != "Active");
        }
        // Removal, window close and project edits never reassign a consumer by position.
        self.open_outputs.retain(|id| definitions.contains_key(id));
        let closed: Vec<_> = self
            .outputs
            .iter()
            .filter(|(_, c)| !c.open())
            .map(|(&id, _)| id)
            .collect();
        for id in closed {
            let failed = self
                .outputs
                .get(&id)
                .is_some_and(|c| unsafe { rastera_shared_errors(c.handle.as_ptr()) } > 0);
            if failed {
                if let Some(editor) = &mut self.editor {
                    editor.destinations.status.insert(
                        id,
                        "Error: native GPU output stopped; re-enable explicitly".into(),
                    );
                }
            }
            self.outputs.remove(&id);
            self.open_outputs.remove(&id);
        }
        self.active_destinations
            .retain(|id, _| self.open_outputs.contains(id));
        self.outputs.retain(|id, _| self.open_outputs.contains(id)); // Drop joins/drains GPU readers.
        if self.initial_output_count.is_some()
            && self.editor.as_ref().is_some_and(|e| !e.busy_loading())
        {
            let count = self.initial_output_count.take().unwrap();
            if let Some(editor) = &self.editor {
                self.open_outputs.extend(
                    editor
                        .doc
                        .viewports
                        .iter()
                        .filter(|v| {
                            !editor
                                .destinations
                                .preferences
                                .get(&v.id)
                                .is_some_and(|d| d.fullscreen)
                        })
                        .take(count)
                        .map(|v| v.id),
                );
            }
        }
        let pool = &self.producer.as_ref().unwrap().pool;
        let mut failures = vec![];
        for &id in &self.open_outputs {
            if let std::collections::btree_map::Entry::Vacant(entry) = self.outputs.entry(id) {
                // A new logical definition may still be preparing. Wait for its completed frame.
                if !pool
                    .acquire()
                    .is_some_and(|f| f.meta.viewport(id).is_some())
                {
                    continue;
                }
                match Consumer::new_deferred(
                    pool,
                    std::ptr::null_mut(),
                    id,
                    self.editor.as_ref().is_some_and(|e| {
                        e.destinations
                            .preferences
                            .get(&id)
                            .is_some_and(|d| d.fullscreen)
                    }),
                ) {
                    Ok(c) => {
                        entry.insert(c);
                    }
                    Err(error) => {
                        failures.push(id);
                        if let Some(editor) = &mut self.editor {
                            editor.error = Some(format!("Could not open viewport {id}: {error}"));
                            editor
                                .destinations
                                .status
                                .insert(id, format!("Error opening native output: {error}"));
                        }
                        log(format!(
                            "shared_error open_viewport_id={id} message={error}\n"
                        ));
                    }
                }
            }
        }
        for id in failures {
            self.open_outputs.remove(&id);
        }
        let mut destination_errors = Vec::new();
        let editor_mode = self.editor.is_some();
        for (&id, out) in self.outputs.iter().filter(|_| editor_mode) {
            if let Some(editor) = &mut self.editor {
                let d = editor
                    .destinations
                    .preferences
                    .get(&id)
                    .cloned()
                    .unwrap_or_default();
                let target = CString::new(if d.fullscreen {
                    d.display.clone().unwrap_or_default()
                } else {
                    String::new()
                })
                .unwrap();
                if unsafe { rastera_shared_destination(out.handle.as_ptr(), target.as_ptr()) } {
                    self.active_destinations.insert(id, d);
                    editor.destinations.status.insert(id, "Active".into());
                } else {
                    editor.destinations.status.insert(
                        id,
                        "Error: native display placement failed; re-enable explicitly".into(),
                    );
                    destination_errors.push(id);
                }
            }
            if let Some(name) = definitions.get(&id) {
                let name = CString::new(name.replace('\0', "")).unwrap();
                unsafe {
                    rastera_shared_title(out.handle.as_ptr(), name.as_ptr());
                    rastera_shared_sync_window_size(out.handle.as_ptr());
                }
            }
        }
        for id in destination_errors {
            self.open_outputs.remove(&id);
            self.active_destinations.remove(&id);
            self.outputs.remove(&id);
        }
    }
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(m) = self.monitor.take() {
            let _ = m.join();
        }
        self.outputs.clear();
        self.preview = None;
        let pool = self.producer.as_ref().unwrap().pool.clone();
        self.producer = None; // Join producer AFTER every native consumer has stopped.
        self.editor = None; // Stop preparation/file workers after live rendering has stopped.
        log(format!(
            "shared_shutdown outstanding_slots={} capacity=4\n",
            pool.leased_slots()
        ));
    }
}
#[allow(dead_code)] // The two desktop binaries share this bridge and baseline runner.
pub fn run() -> eframe::Result {
    run_mode(false)
}
#[allow(dead_code)]
pub fn run_editor() -> eframe::Result {
    run_mode(true)
}
fn run_mode(editor_mode: bool) -> eframe::Result {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .ok_or_else(|| eframe::Error::AppCreation("Metal adapter unavailable".into()))?;
    log(format!("shared_adapter info={:?}\n", adapter.get_info()));
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits {
                max_texture_dimension_2d: 8192,
                ..Default::default()
            },
            ..Default::default()
        },
        None,
    ))
    .map_err(|e| eframe::Error::AppCreation(Box::new(e)))?;
    let editor = editor_mode.then(|| editor::Editor::new(device.clone(), queue.clone()));
    let producer = if let Some(editor) = &editor {
        SharedCanvasProducer::with_scene(device, queue, editor.engine.feed())
    } else {
        SharedCanvasProducer::new(device, queue)
    };
    let start = Instant::now();
    while producer.pool.acquire().is_none() {
        if start.elapsed() > Duration::from_secs(5) {
            return Err(eframe::Error::AppCreation(
                "No completed canvas frame after five seconds".into(),
            ));
        }
        thread::sleep(Duration::from_millis(5));
    }
    let probing = std::env::args().any(|a| a.starts_with("--editor-probe="));
    let display_probing = std::env::args().any(|a| a == "--display-probe");
    let options = eframe::NativeOptions {
        event_loop_builder: probing.then(|| {
            Box::new(
                move |builder: &mut winit::event_loop::EventLoopBuilder<eframe::UserEvent>| {
                    use winit::platform::macos::EventLoopBuilderExtMacOS;
                    builder.with_activate_ignoring_other_apps(display_probing);
                },
            ) as _
        }),
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_active(!probing || display_probing)
            .with_inner_size(if editor_mode {
                [1320., 760.]
            } else {
                [1000., 600.]
            })
            .with_min_inner_size(if editor_mode {
                [900., 450.]
            } else {
                [500., 300.]
            }),
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetupCreateNew {
                instance_descriptor: wgpu::InstanceDescriptor {
                    backends: wgpu::Backends::METAL,
                    ..Default::default()
                },
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        if probing {
            "Rastera — Canvas Editor — Operator / UI preview (automated probe)"
        } else if editor_mode {
            "Rastera — Canvas Editor — Operator / UI preview"
        } else {
            "Rastera — Operator / UI preview"
        },
        options,
        Box::new(move |cc| {
            let host = match cc.window_handle()?.as_raw() {
                RawWindowHandle::AppKit(h) => h.ns_view.as_ptr(),
                _ => return Err("AppKit view required".into()),
            };
            // First consumer is the embedded full-canvas native preview. No eframe texture registration.
            let preview = Consumer::new(&producer.pool, host, 0)?;
            let count: usize = std::env::args()
                .find_map(|a| a.strip_prefix("--outputs=").map(str::to_owned))
                .unwrap_or_else(|| if editor_mode { "0".into() } else { "2".into() })
                .parse()?;
            if count > rastera_core::mapping::MAX_VIEWPORTS {
                return Err("--outputs exceeds viewport limit 8".into());
            }
            let mut outputs = BTreeMap::new();
            let mut open_outputs = BTreeSet::new();
            let initial_output_count =
                if editor_mode && std::env::args().any(|a| a.starts_with("--project=")) {
                    Some(count)
                } else {
                    None
                };
            if let Some(frame) = producer
                .pool
                .acquire()
                .filter(|_| initial_output_count.is_none())
            {
                for v in frame.meta.viewports.iter().take(count) {
                    open_outputs.insert(v.id);
                    outputs.insert(
                        v.id,
                        Consumer::new(&producer.pool, std::ptr::null_mut(), v.id)?,
                    );
                }
            }
            let stats = producer.stats.clone();
            let skipped = producer.skipped.clone();
            let pool = producer.pool.clone();
            let (stop, rx) = mpsc::channel();
            let monitor = thread::spawn(move || loop {
                if rx.recv_timeout(Duration::from_secs(1)).is_ok() {
                    break;
                }
                log(format!("shared_producer host_s={} t_us={} completed={} submitted={} skipped={} leased_slots={} capacity=4\n",host_time(),stats.timings.origin.elapsed().as_micros(),stats.completed.load(Ordering::Relaxed),stats.submitted.load(Ordering::Relaxed),skipped.load(Ordering::Relaxed),pool.leased_slots()));
                log(stats.timings.report());
                // SAFETY: reporter touches only mutex/atomic C++ telemetry, never AppKit.
                unsafe {
                    rastera_shared_report();
                }
            });
            let origin = Instant::now();
            Ok(Box::new(App {
                preview: Some(preview),
                outputs,
                open_outputs,
                initial_output_count,
                active_destinations: BTreeMap::new(),
                display_refresh: Instant::now(),
                producer: Some(producer),
                stop,
                monitor: Some(monitor),
                origin,
                previous: origin,
                callbacks: 0,
                editor,
            }))
        }),
    )
}
