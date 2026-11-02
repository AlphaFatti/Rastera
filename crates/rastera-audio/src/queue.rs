//! Fixed SPSC queue. Exactly one producer and one consumer; ownership is split once.
use std::{
    cell::UnsafeCell,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
struct Ring<T: Copy> {
    slots: Box<[UnsafeCell<T>]>,
    read: AtomicUsize,
    write: AtomicUsize,
}
// SAFETY: endpoint ownership enforces one writer/reader. Release/acquire publishes slot writes.
unsafe impl<T: Copy + Send> Sync for Ring<T> {}
pub struct Sender<T: Copy>(Arc<Ring<T>>);
pub struct Receiver<T: Copy>(Arc<Ring<T>>);
pub fn bounded<T: Copy + Send>(capacity: usize, initial: T) -> (Sender<T>, Receiver<T>) {
    assert!(
        (2..=65536).contains(&capacity),
        "Queue capacity must be between 2 and 65536 items"
    );
    let r = Arc::new(Ring {
        slots: (0..capacity + 1)
            .map(|_| UnsafeCell::new(initial))
            .collect(),
        read: AtomicUsize::new(0),
        write: AtomicUsize::new(0),
    });
    (Sender(r.clone()), Receiver(r))
}
impl<T: Copy> Sender<T> {
    pub fn push(&mut self, v: T) -> bool {
        let w = self.0.write.load(Ordering::Relaxed);
        let next = (w + 1) % self.0.slots.len();
        if next == self.0.read.load(Ordering::Acquire) {
            return false;
        }
        unsafe {
            *self.0.slots[w].get() = v;
        }
        self.0.write.store(next, Ordering::Release);
        true
    }
}
impl<T: Copy> Receiver<T> {
    pub fn pop(&mut self) -> Option<T> {
        let r = self.0.read.load(Ordering::Relaxed);
        if r == self.0.write.load(Ordering::Acquire) {
            return None;
        }
        let v = unsafe { *self.0.slots[r].get() };
        self.0
            .read
            .store((r + 1) % self.0.slots.len(), Ordering::Release);
        Some(v)
    }
    /// Flush only the published snapshot in constant time; an active producer cannot
    /// extend this operation indefinitely. Copy items have no destructor to reclaim.
    pub fn discard(&mut self) {
        let end = self.0.write.load(Ordering::Acquire);
        self.0.read.store(end, Ordering::Release);
    }
    pub fn queued(&self) -> usize {
        let w = self.0.write.load(Ordering::Acquire);
        let r = self.0.read.load(Ordering::Acquire);
        (w + self.0.slots.len() - r) % self.0.slots.len()
    }
    pub fn capacity(&self) -> usize {
        self.0.slots.len() - 1
    }
}
