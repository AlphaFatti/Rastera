//! Bounded completed-frame handoff. Leases must outlive every consumer GPU read.
use crate::renderer::{canvas_texture, PatternGenerator, RenderStats};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

struct State<T> {
    slots: Vec<Arc<T>>,
    latest: Option<(usize, u64)>,
    meta: crate::mapping::FrameMeta,
    base_refs: usize,
}
pub struct FramePool<T> {
    state: Mutex<State<T>>,
}
pub struct FrameLease<T> {
    resource: Arc<T>,
    pub frame_id: u64,
    pub meta: crate::mapping::FrameMeta,
}
impl<T> FrameLease<T> {
    /// Borrow only. Native readers must keep the lease until their GPU commands finish.
    pub fn resource(&self) -> &T {
        &self.resource
    }
}
impl<T> FramePool<T> {
    pub fn new(slots: Vec<T>) -> Self {
        assert!(slots.is_empty() || slots.len() >= 2);
        Self {
            state: Mutex::new(State {
                slots: slots.into_iter().map(Arc::new).collect(),
                latest: None,
                meta: Default::default(),
                base_refs: 1,
            }),
        }
    }
    /// Reserve a non-current slot with no outstanding consumer/producer leases.
    fn reserve(&self) -> Option<(usize, Arc<T>)> {
        let state = self.state.lock().unwrap();
        state
            .slots
            .iter()
            .enumerate()
            .find(|(i, slot)| {
                state.latest.map(|v| v.0) != Some(*i) && Arc::strong_count(slot) == state.base_refs
            })
            .map(|(i, slot)| (i, slot.clone()))
    }
    /// Caller must have finished all GPU writes before publishing this reservation.
    #[cfg(test)]
    fn publish(&self, index: usize, frame_id: u64) {
        self.state.lock().unwrap().latest = Some((index, frame_id));
    }
    fn replace(&self, slots: Vec<Arc<T>>, base_refs: usize) {
        let mut s = self.state.lock().unwrap();
        s.slots = slots;
        s.base_refs = base_refs;
        s.latest = None;
    }
    pub fn acquire(&self) -> Option<FrameLease<T>> {
        let state = self.state.lock().unwrap();
        let (i, frame_id) = state.latest?;
        Some(FrameLease {
            resource: state.slots[i].clone(),
            frame_id,
            meta: state.meta.clone(),
        })
    }
    pub fn leased_slots(&self) -> usize {
        let state = self.state.lock().unwrap();
        state
            .slots
            .iter()
            .filter(|s| Arc::strong_count(s) > state.base_refs)
            .count()
    }
}

