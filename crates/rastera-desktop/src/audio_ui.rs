use super::*;
use eframe::egui;
use rastera_audio::model::{Channel, Interpretation};
impl Audio {
    pub fn ui(&mut self, ui: &mut egui::Ui, doc: &mut Document) -> bool {
        let mut import = false;
        ui.heading("Audio Mixer · Live Edit — changes affect outputs");
        ui.horizontal(|ui| {
            if ui
                .button(if self.active {
                    "Disable Audio Outputs"
                } else {
                    "Enable Audio Outputs"
                })
                .clicked()
            {
                self.active = !self.active;
                native::emergency(!self.active);
            }
            if ui.button("Mute All Audio · ⇧⌘M").clicked() {
                self.active = false;
                native::emergency(true);
            }
            import = ui.button("Import audio file…").clicked();
        });
        ui.label("48 kHz float · sends are post-source-fader · mono pan: equal power, −3 dB center · startup audio activation is always off");
        for e in self.status.errors.iter().rev().take(3) {
            ui.colored_label(egui::Color32::YELLOW, e);
        }
        ui.label(format!(
            "Mixed samples {} · late blocks {}",
            self.status.samples, self.status.late_blocks
        ));
        ui.collapsing("Add embedded playback audio", |ui| {
            for (&id, p) in &doc.playbacks {
                let asset = &doc.media_assets[&p.asset];
                if asset.gif {
                    continue;
                }
                let exists = doc
                    .audio
                    .sources
                    .values()
                    .any(|s| s.origin == Origin::Playback(id));
                if ui
                    .add_enabled(
                        !exists,
                        egui::Button::new(format!(
                            "Probe playback {id} — {}",
                            asset.path.file_name().unwrap_or_default().to_string_lossy()
                        )),
                    )
                    .clicked()
                {
                    self.inspect(
                        asset.path.clone(),
                        Origin::Playback(id),
                        format!("Playback {id}"),
                        None,
                    );
                }
            }
        });
        ui.collapsing("Add explicitly selected audio input",|ui|{ui.label("No default microphone association. Select the exact exposed audio device; capture audio may be separate from its picture device.");for d in self.status.devices.clone().into_iter().filter(|d|d.input_channels>0){if ui.button(format!("Add {} · {} ch · {} Hz",d.name,d.input_channels,d.rate)).clicked(){native::permission(true);let count=match d.input_channels{1=>1,2|3=>2,_=>4};let channels=(0..count).map(|channel|Channel{track:0,channel}).collect();match doc.audio.add(Origin::Input{uid:d.uid},d.name,channels){Ok(id)=>{let s=doc.audio.sources.get_mut(&id).unwrap();s.enabled=true;s.scenes=vec![doc.active_scene];},Err(e)=>self.status.errors.push(e)}}}ui.label(format!("Microphone authorization: {}",match native::permission(false){0=>"not requested",1=>"restricted",2=>"denied",3=>"authorized",_=>"unknown"}));});
        while let Ok(result) = self.probe_rx.try_recv() {
            match result {
                Ok((path, origin, name, probe, edit)) => {
                    let selected = probe
                        .tracks
                        .first()
                        .map(|t| {
                            (0..t.channels.min(4))
                                .map(|channel| Channel {
                                    track: t.id,
                                    channel,
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    self.pending = Some(Pending {
                        path,
                        origin,
                        name,
                        probe,
                        selected,
                        edit,
                    });
                }
                Err(e) => self.status.errors.push(e),
            }
        }
        if let Some(mut pending) = self.pending.take() {
            let mut done = false;
            ui.group(|ui| {
                ui.heading("Explicit track / channel selection");
                ui.label("Only the first track is selected by default. Alternate-language tracks are never combined automatically.");
                for track in &pending.probe.tracks {
                    ui.label(format!("Track {} · {} · {} Hz · {} channels · start {:.6}s",track.id,track.language,track.rate,track.channels,track.start));
                    ui.horizontal_wrapped(|ui| {
                        for c in 0..track.channels.min(32) {
                            let mapping=Channel{track:track.id,channel:c};
                            let mut selected=pending.selected.contains(&mapping);
                            if ui.checkbox(&mut selected,format!("T{} Ch{}",track.id,c+1)).changed() {
                                if selected {pending.selected.push(mapping);} else {pending.selected.retain(|c|*c!=mapping);}
                            }
                        }
                    });
                }
                let mut swap=None;
                for (i,mapping) in pending.selected.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(format!("Slot {} ← Track {} Channel {}",i+1,mapping.track,mapping.channel+1));
                        if i>0&&ui.small_button("↑").clicked(){swap=Some((i,i-1));}
                        if i+1<pending.selected.len()&&ui.small_button("↓").clicked(){swap=Some((i,i+1));}
                    });
                }
                if let Some((a,b))=swap {pending.selected.swap(a,b);}
                let valid=matches!(pending.selected.len(),1|2|4);
                if ui.add_enabled(valid,egui::Button::new("Apply selected channels")).clicked() {
                    let mode=pending.edit.and_then(|id|doc.audio.sources.get(&id)).map(|s|s.interpretation).filter(|m|m.channels()==pending.selected.len()).unwrap_or(match pending.selected.len(){1=>Interpretation::Mono,2=>Interpretation::Stereo,_=>Interpretation::DualStereo});
                    let result=if let Some(id)=pending.edit {doc.audio.interpret(id,mode,pending.selected.clone()).map(|()|id)} else {
                        let origin=match pending.origin {Origin::File{..}=>Origin::File{path:pending.path.clone(),duration:pending.probe.duration},ref other=>other.clone()};
                        doc.audio.add(origin,pending.name.clone(),pending.selected.clone())
                    };
                    match result {Ok(id)=>{if pending.edit.is_none(){let source=doc.audio.sources.get_mut(&id).unwrap();source.scenes=vec![doc.active_scene];source.enabled=true;}done=true;},Err(e)=>self.status.errors.push(e)}
                }
                if ui.button("Cancel channel selection").clicked(){done=true;}
            });
            if !done {
                self.pending = Some(pending);
            }
        }
        let mut remove = None;
        let ids: Vec<_> = doc.audio.sources.keys().copied().collect();
        for id in ids {
            let mut reinterpret = None;
            let source_path = match &doc.audio.sources[&id].origin {
                Origin::Playback(p) => doc
                    .playbacks
                    .get(p)
                    .and_then(|p| doc.media_assets.get(&p.asset))
                    .map(|a| a.path.clone()),
                Origin::File { path, .. } => Some(path.clone()),
                _ => None,
            };
            let scenes: Vec<_> = doc.scenes.iter().map(|s| (s.id, s.name.clone())).collect();
            let captures: Vec<_> = doc
                .captures
                .iter()
                .map(|(&id, s)| (id, s.name.clone()))
                .collect();
            let source = doc.audio.sources.get_mut(&id).unwrap();
            egui::CollapsingHeader::new(format!("{} · source {id}", source.name))
                .id_salt(("audio-source", id))
                .default_open(true)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut source.name);
                        ui.checkbox(&mut source.enabled, "Enabled");
                        ui.checkbox(&mut source.always_on, "Always On");
                        if ui.button("Remove source").clicked() {
                            remove = Some(id);
                        }
                    });
                    if matches!(source.origin,Origin::File{..}|Origin::Input{..}){ui.collapsing("Scene participation (Always On overrides)",|ui|{for(scene,name)in &scenes{let mut member=source.scenes.contains(scene);if ui.checkbox(&mut member,name).changed(){if member{source.scenes.push(*scene);}else{source.scenes.retain(|id|id!=scene);}}}});}
                    if let Some(format)=self.status.source_formats.get(&id){ui.label(format);}
                    if let Some(path)=source_path {if ui.button("Select tracks / channel order…").clicked(){self.inspect(path,source.origin.clone(),source.name.clone(),Some(id));}}
                    if matches!(source.origin,Origin::Input{..}|Origin::Capture(_)) {
                        let uid=match &source.origin{Origin::Input{uid}=>uid.clone(),_=>source.input_uid.clone()};
                        let mut link=match source.origin{Origin::Capture(id)=>Some(id),_=>None};
                        egui::ComboBox::from_id_salt(("audio-picture-link",id)).selected_text(link.map_or("Standalone scene input".to_owned(),|id|format!("Linked picture source {id}"))).show_ui(ui,|ui|{ui.selectable_value(&mut link,None,"Standalone scene input");for(capture,name)in &captures{ui.selectable_value(&mut link,Some(*capture),format!("Explicitly link {name} [{capture}]"));}});
                        source.origin=link.map_or_else(||Origin::Input{uid:uid.clone()},Origin::Capture);source.input_uid=uid.clone();
                        if let Some(d)=self.status.devices.iter().find(|d|d.uid==uid){ui.label(format!("Actual input: {} Hz, {} channels",d.rate,d.input_channels));for c in &source.channels{if c.channel>=d.input_channels{ui.colored_label(egui::Color32::YELLOW,format!("Requested channel {} unavailable; silent and retained",c.channel+1));}}if ui.button("Select input channels…").clicked(){self.pending=Some(Pending{path:PathBuf::new(),origin:source.origin.clone(),name:source.name.clone(),probe:native::Probe{tracks:vec![native::Track{id:0,channels:d.input_channels,rate:d.rate,start:0.,duration:0.,language:"discrete device channels".into(),segments:vec![]}],duration:0.},selected:source.channels.clone(),edit:Some(id)});}}else{ui.colored_label(egui::Color32::YELLOW,"Requested input missing; mapping retained, input silent");}
                        ui.small("Live input: 120 ms base software delay plus this offset and device/SRC latency; minimum offset −20 ms.");
                    }
                    ui.horizontal(|ui| {
                        ui.label("Sync offset (positive = audio delay), ms");
                        ui.add(
                            egui::DragValue::new(&mut source.sync_ms)
                                .range(if matches!(source.origin,Origin::Input{..}|Origin::Capture(_)){-20.0..=500.0}else{-500.0..=500.0})
                                .speed(1.),
                        );
                    });
                    let mode = source.interpretation;
                    ui.horizontal(|ui| {
                        ui.label("Channel Interpretation");
                        match source.channels.len() {
                            1 => {
                                ui.label("Mono");
                            }
                            2 => {
                                ui.selectable_value(
                                    &mut source.interpretation,
                                    Interpretation::Stereo,
                                    "Stereo (1/2)",
                                );
                                ui.selectable_value(
                                    &mut source.interpretation,
                                    Interpretation::DualMono,
                                    "Dual Mono (1 and 2)",
                                );
                            }
                            4 => {
                                ui.selectable_value(
                                    &mut source.interpretation,
                                    Interpretation::DualStereo,
                                    "Dual Stereo (1/2 and 3/4)",
                                );
                                ui.selectable_value(
                                    &mut source.interpretation,
                                    Interpretation::QuadMono,
                                    "Quad Mono (1, 2, 3, 4)",
                                );
                            }
                            _ => {}
                        }
                    });
                    if let Origin::File { duration, .. } = source.origin {
                        if let Some(o)=self.clocks.get(&id).and_then(|clock|clock.load()){ui.label(format!("Playhead {:.3} s",o.at(native::host())as f64/48000.));}
                        ui.checkbox(&mut source.restart_on_activation,"Restart when scene activates");
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut source.playing, "Play").changed(){let command=if source.playing{rastera_audio::timing::Command::Play}else{rastera_audio::timing::Command::Pause};if let Err(e)=self.command_source(id,command){self.status.errors.push(e);}}
                            ui.checkbox(&mut source.looping, "Loop");
                            let mut seek = source.resume_sample as f64 / 48000.;
                            if ui
                                .add(
                                    egui::DragValue::new(&mut seek)
                                        .range(0.0..=duration)
                                        .suffix(" s"),
                                )
                                .changed()
                            {
                                source.resume_sample = (seek * 48000.).round() as i64;
                                if let Err(e)=self.command_source(id,rastera_audio::timing::Command::Seek(source.resume_sample)){self.status.errors.push(e);}
                            }
                            if ui.button("Restart").clicked() {
                                source.resume_sample = 0;
                                source.playing = true;
                                if let Err(e)=self.command_source(id,rastera_audio::timing::Command::Restart){self.status.errors.push(e);}
                            }
                        });
                    } else if matches!(source.origin, Origin::Playback(_)) {
                        ui.label(
                            "Transport is shared with this video's Play/Pause/Seek/Loop controls.",
                        );
                    }
                    for sid in source.active.clone() {
                        if let Some(st) = source.strips.iter_mut().find(|s| s.id == sid) {
                            ui.group(|ui| {
                                ui.text_edit_singleline(&mut st.name);
                                ui.label(
                                    st.mapping
                                        .iter()
                                        .map(|c| {
                                            format!("Track {}, channel {}", c.track, c.channel + 1)
                                        })
                                        .collect::<Vec<_>>()
                                        .join(" → "),
                                );
                                if st.mapping.len() == 1 {
                                    ui.add(
                                        egui::Slider::new(&mut st.pan, -1.0..=1.0)
                                            .text("Pan L / R"),
                                    );
                                }
                                strip(ui, &mut st.dsp, self.status.meters.get(&sid),false);
                                ui.collapsing("Ten post-fader sends (0 = Off)", |ui| {
                                    for b in 0..10 {
                                        ui.add(
                                            egui::Slider::new(&mut st.sends[b], 0.0..=2.0)
                                                .text(format!("Bus {} linear gain", b + 1)),
                                        );
                                    }
                                });
                            });
                        }
                    }
                    if source.interpretation != mode {
                        let next = source.interpretation;
                        source.interpretation = mode;
                        let channels = source.channels.clone();
                        reinterpret = Some((next, channels));
                    }
                });
            if let Some((mode, channels)) = reinterpret {
                if let Err(e) = doc.audio.interpret(id, mode, channels) {
                    self.status.errors.push(e);
                }
            }
        }
        if let Some(id) = remove {
            doc.audio.sources.remove(&id);
            doc.prune_unused();
        }
        ui.separator();
        ui.heading("Ten persistent stereo output buses");
        let before = self.patches.clone();
        for bus in &mut doc.audio.buses {
            egui::CollapsingHeader::new(format!("{} · bus {}", bus.name, bus.id))
                .id_salt(("audio-bus", bus.id))
                .show(ui, |ui| {
                    ui.text_edit_singleline(&mut bus.name);
                    strip(ui, &mut bus.dsp, self.status.meters.get(&bus.id), true);
                    let patch = self.patches.entry(bus.id).or_default();
                    egui::ComboBox::from_id_salt(("audio-device", bus.id))
                        .selected_text(
                            self.status
                                .devices
                                .iter()
                                .find(|d| d.uid == patch.uid)
                                .map_or("Unassigned / missing", |d| d.name.as_str()),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut patch.uid, String::new(), "Unassigned");
                            for d in self
                                .status
                                .devices
                                .iter()
                                .filter(|d| d.output_channels >= 2)
                            {
                                ui.selectable_value(
                                    &mut patch.uid,
                                    d.uid.clone(),
                                    format!(
                                        "{} · {} Hz · {} ch",
                                        d.name, d.rate, d.output_channels
                                    ),
                                );
                            }
                        });
                    if let Some(d) = self.status.devices.iter().find(|d| d.uid == patch.uid) {
                        egui::ComboBox::from_id_salt(("audio-channel-pair", bus.id))
                            .selected_text(format!(
                                "Channels {}/{}",
                                patch.first + 1,
                                patch.first + 2
                            ))
                            .show_ui(ui, |ui| {
                                for first in 0..d.output_channels.saturating_sub(1) {
                                    let left = d
                                        .output_names
                                        .get(first as usize)
                                        .map_or("", String::as_str);
                                    let right = d
                                        .output_names
                                        .get(first as usize + 1)
                                        .map_or("", String::as_str);
                                    ui.selectable_value(
                                        &mut patch.first,
                                        first,
                                        format!(
                                            "{}/{} · {} / {}",
                                            first + 1,
                                            first + 2,
                                            left,
                                            right
                                        ),
                                    );
                                }
                            });
                    }
                    if before
                        .values()
                        .filter(|p| {
                            p.uid == patch.uid && p.first == patch.first && !p.uid.is_empty()
                        })
                        .count()
                        > 1
                    {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            "Shared physical pair: buses SUM in one stream",
                        );
                    }
                    ui.label(format!(
                        "Requested {} — Ch {}/{}",
                        patch.uid,
                        patch.first + 1,
                        patch.first + 2
                    ));
                    let d = self.status.devices.iter().find(|d| d.uid == patch.uid);
                    let state = if patch.uid.is_empty() {
                        "Unassigned"
                    } else if d.is_none() {
                        "Missing device; assignment retained"
                    } else if patch.first + 1 >= d.unwrap().output_channels {
                        "Unavailable channel pair"
                    } else if !self.active {
                        "Disabled"
                    } else if self.status.stats.contains_key(&patch.uid) {
                        "Active device stream"
                    } else {
                        "Opening / error"
                    };
                    ui.label(state);
                    if let Some(d) = d {
                        ui.label(format!(
                            "{} frames buffer · reported output latency {} frames · {:?}",
                            d.buffer_frames,
                            d.latency_frames,
                            self.status.stats.get(&d.uid)
                        ));
                    }
                });
        }
        if before != self.patches {
            self.save_patches();
        }
        ui.label("Buses patched to the same physical channels are summed in one device stream. Final device samples are bounded to ±1; bus meters are before physical summing.");
        import
    }
}
fn strip(
    ui: &mut egui::Ui,
    d: &mut rastera_audio::model::Dsp,
    meter: Option<&rastera_audio::dsp::Meter>,
    bus: bool,
) {
    ui.horizontal(|ui| {
        ui.checkbox(&mut d.mute, "Mute");
        ui.checkbox(&mut d.bypass, "EQ/compressor bypass");
        if ui.button("Reset strip").clicked() {
            let limiter = d.limiter_db;
            *d = Default::default();
            d.limiter_db = limiter;
        }
    });
    ui.add(egui::Slider::new(&mut d.trim_db, -96.0..=24.0).text("Input trim dB"));
    ui.add(egui::Slider::new(&mut d.fader_db, -96.0..=12.0).text("Fader dB"));
    if let Some(m) = meter {
        ui.label(format!(
            "In {:.1} dBFS · Out {:.1} dBFS · GR {:.1} dB {} · non-finite {}",
            20. * m.input.max(1e-8).log10(),
            20. * m.output.max(1e-8).log10(),
            m.reduction_db,
            if m.clipped { "CLIP" } else { "" },
            m.non_finite
        ));
    }
    ui.collapsing("EQ and high-pass", |ui| {
        ui.add(
            egui::Slider::new(&mut d.high_pass_hz, 0.0..=20000.0).text("High-pass Hz (0 = Off)"),
        );
        for (i, b) in d.eq.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("Band {}", i + 1));
                ui.add(
                    egui::DragValue::new(&mut b.hz)
                        .range(20.0..=20000.0)
                        .suffix(" Hz"),
                );
                ui.add(
                    egui::DragValue::new(&mut b.db)
                        .range(-24.0..=24.0)
                        .suffix(" dB"),
                );
                ui.add(
                    egui::DragValue::new(&mut b.q)
                        .range(0.1..=18.0)
                        .prefix("Q "),
                );
            });
        }
    });
    ui.collapsing("Stereo-linked compressor", |ui| {
        let c = &mut d.compressor;
        ui.checkbox(&mut c.bypass, "Compressor bypass");
        ui.add(egui::Slider::new(&mut c.threshold_db, -80.0..=0.0).text("Threshold dBFS"));
        ui.add(egui::Slider::new(&mut c.ratio, 1.0..=40.0).text("Ratio :1"));
        ui.add(egui::Slider::new(&mut c.attack_ms, 0.1..=1000.0).text("Attack ms"));
        ui.add(egui::Slider::new(&mut c.release_ms, 1.0..=5000.0).text("Release ms"));
        ui.add(egui::Slider::new(&mut c.knee_db, 0.0..=24.0).text("Knee dB"));
        ui.add(egui::Slider::new(&mut c.makeup_db, 0.0..=24.0).text("Makeup dB"));
        if ui.button("Reset compressor").clicked() {
            *c = Default::default();
        }
    });
    if bus {
        let mut enabled = d.limiter_db.is_some();
        if ui.checkbox(&mut enabled, "Final limiter enabled").changed() {
            d.limiter_db = enabled.then_some(-1.);
        }
    }
    if let Some(limit) = &mut d.limiter_db {
        ui.add(egui::Slider::new(limit, -24.0..=0.0).text("Final limiter ceiling dBFS"));
    }
}
