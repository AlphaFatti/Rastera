//! Native adapter ownership. These wrappers run on control/decode workers, never an IO callback.
use serde::{Deserialize, Serialize};
use std::ffi::{c_char, c_void, CStr, CString};
unsafe extern "C" {
    fn rastera_audio_devices() -> *mut c_char;
    fn rastera_audio_free_string(p: *mut c_char);
    fn rastera_audio_probe(path: *const c_char) -> *mut c_char;
    fn rastera_audio_permission(request: bool) -> i32;
    fn rastera_audio_reader_create(path: *const c_char, track: i32) -> *mut c_void;
    fn rastera_audio_reader_seek(h: *mut c_void, seconds: f64) -> bool;
    fn rastera_audio_reader_read(
        h: *mut c_void,
        out: *mut f32,
        cap: u32,
        ch: *mut u32,
        pts: *mut f64,
    ) -> i32;
    fn rastera_audio_reader_destroy(h: *mut c_void);
    fn rastera_audio_device_open(uid: *const c_char, input: bool) -> *mut c_void;
    fn rastera_audio_device_close(h: *mut c_void);
    fn rastera_audio_device_write(
        h: *mut c_void,
        host: f64,
        data: *const f32,
        frames: u32,
        ch: u32,
    ) -> u32;
    fn rastera_audio_device_read_48k(
        h: *mut c_void,
        host: *mut f64,
        data: *mut f32,
        frames: u32,
        ch: u32,
    ) -> u32;
    fn rastera_audio_device_stats(h: *mut c_void, values: *mut u64);
    fn rastera_audio_device_details(h: *mut c_void) -> *mut c_char;
    #[allow(dead_code)] // Offline native SRC diagnostics, not the running editor.
    fn rastera_audio_resample_measure(rate: f64, hz: f64, impulse: bool) -> *mut c_char;
    fn rastera_audio_host_time() -> f64;
    fn rastera_audio_emergency(mute: bool);
    fn rastera_audio_is_muted() -> bool;
    fn rastera_audio_shortcut();
}
pub fn host() -> f64 {
    unsafe { rastera_audio_host_time() }
}
fn take_json<T: serde::de::DeserializeOwned>(ptr: *mut c_char) -> Result<T, String> {
    if ptr.is_null() {
        return Err("Native audio adapter returned no metadata".into());
    }
    let result = serde_json::from_slice(unsafe { CStr::from_ptr(ptr) }.to_bytes())
        .map_err(|e| e.to_string());
    unsafe { rastera_audio_free_string(ptr) };
    result
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct DeviceInfo {
    pub uid: String,
    pub name: String,
    pub rate: f64,
    pub buffer_frames: u32,
    pub output_channels: u32,
    pub input_channels: u32,
    pub output_names: Vec<String>,
    #[serde(default)]
    pub input_names: Vec<String>,
    pub latency_frames: u32,
}
pub fn devices() -> Result<Vec<DeviceInfo>, String> {
    take_json(unsafe { rastera_audio_devices() })
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Track {
    pub id: i32,
    pub channels: u32,
    pub rate: f64,
    pub start: f64,
    pub duration: f64,
    pub language: String,
    #[serde(default)]
    pub segments: Vec<TrackSegment>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TrackSegment {
    pub empty: bool,
    pub source_start: Option<f64>,
    pub source_duration: Option<f64>,
    pub target_start: Option<f64>,
    pub target_duration: Option<f64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Probe {
    pub tracks: Vec<Track>,
    pub duration: f64,
}
pub fn probe(path: &std::path::Path) -> Result<Probe, String> {
    let p = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
    take_json(unsafe { rastera_audio_probe(p.as_ptr()) })
}
pub fn permission(request: bool) -> i32 {
    unsafe { rastera_audio_permission(request) }
}
pub struct Reader(*mut c_void);
// SAFETY: created and used by one decode worker. AVAssetReader is never shared concurrently.
unsafe impl Send for Reader {}
impl Reader {
    pub fn new(path: &std::path::Path, track: i32) -> Result<Self, String> {
        let p = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let h = unsafe { rastera_audio_reader_create(p.as_ptr(), track) };
        if h.is_null() {
            Err(format!("Cannot decode track {track}: {}", path.display()))
        } else {
            Ok(Self(h))
        }
    }
    pub fn seek(&mut self, seconds: f64) -> bool {
        unsafe { rastera_audio_reader_seek(self.0, seconds) }
    }
    pub fn read(&mut self, data: &mut [f32; 128 * 32]) -> Result<(usize, usize, f64), String> {
        let (mut channels, mut pts) = (0, 0.);
        let n = unsafe {
            rastera_audio_reader_read(self.0, data.as_mut_ptr(), 128, &mut channels, &mut pts)
        };
        if n < 0 {
            Err(format!("PCM decode failed ({n})"))
        } else {
            Ok((n as usize, channels as usize, pts))
        }
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        unsafe { rastera_audio_reader_destroy(self.0) }
    }
}
pub struct Device {
    handle: *mut c_void,
    pub info: DeviceInfo,
}
// SAFETY: one render worker writes output; IOProc owns consumption. Lifecycle is joined before drop.
unsafe impl Send for Device {}
// SAFETY: backend exclusively assigns ring production/consumption to its render worker; stats are atomic.
unsafe impl Sync for Device {}
impl Device {
    pub fn new(info: DeviceInfo, input: bool) -> Result<Self, String> {
        let uid = CString::new(info.uid.as_bytes()).map_err(|e| e.to_string())?;
        let h = unsafe { rastera_audio_device_open(uid.as_ptr(), input) };
        if h.is_null() {
            Err(format!(
                "{}: unavailable, unauthorized or unsupported non-Float32 format",
                info.name
            ))
        } else {
            Ok(Self { handle: h, info })
        }
    }
    pub fn write(&self, host: f64, data: &[f32], channels: usize) -> usize {
        if channels == 0 || channels > 32 {
            return 0;
        }
        let n = data.len() / channels;
        unsafe {
            rastera_audio_device_write(self.handle, host, data.as_ptr(), n as u32, channels as u32)
                as usize
        }
    }
    /// Returns interleaved 48 kHz PCM (hardware rate remains in info.rate).
    /// Native batches never bridge a capture timestamp discontinuity.
    pub fn read(&self, data: &mut [f32], channels: usize) -> (usize, f64) {
        if channels == 0 || channels > 32 {
            return (0, 0.);
        }
        let mut host = 0.;
        let n = unsafe {
            rastera_audio_device_read_48k(
                self.handle,
                &mut host,
                data.as_mut_ptr(),
                (data.len() / channels) as u32,
                channels as u32,
            )
        };
        (n as usize, host)
    }
    pub fn stats(&self) -> [u64; 6] {
        let mut stats = [0; 6];
        unsafe { rastera_audio_device_stats(self.handle, stats.as_mut_ptr()) };
        stats
    }
}
impl Drop for Device {
    fn drop(&mut self) {
        unsafe { rastera_audio_device_close(self.handle) }
    }
}

pub fn emergency(mute: bool) {
    unsafe { rastera_audio_emergency(mute) }
}
pub fn is_muted() -> bool {
    unsafe { rastera_audio_is_muted() }
}
pub fn shortcut() {
    unsafe { rastera_audio_shortcut() }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceDetails {
    pub interval_start_host: f64,
    pub interval_end_host: f64,
    pub interval_max_callback_gap_seconds: f64,
    pub interval_max_update_misses: u64,
    pub output_frames: u64,
    pub input_frames: u64,
    pub content_frames: u64,
    pub output_peak: f64,
    pub output_clipped_samples: u64,
    pub output_missing_host_timestamps: u64,
    pub input_missing_host_timestamps: u64,
    pub zero_host_timestamps: u64,
    pub first_host: f64,
    pub last_host: f64,
    pub first_device_sample: f64,
    pub last_device_sample: f64,
    pub measured_clock_ppm: Option<f64>,
    pub clock_observation_coherent: bool,
    pub clock_discontinuities: u64,
    pub input_measured_clock_ppm: Option<f64>,
    pub input_clock_observation_coherent: bool,
    pub max_callback_gap_seconds: f64,
    pub format_changed: bool,
    pub capture: bool,
    pub reentrant_access: u64,
    pub converter_errors: u64,
    pub converter_resets: u64,
    pub converted_output_frames: u64,
    pub converter_read_ahead_48k_frames: u32,
    pub input_converter_errors: u64,
    pub input_converter_resets: u64,
    pub input_returned_48k_frames: u64,
    pub input_converter_read_ahead_device_frames: u32,
}
impl Device {
    pub fn details(&self) -> Result<DeviceDetails, String> {
        take_json(unsafe { rastera_audio_device_details(self.handle) })
    }
}

#[allow(dead_code)] // Used by the standalone audio-probe executable.
pub fn resample_measure(rate: f64, hz: f64, impulse: bool) -> Result<serde_json::Value, String> {
    take_json(unsafe { rastera_audio_resample_measure(rate, hz, impulse) })
}
