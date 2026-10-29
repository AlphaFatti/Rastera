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
}
pub struct FramePool<T> {
    state: Mutex<State<T>>,
}
pub struct FrameLease<T> {
    resource: Arc<T>,
    pub frame_id: u64,
}
impl<T> FrameLease<T> {
    /// Borrow only. Native readers must keep the lease until their GPU commands finish.
    pub fn resource(&self) -> &T {
        &self.resource
    }
}
impl<T> FramePool<T> {
    pub fn new(slots: Vec<T>) -> Self {
        assert!(slots.len() >= 2);
        Self {
            state: Mutex::new(State {
                slots: slots.into_iter().map(Arc::new).collect(),
                latest: None,
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
            .find(|(i, slot)| state.latest.map(|v| v.0) != Some(*i) && Arc::strong_count(slot) == 1)
            .map(|(i, slot)| (i, slot.clone()))
    }
    /// Caller must have finished all GPU writes before publishing this reservation.
    fn publish(&self, index: usize, frame_id: u64) {
        self.state.lock().unwrap().latest = Some((index, frame_id));
    }
    pub fn acquire(&self) -> Option<FrameLease<T>> {
        let state = self.state.lock().unwrap();
        let (i, frame_id) = state.latest?;
        Some(FrameLease {
            resource: state.slots[i].clone(),
            frame_id,
        })
    }
    pub fn leased_slots(&self) -> usize {
        self.state
            .lock()
            .unwrap()
            .slots
            .iter()
            .filter(|s| Arc::strong_count(s) > 1)
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
        let pool = Arc::new(FramePool::new(
            (0..4).map(|_| canvas_texture(&device)).collect(),
        ));
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
            loop {
                let begin = Instant::now();
                if let Some(p) = previous {
                    progress.timings.record(0, p, begin);
                }
                previous = Some(begin);
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
                    if let Some(feed) = scene.as_ref() {
                        feed.frame_completed();
                    }
                    // The reservation remains leased until after publication. Native readers
                    // acquire under the same mutex, and only after producer GPU completion.
                    frames.publish(slot, frame);
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
