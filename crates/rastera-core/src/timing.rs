//! Bounded CPU timing samples. Intervals reset on reporting; never GPU timestamps.
use std::{sync::Mutex, time::Instant};

pub const STAGES: [&str; 6] = ["cadence", "record", "finish", "submit", "wait", "notify"];
#[derive(Clone, Copy, Default)]
struct Sample {
    count: u64,
    total_us: u64,
    max_us: u64,
    max_start_us: u64,
}
struct Interval {
    from_us: u64,
    samples: [Sample; 6],
}
pub struct Timings {
    pub origin: Instant,
    interval: Mutex<Interval>,
}
impl Default for Timings {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            interval: Mutex::new(Interval {
                from_us: 0,
                samples: [Sample::default(); 6],
            }),
        }
    }
}
impl Timings {
    pub fn record(&self, stage: usize, start: Instant, end: Instant) {
        let us = end.duration_since(start).as_micros() as u64;
        let mut interval = self.interval.lock().unwrap();
        let sample = &mut interval.samples[stage];
        sample.count += 1;
        sample.total_us += us;
        if us >= sample.max_us {
            sample.max_us = us;
            sample.max_start_us = start.duration_since(self.origin).as_micros() as u64;
        }
    }
    /// A completed call crossing a report boundary is counted in its ending interval.
    pub fn report(&self) -> String {
        let now_us = self.origin.elapsed().as_micros() as u64;
        let previous = std::mem::replace(
            &mut *self.interval.lock().unwrap(),
            Interval {
                from_us: now_us,
                samples: [Sample::default(); 6],
            },
        );
        let mut report = format!(
            "producer_interval t_us={now_us} interval_us={}\n",
            now_us - previous.from_us
        );
        for (name, s) in STAGES.into_iter().zip(previous.samples) {
            report.push_str(&format!(
                "stage t_us={now_us} name={name} count={} total_us={} max_us={} max_start_us={}\n",
                s.count, s.total_us, s.max_us, s.max_start_us
            ));
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reporting_resets_intervals_and_preserves_event_start() {
        let timing = Timings::default();
        let start = timing.origin + std::time::Duration::from_micros(10);
        timing.record(2, start, start + std::time::Duration::from_micros(123));
        let first = timing.report();
        assert!(first.contains("name=finish count=1 total_us=123 max_us=123 max_start_us=10"));
        assert!(timing
            .report()
            .contains("name=finish count=0 total_us=0 max_us=0"));
    }
}
