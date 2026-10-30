//! Bounded file playback services; all decode and AVPlayer calls run outside UI/producer.
use rastera_core::{
    media::{self, Asset, Control},
    scene::{Document, Id},
};
use std::{
    collections::BTreeMap,
    ffi::{c_char, c_void, CStr, CString},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
unsafe extern "C" {
    fn rastera_video_probe(path: *const c_char) -> *mut c_char;
    fn rastera_editor_free_string(p: *mut c_char);
    fn rastera_video_create(path: *const c_char) -> *mut c_void;
    fn rastera_video_step(
        h: *mut c_void,
        c: *mut c_void,
        play: bool,
        seek: f64,
        pos: *mut f64,
    ) -> i32;
    fn rastera_video_destroy(h: *mut c_void);
}
pub fn inspect(path: &Path) -> Result<Asset, String> {
    let path =
        std::fs::canonicalize(path).map_err(|e| format!("Cannot open {}: {e}", path.display()))?;
    let a = if path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"))
    {
        media::inspect_gif(&path)?
    } else {
        let p = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        unsafe {
            let json = rastera_video_probe(p.as_ptr());
            if json.is_null() {
                return Err("Video probe failed".into());
            }
            let v: serde_json::Value = serde_json::from_slice(CStr::from_ptr(json).to_bytes())
                .map_err(|e| e.to_string())?;
            rastera_editor_free_string(json);
            if let Some(e) = v["error"].as_str() {
                return Err(e.into());
            }
            serde_json::from_value(v).map_err(|e| e.to_string())?
        }
    };
    media::validate_asset(&a)?;
    Ok(a)
}
struct Worker {
    asset: Asset,
    settings: media::Playback,
    control: Arc<Control>,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(j) = self.join.take() {
            let _ = j.join();
        }
    }
}
pub struct Media {
    hub: Arc<rastera_core::capture::Hub>,
    workers: BTreeMap<String, Worker>,
}
impl Media {
    pub fn new(hub: Arc<rastera_core::capture::Hub>) -> Self {
        Self {
            hub,
            workers: BTreeMap::new(),
        }
    }
    pub fn control(&self, d: &Document, id: Id) -> Option<Arc<Control>> {
        self.workers
            .get(&media::key(&d.project_id, id))
            .map(|w| w.control.clone())
    }
    pub fn reconcile(&mut self, d: &Document) {
        let wanted: BTreeMap<_, _> = d
            .playbacks
            .iter()
            .map(|(&id, p)| {
                (
                    media::key(&d.project_id, id),
                    (p.clone(), d.media_assets[&p.asset].clone()),
                )
            })
            .collect();
        self.workers.retain(|key, w| {
            wanted.get(key).is_some_and(|(p, a)| {
                *a == w.asset && p.looping == w.settings.looping && p.end == w.settings.end
            })
        });
        for (key, (settings, asset)) in wanted {
            if let Some(w) = self.workers.get_mut(&key) {
                if w.settings.playing != settings.playing {
                    w.control.state.lock().unwrap().playing = settings.playing;
                    w.settings.playing = settings.playing;
                }
                continue;
            }
            let control = Arc::new(Control::new(
                self.hub.slot(&key),
                settings.playing,
                asset.duration,
            ));
            let stop = Arc::new(AtomicBool::new(false));
            let c = control.clone();
            let done = stop.clone();
            let a = asset.clone();
            let p = settings.clone();
            let join = thread::spawn(move || {
                if a.gif {
                    media::run_gif(a, c, done, p.looping, p.end);
                } else {
                    run_video(a, c, done, p.looping.unwrap_or(false), p.end);
                }
            });
            self.workers.insert(
                key,
                Worker {
                    asset,
                    settings,
                    control,
                    stop,
                    join: Some(join),
                },
            );
        }
    }
}
#[unsafe(no_mangle)]
unsafe extern "C" fn rastera_media_pixels(
    context: *mut c_void,
    w: u32,
    h: u32,
    stride: usize,
    data: *const u8,
    len: usize,
) {
    let (control, expected, epoch) = unsafe { &*context.cast::<(Arc<Control>, [u32; 2], u64)>() };
    if data.is_null() || len > 32 * 1024 * 1024 || [w, h] != *expected {
        control.state.lock().unwrap().phase = "Error: unexpected decoded video dimensions".into();
        return;
    }
    let bytes = unsafe { std::slice::from_raw_parts(data, len) };
    if let Ok(bgra) = rastera_core::capture::pack_bgra(w, h, stride, bytes) {
        control.slot.deliver_owned(*epoch, w, h, bgra);
    }
}
fn run_video(
    asset: Asset,
    control: Arc<Control>,
    stop: Arc<AtomicBool>,
    looping: bool,
    end: media::End,
) {
    let Ok(path) = CString::new(asset.path.to_string_lossy().as_bytes()) else {
        return;
    };
    let epoch = control.slot.reset();
    let handle = unsafe { rastera_video_create(path.as_ptr()) };
    if handle.is_null() {
        return;
    }
    let mut ctx = (control.clone(), asset.size, epoch);
    let mut ended = false;
    while !stop.load(Ordering::Relaxed) {
        let (play, mut seek) = {
            let mut s = control.state.lock().unwrap();
            (s.playing, s.seek.take())
        };
        if seek.is_some() {
            ended = false;
        }
        let position = control.state.lock().unwrap().position;
        if position >= asset.duration - 0.025 && play && !ended {
            if looping {
                seek = Some(0.);
            } else {
                ended = true;
                control.state.lock().unwrap().playing = false;
                if end == media::End::Black {
                    control.slot.phase(epoch, "Ended · Black");
                }
            }
        }
        let mut pos = 0.;
        let status = unsafe {
            rastera_video_step(
                handle,
                (&mut ctx as *mut (Arc<Control>, [u32; 2], u64)).cast(),
                play && !ended,
                seek.unwrap_or(-1.),
                &mut pos,
            )
        };
        {
            let mut s = control.state.lock().unwrap();
            if pos.is_finite() {
                s.position = pos.max(0.);
            }
            if status < 0 {
                s.phase = "Error: AVFoundation playback failed".into();
                control.slot.phase(epoch, "Error");
            } else {
                s.phase = if ended {
                    "Ended"
                } else if status == 0 {
                    "Loading"
                } else if play {
                    "Playing · muted"
                } else {
                    "Paused"
                }
                .into();
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    unsafe { rastera_video_destroy(handle) };
}
