//! AVFoundation adapter ownership and operator controls. Sessions never depend on repaint cadence.
use rastera_core::{
    capture::{Hub, Mode, Slot, Source},
    scene::{Document, Id},
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    ffi::{c_char, c_void, CStr, CString},
    ptr::NonNull,
    sync::Arc,
    time::{Duration, Instant},
};
unsafe extern "C" {
    fn rastera_camera_bundle_valid() -> bool;
    fn rastera_camera_authorization() -> i32;
    fn rastera_camera_request() -> bool;
    fn rastera_capture_devices() -> *mut c_char;
    fn rastera_capture_create(
        context: *mut c_void,
        device: *const c_char,
        width: u32,
        height: u32,
        fps: f64,
    ) -> *mut c_void;
    fn rastera_capture_destroy(handle: *mut c_void);
    fn rastera_capture_drain() -> bool;
    fn rastera_editor_free_string(value: *mut c_char);
}
struct Context {
    slot: Arc<Slot>,
    epoch: u64,
    expected: [u32; 2],
}
#[unsafe(no_mangle)]
unsafe extern "C" fn rastera_capture_pixels(
    context: *mut c_void,
    width: u32,
    height: u32,
    stride: usize,
    data: *const u8,
    length: usize,
) {
    // Native owns the context and locked pixel buffer through this synchronous copy.
    let c = unsafe { &*context.cast::<Context>() };
    if data.is_null() || length > 32 * 1024 * 1024 {
        return;
    }
    if [width, height] != c.expected {
        c.slot.phase(
            c.epoch,
            &format!(
                "Error: camera delivered {width}×{height}, requested {}×{}",
                c.expected[0], c.expected[1]
            ),
        );
        return;
    }
    let bytes = unsafe { std::slice::from_raw_parts(data, length) };
    if let Err(error) = c.slot.deliver(c.epoch, width, height, stride, bytes) {
        c.slot.phase(c.epoch, &format!("Error: {error}"));
    }
}
#[unsafe(no_mangle)]
unsafe extern "C" fn rastera_capture_state(context: *mut c_void, message: *const c_char) {
    let c = unsafe { &*context.cast::<Context>() };
    c.slot.phase(
        c.epoch,
        &unsafe { CStr::from_ptr(message) }.to_string_lossy(),
    );
}
#[unsafe(no_mangle)]
unsafe extern "C" fn rastera_capture_negotiated(
    context: *mut c_void,
    _width: u32,
    _height: u32,
    fps: f64,
) {
    let c = unsafe { &*context.cast::<Context>() };
    c.slot.negotiated(c.epoch, fps);
}
#[unsafe(no_mangle)]
unsafe extern "C" fn rastera_capture_release(context: *mut c_void) {
    drop(unsafe { Box::from_raw(context.cast::<Context>()) });
}
struct Session {
    handle: NonNull<c_void>,
    source: Source,
    slot: Arc<Slot>,
}
impl Drop for Session {
    fn drop(&mut self) {
        let epoch = self.slot.reset();
        self.slot.phase(epoch, "Disconnected");
        unsafe { rastera_capture_destroy(self.handle.as_ptr()) };
    }
}
#[derive(Clone, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub model: String,
    pub modes: Vec<Mode>,
}
pub struct Capture {
    hub: Arc<Hub>,
    sessions: BTreeMap<String, Session>,
    slots: BTreeMap<String, Arc<Slot>>,
    pub devices: Vec<Device>,
    pub dialog: bool,
    pub error: Option<String>,
    pub target: Option<Id>,
    chosen: Option<String>,
    mode: usize,
    refreshed: Instant,
}
impl Capture {
    pub fn new(hub: Arc<Hub>) -> Self {
        Self {
            hub,
            sessions: BTreeMap::new(),
            slots: BTreeMap::new(),
            devices: vec![],
            dialog: false,
            error: None,
            target: None,
            chosen: None,
            mode: 0,
            refreshed: Instant::now() - Duration::from_secs(10),
        }
    }
    pub fn edit(&mut self, id: Id, source: &Source) {
        self.target = Some(id);
        self.request();
        self.chosen = Some(source.device_id.clone());
        self.mode = self
            .devices
            .iter()
            .find(|d| d.id == source.device_id)
            .and_then(|d| d.modes.iter().position(|m| m == &source.mode))
            .unwrap_or(0);
    }
    pub fn request(&mut self) {
        self.dialog = true;
        if !unsafe { rastera_camera_bundle_valid() } || !unsafe { rastera_camera_request() } {
            self.error=Some("Camera access requires Rastera.app. Run tools/build-app.sh, then open target/Rastera.app.".into());
        } else {
            self.error = None;
        }
        self.refresh();
    }
    pub fn refresh(&mut self) {
        self.refreshed = Instant::now();
        let raw = unsafe { rastera_capture_devices() };
        if raw.is_null() {
            self.error = Some("Capture discovery failed".into());
            return;
        }
        let result = serde_json::from_slice(unsafe { CStr::from_ptr(raw) }.to_bytes());
        unsafe { rastera_editor_free_string(raw) };
        match result {
            Ok(devices) => self.devices = devices,
            Err(e) => self.error = Some(format!("Capture discovery: {e}")),
        }
    }
    pub fn reconcile(&mut self, doc: &Document) {
        if doc.captures.is_empty() {
            for key in self.slots.keys().cloned().collect::<Vec<_>>() {
                self.sessions.remove(&key);
                self.hub.retire(&key);
            }
            self.slots.clear();
            return;
        }
        if self.refreshed.elapsed() > Duration::from_secs(1) {
            self.refresh();
        }
        let desired:BTreeMap<_,_>=doc.captures.iter().filter(|(id,_)|doc.layers.iter().any(|l|matches!(l.content,rastera_core::scene::Content::Capture(source) if source==**id))).map(|(&id,c)|(Hub::key(&doc.project_id,id),c.clone())).collect();
        let retired: Vec<_> = self
            .sessions
            .iter()
            .filter(|(key, session)| {
                desired.get(*key) != Some(&session.source)
                    || !self
                        .devices
                        .iter()
                        .any(|d| d.id == session.source.device_id)
            })
            .map(|(k, _)| k.clone())
            .collect();
        for key in retired {
            self.sessions.remove(&key);
        }
        let stale: Vec<_> = self
            .slots
            .keys()
            .filter(|key| !desired.contains_key(*key))
            .cloned()
            .collect();
        for key in stale {
            self.hub.retire(&key);
            self.slots.remove(&key);
        }
        for key in desired.keys() {
            self.slots
                .entry(key.clone())
                .or_insert_with(|| self.hub.slot(key));
        }
        let mut claimed: BTreeMap<String, String> = self
            .sessions
            .iter()
            .map(|(key, session)| (session.source.device_id.clone(), key.clone()))
            .collect();
        for (key, source) in &desired {
            let slot = self.hub.slot(key);
            let auth = unsafe { rastera_camera_authorization() };
            let unavailable = if !unsafe { rastera_camera_bundle_valid() } {
                Some("Permission Required: launch Rastera.app")
            } else if auth == 0 {
                Some("Permission Required")
            } else if auth != 3 {
                Some("Permission Denied / Restricted")
            } else if !self.devices.iter().any(|d| d.id == source.device_id) {
                Some("Disconnected")
            } else if claimed
                .get(&source.device_id)
                .is_some_and(|owner| owner != key)
            {
                Some("Error: another source definition already owns this device; duplicate its layer instead")
            } else {
                claimed.insert(source.device_id.clone(), key.clone());
                None
            };
            if let Some(phase) = unavailable {
                self.sessions.remove(key);
                let epoch = slot.epoch.load(std::sync::atomic::Ordering::Acquire);
                slot.phase(epoch, phase);
                continue;
            }
            if self.sessions.contains_key(key) {
                continue;
            }
            let epoch = slot.reset();
            let context = Box::into_raw(Box::new(Context {
                slot: slot.clone(),
                epoch,
                expected: [source.mode.width, source.mode.height],
            }));
            let id = CString::new(source.device_id.clone()).unwrap();
            let handle = unsafe {
                rastera_capture_create(
                    context.cast(),
                    id.as_ptr(),
                    source.mode.width,
                    source.mode.height,
                    source.mode.fps,
                )
            };
            if let Some(handle) = NonNull::new(handle) {
                self.sessions.insert(
                    key.clone(),
                    Session {
                        handle,
                        source: source.clone(),
                        slot,
                    },
                );
            } else {
                unsafe { rastera_capture_release(context.cast()) };
                slot.phase(epoch, "Error: native capture initialization failed");
            }
        }
    }
    pub fn active_count(&self) -> usize {
        self.sessions.len()
    }
    pub fn authorization(&self) -> i32 {
        unsafe { rastera_camera_authorization() }
    }
    pub fn status(&self, doc: &Document, id: Id) -> rastera_core::capture::Status {
        self.hub.slot(&Hub::key(&doc.project_id, id)).status()
    }
    pub fn retry(&mut self, doc: &Document, id: Id) {
        self.sessions.remove(&Hub::key(&doc.project_id, id));
        self.refreshed = Instant::now() - Duration::from_secs(10);
    }
    pub fn dialog_ui(&mut self, ctx: &eframe::egui::Context) -> Option<(Option<Id>, Source)> {
        if !self.dialog {
            return None;
        }
        use eframe::egui;
        let mut selection = None;
        let mut close = false;
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("capture_devices"),egui::ViewportBuilder::default().with_title("Rastera — Add Capture Device").with_inner_size([620.,560.]).with_window_level(egui::WindowLevel::AlwaysOnTop),|ctx,_|{
      egui::CentralPanel::default().show(ctx,|ui|{
        ui.heading("Capture Device · video only");ui.label("Up to two distinct source definitions · SDR BGRA · no audio");
        if let Some(error)=&self.error{ui.colored_label(egui::Color32::YELLOW,error);}
        let auth=unsafe{rastera_camera_authorization()};if auth!=3{ui.label(if auth==0{"Permission Required — allow Rastera camera access"}else{"Permission Denied/Restricted — System Settings → Privacy & Security → Camera → Rastera"});if ui.button("Request camera access").clicked(){self.request();}}
        if ui.button("Refresh devices").clicked(){self.refresh();}
        egui::ScrollArea::vertical().max_height(190.).show(ui,|ui|{for d in &self.devices{if ui.selectable_label(self.chosen.as_ref()==Some(&d.id),format!("{} · {}",d.name,d.model)).on_hover_text(&d.id).clicked(){self.chosen=Some(d.id.clone());self.mode=0;}}});
        if let Some(device)=self.devices.iter().find(|d|Some(&d.id)==self.chosen.as_ref()){
            egui::ComboBox::from_label("Supported resolution / fps").selected_text(device.modes.get(self.mode).map(|m|format!("{} × {} @ {:.3}",m.width,m.height,m.fps)).unwrap_or_else(||"No supported SDR BGRA modes".into())).show_ui(ui,|ui|{for (index,m) in device.modes.iter().enumerate(){ui.selectable_value(&mut self.mode,index,format!("{} × {} @ {:.3} fps",m.width,m.height,m.fps));}});
            if ui.add_enabled(auth==3&&device.modes.get(self.mode).is_some(),egui::Button::new(if self.target.is_some(){"Apply source assignment"}else{"Add source to canvas"})).clicked(){selection=Some((self.target,Source{device_id:device.id.clone(),name:device.name.clone(),mode:device.modes[self.mode].clone()}));close=true;}
        }else if self.devices.is_empty(){ui.label("No AVFoundation video devices found. Missing sources remain saved without substitution.");}
        if ui.button("Cancel").clicked(){close=true;}
      });if ctx.input(|i|i.viewport().close_requested()){close=true;}
    });
        if close {
            self.dialog = false;
        }
        selection
    }
    pub fn shutdown(&mut self) {
        self.sessions.clear();
        unsafe { rastera_capture_drain() };
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.shutdown();
    }
}
