//! Integer 48 kHz positions. Host timestamps are monotonic seconds, never UI frame times.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Play,
    Pause,
    Stop,
    Restart,
    Seek(i64),
    Gain { at: i64, value: f32 },
}
#[derive(Clone, Copy, Debug)]
pub struct Transport {
    pub position: i64,
    pub host: f64,
    pub playing: bool,
    pub generation: u64,
    pub gain: f32,
    scheduled: Option<(i64, f32)>,
}
impl Default for Transport {
    fn default() -> Self {
        Self {
            position: 0,
            host: 0.,
            playing: false,
            generation: 0,
            gain: 1.,
            scheduled: None,
        }
    }
}
impl Transport {
    pub fn tick(&mut self, host: f64) -> (i64, f32) {
        let position = self.at(host);
        if let Some((at, value)) = self.scheduled {
            if position >= at {
                self.gain = value;
                self.scheduled = None;
            }
        }
        (position, self.gain)
    }
    pub fn at(&self, host: f64) -> i64 {
        self.position
            .saturating_add(if self.playing && host.is_finite() {
                ((host - self.host) * 48000.).round() as i64
            } else {
                0
            })
            .max(0)
    }
    pub fn command(&mut self, cmd: Command, host: f64) {
        if !host.is_finite() {
            return;
        }
        self.position = self.tick(host).0;
        self.host = host;
        match cmd {
            Command::Play => self.playing = true,
            Command::Pause => self.playing = false,
            Command::Stop => {
                self.playing = false;
                self.position = 0;
                self.generation = self.generation.wrapping_add(1);
                self.scheduled = None
            }
            Command::Restart => {
                self.position = 0;
                self.playing = true;
                self.generation = self.generation.wrapping_add(1);
                self.scheduled = None
            }
            Command::Seek(p) => {
                self.position = p.max(0);
                self.generation = self.generation.wrapping_add(1);
                self.scheduled = None
            }
            Command::Gain { at, value } => {
                if value.is_finite() && (0.0..=4.).contains(&value) {
                    self.scheduled = Some((at, value));
                }
            }
        }
    }
}
/// Fractional position linear interpolator, bounded correction ±1000 ppm. Device clocks
/// are followed, not forced: queue occupancy trims ratio gently without aggregate devices.
pub struct Resampler {
    pub phase: f64,
    pub ratio: f64,
    base: f64,
}
impl Resampler {
    pub fn new(input: f64, output: f64) -> Self {
        assert!(
            input.is_finite()
                && output.is_finite()
                && (8000.0..=192000.0).contains(&input)
                && (8000.0..=192000.0).contains(&output),
            "Resampling requires finite 8–192 kHz rates"
        );
        Self {
            phase: 0.,
            ratio: input / output,
            base: input / output,
        }
    }
    pub fn clock_error(&mut self, error_frames: f64) {
        if !error_frames.is_finite() {
            return;
        }
        let correction = (error_frames * 0.000001).clamp(-0.001, 0.001);
        self.ratio += (self.base * (1. + correction) - self.ratio) * 0.01;
    }
    pub fn sample(&mut self, a: f32, b: f32) -> (f32, usize) {
        let a = if a.is_finite() { a } else { 0. };
        let b = if b.is_finite() { b } else { 0. };
        let v = a + (b - a) * self.phase as f32;
        self.phase += self.ratio;
        let step = self.phase.floor() as usize;
        self.phase -= step as f64;
        (v, step)
    }
}