/// One producer, four GPU textures, one current-frame mailbox, no per-consumer queue.
pub struct SharedCanvasProducer {
    pub pool: Arc<FramePool<wgpu::Texture>>,
    pub stats: Arc<RenderStats>,
    pub skipped: Arc<AtomicU64>,
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}
impl SharedCanvasProducer {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        Self::new_inner(device, queue, None)
    }
    pub fn with_scene(
        device: wgpu::Device,
        queue: wgpu::Queue,
        feed: crate::composition::SceneFeed,
    ) -> Self {
        Self::new_inner(device, queue, Some(feed))
    }
    fn new_inner(
        device: wgpu::Device,
        queue: wgpu::Queue,
        mut scene: Option<crate::composition::SceneFeed>,
    ) -> Self {
        let pool = Arc::new(FramePool::new(if scene.is_some() {
            vec![]
        } else {
            (0..4).map(|_| canvas_texture(&device)).collect()
        }));
        let stats = Arc::new(RenderStats::default());
        let skipped = Arc::new(AtomicU64::new(0));
        let frames = pool.clone();
        let progress = stats.clone();
        let dropped = skipped.clone();
        let (stop, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let generator = PatternGenerator::new(&device);
            let origin = Instant::now();
            let period = Duration::from_nanos(1_000_000_000 / 60);
            let mut deadline = origin;
            let mut previous = None;
            let mut pending_meta = None;
            let mut active: Option<Arc<crate::composition::Generation>> = None;
            loop {
                let begin = Instant::now();
                if let Some(p) = previous {
                    progress.timings.record(0, p, begin);
                }
                previous = Some(begin);
                if let Some(feed) = scene.as_mut() {
                    if let Some((generation, meta)) = feed.begin_frame() {
                        if active
                            .as_ref()
                            .is_none_or(|old| !Arc::ptr_eq(old, &generation))
                        {
                            frames.replace(generation.textures.clone(), 2);
                            active = Some(generation);
                        }
                        // No callbacks see this metadata until publication below.
                        // Update metadata atomically WITH publication, not before encoding.
                        pending_meta = Some(meta);
                    }
                }
                if let Some((slot, texture)) = frames.reserve() {
                    let target = texture.create_view(&Default::default());
                    let encoder = if let Some(feed) = scene.as_mut() {
                        feed.encode(
                            &device,
                            &queue,
                            &target,
                            origin.elapsed().as_secs_f32(),
                            &generator,
                        )
                    } else {
                        generator.encode(&device, &queue, &target, origin.elapsed().as_secs_f32())
                    };
                    progress.timings.record(1, begin, Instant::now());
                    let start = Instant::now();
                    let command = encoder.finish();
                    progress.timings.record(2, start, Instant::now());
                    let start = Instant::now();
                    let submission = queue.submit([command]);
                    progress.timings.record(3, start, Instant::now());
                    progress.submitted.fetch_add(1, Ordering::Relaxed);
                    let start = Instant::now();
                    let _ = device.poll(wgpu::Maintain::WaitForSubmissionIndex(submission));
                    progress.timings.record(4, start, Instant::now());
                    let frame = progress.completed.fetch_add(1, Ordering::Relaxed) + 1;

                    // The reservation remains leased until after publication. Native readers
                    // acquire under the same mutex, and only after producer GPU completion.
                    {
                        let mut state = frames.state.lock().unwrap();
                        if let Some(meta) = pending_meta.take() {
                            state.meta = meta;
                        }
                        state.latest = Some((slot, frame));
                    }
                    if let Some(feed) = scene.as_ref() {
                        feed.frame_completed();
                    }
                } else {
                    dropped.fetch_add(1, Ordering::Relaxed);
                }
                deadline += period;
                if deadline < Instant::now() {
                    deadline = Instant::now() + period;
                }
                match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => break,
                }
            }
        });
        Self {
            pool,
            stats,
            skipped,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for SharedCanvasProducer {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_reader_must_finish_before_a_frame_slot_is_recycled() {
        let pool = FramePool::new(vec![0, 1, 2]);
        let (a, write) = pool.reserve().unwrap();
        pool.publish(a, 1);
        drop(write);
        let left = pool.acquire().unwrap();
        let right = pool.acquire().unwrap();
        let (b, write) = pool.reserve().unwrap();
        pool.publish(b, 2);
        drop(write);
        drop(left);
        let (c, write) = pool.reserve().unwrap();
        assert_ne!(a, c);
        drop(write);
        drop(right);
        let (free, _) = pool.reserve().unwrap();
        assert_eq!(free, a);
    }
    #[test]
    fn generation_owners_do_not_block_free_slots_or_hide_gpu_readers() {
        let generation: Vec<_> = (0..4).map(Arc::new).collect();
        let pool = FramePool::new(vec![]);
        pool.replace(generation.clone(), 2);
        let mut readers = vec![];
        for frame in 0..4 {
            let (i, write) = pool
                .reserve()
                .expect("generation-owned slot must be reusable");
            pool.publish(i, frame);
            drop(write);
            readers.push(pool.acquire().unwrap());
        }
        assert!(pool.reserve().is_none());
        assert_eq!(pool.leased_slots(), 4);
        readers.remove(0);
        let (free, _) = pool.reserve().unwrap();
        assert_eq!(free, 0);
        assert_eq!(readers[0].frame_id, 1);
    }
    #[test]
    fn generation_switch_keeps_readers_and_metadata_together() {
        let pool = FramePool::new(vec![10, 11, 12, 13]);
        let (i, w) = pool.reserve().unwrap();
        pool.publish(i, 1);
        drop(w);
        let old = pool.acquire().unwrap();
        let weak = Arc::downgrade(&old.resource);
        pool.replace(
            vec![Arc::new(20), Arc::new(21), Arc::new(22), Arc::new(23)],
            1,
        );
        assert!(pool.acquire().is_none());
        assert_eq!(*old.resource(), 10);
        assert!(weak.upgrade().is_some());
        let (i, w) = pool.reserve().unwrap();
        {
            let mut state = pool.state.lock().unwrap();
            state.meta.canvas = [1079, 1919];
            state.latest = Some((i, 2));
        }
        drop(w);
        let new = pool.acquire().unwrap();
        assert_eq!(*new.resource(), 20);
        assert_eq!(new.meta.canvas, [1079, 1919]);
        assert_eq!(old.meta.canvas, [3840, 1080]);
        drop(old);
        assert!(weak.upgrade().is_none());
    }
    #[test]
    fn outstanding_gpu_leases_prevent_reuse_and_bound_work() {
        let pool = FramePool::new(vec![0, 1, 2]);
        assert!(pool.acquire().is_none());
        let (a, write) = pool.reserve().unwrap();
        pool.publish(a, 1);
        drop(write);
        let first = pool.acquire().unwrap();
        let (b, write) = pool.reserve().unwrap();
        assert_ne!(a, b);
        pool.publish(b, 2);
        drop(write);
        let second = pool.acquire().unwrap();
        let (c, write) = pool.reserve().unwrap();
        pool.publish(c, 3);
        drop(write);
        assert!(pool.reserve().is_none()); // Two busy readers plus protected latest.
        assert_eq!(first.frame_id, 1);
        assert_eq!(second.frame_id, 2);
        drop(first); // Simulates the first consumer's command-completed handler.
        let (free, _) = pool.reserve().unwrap();
        assert_eq!(free, a);
        assert_eq!(pool.acquire().unwrap().frame_id, 3);
    }
}
