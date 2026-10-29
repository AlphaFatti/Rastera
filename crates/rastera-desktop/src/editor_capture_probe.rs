//! Explicit local camera action / integration replay, launched only inside Rastera.app.
use super::*;
use serde_json::json;
use std::time::Instant;
pub struct CaptureProbe {
    phase: u32,
    folder: PathBuf,
    layer: Id,
    source: Id,
    copy: Id,
    before: Option<Document>,
    started: Instant,
    first_hash: u64,
    revision: u64,
    second: Option<Id>,
    second_hash: u64,
    original_mode: Option<rastera_core::capture::Mode>,
    rows: Vec<serde_json::Value>,
}
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}
impl CaptureProbe {
    pub fn new(folder: PathBuf) -> Self {
        Self {
            phase: 0,
            folder,
            layer: 0,
            source: 0,
            copy: 0,
            before: None,
            started: Instant::now(),
            first_hash: 0,
            revision: 0,
            second: None,
            second_hash: 0,
            original_mode: None,
            rows: vec![],
        }
    }
    fn hash(e: &Editor, id: Id) -> u64 {
        e.engine
            .captures()
            .slot(&rastera_core::capture::Hub::key(&e.doc.project_id, id))
            .latest()
            .map(|frame| {
                frame
                    .bgra
                    .iter()
                    .step_by(97)
                    .fold(0xcbf29ce484222325u64, |h, b| {
                        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
                    })
            })
            .unwrap_or(0)
    }
    pub fn tick(
        &mut self,
        e: &mut Editor,
        _p: &Consumer,
        outputs: &std::collections::BTreeMap<u64, Consumer>,
        enabled: &mut std::collections::BTreeSet<u64>,
    ) -> Result<bool, String> {
        if std::env::args().any(|a| a == "--capture-reopen-probe") {
            if e.doc.captures.is_empty() {
                return Err("Reopen fixture has no capture sources".into());
            }
            let all = e.doc.captures.keys().all(|&id| {
                let s = e.capture.status(&e.doc, id);
                s.phase == "Live" && s.frames >= 20 && s.uploaded > 5
            });
            if !all {
                if self.started.elapsed().as_secs() > 25 {
                    return Err("Fresh project capture restart timed out".into());
                }
                return Ok(false);
            }
            enabled.extend([1, 2]);
            if outputs.len() < 2 {
                return Ok(false);
            }
            log("capture_reopen_probe passed=1 fresh_process=1 all_sources_live=1 producer_upload=1\n".into());
            return Ok(true);
        }
        match self.phase {
            0 => {
                enabled.clear();
                e.doc.test_background = true;
                for viewport in &mut e.doc.viewports {
                    viewport.fit_canvas(e.doc.canvas);
                }
                e.publish();
                e.capture.request();
                e.capture.dialog = false;
                log("capture_probe permission_action=1 waiting_for_camera_access=1\n".into());
            }
            1 => {
                if e.capture.authorization() != 3 {
                    return Ok(false);
                }
                e.capture.refresh();
                let (device, mode) = e
                    .capture
                    .devices
                    .iter()
                    .find_map(|d| {
                        d.modes
                            .iter()
                            .find(|m| {
                                m.width == 1280 && m.height == 720 && (m.fps - 30.).abs() < 0.01
                            })
                            .map(|m| (d.clone(), m.clone()))
                    })
                    .or_else(|| {
                        e.capture
                            .devices
                            .iter()
                            .find_map(|d| d.modes.first().map(|m| (d.clone(), m.clone())))
                    })
                    .ok_or("No usable real AVFoundation video device")?;
                let source = rastera_core::capture::Source {
                    device_id: device.id,
                    name: device.name.clone(),
                    mode,
                };
                self.layer = e.doc.add_capture(source).map_err(|s| s.to_string())?;
                if let Content::Capture(id) = e.doc.layer(self.layer).unwrap().content {
                    self.source = id
                };
                e.selected = Some(self.layer);
                e.publish();
                self.revision = e.revision;
                self.started = Instant::now();
                self.rows
                    .push(json!({"device":device.name,"model":device.model}));
            }
            2 => {
                let status = e.capture.status(&e.doc, self.source);
                if status.phase.starts_with("Error") {
                    return Err(status.phase);
                }
                if status.frames < 40 || status.uploaded < 10 {
                    if self.started.elapsed().as_secs() > 15 {
                        return Err(format!("Real capture timed out: {}", status.phase));
                    }
                    return Ok(false);
                }
                let source = &e.doc.captures[&self.source];
                check(
                    status.actual == [source.mode.width, source.mode.height],
                    "Actual BGRA size differs from requested",
                )?;
                check(
                    e.revision == self.revision,
                    "Video delivery rebuilt the document",
                )?;
                self.first_hash = Self::hash(e, self.source);
                self.copy = e.doc.duplicate(self.layer).ok_or("Duplicate failed")?;
                let layer = e.doc.layer_mut(self.layer).unwrap();
                layer.rect = Rect {
                    x: 1800.,
                    y: 100.,
                    width: 960.,
                    height: 540.,
                };
                layer.opacity = 0.8;
                e.publish();
            }
            3 => {
                check(
                    e.doc.captures.len() == 1 && e.capture.active_count() == 1,
                    "Duplicated layers opened another session",
                )?;
                e.action(Action::AddText);
                e.file_job(FileJob::Import(
                    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/alpha.png"),
                ));
            }
            4 => {
                let image = e.selected.unwrap();
                e.doc.layer_mut(image).unwrap().rect = Rect {
                    x: 1850.,
                    y: 180.,
                    width: 240.,
                    height: 120.,
                };
                e.publish();
                enabled.extend([1, 2]);
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
            }
            5 => {
                if outputs.len() != 2 {
                    return Ok(false);
                }
                let status = e.capture.status(&e.doc, self.source);
                self.rows.push(json!({"phase":"composed","actual":status.actual,"negotiated_fps":status.negotiated_fps,"observed_fps":status.cadence,"delivered":status.frames,"uploaded":status.uploaded,"bounded_replacements":status.replaced,"input_hash_changed":Self::hash(e,self.source)!=self.first_hash,"sessions":e.capture.active_count()}));
                check(
                    Self::hash(e, self.source) != self.first_hash,
                    "Captured input did not change",
                )?;
                self.before = Some(e.doc.clone());
                e.file_job(FileJob::Save(
                    self.folder.join("capture.rastera.json"),
                    Box::new(e.doc.clone()),
                ));
            }
            6 => e.file_job(FileJob::Load(self.folder.join("capture.rastera.json"))),
            7 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Capture project round trip changed definitions",
                )?;
                enabled.extend([1, 2]);
                e.selected = Some(self.layer);
                e.action(Action::Delete);
            }
            8 => {
                check(
                    e.doc.captures.len() == 1 && e.capture.active_count() == 1,
                    "Removing one layer stopped shared source",
                )?;
                e.selected = Some(self.copy);
                e.action(Action::Delete);
            }
            9 => {
                if e.capture.active_count() != 0 {
                    return Ok(false);
                }
                check(
                    e.doc.captures.is_empty(),
                    "Last reference did not retire definition",
                )?;
                e.action(Action::Undo);
                self.started = Instant::now();
            }
            10 => {
                let status = e.capture.status(&e.doc, self.source);
                if status.frames < 10 || status.phase != "Live" {
                    if self.started.elapsed().as_secs() > 15 {
                        return Err(format!("Undo did not restart capture: {}", status.phase));
                    }
                    return Ok(false);
                }
                let first = e.doc.captures[&self.source].device_id.clone();
                let second = e
                    .capture
                    .devices
                    .iter()
                    .find(|d| d.id != first && !d.modes.is_empty())
                    .cloned();
                if let Some(device) = second {
                    let mode = device
                        .modes
                        .iter()
                        .find(|m| m.width == 1280 && m.height == 720 && (m.fps - 30.).abs() < 0.01)
                        .unwrap_or(&device.modes[0])
                        .clone();
                    let id = e.doc.add_capture(rastera_core::capture::Source {
                        device_id: device.id,
                        name: device.name.clone(),
                        mode,
                    })?;
                    e.doc.layer_mut(id).unwrap().rect = Rect {
                        x: 160.,
                        y: 640.,
                        width: 640.,
                        height: 360.,
                    };
                    if let Content::Capture(source) = e.doc.layer(id).unwrap().content {
                        self.second = Some(source);
                    }
                    e.publish();
                    self.rows
                        .push(json!({"second_device":device.name,"model":device.model}));
                    self.started = Instant::now();
                }
            }
            11 => {
                if let Some(id) = self.second {
                    let s = e.capture.status(&e.doc, id);
                    if self.second_hash == 0 && s.frames > 0 {
                        self.second_hash = Self::hash(e, id);
                    }
                    if self.started.elapsed().as_secs() < 4
                        || (s.frames < 15 && self.started.elapsed().as_secs() < 6)
                    {
                        return Ok(false);
                    }
                    self.rows.push(json!({"second_live":s.phase=="Live","input_hash_changed":Self::hash(e,id)!=self.second_hash,"actual":s.actual,"negotiated_fps":s.negotiated_fps,"observed_fps":s.cadence,"phase":s.phase}));
                }
                let first = e.capture.status(&e.doc, self.source);
                check(
                    first.phase == "Live" && first.uploaded > 0,
                    "First source stopped during second-device attempt",
                )?;
                let current = &e.doc.captures[&self.source];
                if e.capture
                    .devices
                    .iter()
                    .find(|d| d.id == current.device_id)
                    .is_some_and(|d| {
                        d.modes.iter().any(|m| {
                            m.width == 640 && m.height == 480 && (m.fps - 30.).abs() < 0.01
                        })
                    })
                {
                    self.original_mode = Some(current.mode.clone());
                    e.doc.captures.get_mut(&self.source).unwrap().mode =
                        rastera_core::capture::Mode {
                            width: 640,
                            height: 480,
                            fps: 30.,
                        };
                    e.publish();
                    self.started = Instant::now();
                }
            }
            12 => {
                if self.original_mode.is_some() {
                    let s = e.capture.status(&e.doc, self.source);
                    if s.phase != "Live" || s.actual != [640, 480] || s.frames < 15 {
                        if self.started.elapsed().as_secs() > 15 {
                            return Err(format!(
                                "Format change failed: {} {:?}",
                                s.phase, s.actual
                            ));
                        }
                        return Ok(false);
                    }
                    if let Some(second) = self.second {
                        check(
                            e.capture.status(&e.doc, second).phase == "Live",
                            "Other capture stopped on format change",
                        )?;
                    }
                    self.rows.push(json!({"format_change_verified":true,"actual":s.actual,"negotiated_fps":s.negotiated_fps}));
                    e.doc.captures.get_mut(&self.source).unwrap().mode =
                        self.original_mode.clone().unwrap();
                    e.publish();
                    self.started = Instant::now();
                }
            }
            13 => {
                if self.original_mode.is_some() {
                    let s = e.capture.status(&e.doc, self.source);
                    if s.phase != "Live" || s.frames < 15 {
                        if self.started.elapsed().as_secs() > 15 {
                            return Err("Restore original format failed".into());
                        }
                        return Ok(false);
                    }
                }
            }
            14 => {
                e.file_job(FileJob::Save(
                    self.folder.join("review.rastera.json"),
                    Box::new(e.doc.clone()),
                ));
            }
            15 => {
                let result = json!({"passed":true,"video_only":true,"resource_limit":2,"records":self.rows,"second_device_available":self.second.is_some(),"checks":["real BGRA input and producer upload","input frame hash advancement","no document rebuild on video frames","image/text/capture shared composition","duplicate shares session","remove one preserves session","last removal retires session","undo restarts safely","capture project round trip","windowed and available fullscreen consumers"],"limitations":["Human image color/orientation/motion and physical disconnect/reconnect remain separate","No CPU canvas readback or zero-copy claim","Capture pictures remain local and are not committed"]});
                std::fs::write(
                    self.folder.join("capture-results.json"),
                    serde_json::to_vec_pretty(&result).unwrap(),
                )
                .map_err(|e| e.to_string())?;
                log("capture_probe passed=1 actual_device=1 shared_session=1 scene_roundtrip=1 retirement_undo=1 frame_delivery_independent=1\n".into());
                return Ok(true);
            }
            _ => return Ok(true),
        }
        log(format!("capture_probe phase={}\n", self.phase));
        self.phase += 1;
        Ok(false)
    }
}
