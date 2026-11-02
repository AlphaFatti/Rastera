//! Owned native canvas events; fixture paths are self-contained, so installed runs need no repository cwd.
use super::*;
use std::time::Instant;
pub(super) struct PinProbe {
    folder: PathBuf,
    phase: u32,
    layer: Id,
    before: Option<Document>,
    history: Option<History>,
    uploads: u64,
    start: Instant,
}
impl PinProbe {
    pub fn new(folder: PathBuf) -> Self {
        Self {
            folder,
            phase: 0,
            layer: 0,
            before: None,
            history: None,
            uploads: 0,
            start: Instant::now(),
        }
    }
    pub fn tick(
        &mut self,
        e: &mut Editor,
        p: &Consumer,
        enabled: &mut std::collections::BTreeSet<u64>,
    ) -> Result<bool, String> {
        match self.phase {
            0 => {
                let id = e
                    .doc
                    .add_image(scene::inspect_decoded_image(&self.folder.join("grid.png"))?);
                e.doc.layer_mut(id).unwrap().rect = Rect {
                    x: 1400.,
                    y: 220.,
                    width: 1040.,
                    height: 600.,
                };
                self.layer = id;
                let a = e.doc.add_image(scene::inspect_decoded_image(
                    &self.folder.join("alpha.png"),
                )?);
                e.doc.layer_mut(a).unwrap().rect = Rect {
                    x: 200.,
                    y: 700.,
                    width: 500.,
                    height: 240.,
                };
                let t = e.doc.add_text();
                let Content::Text(source) = e.doc.layer(t).unwrap().content else {
                    unreachable!()
                };
                e.doc.texts.get_mut(&source).unwrap().text =
                    "Editable text · source corner pin".into();
                e.doc.layer_mut(t).unwrap().rect = Rect {
                    x: 850.,
                    y: 850.,
                    width: 1800.,
                    height: 180.,
                };
                let v = e.doc.add_media(crate::shared_native::media::inspect(
                    &self.folder.join("test.mp4"),
                )?)?;
                e.doc.layer_mut(v).unwrap().rect = Rect {
                    x: 150.,
                    y: 100.,
                    width: 640.,
                    height: 360.,
                };
                if let Content::Media(id) = e.doc.layer(v).unwrap().content {
                    e.doc.playbacks.get_mut(&id).unwrap().looping = Some(true);
                }
                let g = e.doc.add_media(crate::shared_native::media::inspect(
                    &self.folder.join("test.gif"),
                )?)?;
                e.doc.layer_mut(g).unwrap().rect = Rect {
                    x: 2850.,
                    y: 150.,
                    width: 640.,
                    height: 360.,
                };
                e.selected = Some(id);
                e.tool = Tool::CornerPin;
                e.publish();
                enabled.extend([1, 2]);
                if let Some(screen) = e
                    .destinations
                    .discovery
                    .displays
                    .iter()
                    .find(|s| !s.builtin)
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
                self.phase = 1;
            }
            1 => {
                self.uploads = e.engine.resource_uploads();
                self.before = Some(e.doc.clone());
                self.history = Some(e.history.clone());
                let q = e.doc.layer(self.layer).unwrap().visible_corners()[0];
                Probe::event(p, 1, q);
                Probe::event(p, 2, [q[0] + 140., q[1] + 130.]);
                Probe::event(p, 3, [q[0] + 140., q[1] + 130.]);
                self.phase = 2;
            }
            2 => {
                let before = self.before.as_ref().unwrap();
                let original = before.layer(self.layer).unwrap();
                let layer = e.doc.layer(self.layer).unwrap();
                if layer.pin == original.pin || layer.rect != original.rect {
                    return Err("Corner drag did not change only pin geometry".into());
                }
                let mut history = self.history.take().unwrap();
                history.record(before.clone(), &e.doc);
                if e.history != history {
                    return Err("Pin gesture did not create exactly one history entry".into());
                }
                if e.engine.resource_uploads() != self.uploads {
                    return Err("Corner edit rebuilt prepared picture/text resources".into());
                }
                let after = e.doc.clone();
                e.action(Action::Undo);
                if &e.doc != before {
                    return Err("Pin undo mismatch".into());
                }
                e.action(Action::Redo);
                if e.doc != after {
                    return Err("Pin redo mismatch".into());
                }
                self.before = Some(e.doc.clone());
                let q = e.doc.layer(self.layer).unwrap().visible_corners()[1];
                Probe::event(p, 1, q);
                Probe::event(p, 2, [q[0] - 70., q[1] + 80.]);
                self.phase = 3;
            }
            3 => {
                if e.gesture.is_none() {
                    return Err("Cancel fixture did not start a gesture".into());
                }
                e.cancel_gesture();
                if e.doc != *self.before.as_ref().unwrap() {
                    return Err("Pin gesture cancellation changed document".into());
                }
                let original = e.doc.layer(self.layer).unwrap().clone();
                let point = original.source_canvas([0.5, 0.5]).unwrap();
                e.doc
                    .layer_mut(self.layer)
                    .unwrap()
                    .set_crop(rastera_core::crop::Crop {
                        left: 0.1,
                        top: 0.15,
                        right: 0.85,
                        bottom: 0.9,
                    })?;
                let current = e
                    .doc
                    .layer(self.layer)
                    .unwrap()
                    .source_canvas([0.5, 0.5])
                    .unwrap();
                if (current[0] - point[0]).abs() > 0.01 || (current[1] - point[1]).abs() > 0.01 {
                    return Err("Crop moved surviving warped pixels".into());
                }
                self.before = Some(e.doc.clone());
                e.tool = Tool::Crop;
                let q = e.doc.layer(self.layer).unwrap().visible_corners()[0];
                Probe::event(p, 1, q);
                Probe::event(p, 2, [q[0] + 40., q[1] + 35.]);
                Probe::event(p, 3, [q[0] + 40., q[1] + 35.]);
                self.phase = 4;
            }
            4 => {
                let original = self.before.as_ref().unwrap().layer(self.layer).unwrap();
                let layer = e.doc.layer(self.layer).unwrap();
                if layer.crop == original.crop {
                    return Err("Warped crop handles did not change bounds".into());
                }
                let a = original.source_canvas([0.5, 0.5]).unwrap();
                let b = layer.source_canvas([0.5, 0.5]).unwrap();
                if (a[0] - b[0]).hypot(a[1] - b[1]) > 0.02 {
                    return Err("Warped crop gesture moved content".into());
                }
                e.tool = Tool::Transform;
                let q = layer.visible_corners()[3];
                Probe::event(p, 1, q);
                Probe::event(p, 2, [q[0] + 80., q[1] + 40.]);
                Probe::event(p, 3, [q[0] + 80., q[1] + 40.]);
                self.phase = 5;
            }
            5 => {
                let copy = e.doc.duplicate(self.layer).unwrap();
                e.doc.layer_mut(copy).unwrap().rect.x += 1000.;
                let q = e.doc.layer(copy).unwrap().pin;
                e.doc.layer_mut(copy).unwrap().pin.corners[0][0] += 0.05;
                if e.doc.layer(self.layer).unwrap().pin == e.doc.layer(copy).unwrap().pin {
                    return Err("Duplicate pin is not independent".into());
                }
                e.doc.layer_mut(copy).unwrap().pin = q;
                let text = e
                    .doc
                    .layers
                    .iter()
                    .find(|l| matches!(l.content, Content::Text(_)))
                    .unwrap()
                    .id;
                let mut q = e.doc.layer(text).unwrap().visible_corners();
                q[1][1] -= 50.;
                e.doc.layer_mut(text).unwrap().set_visible_corners(q)?;
                let scene = e.doc.add_scene(true)?;
                e.doc.select_scene(1);
                e.publish();
                e.doc.select_scene(scene);
                e.publish();
                self.phase = 6;
                self.start = Instant::now();
            }
            6 => {
                if self.start.elapsed().as_secs() < 3 {
                    return Ok(false);
                }
                let file = self.folder.join("corner-pin.rastera.json");
                scene::save_project(&e.doc, &file)?;
                if scene::load_project(&file)? != e.doc {
                    return Err("Pinned scene project round trip differs".into());
                }
                e.project = Some(std::fs::canonicalize(file).unwrap());
                e.destinations.persist();
                for &id in e.doc.playbacks.keys() {
                    let c = e
                        .media
                        .control(&e.doc, id)
                        .ok_or("Missing playback worker")?;
                    if c.slot.persistent_snapshot().flatten().is_none() {
                        return Err("Moving source has no frame".into());
                    }
                }
                log(format!("corner_pin_probe passed=1 gesture_history=1 cancel=1 crop_invariant=1 cache_reused=1 text=1 duplicate=1 scene=1 save_reopen=1 outputs={}\n",enabled.len()));
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
}
