//! Native adapter diagnostics. Uses the same portable mixer and native PCM/device adapters as Rastera.
#[cfg(target_os = "macos")]
#[path = "../audio_native.rs"]
#[allow(dead_code)]
mod native;
#[cfg(target_os = "macos")]
mod diagnostic {
    use super::native;
    use rastera_audio::{model, queue, Mixer, Stereo};
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
            Arc,
        },
        thread,
        time::{Duration, Instant},
    };
    pub fn decode(path: &Path) -> Result<serde_json::Value, String> {
        let probe = native::probe(path)?;
        let mut rows = vec![];
        for track in &probe.tracks {
            let mut reader = native::Reader::new(path, track.id)?;
            let mut data = [0.; 128 * 32];
            let mut power = [0f64; 32];
            let mut projection = [[[0f64; 2]; 4]; 32];
            let mut first_nonzero = [None; 32];
            let mut samples = 0usize;
            let mut first = None;
            let mut end: Option<f64> = None;
            let mut gaps = vec![];
            loop {
                let (n, ch, pts) = reader.read(&mut data)?;
                if n == 0 {
                    break;
                }
                first.get_or_insert(pts);
                if let Some(previous) = end {
                    if (pts - previous).abs() > 1. / 48000. && gaps.len() < 64 {
                        gaps.push([previous, pts]);
                    }
                }
                end = Some(pts + n as f64 / 48000.);
                if ch != track.channels as usize {
                    return Err("Reader discarded or added discrete channels".into());
                }
                for i in 0..n {
                    for c in 0..ch {
                        let x = data[i * ch + c] as f64;
                        if !x.is_finite() {
                            return Err("Decoded PCM contains non-finite data".into());
                        }
                        if x.abs() > 0.000001 {
                            first_nonzero[c].get_or_insert(pts + i as f64 / 48000.);
                        }
                        power[c] += x * x;
                        for (f, hz) in [220., 330., 440., 550.].iter().enumerate() {
                            let angle = std::f64::consts::TAU * hz * (pts + i as f64 / 48000.);
                            projection[c][f][0] += x * angle.cos();
                            projection[c][f][1] += x * angle.sin();
                        }
                    }
                }
                samples += n;
                if samples > 48000 * 30 {
                    return Err("Diagnostic fixture exceeds 30 seconds".into());
                }
            }
            let channels = (0..track.channels as usize).map(|c|serde_json::json!({
                "channel":c+1,"rms":if samples>0{(power[c]/samples as f64).sqrt()}else{0.},
                "first_nonzero_pts":first_nonzero[c],
                "frequency_amplitude":projection[c].map(|p|if samples>0{2.*p[0].hypot(p[1])/samples as f64}else{0.})
            })).collect::<Vec<_>>();
            let seek = if probe.duration > 2.5 {
                if !reader.seek(2.25) {
                    return Err("Native reader seek failed".into());
                }
                let (frames, channels, pts) = reader.read(&mut data)?;
                Some(
                    serde_json::json!({"requested":2.25,"first_pts":pts,"frames":frames,"channels":channels}),
                )
            } else {
                None
            };
            rows.push(serde_json::json!({"track":track,"first_pts":first,"end_pts":end,"timestamp_gaps":gaps,
                "samples":samples,"channels":channels,"seek_probe":seek}));
        }
        Ok(serde_json::Value::Array(rows))
    }

    #[derive(Clone, Copy)]
    struct Packet {
        start: i64,
        count: usize,
        channels: usize,
        generation: u64,
        samples: [f32; 128 * 32],
    }
    impl Default for Packet {
        fn default() -> Self {
            Self {
                start: 0,
                count: 0,
                channels: 0,
                generation: 0,
                samples: [0.; 128 * 32],
            }
        }
    }
    struct Cursor {
        rx: queue::Receiver<Packet>,
        packet: Packet,
    }
    impl Cursor {
        fn at(&mut self, sample: i64, channel: u32, generation: u64) -> Option<f32> {
            while self.packet.count == 0
                || self.packet.generation != generation
                || self.packet.start + self.packet.count as i64 <= sample
            {
                self.packet = self.rx.pop()?;
            }
            if sample < self.packet.start || channel as usize >= self.packet.channels {
                None
            } else {
                Some(
                    self.packet.samples[(sample - self.packet.start) as usize
                        * self.packet.channels
                        + channel as usize],
                )
            }
        }
    }
    struct File {
        tracks: BTreeMap<i32, Cursor>,
        model: model::Model,
        source: u64,
        duration: i64,
        wanted: Arc<AtomicI64>,
        generation: Arc<AtomicU64>,
        stop: Arc<AtomicBool>,
        worker: Option<thread::JoinHandle<()>>,
        decoded: Arc<AtomicU64>,
        errors: Arc<AtomicU64>,
        missing_frames: u64,
    }
    impl File {
        fn start(
            path: PathBuf,
            requested: Option<String>,
            interpretation: Option<String>,
        ) -> Result<Self, String> {
            let probe = native::probe(&path)?;
            let ids = if let Some(s) = requested {
                s.split(',')
                    .map(|s| s.parse::<i32>().map_err(|e| e.to_string()))
                    .collect::<Result<Vec<_>, _>>()?
            } else if probe.tracks.len() == 1 {
                vec![probe.tracks[0].id]
            } else {
                return Err("Multitrack file playback requires explicit --tracks=ID,ID; alternate tracks are never combined implicitly".into());
            };
            let mut channels = vec![];
            for id in &ids {
                let t = probe
                    .tracks
                    .iter()
                    .find(|t| t.id == *id)
                    .ok_or("Missing requested track")?;
                channels.extend((0..t.channels).map(|channel| model::Channel {
                    track: *id,
                    channel,
                }));
            }
            if !matches!(channels.len(), 1 | 2 | 4) {
                return Err(
                    "Diagnostic playback selects exactly one, two or four discrete channels".into(),
                );
            }
            let mut model = model::Model::default();
            let source = model.add(
                model::Origin::File {
                    path: path.clone(),
                    duration: probe.duration,
                },
                "File proof".into(),
                channels.clone(),
            )?;
            if let Some(mode) = interpretation {
                let mode = match mode.as_str() {
                    "mono" => model::Interpretation::Mono,
                    "stereo" => model::Interpretation::Stereo,
                    "dual-mono" => model::Interpretation::DualMono,
                    "dual-stereo" => model::Interpretation::DualStereo,
                    "quad-mono" => model::Interpretation::QuadMono,
                    _ => return Err("Unknown interpretation".into()),
                };
                model.interpret(source, mode, channels)?;
            }
            for (i, strip) in model
                .sources
                .get_mut(&source)
                .unwrap()
                .strips
                .iter_mut()
                .enumerate()
            {
                strip.dsp.fader_db = -36.;
                strip.sends = std::array::from_fn(|b| if b == i { 1. } else { 0. });
            }
            let wanted = Arc::new(AtomicI64::new(0));
            let generation = Arc::new(AtomicU64::new(1));
            let stop = Arc::new(AtomicBool::new(false));
            let decoded = Arc::new(AtomicU64::new(0));
            let errors = Arc::new(AtomicU64::new(0));
            let mut tracks = BTreeMap::new();
            let mut outputs = vec![];
            for id in ids {
                let reader = native::Reader::new(&path, id)?;
                let (tx, rx) = queue::bounded(64, Packet::default());
                tracks.insert(
                    id,
                    Cursor {
                        rx,
                        packet: Packet::default(),
                    },
                );
                outputs.push((reader, tx, 0u64, 0i64, false));
            }
            let (wanted_worker, generation_worker, stop_worker, decoded_worker, errors_worker) = (
                wanted.clone(),
                generation.clone(),
                stop.clone(),
                decoded.clone(),
                errors.clone(),
            );
            let worker = thread::spawn(move || {
                while !stop_worker.load(Ordering::Acquire) {
                    let sample = wanted_worker.load(Ordering::Acquire);
                    let gen = generation_worker.load(Ordering::Acquire);
                    let mut progress = false;
                    for (reader, tx, epoch, end, eof) in &mut outputs {
                        if *epoch != gen {
                            if !reader.seek(sample as f64 / 48000.) {
                                errors_worker.fetch_add(1, Ordering::Relaxed);
                            }
                            *epoch = gen;
                            *end = sample;
                            *eof = false;
                        }
                        if *eof || *end > sample + 6000 {
                            continue;
                        }
                        let mut packet = Packet {
                            generation: gen,
                            ..Default::default()
                        };
                        match reader.read(&mut packet.samples) {
                            Ok((n, ch, pts)) if n > 0 => {
                                packet.start = (pts * 48000.).round() as i64;
                                packet.count = n;
                                packet.channels = ch;
                                *end = packet.start + n as i64;
                                while !tx.push(packet) {
                                    if stop_worker.load(Ordering::Acquire) {
                                        return;
                                    }
                                    if generation_worker.load(Ordering::Acquire) != gen {
                                        break;
                                    }
                                    thread::sleep(Duration::from_millis(1));
                                }
                                decoded_worker.fetch_add(n as u64, Ordering::Relaxed);
                                progress = true;
                            }
                            Ok(_) => *eof = true,
                            Err(_) => {
                                errors_worker.fetch_add(1, Ordering::Relaxed);
                                *eof = true;
                            }
                        }
                    }
                    if !progress {
                        thread::sleep(Duration::from_millis(1));
                    }
                }
            });
            Ok(Self {
                tracks,
                model,
                source,
                duration: (probe.duration * 48000.).round() as i64,
                wanted,
                generation,
                stop,
                worker: Some(worker),
                decoded,
                errors,
                missing_frames: 0,
            })
        }
        fn frame(&mut self, sample: i64, generation: u64, values: &mut [Stereo; 14]) {
            let source = &self.model.sources[&self.source];
            let mut missing = false;
            for (i, id) in source.active.iter().enumerate() {
                let strip = source.strips.iter().find(|s| s.id == *id).unwrap();
                for (c, m) in strip.mapping.iter().enumerate() {
                    let value = self
                        .tracks
                        .get_mut(&m.track)
                        .and_then(|t| t.at(sample, m.channel, generation));
                    missing |= value.is_none();
                    values[10 + i][c] = value.unwrap_or(0.);
                }
                if strip.mapping.len() == 1 {
                    values[10 + i][1] = values[10 + i][0];
                }
            }
            self.missing_frames += u64::from(missing);
        }
    }
    impl Drop for File {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(w) = self.worker.take() {
                let _ = w.join();
            }
        }
    }
    struct SilenceOnExit;
    impl Drop for SilenceOnExit {
        fn drop(&mut self) {
            native::emergency(true);
        }
    }
    pub fn playback(args: &[String], selected: String) -> Result<serde_json::Value, String> {
        let find = |prefix: &str| {
            args.iter()
                .find_map(|s| s.strip_prefix(prefix).map(str::to_owned))
        };
        let devices = native::devices()?;
        let mut outputs = vec![];
        for uid in selected.split(',') {
            let rows: Vec<_> = devices.iter().filter(|d| d.uid == uid).collect();
            if rows.len() != 1 {
                return Err("Missing or ambiguous requested output UID".into());
            }
            let info = rows[0].clone();
            if info.output_channels < 2 {
                return Err("Output needs a stereo pair".into());
            }
            outputs.push(native::Device::new(info, false)?);
        }
        let seconds = find("--seconds=")
            .unwrap_or("10".into())
            .parse::<f64>()
            .map_err(|e| e.to_string())?;
        if !seconds.is_finite() || !(1.0..=3600.).contains(&seconds) {
            return Err("Diagnostic duration range: 1–3600 seconds".into());
        }
        let synthetic = !args.iter().any(|s| s == "--no-synthetic");
        let mut file = find("--file=")
            .map(|p| {
                File::start(
                    PathBuf::from(p),
                    find("--tracks="),
                    find("--interpretation="),
                )
            })
            .transpose()?;
        let mut mixer = Box::new(Mixer::default());
        for (i, bus) in model::Model::default().buses.iter().enumerate() {
            mixer.buses[i].configure(&bus.dsp);
        }
        for i in 0..10 {
            let strip = model::Strip {
                id: 20 + i as u64,
                name: format!("Signal {}", i + 1),
                mapping: vec![
                    model::Channel {
                        track: 0,
                        channel: 0,
                    },
                    model::Channel {
                        track: 0,
                        channel: 1,
                    },
                ],
                pan: 0.,
                dsp: model::Dsp {
                    fader_db: -36.,
                    ..Default::default()
                },
                sends: std::array::from_fn(|b| if b == i { 1. } else { 0. }),
            };
            mixer.configure(i, &strip);
        }
        let file_strips = if let Some(f) = &file {
            let source = &f.model.sources[&f.source];
            for (i, id) in source.active.iter().enumerate() {
                mixer.configure(10 + i, source.strips.iter().find(|s| s.id == *id).unwrap());
            }
            source.active.len()
        } else {
            0
        };
        let _silence = SilenceOnExit;
        native::emergency(false);
        let start = native::host() + 0.06;
        let mut sample = 0u64;
        let mut frames = 0u64;
        let mut last_sample = 0;
        let mut generation = 1u64;
        let began = Instant::now();
        let mut previous = native::host();
        let mut lifetime_gap = 0f64;
        let mut interval_gap = 0f64;
        let mut interval_started = native::host();
        let mut previous_counters = vec![[0u64; 6]; outputs.len()];
        let mut intervals = vec![];
        let mut bus_energy = [[0f64; 2]; 10];
        let mut nonzero_mix = 0u64;
        // Buffers are sized once for actual physical channels; no per-block/sample heap work.
        let mut buffers: Vec<_> = outputs
            .iter()
            .map(|d| vec![0.; 128 * d.info.output_channels as usize])
            .collect();
        while began.elapsed().as_secs_f64() < seconds {
            let now = native::host();
            let gap = now - previous;
            previous = now;
            lifetime_gap = lifetime_gap.max(gap);
            interval_gap = interval_gap.max(gap);
            let target = start + sample as f64 / 48000.;
            if target - now > 0.06 {
                thread::sleep(Duration::from_micros(500));
                continue;
            }
            let media_sample = if let Some(f) = &file {
                sample as i64 % f.duration
            } else {
                0
            };
            if media_sample < last_sample {
                generation += 1;
            }
            last_sample = media_sample;
            if let Some(f) = &file {
                f.wanted.store(media_sample, Ordering::Release);
                f.generation.store(generation, Ordering::Release);
            }
            for buffer in &mut buffers {
                buffer.fill(0.);
            }
            for i in 0..128 {
                let t = (sample + i as u64) as f64 / 48000.;
                let mut values = [[0.; 2]; 14];
                if synthetic {
                    for (b, value) in values.iter_mut().take(10).enumerate() {
                        let x = (t * std::f64::consts::TAU * (220. + b as f64 * 55.)).sin() as f32
                            * 0.1;
                        *value = [x, x];
                    }
                }
                if let Some(f) = &mut file {
                    f.frame(media_sample + i as i64, generation, &mut values);
                }
                let bus = mixer.process(&values[..10 + file_strips]);
                let mut sum = [0.; 2];
                for b in 0..10 {
                    for c in 0..2 {
                        bus_energy[b][c] += (bus[b][c] as f64).powi(2);
                        sum[c] += bus[b][c];
                    }
                }
                nonzero_mix += u64::from(sum[0] != 0. || sum[1] != 0.);
                for (d, buffer) in outputs.iter().zip(&mut buffers) {
                    let channels = d.info.output_channels as usize;
                    buffer[i * channels] = sum[0];
                    buffer[i * channels + 1] = sum[1];
                }
            }
            for (d, buffer) in outputs.iter().zip(&buffers) {
                frames += d.write(target, buffer, d.info.output_channels as usize) as u64;
            }
            sample += 128;
            if now - interval_started >= 1. {
                let mut devices = vec![];
                for (i, d) in outputs.iter().enumerate() {
                    let counters = d.stats();
                    let delta = std::array::from_fn::<_, 6, _>(|j| {
                        if j == 5 {
                            counters[j]
                        } else {
                            counters[j].saturating_sub(previous_counters[i][j])
                        }
                    });
                    previous_counters[i] = counters;
                    devices.push(serde_json::json!({"uid":d.info.uid,"delta_callbacks_underrun_overrun_input_overrun_invalid_timestamp_queue":delta,"details":d.details()?}));
                }
                intervals.push(serde_json::json!({"start_host":interval_started,"end_host":now,"phase":if began.elapsed().as_secs_f64()<2.{"startup"}else{"steady"},"worker_max_gap_seconds":interval_gap,"devices":devices}));
                interval_started = now;
                interval_gap = 0.;
            }
        }
        let rows=outputs.iter().map(|d|Ok(serde_json::json!({"device":d.info,"counters_callbacks_underrun_overrun_input_overrun_invalid_timestamp_queue":d.stats(),"details":d.details()?}))).collect::<Result<Vec<_>,String>>()?;
        let file_status=file.as_ref().map(|f|serde_json::json!({"decoded_frames":f.decoded.load(Ordering::Relaxed),"decode_errors":f.errors.load(Ordering::Relaxed),"source_missing_frames":f.missing_frames,"generation":generation,"strips":f.model.sources[&f.source].strips,"default_all_tracks_combined":false}));
        let rms = bus_energy.map(|channels| channels.map(|power| (power / sample as f64).sqrt()));
        Ok(
            serde_json::json!({"seconds":began.elapsed().as_secs_f64(),"mixed_samples":sample,"nonzero_mixed_frames":nonzero_mix,"accepted_48k_input_frames":frames,"worker_lifetime_max_gap_seconds":lifetime_gap,"queue_capacity_device_frames":8191,"device_rate_strategy":"AudioConverter high-quality nominal-rate conversion on worker; HAL host-time interpolation for independent-clock correction","intervals":intervals,"devices":rows,"bus_rms":rms,"file":file_status,"physical_listening":"not verified","physical_AV_offset":"not measured","synthetic_buses":if synthetic{10}else{0}}),
        )
    }
}
#[cfg(target_os = "macos")]
fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().collect();
    let find = |prefix: &str| {
        args.iter()
            .find_map(|s| s.strip_prefix(prefix).map(str::to_owned))
    };
    if args.iter().any(|s| s == "--list") {
        println!(
            "{}",
            serde_json::to_string_pretty(&native::devices()?).unwrap()
        );
        return Ok(());
    }
    if args.iter().any(|s| s == "--permission") {
        println!("{}", native::permission(false));
        return Ok(());
    }
    let result = if let Some(path) = find("--decode=") {
        diagnostic::decode(std::path::Path::new(&path))?
    } else if let Some(rate) = find("--resampler-check=") {
        let rate = rate.parse::<f64>().map_err(|e| e.to_string())?;
        serde_json::json!({"in_band":native::resample_measure(rate,440.,false)?,"above_nyquist":native::resample_measure(rate,23000.,false)?,"impulse":native::resample_measure(rate,0.,true)?})
    } else if let Some(devices) = find("--devices=") {
        diagnostic::playback(&args, devices)?
    } else {
        return Err("Use --list, --permission, --decode=FILE, --resampler-check=44100, or --devices=UID[,UID] --seconds=N [--file=FILE --tracks=IDs --interpretation=MODE] [--no-synthetic]".into());
    };
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    Ok(())
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Native audio diagnostics require macOS");
}
