//! Bounded coherent clock observations and a single latest scheduled-gain command.
//! Publication never allocates or waits. Concurrent writers lose a publication rather than spin.
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Observation {
    pub sample: i64,
    pub host: f64,
    pub playing: bool,
    pub generation: u64,
}
/// Gain belongs to one transport generation. A seek/restart invalidates an obsolete cue.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScheduledGain {
    pub revision: u64,
    pub at: i64,
    pub gain: f32,
    pub generation: u64,
}
pub struct Clock {
    seq: AtomicU64,
    sample: AtomicI64,
    host: AtomicU64,
    playing: AtomicBool,
    generation: AtomicU64,
    gain_seq: AtomicU64,
    gain_at: AtomicI64,
    gain_value: AtomicU64,
    gain_generation: AtomicU64,
}
impl Default for Clock {
    fn default() -> Self {
        Self {
            seq: AtomicU64::new(0),
            sample: AtomicI64::new(0),
            host: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            gain_seq: AtomicU64::new(0),
            gain_at: AtomicI64::new(0),
            gain_value: AtomicU64::new(1f32.to_bits() as u64),
            gain_generation: AtomicU64::new(0),
        }
    }
}
impl Clock {
    pub fn store(&self, o: Observation) {
        if !o.host.is_finite() || o.sample < 0 {
            return;
        }
        let seq = self.seq.load(Ordering::SeqCst);
        if seq & 1 != 0
            || self
                .seq
                .compare_exchange(seq, seq.wrapping_add(1), Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        {
            return;
        }
        // Sequentially consistent atomics make verification order explicit on weak-memory CPUs.
        self.sample.store(o.sample, Ordering::SeqCst);
        self.host.store(o.host.to_bits(), Ordering::SeqCst);
        self.playing.store(o.playing, Ordering::SeqCst);
        self.generation.store(o.generation, Ordering::SeqCst);
        self.seq.store(seq.wrapping_add(2), Ordering::SeqCst);
    }
    /// At most two attempts; caller retains its last good observation during a publication.
    pub fn load(&self) -> Option<Observation> {
        for _ in 0..2 {
            let seq = self.seq.load(Ordering::SeqCst);
            if seq & 1 != 0 {
                continue;
            }
            let o = Observation {
                sample: self.sample.load(Ordering::SeqCst),
                host: f64::from_bits(self.host.load(Ordering::SeqCst)),
                playing: self.playing.load(Ordering::SeqCst),
                generation: self.generation.load(Ordering::SeqCst),
            };
            if self.seq.load(Ordering::SeqCst) == seq {
                return Some(o);
            }
        }
        None
    }
    /// One preallocated latest event. A newer unconsumed command replaces the pending event.
    pub fn schedule_gain(&self, at: i64, gain: f32) -> bool {
        if at < 0 || !gain.is_finite() || !(0.0..=4.0).contains(&gain) {
            return false;
        }
        let Some(clock) = self.load() else {
            return false;
        };
        let seq = self.gain_seq.load(Ordering::SeqCst);
        if seq & 1 != 0
            || self
                .gain_seq
                .compare_exchange(seq, seq.wrapping_add(1), Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        {
            return false;
        }
        self.gain_at.store(at, Ordering::SeqCst);
        self.gain_value
            .store(gain.to_bits() as u64, Ordering::SeqCst);
        self.gain_generation
            .store(clock.generation, Ordering::SeqCst);
        self.gain_seq.store(seq.wrapping_add(2), Ordering::SeqCst);
        true
    }
    pub fn gain_event(&self) -> Option<ScheduledGain> {
        for _ in 0..2 {
            let seq = self.gain_seq.load(Ordering::SeqCst);
            if seq == 0 || seq & 1 != 0 {
                continue;
            }
            let gain = ScheduledGain {
                revision: seq / 2,
                at: self.gain_at.load(Ordering::SeqCst),
                gain: f32::from_bits(self.gain_value.load(Ordering::SeqCst) as u32),
                generation: self.gain_generation.load(Ordering::SeqCst),
            };
            if self.gain_seq.load(Ordering::SeqCst) == seq {
                return Some(gain);
            }
        }
        None
    }
}
impl Observation {
    pub fn at(self, host: f64) -> i64 {
        self.sample
            .saturating_add(if self.playing && host.is_finite() {
                ((host - self.host) * 48000.).round() as i64
            } else {
                0
            })
            .max(0)
    }
}
