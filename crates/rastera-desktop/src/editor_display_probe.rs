//! Opt-in owned-window assertions. No global input, display-mode changes or pixel readback.
use super::*;
use crate::shared_native::destinations::Destination;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};
unsafe extern "C" {
    fn rastera_shared_focus_probe(handle: *mut std::ffi::c_void);
    fn rastera_shared_presentation_probe(handle: *mut std::ffi::c_void) -> *mut c_char;
    fn rastera_shared_instance(handle: *mut std::ffi::c_void) -> u64;
    fn rastera_shared_recovery_probe();
    fn rastera_identify_visible_probe() -> u32;
}
pub struct DisplayProbe {
    phase: u32,
    folder: PathBuf,
    screen: String,
    second: String,
    instance: u64,
    rows: Vec<Value>,
    start: Instant,
    new_id: u64,
}
fn check(ok: bool, message: &str) -> Result<(), String> {
    if ok {
        Ok(())
    } else {
        Err(message.into())
    }
}
fn state(c: &Consumer) -> Value {
    unsafe {
        let ptr = rastera_shared_presentation_probe(c.handle.as_ptr());
        let value = serde_json::from_slice(std::ffi::CStr::from_ptr(ptr).to_bytes()).unwrap();
        rastera_editor_free_string(ptr);
        value
    }
}
impl DisplayProbe {
    pub fn new(folder: PathBuf) -> Self {
        Self {
            phase: 0,
            folder,
            screen: String::new(),
            second: String::new(),
            instance: 0,
            rows: vec![],
            start: Instant::now(),
            new_id: 0,
        }
    }
    fn fullscreen(&mut self, e: &Editor, c: &Consumer, target: &str) -> Result<(), String> {
        let value = state(c);
        let d = e
            .destinations
            .discovery
            .displays
            .iter()
            .find(|d| d.id == target)
            .ok_or("Probe target unavailable")?;
        check(value["fullscreen"] == true, "Not fullscreen")?;
        check(
            serde_json::from_value::<[f64; 4]>(value["frame"].clone()).unwrap() == d.frame,
            "Fullscreen frame differs from NSScreen full frame",
        )?;
        check(
            value["drawable"] == json!(d.pixels),
            "Drawable differs from display backing dimensions",
        )?;
        check(
            value["key"] == false && value["mouse_ignored"] == true,
            "Clean fullscreen stole focus or intercepted mouse",
        )?;
        check(
            value["level"].as_i64().unwrap() > 0,
            "Fullscreen not above ordinary windows",
        )?;
        self.rows.push(json!({"phase":self.phase,"native":value}));
        Ok(())
    }
    pub fn tick(
        &mut self,
        e: &mut Editor,
        p: &Consumer,
        outputs: &BTreeMap<u64, Consumer>,
        enabled: &mut BTreeSet<u64>,
    ) -> Result<bool, String> {
        if !self.folder.join("display-start").exists() {
            return Ok(false);
        }
        match self.phase {
            0 => {
                enabled.clear();
                check(
                    !e.destinations.discovery.displays.is_empty(),
                    "No connected displays",
                )?;
                self.screen = e.destinations.discovery.displays[0].id.clone();
                check(!self.screen.is_empty(), "Display has no durable identity")?;
                self.second = e
                    .destinations
                    .discovery
                    .displays
                    .get(1)
                    .map(|d| d.id.clone())
                    .unwrap_or(self.screen.clone());
                e.doc.test_background = true;
                e.publish();
                e.file_job(FileJob::Save(
                    self.folder.join("show.json"),
                    Box::new(e.doc.clone()),
                ));
            }
            1 => {
                unsafe { rastera_shared_focus_probe(p.handle.as_ptr()) };
                enabled.insert(1);
            }
            2 => {
                let Some(c) = outputs.get(&1) else {
                    return Ok(false);
                };
                let root = state(p);
                log(format!("display_probe_focus before_fullscreen={}\n", root));
                check(
                    root["key_number"] == root["number"],
                    "Operator not focused before fullscreen",
                )?;
                self.rows.push(json!({"phase":2,"windowed":state(c)}));
                e.destinations.identify = true;
                self.start = Instant::now();
                e.destinations.preferences.insert(
                    1,
                    Destination {
                        fullscreen: true,
                        display: Some(self.screen.clone()),
                    },
                );
                e.destinations.persist();
            }
            3 => {
                let c = outputs.get(&1).ok_or("Output disappeared")?;
                self.fullscreen(e, c, &self.screen.clone())?;
                let root = state(p);
                check(
                    root["key_number"] == root["number"],
                    "Operator did not retain keyboard focus",
                )?;
                self.instance = unsafe { rastera_shared_instance(c.handle.as_ptr()) };
                e.apply_viewport_size(1, [1024, 768])?;
            }
            4 => {
                self.fullscreen(e, &outputs[&1], &self.screen.clone())?;
                e.action(Action::Undo);
            }
            5 => {
                self.fullscreen(e, &outputs[&1], &self.screen.clone())?;
                e.destinations.preferences.insert(
                    2,
                    Destination {
                        fullscreen: true,
                        display: Some(self.screen.clone()),
                    },
                );
                enabled.insert(2);
            }
            6 => {
                check(
                    !outputs.contains_key(&2) && !enabled.contains(&2),
                    "Conflict replaced/duplicated an active output",
                )?;
                check(
                    e.destinations
                        .status
                        .get(&2)
                        .is_some_and(|s| s.contains("Conflicting")),
                    "Conflict status missing",
                )?;
                check(
                    unsafe { rastera_shared_instance(outputs[&1].handle.as_ptr()) }
                        == self.instance,
                    "Conflict replaced owner consumer",
                )?;
                e.destinations.preferences.get_mut(&1).unwrap().display =
                    Some("00000000-0000-0000-0000-000000000000".into());
            }
            7 => {
                check(
                    !outputs.contains_key(&1) && !enabled.contains(&1),
                    "Missing target stayed enabled",
                )?;
                check(
                    e.destinations
                        .status
                        .get(&1)
                        .is_some_and(|s| s.contains("Missing")),
                    "Missing status absent",
                )?;
                e.destinations.preferences.get_mut(&1).unwrap().display = Some(self.second.clone());
                enabled.insert(1);
            }
            8 => {
                let Some(c) = outputs.get(&1) else {
                    return Ok(false);
                };
                self.fullscreen(e, c, &self.second.clone())?;
                if self.second != self.screen {
                    enabled.insert(2);
                }
                e.add_viewport();
                self.new_id = e.viewport_selected.unwrap();
                enabled.insert(self.new_id);
                e.action(Action::AddText);
            }
            9 => {
                check(
                    e.doc.layers.len() == 1,
                    "Operator edit failed with output active",
                )?;
                if self.second != self.screen {
                    self.fullscreen(e, &outputs[&2], &self.screen.clone())?;
                }
                e.destinations.preferences.insert(1, Destination::default());
                enabled.remove(&2);
            }
            10 => {
                let value = state(&outputs[&1]);
                check(value["fullscreen"] == false, "Did not return to windowed")?;
                check(
                    value["drawable"] == json!([1920, 1080]),
                    "Windowed logical pixel size not restored",
                )?;
                self.rows.push(json!({"phase":10,"windowed":value}));
                e.remove_viewport(1);
            }
            11 => {
                check(
                    !outputs.contains_key(&1),
                    "Deleted viewport retained consumer",
                )?;
                check(
                    outputs.contains_key(&self.new_id),
                    "Deletion retired unrelated output",
                )?;
                e.action(Action::Undo);
            }
            12 => {
                check(!outputs.contains_key(&1), "Undo reopened output")?;
                enabled.insert(1);
            }
            13 => {
                let Some(c) = outputs.get(&1) else {
                    return Ok(false);
                };
                unsafe { rastera_editor_close_output(c.handle.as_ptr()) };
            }
            14 => {
                check(!outputs.contains_key(&1), "Native close did not retire")?;
                enabled.insert(1);
            }
            15 => {
                if !outputs.contains_key(&1) {
                    return Ok(false);
                }
                e.destinations.preferences.insert(
                    1,
                    Destination {
                        fullscreen: true,
                        display: Some(self.second.clone()),
                    },
                );
                e.destinations.persist();
                e.file_job(FileJob::Save(
                    self.folder.join("show.json"),
                    Box::new(e.doc.clone()),
                ));
            }
            16 => {
                self.fullscreen(e, &outputs[&1], &self.second.clone())?;
                e.file_job(FileJob::Load(self.folder.join("show.json")));
            }
            17 => {
                if !outputs.is_empty() {
                    return Ok(false);
                }
                check(
                    outputs.is_empty() && enabled.is_empty(),
                    "Project reopening enabled saved outputs",
                )?;
                check(
                    e.destinations
                        .preferences
                        .get(&1)
                        .is_some_and(|d| d.fullscreen),
                    "Preferred assignment not restored",
                )?;
                enabled.insert(1);
            }
            18 => {
                let Some(c) = outputs.get(&1) else {
                    return Ok(false);
                };
                if !self.folder.join("transition-ready").exists() {
                    self.fullscreen(e, c, &self.second.clone())?;
                    std::fs::write(self.folder.join("transition-ready"), "ready")
                        .map_err(|e| e.to_string())?;
                }
                if !self.folder.join("transitions-done").exists() {
                    return Ok(false);
                }
                e.destinations.preferences.get_mut(&1).unwrap().display = Some(self.screen.clone());
            }
            19 => {
                self.fullscreen(e, &outputs[&1], &self.screen.clone())?;
                unsafe { rastera_shared_recovery_probe() };
            }
            20 => {
                if !outputs.is_empty() || !enabled.is_empty() {
                    return Ok(false);
                }
                if self.start.elapsed().as_secs_f64() < 3.3 {
                    return Ok(false);
                }
                check(
                    unsafe { rastera_identify_visible_probe() } == 0,
                    "Identify labels did not expire",
                )?;
                let evidence = json!({"passed":true,"display_count":e.destinations.discovery.displays.len(),"external_count":e.destinations.discovery.displays.iter().filter(|d|!d.builtin).count(),"independent_displays_tested":self.second!=self.screen,"native_states":self.rows,"limitations":["Local native replay, not human picture or physical scanout acceptance","Disconnect/reconnect and mixed-scale hardware need actual device checks","Existing transition/capture limitations remain"]});
                std::fs::write(
                    self.folder.join("display-results.json"),
                    serde_json::to_vec_pretty(&evidence).unwrap(),
                )
                .map_err(|e| e.to_string())?;
                log("display_output_probe passed=1 coverage=1 focus=1 fullscreen_resolution_undo=1 mode_switch=1 conflict_owner_preserved=1 missing_no_fallback=1 delete_reopen=1 project_disabled=1 local_recovery_shortcut=1 identify_expired=1\n".into());
                return Ok(true);
            }
            _ => return Ok(true),
        }
        log(format!("display_output_probe phase={}\n", self.phase));
        self.phase += 1;
        Ok(false)
    }
}
