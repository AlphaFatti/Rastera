//! Isolated installed-app workflow. Synthetic file preparation runs on a worker.
use super::*;
use rastera_audio::model::{Channel, Interpretation, Origin};
use std::{collections::BTreeMap, time::Instant};
type Prepared = Result<(Document, Id, Id, Id), String>;
pub(super) struct AudioProbe {
    folder: PathBuf,
    phase: u32,
    started: Instant,
    trial: Instant,
    prepared: Option<mpsc::Receiver<Prepared>>,
    movie: Id,
    movie_audio: Id,
    file_audio: Id,
    mic: Option<Id>,
    pair_ids: Vec<u64>,
    last_sample: u64,
    last_time: Instant,
    intervals: Vec<serde_json::Value>,
    next_report: u64,
    seconds: u64,
    camera: bool,
    camera_source: Option<Id>,
}
impl AudioProbe {
    pub fn new(folder: PathBuf) -> Self {
        Self {
            folder,
            phase: 0,
            started: Instant::now(),
            trial: Instant::now(),
            prepared: None,
            movie: 0,
            movie_audio: 0,
            file_audio: 0,
            mic: None,
            pair_ids: vec![],
            last_sample: 0,
            last_time: Instant::now(),
            intervals: vec![],
            next_report: 0,
            seconds: std::env::args()
                .find_map(|a| {
                    a.strip_prefix("--audio-seconds=")
                        .and_then(|s| s.parse().ok())
                })
                .unwrap_or(600),
            camera: false,
            camera_source: None,
        }
    }
    pub fn tick(
        &mut self,
        e: &mut Editor,
        _preview: &Consumer,
        outputs: &BTreeMap<u64, Consumer>,
        enabled: &mut std::collections::BTreeSet<u64>,
    ) -> Result<bool, String> {
        match self.phase {
            0 => {
                let (tx, rx) = mpsc::sync_channel(1);
                let folder = self.folder.clone();
                thread::spawn(move || {
                    let result = (|| {
                        let mut d = Document::default();
                        let layer = d.add_media(crate::shared_native::media::inspect(
                            &folder.join("4ch.mov"),
                        )?)?;
                        let Content::Media(movie) = d.layer(layer).unwrap().content else {
                            unreachable!()
                        };
                        d.layer_mut(layer).unwrap().rect = Rect {
                            x: 280.,
                            y: 100.,
                            width: 3200.,
                            height: 900.,
                        };
                        d.playbacks.get_mut(&movie).unwrap().looping = Some(true);
                        let probe =
                            crate::shared_native::audio_native::probe(&folder.join("4ch.mov"))?;
                        let track = probe.tracks.first().ok_or("No fixture audio track")?;
                        if track.channels != 4 {
                            return Err("Four-channel fixture did not expose all channels".into());
                        }
                        let movie_audio = d.audio.add(
                            Origin::Playback(movie),
                            "Video four-channel".into(),
                            (0..4)
                                .map(|channel| Channel {
                                    track: track.id,
                                    channel,
                                })
                                .collect(),
                        )?;
                        let probe =
                            crate::shared_native::audio_native::probe(&folder.join("stereo.wav"))?;
                        let track = probe.tracks.first().ok_or("No WAV track")?;
                        let file_audio = d.audio.add(
                            Origin::File {
                                path: folder.join("stereo.wav"),
                                duration: probe.duration,
                            },
                            "WAV file".into(),
                            (0..2)
                                .map(|channel| Channel {
                                    track: track.id,
                                    channel,
                                })
                                .collect(),
                        )?;
                        for s in d.audio.sources.values_mut() {
                            s.enabled = true;
                            s.scenes = vec![d.active_scene];
                            s.playing = true;
                            s.looping = true;
                            for st in &mut s.strips {
                                st.sends = std::array::from_fn(|b| (b + 1) as f32 * 0.05);
                            }
                        }
                        for bus in &mut d.audio.buses {
                            bus.dsp.fader_db = -30.;
                        }
                        d.add_text();
                        d.validate()?;
                        Ok((d, movie, movie_audio, file_audio))
                    })();
                    let _ = tx.send(result);
                });
                self.prepared = Some(rx);
                self.phase = 1;
                return Ok(false);
            }
            1 => {
                let Ok(prepared) = self.prepared.as_ref().unwrap().try_recv() else {
                    return Ok(false);
                };
                let (d, movie, movie_audio, file_audio) = prepared?;
                self.movie = movie;
                self.movie_audio = movie_audio;
                self.file_audio = file_audio;
                self.pair_ids = d.audio.sources[&movie_audio].active.clone();
                e.doc = d;
                e.publish();
                self.phase = 2;
                self.started = Instant::now();
            }
            2 => {
                if self.started.elapsed().as_secs() < 2 || e.audio.status.devices.is_empty() {
                    return Ok(false);
                }
                let speaker = e
                    .audio
                    .status
                    .devices
                    .iter()
                    .find(|d| d.uid == "BuiltInSpeakerDevice" && d.output_channels >= 2)
                    .or_else(|| {
                        e.audio
                            .status
                            .devices
                            .iter()
                            .find(|d| d.output_channels >= 2)
                    })
                    .ok_or("No actual stereo output device")?;
                e.audio.patches.insert(
                    1,
                    crate::shared_native::audio::Patch {
                        uid: speaker.uid.clone(),
                        first: 0,
                    },
                );
                if let Some(external) = e
                    .audio
                    .status
                    .devices
                    .iter()
                    .find(|d| d.name == "Crestron" && d.output_channels >= 2)
                {
                    e.audio.patches.insert(
                        2,
                        crate::shared_native::audio::Patch {
                            uid: external.uid.clone(),
                            first: 0,
                        },
                    );
                }
                if let Some(display) = e
                    .destinations
                    .discovery
                    .displays
                    .iter()
                    .find(|d| !d.builtin)
                {
                    e.destinations.preferences.insert(
                        2,
                        crate::shared_native::destinations::Destination {
                            fullscreen: true,
                            display: Some(display.id.clone()),
                        },
                    );
                }
                enabled.extend([1, 2]);
                e.audio.active = true;
                crate::shared_native::audio_native::emergency(false);
                self.phase = 3;
                self.started = Instant::now();
                log("audio_probe baseline_started=1 quiet_bus_faders_db=-30\n".into());
            }
            3 => {
                if self.started.elapsed().as_secs() < 4 {
                    return Ok(false);
                }
                for id in e.doc.audio.sources[&self.movie_audio]
                    .active
                    .iter()
                    .chain(&e.doc.audio.sources[&self.file_audio].active)
                {
                    if e.audio.status.meters.get(id).is_none_or(|m| m.input < 0.01) {
                        return Err(format!("No real decoded PCM in source strip {id}"));
                    }
                }
                for id in 1..=10 {
                    if e.audio
                        .status
                        .meters
                        .get(&id)
                        .is_none_or(|m| m.output < 1e-6)
                    {
                        return Err(format!("Bus {id} has no mixed signal"));
                    }
                }
                if outputs.len() != 2 {
                    return Err("Native picture outputs did not open".into());
                }
                if e.audio
                    .status
                    .details
                    .values()
                    .filter(|d| d.content_frames > 0 && d.output_peak > 1e-6)
                    .count()
                    < e.audio
                        .patches
                        .values()
                        .map(|p| &p.uid)
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                {
                    return Err("Device callbacks did not contain nonzero mixer content".into());
                }
                let c = e
                    .media
                    .control(&e.doc, self.movie)
                    .ok_or("Missing shared video transport")?;
                c.state.lock().unwrap().playing = false;
                self.phase = 4;
                self.started = Instant::now();
            }
            4 => {
                if self.started.elapsed().as_secs() < 1 {
                    return Ok(false);
                }
                let c = e.media.control(&e.doc, self.movie).unwrap();
                if c.audio_clock.load().is_none_or(|o| o.playing) {
                    return Err("Paused video still marks audio transport playing".into());
                }
                for id in &e.doc.audio.sources[&self.movie_audio].active {
                    if e.audio
                        .status
                        .meters
                        .get(id)
                        .is_some_and(|m| m.input > 1e-5)
                    {
                        return Err("Paused video did not silence its mixer inputs".into());
                    }
                }
                {
                    let mut s = c.state.lock().unwrap();
                    s.seek = Some(1.25);
                    s.playing = true;
                }
                self.phase = 5;
                self.started = Instant::now();
            }
            5 => {
                if self.started.elapsed().as_secs() < 2 {
                    return Ok(false);
                }
                if let Some(o) = e
                    .media
                    .control(&e.doc, self.movie)
                    .and_then(|c| c.audio_clock.load())
                {
                    e.audio.schedule_gain(
                        self.movie_audio,
                        o.at(crate::shared_native::audio_native::host()) + 2400,
                        0.5,
                    )?;
                }
                let channels = e.doc.audio.sources[&self.movie_audio].channels.clone();
                e.doc
                    .audio
                    .interpret(self.movie_audio, Interpretation::QuadMono, channels)?;
                self.phase = 6;
                self.started = Instant::now();
            }
            6 => {
                if self.started.elapsed().as_secs() < 2 {
                    return Ok(false);
                }
                let s = &e.doc.audio.sources[&self.movie_audio];
                if s.active.len() != 4
                    || s.active
                        .iter()
                        .any(|id| e.audio.status.meters.get(id).is_none_or(|m| m.input < 0.01))
                {
                    return Err("Quad mono did not preserve all four decoded channels".into());
                }
                let channels = s.channels.clone();
                e.doc
                    .audio
                    .interpret(self.movie_audio, Interpretation::DualStereo, channels)?;
                if e.doc.audio.sources[&self.movie_audio].active != self.pair_ids {
                    return Err("Interpretation did not restore stable stereo strip IDs".into());
                }
                let layer = e
                    .doc
                    .layers
                    .iter()
                    .find(|l| l.content == Content::Media(self.movie))
                    .unwrap()
                    .id;
                let count = e.doc.audio.sources.len();
                e.doc.duplicate(layer).unwrap();
                if e.doc.audio.sources.len() != count {
                    return Err("Visual duplicate duplicated audio".into());
                }
                let next = e.doc.add_scene(true)?;
                e.doc.select_scene(next);
                e.publish();
                self.phase = 7;
                self.started = Instant::now();
            }
            7 => {
                if self.started.elapsed().as_secs() < 2 {
                    return Ok(false);
                }
                if e.doc.audio_participation().len() != 2 {
                    return Err("Duplicated scene lost shared/audio-only participation".into());
                }
                let mut candidate = e.doc.clone();
                e.audio.snapshot_document(&mut candidate);
                let file = self.folder.join("audio.rastera.json");
                let (tx, rx) = mpsc::sync_channel(1);
                thread::spawn(move || {
                    let result = scene::save_project(&candidate, &file)
                        .and_then(|()| scene::load_project(&file))
                        .and_then(|loaded| {
                            if loaded == candidate {
                                Ok((loaded, 0, 0, 0))
                            } else {
                                Err("Audio project round trip differs".into())
                            }
                        });
                    let _ = tx.send(result);
                });
                self.prepared = Some(rx);
                self.phase = 8;
            }
            8 => {
                let Ok(result) = self.prepared.as_ref().unwrap().try_recv() else {
                    return Ok(false);
                };
                result?;
                e.project = Some(self.folder.join("audio.rastera.json"));
                if std::env::args().any(|a| a.starts_with("--audio-input=")) {
                    crate::shared_native::audio_native::permission(true);
                }
                self.phase = 9;
                self.started = Instant::now();
            }
            9 => {
                if !e.audio.active {
                    return Err("Save As unexpectedly disabled running audio".into());
                }
                if self.camera_source.is_none() && e.capture.authorization() == 3 {
                    if let Some(device) = e
                        .capture
                        .devices
                        .iter()
                        .find(|d| d.name.contains("FaceTime"))
                    {
                        if let Some(mode) = device
                            .modes
                            .iter()
                            .find(|m| {
                                m.width == 1280 && m.height == 720 && (m.fps - 30.).abs() < 0.1
                            })
                            .cloned()
                        {
                            let layer = e.doc.add_capture(rastera_core::capture::Source {
                                device_id: device.id.clone(),
                                name: device.name.clone(),
                                mode,
                            })?;
                            if let Content::Capture(id) = e.doc.layer(layer).unwrap().content {
                                self.camera_source = Some(id);
                            }
                            e.doc.layer_mut(layer).unwrap().rect = Rect {
                                x: 3100.,
                                y: 650.,
                                width: 640.,
                                height: 360.,
                            };
                            e.publish();
                        }
                    }
                }
                let uid = std::env::args()
                    .find_map(|a| a.strip_prefix("--audio-input=").map(str::to_owned));
                if let Some(uid) = uid {
                    let permission = crate::shared_native::audio_native::permission(false);
                    if permission == 0 && self.started.elapsed().as_secs() < 30 {
                        return Ok(false);
                    }
                    if permission == 3 {
                        if let Some(d) = e
                            .audio
                            .status
                            .devices
                            .iter()
                            .find(|d| d.uid == uid && d.input_channels > 0)
                        {
                            let id = e.doc.audio.add(
                                self.camera_source.map_or_else(
                                    || Origin::Input { uid: d.uid.clone() },
                                    Origin::Capture,
                                ),
                                format!("Explicit test input: {}", d.name),
                                vec![Channel {
                                    track: 0,
                                    channel: 0,
                                }],
                            )?;
                            let s = e.doc.audio.sources.get_mut(&id).unwrap();
                            s.enabled = true;
                            s.input_uid = d.uid.clone();
                            s.always_on = true;
                            for st in &mut s.strips {
                                st.sends = [0.; 10];
                            }
                            self.mic = Some(id);
                        } else {
                            log("audio_probe capture_input=missing_requested_device\n".into());
                        }
                    } else {
                        log(format!("audio_probe capture_input=permission_unavailable status={permission}\n"));
                    }
                }
                e.audio.open = true;
                self.trial = Instant::now();
                self.last_time = Instant::now();
                self.last_sample = e.audio.status.samples;
                self.phase = 10;
                log(format!(
                    "audio_trial started=1 seconds={} capture={}\n",
                    self.seconds,
                    self.mic.is_some() as u8
                ));
            }
            10 => {
                let elapsed = self.trial.elapsed().as_secs();
                if elapsed >= self.next_report {
                    self.next_report = elapsed + 1;
                    let now = Instant::now();
                    let sample = e.audio.status.samples;
                    let input = self
                        .mic
                        .and_then(|id| e.doc.audio.sources.get(&id))
                        .and_then(|s| s.active.first())
                        .and_then(|id| e.audio.status.meters.get(id))
                        .map_or(0., |m| m.input);
                    if let Some(id) = self.camera_source {
                        self.camera |= e.capture.status(&e.doc, id).frames > 5;
                    }
                    let record = serde_json::json!({"elapsed_seconds":self.trial.elapsed().as_secs_f64(),"interval_seconds":now.duration_since(self.last_time).as_secs_f64(),"mixed_sample_delta":sample.saturating_sub(self.last_sample),"late_blocks_lifetime":e.audio.status.late_blocks,"device_counters":e.audio.status.stats,"device_details":e.audio.status.details,"source_stats":e.audio.status.source_stats,"source_formats":e.audio.status.source_formats,"input_peak":input,"errors":e.audio.status.errors});
                    self.intervals.push(record);
                    self.last_time = now;
                    self.last_sample = sample;
                    log(format!(
                        "audio_trial elapsed={elapsed} sample={sample} input_peak={input}\n"
                    ));
                }
                if elapsed < self.seconds {
                    return Ok(false);
                }
                let file = self.folder.join("audio-trial-results.json");
                let result = serde_json::json!({"requested_seconds":self.seconds,"elapsed_seconds":self.trial.elapsed().as_secs_f64(),"capture_input_available":self.mic.is_some(),"camera_frame_delivery_verified":self.camera,"devices":e.audio.status.devices,"intervals":self.intervals,"native_outputs":outputs.len(),"all_buses_mixed":10,"physical_listening":"pending","physical_av_offset_loopback":"unavailable","interpretation":"4-channel track DualStereo→QuadMono→DualStereo, original strip IDs preserved"});
                std::fs::write(file, serde_json::to_vec_pretty(&result).unwrap())
                    .map_err(|e| e.to_string())?;
                log("audio_probe passed=1 file_pcm=1 pause_silence=1 seek=1 quad_channels=1 mode_ids=1 layer_dedup=1 scene=1 roundtrip=1 trial_complete=1\n".into());
                self.phase = 11;
                return Ok(true);
            }
            _ => return Ok(true),
        }
        Ok(false)
    }
}
