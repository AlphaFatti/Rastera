//! RBJ biquads; linked peak compressor; constant-power mono pan; sample-ramped controls.
use crate::{model::Dsp, Stereo, RATE};
pub fn db(x: f32) -> f32 {
    10f32.powf(x / 20.)
}
pub fn clean(x: f32) -> f32 {
    if x.is_finite() {
        x.clamp(-16., 16.)
    } else {
        0.
    }
}
#[derive(Clone, Copy)]
pub struct Smooth {
    value: f32,
    end: f32,
    step: f32,
    left: u32,
}
impl Smooth {
    pub const fn new(value: f32) -> Self {
        Self {
            value,
            end: value,
            step: 0.,
            left: 0,
        }
    }
    pub fn target(&mut self, end: f32) {
        if end.is_finite() && end != self.end {
            self.end = end;
            self.left = 480;
            self.step = (end - self.value) / 480.;
        }
    }
    pub fn next_sample(&mut self) -> f32 {
        if self.left > 0 {
            self.value += self.step;
            self.left -= 1;
            if self.left == 0 {
                self.value = self.end;
            }
        }
        self.value
    }
}
#[derive(Clone, Copy)]
struct Biquad {
    c: [f32; 5],
    z: [[f32; 2]; 2],
}
impl Default for Biquad {
    fn default() -> Self {
        Self {
            c: [1., 0., 0., 0., 0.],
            z: [[0.; 2]; 2],
        }
    }
}
impl Biquad {
    fn coefficients(&mut self, hz: f32, q: f32, gain: f32, hp: bool) {
        let w = 2. * std::f32::consts::PI * hz / RATE as f32;
        let a = w.sin() / (2. * q);
        let c = w.cos();
        let amp = 10f32.powf(gain / 40.);
        let (b0, b1, b2, a0, a1, a2) = if hp {
            (
                (1. + c) / 2.,
                -(1. + c),
                (1. + c) / 2.,
                1. + a,
                -2. * c,
                1. - a,
            )
        } else {
            (
                1. + a * amp,
                -2. * c,
                1. - a * amp,
                1. + a / amp,
                -2. * c,
                1. - a / amp,
            )
        };
        self.c = [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0];
    }
    fn sample(&mut self, x: Stereo) -> Stereo {
        std::array::from_fn(|i| {
            let y = clean(self.c[0] * x[i] + self.z[i][0]);
            self.z[i][0] = self.c[1] * x[i] - self.c[3] * y + self.z[i][1];
            self.z[i][1] = self.c[2] * x[i] - self.c[4] * y;
            y
        })
    }
}
#[derive(Default, Clone, Copy)]
pub struct Meter {
    pub input: f32,
    pub output: f32,
    pub reduction_db: f32,
    pub clipped: bool,
    pub non_finite: u64,
}
#[derive(Clone)]
pub struct Strip {
    config: Dsp,
    filters: [Biquad; 5],
    old_filters: [Biquad; 5],
    filter_enabled: [bool; 5],
    old_filter_enabled: [bool; 5],
    filter_mix: Smooth,
    trim: Smooth,
    fader: Smooth,
    bypass: Smooth,
    comp_gain: f32,
    comp_threshold: Smooth,
    comp_ratio: Smooth,
    comp_knee: Smooth,
    comp_makeup: Smooth,
    comp_wet: Smooth,
    limiter: f32,
    pub meter: Meter,
}
impl Default for Strip {
    fn default() -> Self {
        Self {
            config: Dsp::default(),
            filters: [Biquad::default(); 5],
            old_filters: [Biquad::default(); 5],
            filter_enabled: [false; 5],
            old_filter_enabled: [false; 5],
            filter_mix: Smooth::new(1.),
            trim: Smooth::new(1.),
            fader: Smooth::new(0.),
            bypass: Smooth::new(0.),
            comp_gain: 1.,
            comp_threshold: Smooth::new(-18.),
            comp_ratio: Smooth::new(4.),
            comp_knee: Smooth::new(6.),
            comp_makeup: Smooth::new(0.),
            comp_wet: Smooth::new(0.),
            limiter: 1.,
            meter: Meter::default(),
        }
    }
}
impl Strip {
    pub fn configure(&mut self, c: &Dsp) {
        if !c.valid() {
            return;
        }
        if c.eq != self.config.eq || c.high_pass_hz != self.config.high_pass_hz {
            self.old_filters = self.filters;
            self.old_filter_enabled = self.filter_enabled;
            self.filter_enabled = std::array::from_fn(|i| {
                if i == 0 {
                    c.high_pass_hz > 0.
                } else {
                    c.eq[i - 1].db != 0.
                }
            });
            self.filter_mix = Smooth::new(0.);
            self.filter_mix.target(1.);
            self.filters = [Biquad::default(); 5];
            if c.high_pass_hz > 0. {
                self.filters[0].coefficients(c.high_pass_hz, 0.707, 0., true);
            }
            for (i, b) in c.eq.iter().enumerate() {
                self.filters[i + 1].coefficients(b.hz, b.q, b.db, false);
            }
        }
        self.trim.target(db(c.trim_db));
        self.fader.target(if c.mute { 0. } else { db(c.fader_db) });
        self.bypass.target(if c.bypass { 1. } else { 0. });
        self.comp_threshold.target(c.compressor.threshold_db);
        self.comp_ratio.target(c.compressor.ratio);
        self.comp_knee.target(c.compressor.knee_db);
        self.comp_makeup.target(c.compressor.makeup_db);
        self.comp_wet
            .target(if c.compressor.bypass { 0. } else { 1. });
        self.config = c.clone();
    }
    pub fn process(&mut self, input: Stereo) -> Stereo {
        let mut x = input.map(|v| {
            if !v.is_finite() {
                self.meter.non_finite += 1;
            }
            clean(v)
        });
        self.meter.input = self.meter.input.max(x[0].abs().max(x[1].abs()));
        self.meter.clipped |= x[0].abs() >= 1. || x[1].abs() >= 1.;
        let gain = self.trim.next_sample();
        x = x.map(|v| v * gain);
        let dry = x;
        let mut old = x;
        let blend = self.filter_mix.next_sample();
        for (i, f) in self.filters.iter_mut().enumerate() {
            if self.filter_enabled[i] {
                x = f.sample(x);
            }
        }
        if blend < 1. {
            for (i, f) in self.old_filters.iter_mut().enumerate() {
                if self.old_filter_enabled[i] {
                    old = f.sample(old);
                }
            }
            x = std::array::from_fn(|i| old[i] + blend * (x[i] - old[i]));
        }
        let c = &self.config.compressor;
        let peak = x[0].abs().max(x[1].abs()).max(1e-12);
        let threshold = self.comp_threshold.next_sample();
        let ratio = self.comp_ratio.next_sample();
        let knee = self.comp_knee.next_sample();
        let makeup = self.comp_makeup.next_sample();
        let wet = self.comp_wet.next_sample();
        if wet > 0. || !c.bypass {
            let over = 20. * peak.log10() - threshold;
            let reduction = if over < -knee / 2. {
                0.
            } else if over < knee / 2. && knee > 0. {
                (1. - 1. / ratio) * (over + knee / 2.).powi(2) / (2. * knee)
            } else {
                over.max(0.) * (1. - 1. / ratio)
            };
            let target = db(-reduction);
            let time = if target < self.comp_gain {
                c.attack_ms
            } else {
                c.release_ms
            };
            let coeff = (-1. / (0.001 * time * RATE as f32)).exp();
            self.comp_gain = coeff * self.comp_gain + (1. - coeff) * target;
            self.meter.reduction_db = wet * (-20. * self.comp_gain.max(1e-12).log10());
            let compressor_gain = (1. - wet) + wet * self.comp_gain * db(makeup);
            x = x.map(|v| v * compressor_gain);
        } else {
            self.comp_gain = 1.;
            self.meter.reduction_db = 0.;
        }
        let bypass = self.bypass.next_sample();
        let fader = self.fader.next_sample();
        x = std::array::from_fn(|i| (x[i] * (1. - bypass) + dry[i] * bypass) * fader);
        if let Some(limit) = self.config.limiter_db {
            let ceiling = db(limit);
            let peak = x[0].abs().max(x[1].abs());
            let target = (ceiling / peak.max(ceiling)).min(1.);
            self.limiter = if target < self.limiter {
                target
            } else {
                self.limiter + (target - self.limiter) * 0.0005
            };
            x = x.map(|v| (v * self.limiter).clamp(-ceiling, ceiling));
        }
        self.meter.output = self.meter.output.max(x[0].abs().max(x[1].abs()));
        self.meter.clipped |= x[0].abs() >= 1. || x[1].abs() >= 1.;
        x.map(clean)
    }
    pub fn clear_meters(&mut self) {
        self.meter.input = 0.;
        self.meter.output = 0.;
        self.meter.reduction_db = 0.;
    }
}
pub fn pan(sample: f32, pan: f32) -> Stereo {
    let a = (pan.clamp(-1., 1.) + 1.) * std::f32::consts::FRAC_PI_4;
    [sample * a.cos(), sample * a.sin()]
}
