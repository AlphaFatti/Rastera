use super::*;
use std::time::Instant;
pub(super) struct MediaProbe {
    phase: u32,
    start: Instant,
    source: Id,
    sequence: u64,
    folder: PathBuf,
    camera: Option<Id>,
    last_scene: u64,
    crop_before: Option<Document>,
    crop_history: Option<History>,
}
impl MediaProbe {
    pub fn new(folder: PathBuf) -> Self {
        Self {
            phase: 0,
            start: Instant::now(),
            source: 0,
            sequence: 0,
            folder,
            camera: None,
            last_scene: 1,
            crop_before: None,
            crop_history: None,
        }
    }
    pub fn tick(
        &mut self,
        e: &mut Editor,
        preview: &Consumer,
        enabled: &mut std::collections::BTreeSet<u64>,
    ) -> Result<bool, String> {
        match self.phase {
            0 => {
                let asset = crate::shared_native::media::inspect(&self.folder.join("test.mp4"))?;
                let layer = e.doc.add_media(asset)?;
                e.doc.layer_mut(layer).unwrap().rect = Rect {
                    x: 100.,
                    y: 100.,
                    width: 960.,
                    height: 540.,
                };
                if let Content::Media(id) = e.doc.layer(layer).unwrap().content {
                    self.source = id;
                }
                let gif = crate::shared_native::media::inspect(&self.folder.join("test.gif"))?;
                let id = e.doc.add_media(gif)?;
                e.doc.layer_mut(id).unwrap().rect = Rect {
                    x: 1200.,
                    y: 100.,
                    width: 640.,
                    height: 360.,
                };
                e.doc.add_text();
                e.publish();
                enabled.insert(1);
                self.phase = 1;
                self.start = Instant::now();
            }
            1 => {
                if self.start.elapsed().as_secs() < 3 {
                    return Ok(false);
                }
                let c = e
                    .media
                    .control(&e.doc, self.source)
                    .ok_or("No video worker")?;
                let f =
                    c.slot.persistent_snapshot().flatten().ok_or_else(|| {
                        format!("Video has no frame: {:?}", c.state.lock().unwrap())
                    })?;
                if f.sequence < 10 {
                    return Err(format!("Video cadence did not advance: {}", f.sequence));
                }
                c.state.lock().unwrap().playing = false;
                self.sequence = f.sequence;
                self.phase = 2;
                self.start = Instant::now();
            }
            2 => {
                if self.start.elapsed().as_secs() < 3 {
                    return Ok(false);
                }
                let c = e.media.control(&e.doc, self.source).unwrap();
                let f = c
                    .slot
                    .persistent_snapshot()
                    .flatten()
                    .ok_or("Paused video incorrectly became stale")?;
                if f.sequence > self.sequence + 3 {
                    return Err("Paused video kept decoding".into());
                }
                c.state.lock().unwrap().seek = Some(0.5);
                c.state.lock().unwrap().playing = true;
                self.phase = 3;
                self.start = Instant::now();
            }
            3 => {
                if self.start.elapsed().as_secs() < 2 {
                    return Ok(false);
                }
                scene::save_project(&e.doc, &self.folder.join("media.rastera.json"))?;
                let d = scene::load_project(&self.folder.join("media.rastera.json"))?;
                if d != e.doc {
                    std::fs::write(
                        self.folder.join("before.json"),
                        serde_json::to_vec_pretty(&e.doc).unwrap(),
                    )
                    .unwrap();
                    std::fs::write(
                        self.folder.join("after.json"),
                        serde_json::to_vec_pretty(&d).unwrap(),
                    )
                    .unwrap();
                    return Err("Media project round trip differs".into());
                }
                log("media_probe passed=1 video_advances=1 paused_frame_valid=1 gif_prepared=1 save_reopen=1\n".into());
                e.doc.playbacks.get_mut(&self.source).unwrap().looping = Some(true);
                e.capture.refresh();
                if let Some(device) = e
                    .capture
                    .devices
                    .iter()
                    .find(|d| d.name.contains("FaceTime"))
                {
                    let mode = device
                        .modes
                        .iter()
                        .find(|m| m.width == 1280 && m.height == 720 && (m.fps - 30.).abs() < 0.1)
                        .cloned()
                        .ok_or("Camera 720p30 unavailable")?;
                    let source = rastera_core::capture::Source {
                        device_id: device.id.clone(),
                        name: device.name.clone(),
                        mode,
                    };
                    let layer = e.doc.add_capture(source)?;
                    if let Content::Capture(id) = e.doc.layer(layer).unwrap().content {
                        self.camera = Some(id);
                    }
                    e.doc.layer_mut(layer).unwrap().rect = Rect {
                        x: 2000.,
                        y: 100.,
                        width: 1280.,
                        height: 720.,
                    };
                    let copy = e.doc.duplicate(layer).unwrap();
                    e.doc
                        .layer_mut(copy)
                        .unwrap()
                        .set_crop(rastera_core::crop::Crop {
                            left: 0.25,
                            top: 0.1,
                            right: 0.75,
                            bottom: 0.9,
                        })?;
                    e.doc.layer_mut(copy).unwrap().rect.x = 2300.;
                }
                let image = e.doc.add_image(scene::inspect_decoded_image(
                    &std::env::current_dir().unwrap().join("examples/alpha.png"),
                )?);
                e.doc.layer_mut(image).unwrap().rect = Rect {
                    x: 200.,
                    y: 800.,
                    width: 480.,
                    height: 220.,
                };
                e.doc.background = rastera_core::background::Background::Solid([20, 35, 55, 255]);
                e.doc.name = "Solid + mixed sources".into();
                let first = e.doc.active_scene;
                let second = e.doc.add_scene(true)?;
                e.doc.name = "Shared looping background".into();
                e.doc.background = rastera_core::background::Background::Media {
                    source: self.source,
                    fit: rastera_core::background::Fit::Fill,
                };
                let third = e.doc.add_scene(true)?;
                e.doc.name = "Image backdrop".into();
                let Content::Image(image) = e.doc.scenes[0].layers.last().unwrap().content else {
                    panic!()
                };
                e.doc.background = rastera_core::background::Background::Image {
                    source: image,
                    fit: rastera_core::background::Fit::Fit,
                };
                for v in &mut e.doc.viewports {
                    v.fit_canvas(e.doc.canvas);
                }
                if let Some(screen) = e
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
                            display: Some(screen.id.clone()),
                        },
                    );
                    enabled.insert(2);
                }
                for id in [third, first, second, first, third, second] {
                    e.doc.select_scene(id);
                    e.publish();
                }
                self.last_scene = second;
                self.phase = 4;
                self.start = Instant::now();
            }
            4 => {
                if self.start.elapsed().as_secs() < 4 {
                    return Ok(false);
                }
                if e.doc.active_scene != self.last_scene || e.engine.applied() != e.revision {
                    return Err("Newest scene did not become live".into());
                }
                for &id in e.doc.playbacks.keys() {
                    let c = e.media.control(&e.doc, id).ok_or("Missing media worker")?;
                    let f = c
                        .slot
                        .persistent_snapshot()
                        .flatten()
                        .ok_or("Mixed source has no frame")?;
                    if f.sequence < 5 {
                        return Err("Mixed media did not advance".into());
                    }
                }
                if let Some(id) = self.camera {
                    let s = e.capture.status(&e.doc, id);
                    if s.phase != "Live" || s.frames < 20 || s.uploaded < 5 {
                        return Err(format!("Mixed camera not live: {}", s.phase));
                    }
                }
                let path = self.folder.join("combined.rastera.json");
                scene::save_project(&e.doc, &path)?;
                let loaded = scene::load_project(&path)?;
                if loaded != e.doc {
                    return Err("Scene project round trip differs".into());
                }
                e.project = Some(std::fs::canonicalize(path).unwrap());
                e.saved = Some(e.doc.clone());
                e.doc.name = "Unsaved recovered scene".into();
                e.publish();
                let layer = e
                    .doc
                    .layers
                    .iter()
                    .find(|l| matches!(l.content, Content::Media(_)))
                    .unwrap()
                    .clone();
                e.selected = Some(layer.id);
                e.crop_edit = true;
                self.crop_before = Some(e.doc.clone());
                self.crop_history = Some(e.history.clone());
                Probe::event(preview, 1, [layer.rect.x, layer.rect.y]);
                Probe::event(
                    preview,
                    2,
                    [
                        layer.rect.x + layer.rect.width * 0.1,
                        layer.rect.y + layer.rect.height * 0.1,
                    ],
                );
                Probe::event(
                    preview,
                    3,
                    [
                        layer.rect.x + layer.rect.width * 0.1,
                        layer.rect.y + layer.rect.height * 0.1,
                    ],
                );
                self.phase = 5;
                self.start = Instant::now();
            }
            5 => {
                let before = self.crop_before.take().unwrap();
                let after = e.doc.clone();
                let mut expected = self.crop_history.take().unwrap();
                expected.record(before.clone(), &after);
                if e.history != expected || before == after {
                    return Err("Native crop gesture history mismatch".into());
                }
                let layer = e.selected.unwrap();
                if e.doc.layer(layer).unwrap().crop.left < 0.09 {
                    return Err("Native crop handles did not change source bounds".into());
                }
                e.action(Action::Undo);
                if e.doc != before {
                    return Err("Native crop undo failed".into());
                }
                e.action(Action::Redo);
                if e.doc != after {
                    return Err("Native crop redo failed".into());
                }
                e.crop_edit = false;
                self.phase = 6;
                self.start = Instant::now();
            }
            6 => {
                if self.start.elapsed().as_secs() < 3 {
                    return Ok(false);
                }
                let saved = e
                    .session
                    .remembered
                    .as_ref()
                    .ok_or("No recovery snapshot sampled")?;
                if saved.document != e.doc || saved.playheads.len() != 2 {
                    return Err("Recovery snapshot missing final composition/playheads".into());
                }
                e.doc
                    .playbacks
                    .get_mut(&self.source)
                    .unwrap()
                    .restart_on_activation = true;
                e.publish();
                e.media
                    .control(&e.doc, self.source)
                    .unwrap()
                    .state
                    .lock()
                    .unwrap()
                    .seek = Some(3.);
                self.phase = 7;
                self.start = Instant::now();
            }
            7 => {
                if self.start.elapsed().as_millis() < 400 {
                    return Ok(false);
                }
                if e.media
                    .control(&e.doc, self.source)
                    .unwrap()
                    .state
                    .lock()
                    .unwrap()
                    .position
                    < 2.
                {
                    return Err("Pre-cue seek did not land".into());
                }
                e.doc.select_scene(1);
                e.publish();
                self.phase = 8;
                self.start = Instant::now();
            }
            8 => {
                if self.start.elapsed().as_secs() < 1 {
                    return Ok(false);
                }
                if e.media
                    .control(&e.doc, self.source)
                    .unwrap()
                    .state
                    .lock()
                    .unwrap()
                    .position
                    > 2.
                {
                    return Err("Producer scene-adoption restart cue did not reset playback".into());
                }
                e.doc
                    .playbacks
                    .get_mut(&self.source)
                    .unwrap()
                    .restart_on_activation = false;
                e.doc.select_scene(self.last_scene);
                e.publish();
                self.phase = 9;
                self.start = Instant::now();
            }
            9 => {
                if self.start.elapsed().as_secs() < 2 {
                    return Ok(false);
                }
                log(format!("media_scenes_probe passed=1 camera={} scenes={} newest_scene={} enabled_outputs={} recovery_unsaved=1\n",self.camera.is_some() as u8,e.doc.scenes.len(),e.doc.active_scene,enabled.len()));
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
}
