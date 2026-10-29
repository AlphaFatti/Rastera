//! Explicit, noninteractive native replay for busy test desktops. No system input.
use super::*;
#[path = "editor_dynamic_probe.rs"]
mod dynamic;
unsafe extern "C" {
    fn rastera_editor_close_output(handle: *mut std::ffi::c_void);
    fn rastera_editor_replay(
        handle: *mut std::ffi::c_void,
        kind: u32,
        x: f32,
        y: f32,
        modifiers: u64,
    );
}
pub(super) struct Probe {
    folder: PathBuf,
    phase: u32,
    frame: u32,
    before: Option<Document>,
    handle: Option<[f32; 2]>,
    mapping: bool,
    selection: bool,
    dynamic: Option<dynamic::Dynamic>,
}
impl Probe {
    pub fn new(folder: PathBuf) -> Self {
        Self {
            dynamic: std::env::args()
                .any(|a| a == "--dynamic-probe")
                .then(|| dynamic::Dynamic::new(folder.clone())),
            folder,
            phase: 0,
            frame: 0,
            before: None,
            handle: None,
            mapping: std::env::args().any(|a| a == "--mapping-probe"),
            selection: std::env::args().any(|a| a == "--selection-probe"),
        }
    }
    fn event(preview: &Consumer, kind: u32, p: [f32; 2]) {
        Self::modified_event(preview, kind, p, 0);
    }
    fn modified_event(preview: &Consumer, kind: u32, p: [f32; 2], modifiers: u64) {
        // SAFETY: main-thread live preview; native replay dispatches only to its own view.
        unsafe {
            rastera_editor_replay(preview.handle.as_ptr(), kind, p[0], p[1], modifiers);
        }
    }
    pub fn tick(
        &mut self,
        e: &mut Editor,
        preview: &Consumer,
        outputs: &std::collections::BTreeMap<u64, Consumer>,
        enabled: &mut std::collections::BTreeSet<u64>,
    ) -> bool {
        if e.busy_file || e.pending_load.is_some() || e.engine.applied() != e.revision {
            return false;
        }
        if let Some(dynamic) = &mut self.dynamic {
            return match dynamic.tick(e, preview, outputs, enabled) {
                Ok(done) => done,
                Err(error) => {
                    log(format!("dynamic_viewport_probe failed=1 error={error}\n"));
                    e.error = Some(error);
                    true
                }
            };
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
        outputs: &std::collections::BTreeMap<u64, Consumer>,
        enabled: &mut std::collections::BTreeSet<u64>,
    ) -> Result<bool, String> {
        if let Some(error) = &e.error {
            if self.phase != 18 && self.phase != 56 && self.phase != 57 {
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
                    Box::new(e.doc.clone()),
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
                    Box::new(e.doc.clone()),
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
                        outputs.get(&1).ok_or("Left missing")?.handle.as_ptr(),
                    );
                }
            }
            28 => {
                check(!enabled.contains(&1), "Left close was not observed")?;
                enabled.insert(1);
            }
            29 => {
                check(enabled.contains(&1), "Left reopen failed")?;
                unsafe {
                    rastera_editor_close_output(
                        outputs.get(&2).ok_or("Right missing")?.handle.as_ptr(),
                    );
                }
            }
            30 => {
                check(!enabled.contains(&2), "Right close was not observed")?;
                enabled.insert(2);
            }
            31 => {
                check(
                    enabled.contains(&1) && enabled.contains(&2),
                    "Right reopen failed",
                )?;
                log(format!("editor_probe passed=1 import_png=1 add_text=1 native_drag_resize=1 single_gesture_undo_redo=1 reorder_duplicate_delete=1 project_roundtrip=1 failed_load_preserved=1 partially_off_canvas=1 native_output_close_reopen=1 result={}\n",self.folder.join("result.rastera.json").display()));
                if !self.mapping {
                    return Ok(true);
                }
                self.before = Some(e.doc.clone());
                let mut candidate = e.doc.clone();
                candidate.canvas = [1079, 1919];
                e.stage_document(candidate, None);
            }
            32 => {
                check(
                    e.doc.canvas == [1079, 1919],
                    "Portrait allocation/commit failed",
                )?;
                let old = self.before.as_ref().unwrap();
                check(
                    e.doc.layers == old.layers && e.doc.viewports == old.viewports,
                    "Resize moved document coordinates",
                )?;
                e.action(Action::Undo);
            }
            33 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Canvas undo failed",
                )?;
                e.action(Action::Redo);
            }
            34 => {
                check(e.doc.canvas == [1079, 1919], "Canvas redo failed")?;
                e.change_mode(Mode::Viewports);
                let before = e.doc.clone();
                e.doc.viewports[1].fit_canvas(e.doc.canvas); // Deliberately overlap the selected Left.
                e.history.record(before, &e.doc);
                e.publish();
                e.viewport_selected = Some(1);
                Self::event(p, 1, [960., 540.]);
            }
            35 => Self::event(p, 2, [800., 700.]),
            36 => Self::event(p, 3, [800., 700.]),
            37 => {
                check(
                    e.doc.viewports[0]
                        .center
                        .iter()
                        .zip([800., 700.])
                        .all(|(a, b)| (*a - b).abs() < 0.2),
                    "Viewport native pan failed",
                )?;
                e.action(Action::Undo);
            }
            38 => {
                check(
                    e.doc.viewports[0].center == [960., 540.],
                    "Viewport gesture undo failed",
                )?;
                e.action(Action::Redo);
            }
            39 => {
                check(
                    (e.doc.viewports[0].center[0] - 800.).abs() < 0.2,
                    "Viewport gesture redo failed",
                )?;
                Self::event(p, 1, e.doc.viewports[0].corners()[2]);
            }
            40 => Self::event(p, 2, [1280., 970.]),
            41 => Self::event(p, 3, [1280., 970.]),
            42 => {
                check(
                    (e.doc.viewports[0].zoom - 2.).abs() < 0.002,
                    "Uniform viewport corner zoom failed",
                )?;
                let handle = e.rotation_handle(p, 1);
                self.handle = Some(handle);
                Self::event(p, 1, handle);
            }
            43 | 44 => {
                let c = e.doc.viewports[0].center;
                let h = self.handle.unwrap();
                Self::event(
                    p,
                    if self.phase == 43 { 2 } else { 3 },
                    [c[0] + c[1] - h[1], c[1]],
                );
            }
            45 => {
                check(
                    (e.doc.viewports[0].rotation - 90.).abs() < 0.05,
                    "Viewport rotation handle failed",
                )?;
                let before = e.doc.clone();
                e.doc.viewports[1].fit_canvas(e.doc.canvas);
                check(
                    e.doc.viewports[1].center == [539.5, 959.5]
                        && e.doc.viewports[1].rotation == 0.,
                    "Fit Canvas failed",
                )?;
                e.doc.viewports[0].name = "Detail".into();
                e.doc.viewports[0].color = rastera_core::theme::HOT_MAGENTA;
                e.history.record(before, &e.doc);
                e.publish();
                e.change_mode(Mode::Canvas);
                e.selected = Some(4);
                e.font_search = "Menlo".into();
                e.font_dialog = true;
                self.frame = 0;
            }
            46 => {
                self.frame += 1;
                if self.frame < 120 || !e.library.ready.load(Ordering::Acquire) {
                    return Ok(false);
                }
                let face = e
                    .library
                    .faces
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|f| f.postscript == "Menlo-Bold")
                    .cloned()
                    .ok_or("Menlo Bold not discovered")?;
                let font = rastera_core::fonts::FontRef {
                    family: face.family,
                    style: face.style,
                    postscript: face.postscript,
                };
                let defs = e.library.definitions(Some(&font));
                check(
                    defs.font_data.contains_key("installed"),
                    "Installed font bytes/collection face were not loaded",
                )?;
                log(format!(
                    "canvas_mapping_probe installed_font={} loaded_face=1\n",
                    font.postscript
                ));
                let before = e.doc.clone();
                e.doc.texts.get_mut(&3).unwrap().font = Some(font);
                e.history.record(before, &e.doc);
                e.publish();
                e.font_dialog = false;
                e.settings = Some(e.doc.canvas);
                self.frame = 0;
            }
            47 => {
                self.frame += 1;
                if self.frame < 120 {
                    return Ok(false);
                }
                e.settings = None;
                let mut candidate = e.doc.clone();
                candidate.canvas = [2559, 1439];
                e.stage_document(candidate, None);
            }
            48 => {
                check(
                    e.doc.canvas == [2559, 1439] && e.doc.viewports[0].rotation.abs() > 89.,
                    "Landscape resize lost mapping",
                )?;
                self.before = Some(e.doc.clone());
                e.file_job(FileJob::Save(
                    self.folder.join("mapping.rastera.json"),
                    Box::new(e.doc.clone()),
                ));
            }
            49 => e.file_job(FileJob::Load(self.folder.join("mapping.rastera.json"))),
            50 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Custom canvas/font/mapping project round trip failed",
                )?;
                unsafe {
                    rastera_editor_close_output(
                        outputs.get(&1).ok_or("Detail missing")?.handle.as_ptr(),
                    );
                }
            }
            51 => {
                check(!enabled.contains(&1), "Mapped output close not observed")?;
                enabled.insert(1);
            }
            52 => {
                check(
                    enabled.contains(&1) && e.doc == *self.before.as_ref().unwrap(),
                    "Mapped output reopen lost settings",
                )?;
                let font = e.doc.texts[&3].font.clone().unwrap();
                check(
                    e.library.available(&font),
                    "Selected font unexpectedly fell back",
                )?;
                let mut candidate = e.doc.clone();
                candidate.texts.get_mut(&3).unwrap().font = Some(rastera_core::fonts::FontRef {
                    family: "Unavailable test font".into(),
                    style: "Regular".into(),
                    postscript: "RasteraMissingTestFont".into(),
                });
                e.stage_document(candidate, None);
            }
            53 => {
                let font = e.doc.texts[&3].font.as_ref().unwrap();
                check(
                    !e.library.available(font) && font.postscript == "RasteraMissingTestFont",
                    "Missing-font identity/fallback indication failed",
                )?;
                e.stage_document(self.before.as_ref().unwrap().clone(), None);
            }
            54 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Missing-font recovery changed project",
                )?;
                let mut candidate = e.doc.clone();
                candidate.canvas = [8192, 8192];
                check(candidate.validate().is_err(), "Over-budget canvas accepted")?;
                e.settings = Some(candidate.canvas);
                self.frame = 0;
            }
            55 => {
                self.frame += 1;
                if self.frame < 60 {
                    return Ok(false);
                }
                e.settings = None;
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Cancel changed live document",
                )?;
                let mut candidate = e.doc.clone();
                candidate.canvas = [2558, 1438];
                candidate.images.get_mut(&1).unwrap().path = self.folder.join("absent.png");
                e.stage_document(candidate, None);
            }
            56 => {
                check(e.error.is_some(), "Failed candidate unexpectedly applied")?;
            }
            57 => {
                check(
                    e.error.is_some() && e.doc == *self.before.as_ref().unwrap(),
                    "Failed scene preparation changed live document",
                )?;
                e.error = None;
                unsafe {
                    rastera_editor_close_output(
                        outputs
                            .get(&2)
                            .ok_or("Mapped Right missing")?
                            .handle
                            .as_ptr(),
                    );
                }
            }
            58 => {
                check(!enabled.contains(&2), "Mapped Right close not observed")?;
                enabled.insert(2);
            }
            59 => {
                check(
                    enabled.contains(&2) && e.doc == *self.before.as_ref().unwrap(),
                    "Mapped Right reopen lost settings",
                )?;
                e.change_mode(Mode::Viewports);
                e.viewport_selected = Some(1);
                log(format!("canvas_mapping_probe passed=1 portrait_odd_resize=1 retained_coordinates=1 resize_undo_redo=1 viewport_native_pan_zoom_rotate=1 gesture_undo_redo=1 fit_canvas=1 installed_font_face=1 dialogs_native=1 mapping_font_roundtrip=1 mapped_output_reopen=1 missing_font_fallback=1 budget_rejection=1 cancel_preserved=1 failed_candidate_preserved=1 result={}\n",self.folder.join("mapping.rastera.json").display()));
                if !self.selection {
                    return Ok(true);
                }
                let mut candidate = e.doc.clone();
                candidate.canvas = [3840, 1080];
                candidate.viewports = rastera_core::mapping::defaults();
                candidate.viewports[0].center = [1400., 540.];
                candidate.viewports[0].zoom = 2.;
                candidate.viewports[1].center = [1400., 540.];
                candidate.viewports[1].zoom = 0.75;
                e.stage_document(candidate, None);
            }
            60 => {
                e.history = History::default();
                e.select_viewport(2);
                self.before = Some(e.doc.clone());
                let (map, scale) = e.preview_mapping(p);
                let world_per_point = e.doc.canvas[0] as f32 * scale / map.fit[2];
                let q = e.doc.viewports[0].corners()[0];
                let label = [q[0] + 12. * world_per_point, q[1] + 12. * world_per_point];
                self.handle = Some(label);
                Self::event(p, 1, label);
            }
            61 => {
                check(
                    e.viewport_selected == Some(1),
                    "Enclosing Right stole Left label",
                )?;
                let q = self.handle.unwrap();
                Self::event(p, 2, [q[0] + 200., q[1] + 50.]);
            }
            62 => {
                let q = self.handle.unwrap();
                Self::event(p, 3, [q[0] + 200., q[1] + 50.]);
            }
            63 => {
                check(
                    (e.doc.viewports[0].center[0] - 1600.).abs() < 0.2
                        && e.doc.viewports[1] == self.before.as_ref().unwrap().viewports[1],
                    "Label drag changed wrong viewport",
                )?;
                e.action(Action::Undo);
            }
            64 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Label drag undo failed",
                )?;
                let mut h = e.history.clone();
                let mut d = e.doc.clone();
                check(!h.undo(&mut d), "Selection/gesture created extra history")?;
                e.select_viewport(2);
                Self::event(p, 1, [1880., 540.]);
            }
            65 => {
                check(
                    e.viewport_selected == Some(1),
                    "Enclosing Right stole Left outline",
                )?;
                Self::event(p, 2, [1780., 580.]);
            }
            66 => Self::event(p, 3, [1780., 580.]),
            67 => {
                check(
                    (e.doc.viewports[0].center[0] - 1300.).abs() < 0.2
                        && e.doc.viewports[1] == self.before.as_ref().unwrap().viewports[1],
                    "Outline drag changed wrong viewport",
                )?;
                e.action(Action::Undo);
            }
            68 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Outline undo failed",
                )?;
                e.select_viewport(1);
                Self::event(p, 1, [120., 540.]);
            }
            69 => {
                check(
                    e.viewport_selected == Some(2),
                    "Right outline selection failed",
                )?;
                Self::event(p, 2, [220., 500.]);
            }
            70 => Self::event(p, 3, [220., 500.]),
            71 => {
                check(
                    (e.doc.viewports[1].center[0] - 1500.).abs() < 0.2
                        && e.doc.viewports[0] == self.before.as_ref().unwrap().viewports[0],
                    "Right drag moved Left",
                )?;
                e.action(Action::Undo);
            }
            72 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Right drag undo failed",
                )?;
                e.select_viewport(1);
                Self::event(p, 1, [1400., 540.]);
            }
            73 => Self::modified_event(
                p,
                2,
                [2200., 600.],
                rastera_core::viewport_hit::OPTION_MODIFIER,
            ),
            74 => Self::event(p, 3, [2200., 600.]),
            75 => {
                check(
                    (e.doc.viewports[0].center[0] - 2200.).abs() < 0.2
                        && e.doc.viewports[1] == self.before.as_ref().unwrap().viewports[1],
                    "Sidebar drag did not latch chosen viewport",
                )?;
                e.action(Action::Undo);
            }
            76 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Sidebar drag undo failed",
                )?;
                e.doc.viewports[1].center = e.doc.viewports[0].center;
                e.doc.viewports[1].zoom = e.doc.viewports[0].zoom;
                e.publish();
                e.history = History::default();
                self.before = Some(e.doc.clone());
                e.select_viewport(1);
                Self::modified_event(
                    p,
                    1,
                    [1400., 540.],
                    rastera_core::viewport_hit::OPTION_MODIFIER,
                );
            }
            77 => {
                check(
                    e.viewport_selected == Some(2) && e.view_gesture.is_none(),
                    "Option modifier/cycle did not reach Rust",
                )?;
                Self::modified_event(
                    p,
                    2,
                    [1600., 600.],
                    rastera_core::viewport_hit::OPTION_MODIFIER,
                );
            }
            78 => Self::event(p, 3, [1600., 600.]),
            79 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Option-click unexpectedly dragged",
                )?;
                let mut h = e.history.clone();
                let mut d = e.doc.clone();
                check(!h.undo(&mut d), "Selection alone added history")?;
                Self::modified_event(
                    p,
                    1,
                    [1400., 540.],
                    rastera_core::viewport_hit::OPTION_MODIFIER,
                );
            }
            80 => {
                check(
                    e.viewport_selected == Some(1),
                    "Coincident cycle did not wrap",
                )?;
                Self::event(p, 3, [1400., 540.]);
            }
            81 => Self::event(p, 1, [1400., 540.]),
            82 => Self::event(p, 2, [1440., 570.]),
            83 => Self::event(p, 3, [1440., 570.]),
            84 => {
                check(
                    (e.doc.viewports[0].center[0] - 1440.).abs() < 0.2
                        && e.doc.viewports[1] == self.before.as_ref().unwrap().viewports[1],
                    "Ordinary drag lost Option-selected viewport",
                )?;
                e.action(Action::Undo);
            }
            85 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Coincident drag undo failed",
                )?;
                e.doc.viewports[1].zoom = 0.75;
                e.publish();
                e.history = History::default();
                e.file_job(FileJob::Save(
                    self.folder.join("selection.rastera.json"),
                    Box::new(e.doc.clone()),
                ));
            }
            86 => {
                log("viewport_selection_probe passed=1 nested_label=1 nested_outline=1 move_both_independently=1 sidebar_drag_latched=1 option_native_modifiers=1 coincident_cycle=1 selection_no_history=1 single_gesture_undo=1 baseline_resize_rotation=1\n".into());
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
