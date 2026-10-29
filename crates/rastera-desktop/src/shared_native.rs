#[path = "editor.rs"]
mod editor;
use eframe::egui;
use foreign_types::ForeignType;
use rastera_core::{
    shared_frames::{FrameLease, FramePool, SharedCanvasProducer},
    Slice,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
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
    maps: [[f32; 8]; 2],
}
const _: [(); 96] = [(); std::mem::size_of::<NativeFrame>()];
unsafe extern "C" {
    fn rastera_shared_create(
        source: *mut c_void,
        host: *mut c_void,
        role: i32,
        slice: *const u32,
    ) -> *mut c_void;
    fn rastera_shared_destroy(handle: *mut c_void);
    fn rastera_shared_is_open(handle: *mut c_void) -> bool;
    fn rastera_shared_report();
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
    let meta = lease.meta.clone();
    let lease = Box::into_raw(Box::new(lease)).cast();
    // SAFETY: native caller passes a valid out pointer, and must release once.
    unsafe {
        out.write(NativeFrame {
            texture,
            lease,
            frame,
            width: meta.canvas[0],
            height: meta.canvas[1],
            maps: meta.maps,
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
    fn new(pool: &Arc<Pool>, host: *mut c_void, role: i32, slice: Slice) -> std::io::Result<Self> {
        let source = Box::into_raw(Box::new(pool.clone()));
        let rect = [slice.x, slice.y, slice.width, slice.height];
        // SAFETY: called on eframe main; host is either the live AppKit view or nil.
        let handle = unsafe { rastera_shared_create(source.cast(), host, role, rect.as_ptr()) };
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
    outputs: [Option<Consumer>; 2],
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
        if let Some(editor) = &mut self.editor {
            // Retire native-close handles before checkbox input; a reopen click in
            // the first UI frame after close must create a fresh consumer.
            for output in &mut self.outputs {
                if output.as_ref().is_some_and(|o| !o.open()) {
                    *output = None;
                }
            }
            if let Some(preview) = &self.preview {
                editor.pointers(preview, ctx);
            }
            let mut enabled =
                std::array::from_fn(|i| self.outputs[i].as_ref().is_some_and(Consumer::open));
            let area = editor.ui(ctx, &mut enabled);
            if let Some(preview) = &self.preview {
                editor.native_layout(preview, ctx, area);
                if editor.settings_requested() {
                    editor.settings_menu(preview);
                }
                editor.probe_tick(preview, &self.outputs, &mut enabled);
            }
            let pool = &self.producer.as_ref().unwrap().pool;
            for (i, wanted) in enabled.into_iter().enumerate() {
                // Native close can occur during a modal file panel inside ui().
                // It wins over the pre-panel checkbox snapshot; do not auto-reopen.
                if !wanted || self.outputs[i].as_ref().is_some_and(|c| !c.open()) {
                    self.outputs[i] = None;
                } else if self.outputs[i].is_none() {
                    self.outputs[i] = None;
                    match Consumer::new(
                        pool,
                        std::ptr::null_mut(),
                        (i + 1) as i32,
                        if i == 0 { Slice::LEFT } else { Slice::RIGHT },
                    ) {
                        Ok(c) => self.outputs[i] = Some(c),
                        Err(e) => log(format!("shared_error open_output={i} message={e}\n")),
                    }
                }
            }
            for (i, out) in self.outputs.iter().enumerate() {
                if let Some(out) = out {
                    let name =
                        CString::new(editor.doc.viewports[i].name.replace('\0', "")).unwrap();
                    unsafe {
                        rastera_shared_title(out.handle.as_ptr(), name.as_ptr());
                    }
                }
            }
        } else {
            egui::TopBottomPanel::top("controls").exact_height(170.0).show(ctx,|ui| {
            ui.heading("Operator / UI preview — shared canvas integration");
            ui.monospace(format!("UI elapsed {:.3}s · callback {} (UI may pause during native transitions)",self.origin.elapsed().as_secs_f64(),self.callbacks));
            ui.label("Preview below and clean outputs read completed frames from one core producer.");
            let pool=&self.producer.as_ref().unwrap().pool;
            for (i,name) in ["Left native clean output","Right native clean output"].iter().enumerate() {
                if self.outputs[i].as_ref().is_some_and(|c|!c.open()) { self.outputs[i]=None; }
                let mut enabled=self.outputs[i].is_some();
                if ui.checkbox(&mut enabled,*name).changed() {
                    if enabled {
                        match Consumer::new(pool,std::ptr::null_mut(),(i+1) as i32,if i==0 {Slice::LEFT}else{Slice::RIGHT}) {
                            Ok(c)=>self.outputs[i]=Some(c),Err(e)=>log(format!("shared_error open_output={i} message={e}\n")),
                        }
                    } else { self.outputs[i]=None; }
                }
            }
            ui.label("Native canvas preview: full 3840 × 1080. Source dimensions never follow window size.");
        });
            egui::CentralPanel::default().show(ctx, |_| {});
        }
    }
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(m) = self.monitor.take() {
            let _ = m.join();
        }
        self.outputs = [None, None];
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
    let options = eframe::NativeOptions {
        event_loop_builder: probing.then(|| {
            Box::new(
                |builder: &mut winit::event_loop::EventLoopBuilder<eframe::UserEvent>| {
                    use winit::platform::macos::EventLoopBuilderExtMacOS;
                    builder.with_activate_ignoring_other_apps(false);
                },
            ) as _
        }),
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_active(!probing)
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
            let preview = Consumer::new(&producer.pool, host, 0, Slice::FULL)?;
            let mut outputs = [None, None];
            let count = std::env::args()
                .find_map(|a| a.strip_prefix("--outputs=").map(str::to_owned))
                .unwrap_or_else(|| "2".into());
            let count: usize = count.parse()?;
            if count > 2 {
                return Err("--outputs must be 0, 1 or 2".into());
            }
            for (i, out) in outputs.iter_mut().enumerate().take(count) {
                *out = Some(Consumer::new(
                    &producer.pool,
                    std::ptr::null_mut(),
                    (i + 1) as i32,
                    if i == 0 { Slice::LEFT } else { Slice::RIGHT },
                )?);
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
