//! Reusable 48 kHz mixer. No UI, device APIs, files, GPU, or callback-owned heap resources.
pub mod clock;
pub mod dsp;
pub mod model;
pub mod queue;
pub mod timing;
pub const RATE: u32 = 48_000;
pub const BLOCK: usize = 128;
pub const BUSES: usize = 10;
pub const MAX_STRIPS: usize = 64;
pub type Stereo = [f32; 2];
/// A future cue/slide producer supplies timestamped discrete PCM, not a device stream.
pub trait Producer {
    fn sample(&mut self, sample_position: i64, channel: usize) -> f32;
}
/// A reusable submix is processed by the same channel strip as an ordinary source.
pub trait Submix {
    fn frame(&mut self, sample_position: i64) -> Stereo;
}
#[derive(Clone)]
pub struct Mixer {
    pub sources: [dsp::Strip; MAX_STRIPS],
    pub buses: [dsp::Strip; BUSES],
    pub bus_frames: [Stereo; BUSES],
    sends: [[dsp::Smooth; BUSES]; MAX_STRIPS],
    pans: [dsp::Smooth; MAX_STRIPS],
    mono: [bool; MAX_STRIPS],
    active: [bool; MAX_STRIPS],
}
impl Default for Mixer {
    fn default() -> Self {
        Self {
            sources: std::array::from_fn(|_| dsp::Strip::default()),
            buses: std::array::from_fn(|_| {
                let mut strip = dsp::Strip::default();
                strip.configure(&model::Dsp::default());
                strip
            }),
            bus_frames: [[0.; 2]; BUSES],
            pans: [dsp::Smooth::new(0.); MAX_STRIPS],
            mono: [false; MAX_STRIPS],
            active: [false; MAX_STRIPS],
            sends: std::array::from_fn(|_| std::array::from_fn(|_| dsp::Smooth::new(0.))),
        }
    }
}
impl Mixer {
    /// Configuration changes are installed between blocks; coefficients never allocate.
    pub fn configure(&mut self, index: usize, config: &model::Strip) {
        self.active[index] = true;
        self.sources[index].configure(&config.dsp);
        self.pans[index].target(config.pan);
        self.mono[index] = config.mapping.len() == 1;
        for (s, &v) in self.sends[index].iter_mut().zip(&config.sends) {
            s.target(v);
        }
    }
    pub fn clear_slot(&mut self, index: usize) {
        self.sources[index] = dsp::Strip::default();
        self.sends[index] = [dsp::Smooth::new(0.); BUSES];
        self.pans[index] = dsp::Smooth::new(0.);
        self.mono[index] = false;
        self.active[index] = false;
    }
    /// Source summing and bus DSP are separate so bounded transition voices can join a submix.
    pub fn sum_inputs(&mut self, inputs: &[Stereo]) -> [Stereo; BUSES] {
        self.bus_frames.fill([0.; 2]);
        for (i, &input) in inputs.iter().take(MAX_STRIPS).enumerate() {
            if !self.active[i] {
                continue;
            }
            let mut x = self.sources[i].process(input);
            let pan = self.pans[i].next_sample();
            if self.mono[i] {
                x = dsp::pan(x[0], pan);
            }
            for b in 0..BUSES {
                let gain = self.sends[i][b].next_sample();
                if gain == 0. {
                    continue;
                }
                for (d, s) in self.bus_frames[b].iter_mut().zip(x) {
                    *d += s * gain;
                }
            }
        }
        self.bus_frames
    }
    pub fn process_buses(&mut self, frames: [Stereo; BUSES]) -> [Stereo; BUSES] {
        std::array::from_fn(|b| self.buses[b].process(frames[b]))
    }
    pub fn process(&mut self, inputs: &[Stereo]) -> [Stereo; BUSES] {
        let frames = self.sum_inputs(inputs);
        self.process_buses(frames)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ten_routes_sum_mute_and_gain_ramps() {
        let mut m = Mixer::default();
        let s = model::Strip {
            id: 20,
            name: "A".into(),
            mapping: vec![
                model::Channel {
                    track: 1,
                    channel: 0,
                },
                model::Channel {
                    track: 1,
                    channel: 1,
                },
            ],
            pan: 0.,
            dsp: model::Dsp::default(),
            sends: std::array::from_fn(|i| (i + 1) as f32 / 10.),
        };
        m.configure(0, &s);
        m.configure(1, &s);
        for _ in 0..1000 {
            m.process(&[[0.1, 0.2], [0.1, 0.2]]);
        }
        let v = m.process(&[[0.1, 0.2], [0.1, 0.2]]);
        for (i, b) in v.iter().enumerate() {
            assert!((b[0] - 0.2 * (i + 1) as f32 / 10.).abs() < 1e-4);
            assert!((b[1] - 2. * b[0]).abs() < 1e-5);
        }
        let mut muted = s.clone();
        muted.dsp.mute = true;
        m.configure(0, &muted);
        m.configure(1, &muted);
        let first = m.process(&[[0.1, 0.2], [0.1, 0.2]]);
        assert!(first[9][0] > 0.1);
        for _ in 0..1000 {
            m.process(&[[0.1, 0.2], [0.1, 0.2]]);
        }
        assert_eq!(m.process(&[[0.1, 0.2], [0.1, 0.2]]), [[0.; 2]; 10]);
    }
    #[test]
    fn eq_frequency_response_bypass_and_nonfinite() {
        let power = |hz: f32, eq: f32, bypass: bool| {
            let mut s = dsp::Strip::default();
            let mut c = model::Dsp {
                bypass,
                ..model::Dsp::default()
            };
            c.eq[1].hz = 1000.;
            c.eq[1].db = eq;
            s.configure(&c);
            let mut sum = 0.;
            for i in 0..48000 {
                let x = 0.1 * (i as f32 * hz * std::f32::consts::TAU / 48000.).sin();
                let y = s.process([x, x]);
                if i > 24000 {
                    sum += y[0] * y[0];
                }
            }
            sum
        };
        let plain = power(1000., 0., false);
        assert!(power(1000., 12., false) / plain > 14.);
        assert!((power(1000., 12., true) / plain - 1.).abs() < 0.01);
        let mut s = dsp::Strip::default();
        assert_eq!(s.process([f32::NAN, f32::INFINITY]), [0.; 2]);
        assert_eq!(s.meter.non_finite, 2);
    }
    #[test]
    fn high_pass_rejects_low_frequency() {
        let run = |hp| {
            let mut s = dsp::Strip::default();
            s.configure(&model::Dsp {
                high_pass_hz: hp,
                ..Default::default()
            });
            let mut sum = 0.;
            for i in 0..48000 {
                let x = (i as f32 * std::f32::consts::TAU * 30. / 48000.).sin() * 0.1;
                let y = s.process([x, x]);
                if i > 24000 {
                    sum += y[0] * y[0];
                }
            }
            sum
        };
        assert!(run(300.) / run(0.) < 0.001);
    }
    #[test]
    fn compressor_linking_and_limiter() {
        let mut s = dsp::Strip::default();
        let mut c = model::Dsp::default();
        c.compressor.bypass = false;
        c.compressor.threshold_db = -20.;
        c.compressor.ratio = 10.;
        c.compressor.attack_ms = 1.;
        s.configure(&c);
        let mut y = [0.; 2];
        for _ in 0..48000 {
            y = s.process([0.8, 0.08]);
        }
        assert!(y[0] < 0.17);
        assert!((y[0] / y[1] - 10.).abs() < 0.001);
        assert!(s.meter.reduction_db > 14.);
        c.limiter_db = Some(-6.);
        c.compressor.bypass = true;
        s.configure(&c);
        for _ in 0..1000 {
            y = s.process([4., 2.]);
            assert!(y[0] <= dsp::db(-6.) + 1e-6);
            assert!((y[0] / y[1] - 2.).abs() < 1e-4);
        }
    }
    #[test]
    fn pan_equal_power() {
        for p in [-1., -0.5, 0., 0.5, 1.] {
            let v = dsp::pan(1., p);
            assert!((v[0] * v[0] + v[1] * v[1] - 1.).abs() < 1e-6);
        }
        assert!(dsp::pan(1., -1.)[1].abs() < 1e-6);
        assert!(dsp::pan(1., 1.)[0].abs() < 1e-6);
    }
    #[test]
    fn interpretation_settings_and_id_survive_mode_changes() {
        let mut m = model::Model::default();
        let channels = (0..4)
            .map(|channel| model::Channel { track: 7, channel })
            .collect::<Vec<_>>();
        let id = m
            .add(
                model::Origin::Playback(4),
                "Camera 1".into(),
                channels.clone(),
            )
            .unwrap();
        let pair = m.sources[&id].active[1];
        m.sources.get_mut(&id).unwrap().strips[1].dsp.fader_db = -12.;
        m.interpret(id, model::Interpretation::QuadMono, channels.clone())
            .unwrap();
        let mono = m.sources[&id].active.clone();
        assert_eq!(mono.len(), 4);
        assert!(mono.iter().all(|id| *id != pair));
        m.interpret(id, model::Interpretation::DualStereo, channels)
            .unwrap();
        let s = &m.sources[&id];
        assert_eq!(s.active[1], pair);
        assert_eq!(
            s.strips.iter().find(|s| s.id == pair).unwrap().dsp.fader_db,
            -12.
        );
        m.validate().unwrap();
    }
    #[test]
    fn bounded_queue_and_cross_thread_order() {
        let (mut tx, mut rx) = queue::bounded(4, 0u32);
        for i in 0..4 {
            assert!(tx.push(i));
        }
        assert!(!tx.push(99));
        for i in 0..4 {
            assert_eq!(rx.pop(), Some(i));
        }
        assert_eq!(rx.pop(), None);
        let j = std::thread::spawn(move || {
            for i in 0..10000 {
                while !tx.push(i) {
                    std::thread::yield_now();
                }
            }
        });
        for i in 0..10000 {
            loop {
                if let Some(v) = rx.pop() {
                    assert_eq!(v, i);
                    break;
                }
                std::thread::yield_now();
            }
        }
        j.join().unwrap();
    }
    #[test]
    fn transport_pause_seek_restart_scheduled_gain() {
        use timing::*;
        let mut t = Transport::default();
        t.command(Command::Play, 10.);
        assert_eq!(t.at(11.), 48000);
        t.command(Command::Pause, 11.);
        assert_eq!(t.at(20.), 48000);
        t.command(Command::Seek(24000), 20.);
        assert_eq!(t.generation, 1);
        t.command(Command::Play, 20.);
        t.command(
            Command::Gain {
                at: 48000,
                value: 0.5,
            },
            20.,
        );
        assert_eq!(t.tick(20.1).1, 1.);
        assert_eq!(t.tick(20.5).1, 0.5);
        t.command(Command::Restart, 22.);
        assert_eq!(t.at(22.), 0);
        t.command(Command::Stop, 23.);
        assert!(!t.playing);
        assert_eq!(t.position, 0);
    }
    #[test]
    fn resampling_rate_and_independent_clock_error() {
        let mut r = timing::Resampler::new(48000., 44100.);
        let mut consumed = 0;
        for _ in 0..44100 {
            consumed += r.sample(0., 1.).1;
        }
        assert!((consumed as i64 - 48000).abs() <= 1);
        for _ in 0..10000 {
            r.clock_error(1000.);
        }
        assert!(r.ratio > 48000. / 44100.);
        assert!(r.ratio <= (48000. / 44100.) * 1.00101);
    }
}

#[cfg(test)]
mod regression_tests;
