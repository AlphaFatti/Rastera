//! Explicit, noninteractive native replay for busy test desktops. No system input.
use super::*;
unsafe extern "C" {
    fn rastera_editor_close_output(handle: *mut std::ffi::c_void);
    fn rastera_editor_replay(handle: *mut std::ffi::c_void, kind: u32, x: f32, y: f32);
}
pub(super) struct Probe {
    folder: PathBuf,
    phase: u32,
    frame: u32,
    before: Option<Document>,
}
impl Probe {
    pub fn new(folder: PathBuf) -> Self {
        Self {
            folder,
            phase: 0,
            frame: 0,
            before: None,
        }
    }
    fn event(preview: &Consumer, kind: u32, p: [f32; 2]) {
        // SAFETY: main-thread live preview; native replay dispatches only to its own view.
        unsafe {
            rastera_editor_replay(preview.handle.as_ptr(), kind, p[0], p[1]);
        }
    }
    pub fn tick(
        &mut self,
        e: &mut Editor,
        preview: &Consumer,
        outputs: &[Option<Consumer>; 2],
        enabled: &mut [bool; 2],
    ) -> bool {
        if e.busy_file || e.pending_load.is_some() || e.engine.applied() != e.revision {
            return false;
        }
        let result = self.step(e, preview, outputs, enabled);
        match result {
            Ok(done) => done,
            Err(error) => {
                log(format!(
                    "editor_probe failed_phase={} error={}\n",
                    self.phase, error
                ));
                e.error = Some(error);
                true
            }
        }
    }
    fn step(
        &mut self,
        e: &mut Editor,
        p: &Consumer,
        outputs: &[Option<Consumer>; 2],
        enabled: &mut [bool; 2],
    ) -> Result<bool, String> {
        if let Some(error) = &e.error {
            if self.phase != 18 {
                return Err(error.clone());
            }
        }
        let check = |ok: bool, message: &str| if ok { Ok(()) } else { Err(message.to_owned()) };
        match self.phase {
            0 => {
                e.doc.test_background = true;
                e.publish();
                e.file_job(FileJob::Import(self.folder.join("alpha.png")));
            }
            1 => {
                check(e.doc.layers.len() == 1, "PNG import failed")?;
                e.action(Action::AddText);
                let before = e.doc.clone();
                e.doc.texts.get_mut(&3).unwrap().text="Editable Rastera canvas text, wrapping across the seam.\nLive Edit affects outputs.".into();
                e.history.record(before, &e.doc);
                e.publish();
            }
            2 => {
                check(e.doc.layers.len() == 2, "Add text failed")?;
                self.before = Some(e.doc.clone());
                e.selected = Some(2);
                Self::event(p, 1, [1920., 250.]);
            }
            3 => {
                self.frame += 1;
                Self::event(p, 2, [1920. + 900. * self.frame as f32 / 12., 250.]);
                if self.frame < 12 {
                    return Ok(false);
                }
            }
            4 => Self::event(p, 3, [2820., 250.]),
            5 => {
                check(
                    (e.doc.layer(2).unwrap().rect.x - 2220.).abs() < 0.1,
                    "Native drag mapping failed",
                )?;
                e.action(Action::Undo);
            }
            6 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Drag undo did not restore one whole gesture",
                )?;
                e.action(Action::Redo);
            }
            7 => {
                check(
                    (e.doc.layer(2).unwrap().rect.x - 2220.).abs() < 0.1,
                    "Drag redo failed",
                )?;
                e.selected = Some(2);
                Self::event(p, 1, [3420., 840.]);
            }
            8 => Self::event(p, 2, [3820., 1040.]),
            9 => Self::event(p, 3, [3820., 1040.]),
            10 => {
                let r = e.doc.layer(2).unwrap().rect;
                check(
                    (r.width - 1600.).abs() < 0.2 && (r.height - 800.).abs() < 0.2,
                    "Image corner resize/aspect failed",
                )?;
                e.selected = Some(4);
                Self::event(p, 1, [1952., 470.]);
            }
            11 => Self::event(p, 2, [1552., 570.]),
            12 => Self::event(p, 3, [1552., 570.]),
            13 => {
                check(
                    (e.doc.layer(4).unwrap().rect.x - 752.).abs() < 0.2,
                    "Text drag failed",
                )?;
                Self::event(p, 1, [2352., 770.]);
            }
            14 => Self::event(p, 2, [1652., 1070.]),
            15 => {
                // Finish the text-box gesture.
                Self::event(p, 3, [1652., 1070.]);
            }
            16 => {
                let r = e.doc.layer(4).unwrap().rect;
                check(
                    (r.width - 900.).abs() < 0.2
                        && (r.height - 700.).abs() < 0.2
                        && e.doc.texts[&3].font_size == 80.,
                    "Text box resize changed font size or failed",
                )?;
                e.action(Action::Down);
                check(e.doc.layers[0].id == 4, "Reorder failed")?;
                e.action(Action::Duplicate);
                check(
                    e.doc.texts.len() == 2,
                    "Duplicate did not clone editable text",
                )?;
                e.action(Action::Delete);
                check(
                    e.doc.layers.len() == 2 && e.doc.texts.len() == 1,
                    "Delete/prune failed",
                )?;
                e.file_job(FileJob::Save(
                    self.folder.join("result.rastera.json"),
                    e.doc.clone(),
                ));
            }
            17 => {
                self.before = Some(e.doc.clone());
                e.file_job(FileJob::Load(self.folder.join("missing.json")));
            }
            18 => {
                check(
                    e.error.is_some() && e.doc == *self.before.as_ref().unwrap(),
                    "Failed load changed document",
                )?;
                e.error = None;
                e.file_job(FileJob::Load(self.folder.join("result.rastera.json")));
            }
            19 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Project round trip changed document",
                )?;
                e.selected = Some(2);
                Self::event(p, 1, [3020., 256.]);
            }
            20 => Self::event(p, 2, [700., -34.]),
            21 => Self::event(p, 3, [700., -34.]),
            22 => {
                let r = e.doc.layer(2).unwrap().rect;
                check(
                    (r.x + 100.).abs() < 0.2 && (r.y + 50.).abs() < 0.2,
                    "Off-canvas drag failed",
                )?;
                e.selected = Some(4);
                Self::event(p, 1, [1600., 500.]);
            }
            23 => Self::event(p, 2, [2148., 500.]),
            24 => Self::event(p, 3, [2148., 500.]),
            25 => {
                check(
                    (e.doc.layer(4).unwrap().rect.x - 1300.).abs() < 0.2,
                    "Text seam move failed",
                )?;
                self.before = Some(e.doc.clone());
                e.file_job(FileJob::Save(
                    self.folder.join("result.rastera.json"),
                    e.doc.clone(),
                ));
            }
            26 => e.file_job(FileJob::Load(self.folder.join("result.rastera.json"))),
            27 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Off-canvas project round trip changed document",
                )?;
                e.selected = Some(4);
                // SAFETY: main-only, live native output; same performClose lifecycle as chrome.
                unsafe {
                    rastera_editor_close_output(
                        outputs[0].as_ref().ok_or("Left missing")?.handle.as_ptr(),
                    );
                }
            }
            28 => {
                check(!enabled[0], "Left close was not observed")?;
                enabled[0] = true;
            }
            29 => {
                check(enabled[0], "Left reopen failed")?;
                unsafe {
                    rastera_editor_close_output(
                        outputs[1].as_ref().ok_or("Right missing")?.handle.as_ptr(),
                    );
                }
            }
            30 => {
                check(!enabled[1], "Right close was not observed")?;
                enabled[1] = true;
            }
            31 => {
                check(enabled[0] && enabled[1], "Right reopen failed")?;
                log(format!("editor_probe passed=1 import_png=1 add_text=1 native_drag_resize=1 single_gesture_undo_redo=1 reorder_duplicate_delete=1 project_roundtrip=1 failed_load_preserved=1 partially_off_canvas=1 native_output_close_reopen=1 result={}\n",self.folder.join("result.rastera.json").display()));
                return Ok(true);
            }
            _ => return Err("Unknown probe phase".into()),
        }
        self.phase += 1;
        log(format!(
            "editor_probe phase={} applied_revision={}\n",
            self.phase,
            e.engine.applied()
        ));
        Ok(false)
    }
}
