//! Opt-in native lifecycle/metadata regression, keyed only by persistent viewport IDs.
use super::*;
use rastera_core::mapping::{FrameViewport, MAX_VIEWPORTS};
use rastera_core::shared_frames::FrameLease;
use std::collections::{BTreeMap, BTreeSet};
unsafe extern "C" {
    fn rastera_editor_metrics(handle: *mut std::ffi::c_void, out: *mut f32);
    fn rastera_shared_pending_read_probe(handle: *mut std::ffi::c_void) -> bool;
    fn rastera_shared_instance(handle: *mut std::ffi::c_void) -> u64;
    fn rastera_shared_inflight(handle: *mut std::ffi::c_void) -> u32;
}
pub(super) struct Dynamic {
    phase: u32,
    frames: u32,
    folder: PathBuf,
    ids: Vec<u64>,
    handles: BTreeMap<u64, usize>,
    before: Option<Document>,
    history: Option<History>,
    held: Option<FrameLease<wgpu::Texture>>,
}
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}
impl Dynamic {
    pub fn new(folder: PathBuf) -> Self {
        Self {
            phase: 0,
            frames: 0,
            folder,
            ids: vec![],
            handles: BTreeMap::new(),
            before: None,
            history: None,
            held: None,
        }
    }
    fn frame(p: &Consumer) -> FrameLease<wgpu::Texture> {
        // SAFETY: the live preview owns this context; acquire returns an independent completion-style lease.
        unsafe { (&*p.source).acquire().expect("completed preview frame") }
    }
    fn metadata(e: &Editor, p: &Consumer) -> Result<(), String> {
        let f = Self::frame(p);
        check(
            f.meta.canvas == e.doc.canvas,
            "Frame canvas metadata mismatch",
        )?;
        check(
            f.meta.viewports.len() == e.doc.viewports.len(),
            "Frame viewport count mismatch",
        )?;
        for v in &e.doc.viewports {
            check(
                f.meta.viewport(v.id) == Some(&FrameViewport::from(v)),
                "ID lookup returned wrong dimensions/mapping",
            )?;
        }
        Ok(())
    }
    fn snapshot(&mut self, outputs: &BTreeMap<u64, Consumer>) {
        self.handles = outputs
            .iter()
            .map(|(&id, c)| {
                (id, unsafe { rastera_shared_instance(c.handle.as_ptr()) }
                    as usize)
            })
            .collect();
    }
    fn unchanged(&self, outputs: &BTreeMap<u64, Consumer>, except: u64) -> Result<(), String> {
        for (&id, &handle) in &self.handles {
            if id != except {
                check(
                    outputs
                        .get(&id)
                        .is_some_and(|c| unsafe { rastera_shared_instance(c.handle.as_ptr()) } as usize == handle),
                    "Unaffected consumer changed identity/lifetime",
                )?;
            }
        }
        Ok(())
    }
    pub fn tick(
        &mut self,
        e: &mut Editor,
        p: &Consumer,
        outputs: &BTreeMap<u64, Consumer>,
        open: &mut BTreeSet<u64>,
    ) -> Result<bool, String> {
        if let Some(error) = &e.error {
            return Err(error.clone());
        }
        match self.phase {
            0 => {
                e.change_mode(Mode::Viewports);
                e.doc.test_background = true;
                let ids: Vec<_> = e.doc.viewports.iter().map(|v| v.id).collect();
                for id in ids {
                    e.remove_viewport(id);
                }
                e.publish();
            }
            1 => {
                check(
                    outputs.is_empty(),
                    "Removing every viewport did not retire outputs",
                )?;
                Self::metadata(e, p)?;
                e.file_job(FileJob::Save(
                    self.folder.join("empty.json"),
                    Box::new(e.doc.clone()),
                ));
            }
            2 => e.file_job(FileJob::Load(self.folder.join("empty.json"))),
            3 => {
                check(
                    e.doc.viewports.is_empty(),
                    "Empty project reopened with default viewports",
                )?;
                e.add_viewport();
                let id = e.viewport_selected.unwrap();
                check(
                    !open.contains(&id),
                    "Add Viewport unexpectedly opened a window",
                )?;
                self.ids.push(id);
                open.insert(id);
            }
            4 => {
                if !outputs.contains_key(&self.ids[0]) {
                    return Ok(false);
                }
                for size in [[1080, 1920], [1024, 768], [960, 192]] {
                    e.add_viewport();
                    let id = e.viewport_selected.unwrap();
                    e.apply_viewport_size(id, size)?;
                    check(!open.contains(&id), "New viewport window was not separate")?;
                    self.ids.push(id);
                    open.insert(id);
                }
            }
            5 => {
                if outputs.len() != 4 {
                    return Ok(false);
                }
                Self::metadata(e, p)?;
                if self.frames == 0 {
                    for v in &e.doc.viewports {
                        let mut metrics = [0.; 3];
                        unsafe {
                            rastera_editor_metrics(
                                outputs[&v.id].handle.as_ptr(),
                                metrics.as_mut_ptr(),
                            );
                        }
                        check(
                            metrics[0] == v.size[0] as f32 && metrics[1] == v.size[1] as f32,
                            "Initial native backing size differs from output resolution",
                        )?;
                    }
                    self.snapshot(outputs);
                    self.before = Some(e.doc.clone());
                    std::fs::write(
                        self.folder.join("dynamic-open.json"),
                        serde_json::to_vec_pretty(&e.doc).unwrap(),
                    )
                    .map_err(|v| v.to_string())?;
                    log("dynamic_viewport_probe windows_ready=1 count=4\n".into());
                }
                self.frames += 1;
                if self.frames < 120
                    || (std::env::args().any(|a| a == "--dynamic-layout-driver")
                        && !self.folder.join("layout-ready").exists())
                {
                    return Ok(false);
                }
                self.frames = 0;
                e.apply_viewport_size(self.ids[1], [1920, 1200])?;
            }
            6 => {
                Self::metadata(e, p)?;
                self.unchanged(outputs, 0)?;
                let id = self.ids[1];
                let mut metrics = [0.; 3];
                unsafe {
                    rastera_editor_metrics(outputs[&id].handle.as_ptr(), metrics.as_mut_ptr());
                }
                if metrics[0] != 1920. || metrics[1] != 1200. {
                    return Ok(false);
                }
                let old = self.before.as_ref().unwrap().viewport(id).unwrap();
                let v = e.doc.viewport(id).unwrap();
                check(
                    (v.center, v.zoom, v.rotation) == (old.center, old.zoom, old.rotation),
                    "Dimensions changed the transform",
                )?;
                e.action(Action::Undo);
            }
            7 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Dimension Apply was not one undo entry",
                )?;
                let id = self.ids[0];
                let c = outputs.get(&id).ok_or("first output missing")?;
                // Sample actual native command activity before removing an open definition.
                // SAFETY: live main-thread consumer; opt-in GPU-only diagnostic read.
                if !unsafe { rastera_shared_pending_read_probe(c.handle.as_ptr()) } {
                    return Ok(false);
                }
                let inflight = unsafe { rastera_shared_inflight(c.handle.as_ptr()) };
                check(inflight > 0, "Pending GPU read was not held in flight")?;
                log(format!(
                    "dynamic_viewport_probe remove_id={id} sampled_inflight={inflight}\n"
                ));
                self.held = Some(Self::frame(p));
                e.remove_viewport(id);
                self.frames = 0;
            }
            8 => {
                let id = self.ids[0];
                if outputs.contains_key(&id) {
                    return Ok(false);
                }
                self.unchanged(outputs, id)?;
                Self::metadata(e, p)?;
                check(
                    Self::frame(p).meta.viewport(id).is_none(),
                    "Deleted ID silently mapped to another row",
                )?;
                check(
                    self.held.as_ref().unwrap().meta.viewport(id).is_some(),
                    "Old lease lost immutable metadata",
                )?;
                e.action(Action::Undo);
            }
            9 => {
                let id = self.ids[0];
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Removal undo lost definition/ID/settings",
                )?;
                check(
                    !outputs.contains_key(&id) && !open.contains(&id),
                    "Undo restored window lifetime instead of only logical definition",
                )?;
                self.unchanged(outputs, id)?;
                e.action(Action::Redo);
                check(
                    e.doc.viewport(id).is_none(),
                    "Removal redo retained definition",
                )?;
                e.action(Action::Undo);
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Repeated undo lost definition",
                )?;
                check(!open.contains(&id), "History reopened a native window")?;
                open.insert(id);
                self.held = None;
            }
            10 => {
                if !outputs.contains_key(&self.ids[0]) {
                    return Ok(false);
                }
                Self::metadata(e, p)?;
                self.snapshot(outputs);
                unsafe {
                    rastera_editor_close_output(outputs[&self.ids[2]].handle.as_ptr());
                }
            }
            11 => {
                let id = self.ids[2];
                if outputs.contains_key(&id) {
                    return Ok(false);
                }
                check(
                    e.doc.viewport(id).is_some() && !open.contains(&id),
                    "Window close deleted definition",
                )?;
                open.insert(id);
            }
            12 => {
                if !outputs.contains_key(&self.ids[2]) {
                    return Ok(false);
                }
                e.add_viewport();
                let id = e.viewport_selected.unwrap();
                self.ids.push(id);
                e.apply_viewport_size(id, [3440, 1440])?;
                open.insert(id);
            }
            13 => {
                if outputs.len() != 5 {
                    return Ok(false);
                }
                Self::metadata(e, p)?;
                self.frames += 1;
                if self.frames < 60 {
                    return Ok(false);
                }
                self.frames = 0;
                self.snapshot(outputs);
                e.remove_viewport(self.ids[4]);
            }
            14 => {
                if outputs.contains_key(&self.ids[4]) {
                    return Ok(false);
                }
                self.unchanged(outputs, self.ids[4])?;
                e.action(Action::Undo);
            }
            15 => {
                let id = self.ids[4];
                check(
                    e.doc.viewport(id).unwrap().size == [3440, 1440] && !open.contains(&id),
                    "Added output removal undo failed",
                )?;
                e.apply_viewport_size(id, [1080, 1080])?;
                open.insert(id);
            }
            16 => {
                if outputs.len() != 5 {
                    return Ok(false);
                }
                Self::metadata(e, p)?;
                self.snapshot(outputs);
                self.before = Some(e.doc.clone());
                self.history = Some(e.history.clone());
                e.select_viewport(self.ids[4]);
                Probe::modified_event(
                    p,
                    1,
                    [1920., 540.],
                    rastera_core::viewport_hit::OPTION_MODIFIER,
                );
            }
            17 => {
                check(
                    e.viewport_selected == Some(self.ids[0]) && e.view_gesture.is_none(),
                    "Dynamic overlap cycle was not ID ordered",
                )?;
                Probe::modified_event(
                    p,
                    2,
                    [1980., 570.],
                    rastera_core::viewport_hit::OPTION_MODIFIER,
                );
            }
            18 => Probe::event(p, 3, [1980., 570.]),
            19 => {
                check(
                    e.doc == *self.before.as_ref().unwrap()
                        && e.history == *self.history.as_ref().unwrap(),
                    "Dynamic selection changed document/history",
                )?;
                Probe::event(p, 1, [1920., 540.]);
            }
            20 => Probe::event(p, 2, [1980., 570.]),
            21 => Probe::event(p, 3, [1980., 570.]),
            22 => {
                for v in &e.doc.viewports {
                    if v.id != self.ids[0] {
                        check(
                            Some(v) == self.before.as_ref().unwrap().viewport(v.id),
                            "Dynamic drag shifted another definition",
                        )?;
                    }
                }
                check(
                    (e.doc.viewport(self.ids[0]).unwrap().center[0] - 1980.).abs() < 0.2,
                    "Dynamic selected drag failed",
                )?;
                e.action(Action::Undo);
            }
            23 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Dynamic drag undo failed",
                )?;
                e.file_job(FileJob::Save(
                    self.folder.join("dynamic.json"),
                    Box::new(e.doc.clone()),
                ));
            }
            24 => e.file_job(FileJob::Load(self.folder.join("dynamic.json"))),
            25 => {
                check(
                    e.doc == *self.before.as_ref().unwrap(),
                    "Dynamic project round trip changed definitions",
                )?;
                self.unchanged(outputs, 0)?;
                while e.doc.viewports.len() < MAX_VIEWPORTS {
                    e.add_viewport();
                    check(
                        !open.contains(&e.viewport_selected.unwrap()),
                        "Added window auto-opened",
                    )?;
                }
                let high = e.doc.next_viewport_id;
                e.add_viewport();
                check(
                    e.error.as_ref().is_some_and(|s| s.contains("limit")),
                    "Viewport-count limit missing",
                )?;
                e.error = None;
                e.action(Action::Undo);
                e.add_viewport();
                check(
                    e.viewport_selected.unwrap() >= high,
                    "Undo branch reused a viewport ID",
                )?;
            }
            26 => {
                Self::metadata(e, p)?;
                check(
                    e.doc.viewports.len() == MAX_VIEWPORTS,
                    "Maximum metadata count not published",
                )?;
                let ids: Vec<_> = e.doc.viewports.iter().map(|v| v.id).collect();
                for id in ids {
                    e.remove_viewport(id);
                }
            }
            27 => {
                if !outputs.is_empty() {
                    return Ok(false);
                }
                Self::metadata(e, p)?;
                e.file_job(FileJob::Save(
                    self.folder.join("empty.json"),
                    Box::new(e.doc.clone()),
                ));
            }
            28 => e.file_job(FileJob::Load(self.folder.join("empty.json"))),
            29 => {
                check(
                    e.doc.viewports.is_empty() && open.is_empty(),
                    "Empty saved list was repopulated",
                )?;
                let high = e.doc.next_viewport_id;
                e.add_viewport();
                check(
                    e.viewport_selected.unwrap() >= high,
                    "Empty reload recycled allocator",
                )?;
                e.apply_viewport_size(e.viewport_selected.unwrap(), [1920, 1200])?;
            }
            30 => {
                check(
                    outputs.is_empty() && e.doc.viewports.len() == 1,
                    "One logical viewport requires an unwanted window",
                )?;
                Self::metadata(e, p)?;
                e.file_job(FileJob::Load(self.folder.join("dynamic.json")));
            }
            31 => {
                for &id in self.ids.iter().take(4) {
                    open.insert(id);
                }
            }
            32 => {
                if outputs.len() != 4 {
                    return Ok(false);
                }
                Self::metadata(e, p)?;
                e.file_job(FileJob::Save(
                    self.folder.join("review.rastera.json"),
                    Box::new(e.doc.clone()),
                ));
            }
            33 => {
                log("dynamic_viewport_probe passed=1 zero_one_many=1 four_aspects=1 add_remove_live=1 size_apply_undo=1 id_metadata=1 unaffected_consumers=1 immutable_old_lease=1 window_lifetime_separate=1 overlap_cycle_drag=1 project_empty_roundtrip=1 allocator_limit=1\n".into());
                return Ok(true);
            }
            _ => return Err("Unknown dynamic probe phase".into()),
        }
        self.phase += 1;
        log(format!(
            "dynamic_viewport_probe phase={} count={} outputs={}\n",
            self.phase,
            e.doc.viewports.len(),
            outputs.len()
        ));
        Ok(false)
    }
}
