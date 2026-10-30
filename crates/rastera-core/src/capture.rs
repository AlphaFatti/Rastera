//! Reusable capture definitions and a bounded CPU latest-frame handoff. No AVFoundation/UI.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
pub const MAX_SOURCES: usize = 2;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}
impl Mode {
    pub fn validate(&self) -> Result<(), String> {
        if self.width == 0
            || self.height == 0
            || self.width > 1920
            || self.height > 1920
            || u64::from(self.width) * u64::from(self.height) > 1920 * 1080
            || !self.fps.is_finite()
            || !(1.0..=60.0).contains(&self.fps)
        {
            Err("Capture supports SDR BGRA up to 1920×1080 (either orientation), 1–60 fps".into())
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub device_id: String,
    pub name: String,
    pub mode: Mode,
}
impl Source {
    pub fn validate(&self) -> Result<(), String> {
        if self.device_id.is_empty()
            || self.device_id.len() > 1024
            || self.device_id.contains('\0')
            || self.name.len() > 512
        {
            return Err("Invalid capture device reference".into());
        }
        self.mode.validate()
    }
}
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub sequence: u64,
    pub arrived: Instant,
    pub bgra: Vec<u8>,
}
pub fn pack_bgra(width: u32, height: u32, stride: usize, bytes: &[u8]) -> Result<Vec<u8>, String> {
    Mode {
        width,
        height,
        fps: 30.,
    }
    .validate()?;
    let row = width as usize * 4;
    let required = stride
        .checked_mul(height as usize)
        .ok_or("Capture stride overflow")?;
    if stride < row || bytes.len() < required || required > 32 * 1024 * 1024 {
        return Err("Invalid BGRA row stride or buffer length".into());
    }
    let mut packed = Vec::with_capacity(row * height as usize);
    for source in bytes[..required].chunks_exact(stride) {
        packed.extend_from_slice(&source[..row]);
    }
    // Camera BGRA is opaque. Ignore any unspecified device alpha bytes.
    for alpha in packed.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }
    Ok(packed)
}
struct State {
    latest: Option<Arc<Frame>>,
    phase: String,
    first: Option<Instant>,
    reset_at: Instant,
    frames: u64,
    replaced: u64,
    actual: [u32; 2],
    negotiated_fps: f64,
}
pub struct Slot {
    state: Mutex<State>,
    pub epoch: AtomicU64,
    uploaded: AtomicU64,
}
#[derive(Clone, Debug)]
pub struct Status {
    pub phase: String,
    pub frames: u64,
    pub replaced: u64,
    pub actual: [u32; 2],
    pub cadence: f64,
    pub uploaded: u64,
    pub negotiated_fps: f64,
}
impl Slot {
    fn new() -> Self {
        Self {
            state: Mutex::new(State {
                latest: None,
                phase: "Permission Required".into(),
                first: None,
                reset_at: Instant::now(),
                frames: 0,
                replaced: 0,
                actual: [0, 0],
                negotiated_fps: 0.,
            }),
            epoch: AtomicU64::new(0),
            uploaded: AtomicU64::new(0),
        }
    }
    pub fn reset(&self) -> u64 {
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel) + 1;
        let mut s = self.state.lock().unwrap();
        s.latest = None;
        s.phase = "Starting".into();
        s.first = None;
        s.reset_at = Instant::now();
        s.frames = 0;
        s.replaced = 0;
        s.actual = [0, 0];
        s.negotiated_fps = 0.;
        self.uploaded.store(0, Ordering::Relaxed);
        epoch
    }
    pub fn phase(&self, epoch: u64, message: &str) {
        let mut s = self.state.lock().unwrap();
        if self.epoch.load(Ordering::Acquire) != epoch {
            return;
        }
        s.phase = message.into();
        if message != "Live" {
            s.latest = None;
        }
    }
    pub fn deliver(
        &self,
        epoch: u64,
        width: u32,
        height: u32,
        stride: usize,
        bytes: &[u8],
    ) -> Result<(), String> {
        if self.epoch.load(Ordering::Acquire) != epoch {
            return Ok(());
        }
        let pixels = pack_bgra(width, height, stride, bytes)?;
        let now = Instant::now();
        let mut s = self.state.lock().unwrap();
        if self.epoch.load(Ordering::Acquire) != epoch {
            return Ok(());
        }
        s.frames += 1;
        if s.latest.is_some() {
            s.replaced += 1;
        }
        let sequence = s.frames;
        s.first.get_or_insert(now);
        s.actual = [width, height];
        s.phase = "Live".into();
        s.latest = Some(Arc::new(Frame {
            width,
            height,
            sequence,
            arrived: now,
            bgra: pixels,
        }));
        Ok(())
    }
    /// Producer never waits for the callback/manager mutex; a busy handoff preserves the last GPU input for this frame.
    pub fn snapshot(&self) -> Option<Option<Arc<Frame>>> {
        let s = self.state.try_lock().ok()?;
        Some(if s.phase == "Live" {
            s.latest
                .as_ref()
                .filter(|f| f.arrived.elapsed() < Duration::from_secs(2))
                .cloned()
        } else {
            None
        })
    }
    /// File-media frames remain valid while paused, held at end, or on a long GIF delay.
    pub fn persistent_snapshot(&self) -> Option<Option<Arc<Frame>>> {
        Some(self.state.try_lock().ok()?.latest.clone())
    }
    pub fn deliver_owned(&self, epoch: u64, width: u32, height: u32, bgra: Vec<u8>) {
        if bgra.len() != width as usize * height as usize * 4 {
            return;
        }
        let mut s = self.state.lock().unwrap();
        if self.epoch.load(Ordering::Acquire) != epoch {
            return;
        }
        s.frames += 1;
        let sequence = s.frames;
        s.phase = "Live".into();
        s.actual = [width, height];
        s.latest = Some(Arc::new(Frame {
            width,
            height,
            bgra,
            sequence,
            arrived: Instant::now(),
        }));
    }
    pub fn latest(&self) -> Option<Arc<Frame>> {
        self.snapshot().flatten()
    }
    pub fn negotiated(&self, epoch: u64, fps: f64) {
        let mut s = self.state.lock().unwrap();
        if self.epoch.load(Ordering::Acquire) == epoch {
            s.negotiated_fps = fps;
        }
    }
    pub fn uploaded(&self) {
        self.uploaded.fetch_add(1, Ordering::Relaxed);
    }
    pub fn status(&self) -> Status {
        let s = self.state.lock().unwrap();
        let stale = (s.phase == "Starting"
            && s.first.is_none()
            && s.reset_at.elapsed() > Duration::from_secs(5))
            || (s.phase == "Live"
                && s.latest
                    .as_ref()
                    .is_none_or(|f| f.arrived.elapsed() > Duration::from_secs(2)));
        let phase = if stale {
            "No Signal".into()
        } else {
            s.phase.clone()
        };
        Status {
            phase,
            frames: s.frames,
            replaced: s.replaced,
            actual: s.actual,
            cadence: s
                .first
                .map(|t| s.frames.saturating_sub(1) as f64 / t.elapsed().as_secs_f64().max(0.001))
                .unwrap_or(0.),
            uploaded: self.uploaded.load(Ordering::Relaxed),
            negotiated_fps: s.negotiated_fps,
        }
    }
}
#[derive(Default)]
pub struct Hub {
    slots: Mutex<BTreeMap<String, std::sync::Weak<Slot>>>,
}
impl Hub {
    pub fn key(project: &str, id: u64) -> String {
        format!("{project}:{id}")
    }
    pub fn slot(&self, key: &str) -> Arc<Slot> {
        let mut slots = self.slots.lock().unwrap();
        slots.retain(|_, slot| slot.strong_count() > 0);
        if let Some(slot) = slots.get(key).and_then(std::sync::Weak::upgrade) {
            return slot;
        }
        let slot = Arc::new(Slot::new());
        slots.insert(key.into(), Arc::downgrade(&slot));
        slot
    }
    pub fn retire(&self, key: &str) {
        if let Some(slot) = self
            .slots
            .lock()
            .unwrap()
            .remove(key)
            .and_then(|slot| slot.upgrade())
        {
            let epoch = slot.reset();
            slot.phase(epoch, "Disconnected");
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn padded_bgra_stride_channel_order_and_validation() {
        assert_eq!(
            pack_bgra(
                1,
                2,
                8,
                &[10, 20, 30, 0, 1, 2, 3, 4, 40, 50, 60, 0, 1, 2, 3, 4]
            )
            .unwrap(),
            [10, 20, 30, 255, 40, 50, 60, 255]
        );
        assert!(pack_bgra(2, 1, 4, &[0; 8]).is_err());
        assert!(pack_bgra(1920, 1080, 7680, &[]).is_err());
        assert!(Mode {
            width: 3840,
            height: 2160,
            fps: 30.
        }
        .validate()
        .is_err());
        assert!(Mode {
            width: 1280,
            height: 720,
            fps: f64::NAN
        }
        .validate()
        .is_err());
    }
    #[test]
    fn replacement_epochs_and_retirement_hold_owned_frames() {
        let hub = Hub::default();
        let slot = hub.slot("p:1");
        assert!(Arc::ptr_eq(&slot, &hub.slot("p:1")));
        let epoch = slot.reset();
        for x in 0..10 {
            slot.deliver(epoch, 1, 1, 4, &[x, 0, 0, 255]).unwrap();
        }
        let old = slot.latest().unwrap();
        assert_eq!(old.bgra[0], 9);
        assert_eq!(slot.status().replaced, 9);
        let next = slot.reset();
        slot.deliver(epoch, 1, 1, 4, &[44, 0, 0, 255]).unwrap();
        assert!(slot.latest().is_none());
        slot.deliver(next, 1, 1, 4, &[42, 0, 0, 255]).unwrap();
        assert_eq!(old.bgra[0], 9);
        hub.retire("p:1");
        assert!(slot.latest().is_none());
        assert!(!Arc::ptr_eq(&slot, &hub.slot("p:1")));
    }
}

#[cfg(test)]
mod document_tests {
    use super::*;
    use crate::scene::{Content, Document, History};
    fn source() -> Source {
        Source {
            device_id: "missing-but-preserved".into(),
            name: "Camera".into(),
            mode: Mode {
                width: 1280,
                height: 720,
                fps: 30.,
            },
        }
    }
    #[test]
    fn ordinary_instances_share_source_and_final_reference_is_pruned() {
        let mut doc = Document::default();
        let layer = doc.add_capture(source()).unwrap();
        let id = if let Content::Capture(id) = doc.layer(layer).unwrap().content {
            id
        } else {
            panic!()
        };
        let copy = doc.duplicate(layer).unwrap();
        assert_eq!(doc.captures.len(), 1);
        assert!(matches!(doc.layer(copy).unwrap().content,Content::Capture(c) if c==id));
        assert_eq!(doc.aspect(copy), Some(1280. / 720.));
        doc.layers.retain(|l| l.id != layer);
        doc.prune_unused();
        assert_eq!(doc.captures.len(), 1);
        let before = doc.clone();
        doc.layers.retain(|l| l.id != copy);
        doc.prune_unused();
        assert!(doc.captures.is_empty());
        let mut history = History::default();
        history.record(before.clone(), &doc);
        assert!(history.undo(&mut doc));
        assert_eq!(doc.captures, before.captures);
        assert!(doc.validate().is_ok());
        assert!(history.redo(&mut doc));
        assert!(doc.captures.is_empty());
    }
    #[test]
    fn capture_migration_missing_device_roundtrip_and_limits() {
        let folder =
            std::env::temp_dir().join(format!("rastera-capture-schema-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("show.json");
        let original = Document::default();
        let mut legacy = crate::scenes::legacy_value(&original);
        legacy["version"] = 4.into();
        legacy.as_object_mut().unwrap().remove("captures");
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let mut doc = crate::scene::load_project(&path).unwrap();
        assert_eq!(doc.version, 9);
        assert!(doc.captures.is_empty());
        assert_eq!(doc.project_id, original.project_id);
        doc.add_capture(source()).unwrap();
        crate::scene::save_project(&doc, &path).unwrap();
        assert_eq!(crate::scene::load_project(&path).unwrap(), doc);
        doc.add_capture(source()).unwrap();
        assert!(doc.add_capture(source()).is_err());
        let id = *doc.captures.keys().next().unwrap();
        doc.captures.remove(&id);
        assert!(doc.validate().is_err());
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn stale_and_failed_sources_blacken_without_waiting() {
        let slot = Slot::new();
        let epoch = slot.reset();
        slot.deliver(epoch, 1, 1, 4, &[1, 2, 3, 255]).unwrap();
        {
            let mut s = slot.state.lock().unwrap();
            s.latest = Some(Arc::new(Frame {
                width: 1,
                height: 1,
                sequence: 1,
                arrived: Instant::now() - Duration::from_secs(3),
                bgra: vec![1, 2, 3, 255],
            }));
        }
        assert_eq!(slot.status().phase, "No Signal");
        assert!(slot.latest().is_none());
        slot.phase(epoch, "Disconnected");
        assert!(slot.latest().is_none());
        let guard = slot.state.lock().unwrap();
        assert!(slot.snapshot().is_none());
        drop(guard);
    }
}

#[cfg(test)]
mod assignment_tests {
    use super::*;
    use crate::scene::{Content, Document};
    #[test]
    fn reassignment_keeps_identity_and_fits_unlocked_instances() {
        let mut doc = Document::default();
        let source = Source {
            device_id: "first".into(),
            name: "First".into(),
            mode: Mode {
                width: 1280,
                height: 720,
                fps: 30.,
            },
        };
        let layer = doc.add_capture(source.clone()).unwrap();
        let id = if let Content::Capture(id) = doc.layer(layer).unwrap().content {
            id
        } else {
            panic!()
        };
        let copy = doc.duplicate(layer).unwrap();
        doc.layer_mut(copy).unwrap().locked = true;
        let locked = doc.layer(copy).unwrap().rect;
        let before = doc.layer(layer).unwrap().rect;
        let mut next = source;
        next.device_id = "missing-exact-target".into();
        next.mode = Mode {
            width: 640,
            height: 480,
            fps: 30.,
        };
        doc.assign_capture(id, next).unwrap();
        let rect = doc.layer(layer).unwrap().rect;
        assert_eq!(rect.width / rect.height, 4. / 3.);
        assert_eq!(
            [rect.x + rect.width / 2., rect.y + rect.height / 2.],
            [before.x + before.width / 2., before.y + before.height / 2.]
        );
        assert_eq!(doc.layer(copy).unwrap().rect, locked);
        assert_eq!(doc.captures.len(), 1);
        assert!(doc.validate().is_ok());
    }
}
