use crate::{
    clock::{Clock, Observation},
    dsp,
    model::*,
    timing::*,
    Mixer,
};
fn channels(n: u32) -> Vec<Channel> {
    (0..n)
        .map(|channel| Channel { track: 7, channel })
        .collect()
}
fn strip() -> Strip {
    Strip {
        id: 99,
        name: "signal".into(),
        mapping: channels(2),
        pan: 0.,
        dsp: Dsp::default(),
        sends: [1.; 10],
    }
}
#[test]
fn invalid_changes_are_transactional_and_allocator_never_reuses_deleted_ids() {
    let mut model = Model::default();
    let before = model.clone();
    assert!(model
        .add(
            Origin::Playback(1),
            "bad".into(),
            vec![
                Channel {
                    track: 1,
                    channel: 0
                };
                2
            ]
        )
        .is_err());
    assert_eq!(model, before);
    let id = model
        .add(Origin::Playback(1), "good".into(), channels(4))
        .unwrap();
    let before = model.clone();
    assert!(model
        .interpret(id, Interpretation::DualStereo, channels(2))
        .is_err());
    assert_eq!(model, before);
    assert!(model
        .add(Origin::Playback(1), "duplicate visual".into(), channels(2))
        .is_err());
    assert_eq!(model, before);
    let high = model.next_id;
    model.sources.remove(&id);
    let next = model
        .add(Origin::Playback(2), "independent".into(), channels(1))
        .unwrap();
    assert!(next >= high);
}
#[test]
fn invalid_serialized_graphs_are_rejected_without_discarding_requested_missing_channels() {
    let mut model = Model::default();
    let id = model
        .add(
            Origin::Input {
                uid: "device-exposes-only-two-later".into(),
            },
            "Input".into(),
            channels(4),
        )
        .unwrap();
    model.validate().unwrap(); // Channel 3/4 requests remain, irrespective of current hardware.
    let good = model.clone();
    model.sources.get_mut(&id).unwrap().active.swap(0, 1);
    assert!(model.validate().is_err());
    model = good.clone();
    model.sources.get_mut(&id).unwrap().strips[0].sends[3] = f32::NAN;
    assert!(model.validate().is_err());
    model = good.clone();
    model.sources.get_mut(&id).unwrap().strips[1].id = model.sources[&id].strips[0].id;
    assert!(model.validate().is_err());
    model = good.clone();
    model.sources.get_mut(&id).unwrap().channels[0].channel = 32;
    assert!(model.validate().is_err());
    model = good;
    model.next_id = u64::MAX;
    assert!(model.validate().is_err());
}
#[test]
fn defaults_split_merge_transfer_and_dormant_strips_are_stable() {
    let mut m = Model::default();
    let id = m
        .add(Origin::Playback(4), "Camera 1".into(), channels(2))
        .unwrap();
    assert_eq!(m.sources[&id].interpretation, Interpretation::Stereo);
    let pair = m.sources[&id].active[0];
    let s = &mut m.sources.get_mut(&id).unwrap().strips[0];
    s.dsp.fader_db = -11.;
    s.sends[9] = 0.25;
    s.dsp.compressor.threshold_db = -32.;
    m.interpret(id, Interpretation::DualMono, channels(2))
        .unwrap();
    let source = &m.sources[&id];
    let mono = source.active.clone();
    assert_eq!(mono.len(), 2);
    for st in source.strips.iter().filter(|s| mono.contains(&s.id)) {
        assert_eq!(st.dsp.fader_db, -11.);
        assert_eq!(st.sends[9], 0.25);
    }
    m.sources
        .get_mut(&id)
        .unwrap()
        .strips
        .iter_mut()
        .find(|s| s.id == mono[1])
        .unwrap()
        .dsp
        .fader_db = -3.;
    m.interpret(id, Interpretation::Stereo, channels(2))
        .unwrap();
    assert_eq!(m.sources[&id].active, [pair]);
    m.interpret(id, Interpretation::DualMono, channels(2))
        .unwrap();
    assert_eq!(m.sources[&id].active, mono);
    assert_eq!(
        m.sources[&id]
            .strips
            .iter()
            .find(|s| s.id == mono[1])
            .unwrap()
            .dsp
            .fader_db,
        -3.
    );
}
#[test]
fn sixty_four_independent_active_strips_and_source_limit() {
    let mut m = Model::default();
    for n in 0..16 {
        let id = m
            .add(Origin::Playback(n + 1), format!("Source {n}"), channels(4))
            .unwrap();
        assert_eq!(m.sources[&id].interpretation, Interpretation::DualStereo);
        m.interpret(id, Interpretation::QuadMono, channels(4))
            .unwrap();
    }
    assert_eq!(
        m.sources.values().map(|s| s.active.len()).sum::<usize>(),
        64
    );
    m.validate().unwrap();
    let before = m.clone();
    assert!(m
        .add(Origin::Playback(17), "17".into(), channels(1))
        .is_err());
    assert_eq!(m, before);
}
#[test]
fn isolated_sends_mono_pan_and_stereo_identity_with_independent_dsp() {
    let mut m = Mixer::default();
    let mut left = strip();
    left.mapping = channels(1);
    left.pan = -1.;
    left.sends = [0.; 10];
    left.sends[2] = 1.;
    let mut right = left.clone();
    right.pan = 1.;
    right.sends[2] = 0.;
    right.sends[9] = 0.5;
    right.dsp.fader_db = -6.0206;
    m.configure(0, &left);
    m.configure(1, &right);
    for _ in 0..1200 {
        m.process(&[[0.2, 0.2], [0.4, 0.4]]);
    }
    let v = m.process(&[[0.2, 0.2], [0.4, 0.4]]);
    assert!((v[2][0] - 0.2).abs() < 1e-4);
    assert!(v[2][1].abs() < 1e-5);
    assert!((v[9][1] - 0.1).abs() < 1e-4);
    assert!(v[9][0].abs() < 1e-5);
    for (i, b) in v.iter().enumerate() {
        if i != 2 && i != 9 {
            assert_eq!(*b, [0.; 2]);
        }
    }
    let mut stereo = strip();
    stereo.sends = [0.; 10];
    stereo.sends[1] = 1.;
    m.configure(2, &stereo);
    for _ in 0..600 {
        m.process(&[[0.; 2], [0.; 2], [0.3, -0.1]]);
    }
    let v = m.process(&[[0.; 2], [0.; 2], [0.3, -0.1]]);
    assert!((v[1][0] - 0.3).abs() < 1e-5 && (v[1][1] + 0.1).abs() < 1e-5);
}
#[test]
fn mixer_clone_clear_slot_and_bus_stage_do_not_crosswire_channels() {
    let mut m = Mixer::default();
    m.configure(0, &strip());
    for _ in 0..700 {
        m.process(&[[0.25, -0.1]]);
    }
    let mut cloned = m.clone();
    assert_eq!(m.process(&[[0.25, -0.1]]), cloned.process(&[[0.25, -0.1]]));
    m.clear_slot(0);
    assert_eq!(m.process(&[[0.25, -0.1]]), [[0.; 2]; 10]);
    let sum = cloned.sum_inputs(&[[0.25, -0.1]]);
    let bus = cloned.process_buses(sum);
    assert!(bus
        .iter()
        .all(|x| (x[0] - 0.25).abs() < 1e-5 && (x[1] + 0.1).abs() < 1e-5));
}
#[test]
fn ramps_are_exact_bounded_and_can_retarget_without_a_step() {
    let mut s = dsp::Smooth::new(0.);
    s.target(1.);
    let mut previous = 0.;
    for _ in 0..200 {
        let now = s.next_sample();
        assert!((now - previous - 1. / 480.).abs() < 1e-6);
        previous = now;
    }
    s.target(0.);
    let next = s.next_sample();
    assert!(next < previous && (next - previous).abs() < 0.001);
    for _ in 0..479 {
        s.next_sample();
    }
    assert_eq!(s.next_sample(), 0.);
}
#[test]
fn compressor_release_bypass_trim_and_fader_units() {
    let mut c = Dsp {
        trim_db: 6.0206,
        fader_db: -6.0206,
        ..Dsp::default()
    };
    c.eq[0].db = 18.;
    c.compressor.bypass = false;
    c.compressor.threshold_db = -30.;
    c.compressor.ratio = 20.;
    c.compressor.attack_ms = 0.1;
    c.compressor.release_ms = 50.;
    let mut p = dsp::Strip::default();
    p.configure(&c);
    for _ in 0..10000 {
        p.process([0.5, 0.25]);
    }
    assert!(p.meter.reduction_db > 20.);
    for _ in 0..48000 {
        p.process([0.; 2]);
    }
    assert!(p.meter.reduction_db < 0.01);
    c.bypass = true;
    p.configure(&c);
    let mut y = [0.; 2];
    for _ in 0..1000 {
        y = p.process([0.2, -0.1]);
    }
    assert!((y[0] - 0.2).abs() < 1e-5 && (y[1] + 0.1).abs() < 1e-5);
}
#[test]
fn simulated_independent_device_clock_drift_stays_bounded() {
    for ppm in [-300., 300.] {
        let mut r = Resampler::new(48000., 48000.);
        let mut queue = 4096.;
        let mut largest = 0f64;
        for _ in 0..225000 {
            let error = queue - 4096.;
            r.clock_error(error);
            queue += 128. / (1. + ppm / 1e6) - 128. * r.ratio;
            largest = largest.max((queue - 4096.).abs());
        }
        assert!(largest < 450., "queue drift {largest}");
        let wanted = 1. / (1. + ppm / 1e6);
        assert!((r.ratio - wanted).abs() < 1e-7);
    }
}
#[test]
fn transport_discontinuity_invalidates_old_gain_and_rejects_nonfinite_time() {
    let mut t = Transport::default();
    t.command(Command::Play, 10.);
    t.command(
        Command::Gain {
            at: 96000,
            value: 0.1,
        },
        10.,
    );
    t.command(Command::Seek(48000), 11.);
    assert_eq!(t.tick(12.5).1, 1.);
    let before = t.position;
    t.command(Command::Stop, f64::NAN);
    assert_eq!(t.position, before);
    assert_eq!(t.at(f64::INFINITY), before);
}
#[test]
fn clock_snapshots_are_coherent_and_scheduled_gain_is_latest_and_generation_scoped() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let c = Arc::new(Clock::default());
    let finished = Arc::new(AtomicBool::new(false));
    let writer = c.clone();
    let stop = finished.clone();
    let j = std::thread::spawn(move || {
        for n in 1..100000 {
            writer.store(Observation {
                sample: n,
                host: n as f64,
                playing: n % 2 == 0,
                generation: n as u64,
            });
        }
        stop.store(true, Ordering::Release);
    });
    while !finished.load(Ordering::Acquire) {
        if let Some(o) = c.load() {
            assert_eq!(o.sample as u64, o.generation);
            assert_eq!(o.host, o.sample as f64);
            assert_eq!(o.playing, o.sample != 0 && o.sample % 2 == 0);
        }
    }
    j.join().unwrap();
    let generation = c.load().unwrap().generation;
    assert!(c.gain_event().is_none());
    assert!(c.schedule_gain(100, 0.25));
    assert!(c.schedule_gain(200, 0.75));
    let e = c.gain_event().unwrap();
    assert_eq!(
        (e.revision, e.at, e.gain, e.generation),
        (2, 200, 0.75, generation)
    );
    assert!(!c.schedule_gain(-1, 0.));
    assert!(!c.schedule_gain(0, f32::NAN));
    assert_eq!(c.gain_event(), Some(e));
    c.store(Observation {
        generation: generation + 1,
        ..c.load().unwrap()
    });
    assert_ne!(c.load().unwrap().generation, e.generation);
}
