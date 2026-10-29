//! Canvas settings, font dialogs and viewport interaction; no producer work.
use super::*;

use rastera_core::{mapping::Viewport, theme as t};
use std::ffi::CString;
#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum Mode {
    #[default]
    Canvas,
    Viewports,
}
pub(super) struct ViewGesture {
    before: Document,
    id: u64,
    start: [f32; 2],
    original: Viewport,
    kind: u32,
}
#[repr(C)]
struct Line {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    color: [f32; 4],
    width: f32,
    kind: u32,
}
#[repr(C)]
struct Label {
    x: f32,
    y: f32,
    text: *const c_char,
    color: [f32; 4],
}
unsafe extern "C" {
    fn rastera_editor_overlay(
        handle: *mut std::ffi::c_void,
        canvas: *const u32,
        lines: *const Line,
        count: u32,
        labels: *const Label,
        label_count: u32,
    );
    fn rastera_editor_metrics(handle: *mut std::ffi::c_void, out: *mut f32);
    fn rastera_editor_label_size(text: *const c_char, out: *mut f32);
    fn rastera_font_catalog() -> *mut c_char;
    fn rastera_settings_menu(handle: *mut std::ffi::c_void) -> i32;
}
pub(super) fn library() -> Arc<rastera_core::fonts::FontLibrary> {
    let lib = Arc::new(rastera_core::fonts::FontLibrary::default());
    lib.ready.store(false, Ordering::Release);
    let worker = lib.clone();
    thread::spawn(move || {
        // SAFETY: Core Text discovery is thread safe; returned UTF-8 allocation freed once.
        unsafe {
            let p = rastera_font_catalog();
            if !p.is_null() {
                let json = CStr::from_ptr(p).to_string_lossy();
                if let Ok(mut faces) =
                    serde_json::from_str::<Vec<rastera_core::fonts::InstalledFace>>(&json)
                {
                    faces.sort_by(|a, b| (&a.family, &a.style).cmp(&(&b.family, &b.style)));
                    faces.dedup_by(|a, b| a.postscript == b.postscript);
                    *worker.faces.lock().unwrap() = faces;
                }
                rastera_editor_free_string(p);
            }
        }
        worker.ready.store(true, Ordering::Release);
    });
    lib
}
pub(super) fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = color(t::WORKSPACE);
    v.window_fill = color(t::WORKSPACE);
    v.extreme_bg_color = color(t::WORKSPACE);
    v.faint_bg_color = color(t::DEEP_PLUM);
    v.override_text_color = Some(color(t::SIGNAL_WHITE));
    v.selection.bg_fill = color(t::RASTER_PURPLE);
    v.selection.stroke = egui::Stroke::new(1_f32, color(t::SIGNAL_MINT));
    v.widgets.inactive.bg_fill = color(t::DEEP_PLUM);
    v.widgets.inactive.weak_bg_fill = color(t::DEEP_PLUM);
    v.widgets.noninteractive.bg_stroke.color = color(t::DEEP_PLUM);
    v.widgets.active.bg_fill = color(t::RASTER_PURPLE);
    v.widgets.active.fg_stroke.color = color(t::SIGNAL_MINT);
    v.widgets.hovered.bg_fill = color(t::DEEP_PLUM);
    v.window_stroke = egui::Stroke::new(1_f32, color(t::RETRO_LAVENDER));
    ctx.set_visuals(v);
}
fn color(c: [u8; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}
impl Editor {
    pub fn settings_requested(&mut self) -> bool {
        std::mem::take(&mut self.settings_requested)
    }
    pub(super) fn change_mode(&mut self, mode: Mode) {
        if mode != self.mode {
            self.finish_gesture();
            self.finish_view_gesture();
            self.finish_inspector();
            self.mode = mode;
        }
    }
    pub(super) fn finish_view_gesture(&mut self) {
        if let Some(g) = self.view_gesture.take() {
            self.history.record(g.before, &self.doc);
        }
    }
    fn rotation_point(&self, id: u64, map: PreviewMap, scale: f32) -> [f32; 2] {
        let f = rastera_core::viewport_hit::Footprint::new(
            self.doc.viewport(id).unwrap(),
            map,
            scale,
            [0.; 2],
        );
        map.canvas(f.rotation.map(|x| x * scale), true).unwrap_or(
            self.doc
                .viewport(id)
                .unwrap()
                .canvas_point([self.doc.viewport(id).unwrap().size[0] as f32 / 2., 0.]),
        )
    }
    fn label_sizes(&self) -> Vec<[f32; 2]> {
        self.doc
            .viewports
            .iter()
            .map(|v| {
                let name =
                    CString::new(format!("{} · {}", v.name.replace('\0', ""), v.id)).unwrap();
                let mut size = [0.; 2];
                // SAFETY: main-thread native font measurement writes exactly two floats.
                unsafe {
                    rastera_editor_label_size(name.as_ptr(), size.as_mut_ptr());
                }
                size
            })
            .collect()
    }

    pub(super) fn preview_mapping(&self, preview: &Consumer) -> (PreviewMap, f32) {
        let mut metrics = [0.; 3];
        // SAFETY: main-thread query writes three floats for a live preview.
        unsafe {
            rastera_editor_metrics(preview.handle.as_ptr(), metrics.as_mut_ptr());
        }
        (
            PreviewMap::for_canvas(self.doc.canvas, [metrics[0], metrics[1]], 20. * metrics[2]),
            metrics[2],
        )
    }
    pub(super) fn select_viewport(&mut self, id: u64) {
        self.finish_view_gesture();
        self.finish_inspector();
        self.viewport_selected = Some(id);
    }
    pub(super) fn rotation_handle(&self, preview: &Consumer, id: u64) -> [f32; 2] {
        let mut metrics = [0.; 3];
        // SAFETY: main-thread metrics for this live preview; writes exactly three floats.
        unsafe {
            rastera_editor_metrics(preview.handle.as_ptr(), metrics.as_mut_ptr());
        }
        let map =
            PreviewMap::for_canvas(self.doc.canvas, [metrics[0], metrics[1]], 20. * metrics[2]);
        self.rotation_point(id, map, metrics[2])
    }
    pub(super) fn view_pointer(&mut self, event: &Pointer, map: PreviewMap, p: [f32; 2]) {
        match event.kind {
            1 => {
                self.finish_view_gesture();
                self.finish_inspector();
                use rastera_core::viewport_hit::{self, HitKind};
                let picked = viewport_hit::pick(
                    &self.doc.viewports,
                    map,
                    event.scale,
                    &self.label_sizes(),
                    self.viewport_selected,
                    [event.x, event.y],
                    event.modifiers & viewport_hit::OPTION_MODIFIER != 0,
                );
                self.viewport_selected = picked.map(|hit| hit.id);
                if let Some(hit) = picked.filter(|h| h.kind != HitKind::Cycle) {
                    let id = hit.id;
                    let kind = match hit.kind {
                        HitKind::Pan => 0,
                        HitKind::Resize => 1,
                        HitKind::Rotate => 2,
                        HitKind::Cycle => unreachable!(),
                    };
                    self.view_gesture = Some(ViewGesture {
                        before: self.doc.clone(),
                        id,
                        start: p,
                        original: self.doc.viewport(id).unwrap().clone(),
                        kind,
                    });
                }
            }
            2 | 3 => {
                if let Some(g) = &self.view_gesture {
                    let mut v = g.original.clone();
                    match g.kind {
                        0 => {
                            v.center = [
                                (v.center[0] + (p[0] - g.start[0])).clamp(-65536., 65536.),
                                (v.center[1] + (p[1] - g.start[1])).clamp(-65536., 65536.),
                            ];
                        }
                        1 => {
                            let a = (g.start[0] - v.center[0]).hypot(g.start[1] - v.center[1]);
                            let b = (p[0] - v.center[0]).hypot(p[1] - v.center[1]);
                            v.zoom = (v.zoom * a / b.max(0.01))
                                .clamp(rastera_core::mapping::MIN_VIEWPORT_ZOOM, 8192.);
                        }
                        _ => {
                            let a = (g.start[1] - v.center[1]).atan2(g.start[0] - v.center[0]);
                            let b = (p[1] - v.center[1]).atan2(p[0] - v.center[0]);
                            v.rotation =
                                (v.rotation + (b - a).to_degrees() + 180.).rem_euclid(360.) - 180.;
                        }
                    }
                    if let Some(target) = self.doc.viewport_mut(g.id) {
                        *target = v;
                    } else {
                        self.view_gesture = None;
                        return;
                    }
                    self.publish();
                }
                if event.kind == 3 {
                    self.finish_view_gesture();
                }
            }
            4 => {
                if let Some(g) = self.view_gesture.take() {
                    self.doc = g.before;
                    self.publish();
                }
            }
            _ => {}
        }
    }
    pub(super) fn overlays(&self, preview: &Consumer) {
        let mut metrics = [0.; 3];
        unsafe {
            rastera_editor_metrics(preview.handle.as_ptr(), metrics.as_mut_ptr());
        }
        let map =
            PreviewMap::for_canvas(self.doc.canvas, [metrics[0], metrics[1]], 20. * metrics[2]);
        let mut lines = vec![];
        let mut names = vec![];
        let mut label_values = vec![];
        let mut line = |a: [f32; 2], b: [f32; 2], c: [u8; 4], width: f32, kind: u32| {
            lines.push(Line {
                x1: a[0],
                y1: a[1],
                x2: b[0],
                y2: b[1],
                color: t::floats(c),
                width,
                kind,
            })
        };
        let w = self.doc.canvas[0] as f32;
        let h = self.doc.canvas[1] as f32;
        let boundary = [[0., 0.], [w, 0.], [w, h], [0., h]];
        for i in 0..4 {
            line(boundary[i], boundary[(i + 1) % 4], t::BOUNDARY, 1., 0);
        }
        if self.mode == Mode::Canvas {
            if let Some(l) = self
                .selected
                .and_then(|id| self.doc.layer(id))
                .filter(|l| l.visible)
            {
                let q = l.rect.corners();
                let order = [0, 1, 3, 2];
                for i in 0..4 {
                    line(
                        q[order[i]],
                        q[order[(i + 1) % 4]],
                        if l.locked {
                            t::RETRO_LAVENDER
                        } else {
                            t::SIGNAL_MINT
                        },
                        1.5,
                        0,
                    );
                    if !l.locked {
                        line(q[i], q[i], t::SIGNAL_MINT, 1., 1);
                    }
                }
            }
        } else {
            for v in &self.doc.viewports {
                let q = v.corners();
                let selected = self.viewport_selected == Some(v.id);
                for k in 0..4 {
                    line(
                        q[k],
                        q[(k + 1) % 4],
                        v.color,
                        if selected { 2. } else { 1. },
                        0,
                    );
                    if selected {
                        line(q[k], q[k], v.color, 1., 1);
                    }
                }
                let top = v.canvas_point([v.size[0] as f32 / 2., 0.]);
                line(v.center, top, v.color, 1., 0);
                line(
                    top,
                    v.canvas_point([v.size[0] as f32 * 0.47, v.size[1] as f32 * 0.15]),
                    v.color,
                    2.,
                    0,
                );
                line(
                    top,
                    v.canvas_point([v.size[0] as f32 * 0.53, v.size[1] as f32 * 0.15]),
                    v.color,
                    2.,
                    0,
                );
                if selected {
                    let r = self.rotation_point(v.id, map, metrics[2]);
                    line(top, r, v.color, 1., 0);
                    line(r, r, v.color, 1., 2);
                }
                names.push(
                    CString::new(format!("{} · {}", v.name.replace('\0', ""), v.id)).unwrap(),
                );
                label_values.push((q[0], v.color));
            }
        }
        let labels: Vec<_> = label_values
            .iter()
            .zip(&names)
            .map(|((p, c), name)| Label {
                x: p[0],
                y: p[1],
                text: name.as_ptr(),
                color: t::floats(*c),
            })
            .collect();
        // SAFETY: synchronous main-thread copy of POD geometry and UTF-8 labels.
        unsafe {
            rastera_editor_overlay(
                preview.handle.as_ptr(),
                self.doc.canvas.as_ptr(),
                lines.as_ptr(),
                lines.len() as u32,
                labels.as_ptr(),
                labels.len() as u32,
            );
        }
    }
    pub(super) fn add_viewport(&mut self) {
        self.finish_view_gesture();
        self.finish_inspector();
        let before = self.doc.clone();
        match self.doc.add_viewport() {
            Ok(id) => {
                self.viewport_selected = Some(id);
                self.history.record(before, &self.doc);
                self.publish();
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn remove_viewport(&mut self, id: u64) {
        self.finish_view_gesture();
        self.finish_inspector();
        let before = self.doc.clone();
        if self.doc.remove_viewport(id) {
            self.viewport_selected = None;
            self.viewport_size = None;
            self.history.record(before, &self.doc);
            self.publish();
        }
    }
    pub(super) fn apply_viewport_size(&mut self, id: u64, size: [u32; 2]) -> Result<(), String> {
        rastera_core::mapping::validate_output(size)?;
        self.finish_view_gesture();
        self.finish_inspector();
        let before = self.doc.clone();
        self.doc
            .viewport_mut(id)
            .ok_or("Viewport no longer exists")?
            .size = size;
        self.viewport_size = None;
        self.history.record(before, &self.doc);
        self.publish();
        Ok(())
    }
    pub(super) fn viewport_panel(
        &mut self,
        ctx: &egui::Context,
        outputs: &mut std::collections::BTreeSet<u64>,
    ) {
        let blocked = self.busy_file
            || self.pending_load.is_some()
            || self.settings.is_some()
            || self.font_dialog;
        let mut add = false;
        let mut remove = false;
        egui::SidePanel::left("viewport_list")
            .default_width(230.)
            .show(ctx, |ui| {
                if blocked {
                    ui.disable();
                }
                ui.heading("Output viewports");
                if ui.button("Identify Displays · 3 seconds").clicked(){self.destinations.identify=true;}
                if ui.button("Refresh displays").clicked(){self.destinations.refresh=true;}
                ui.small("Fullscreen on the operator display covers the controls. Press ⌘⇧Esc to disable all outputs.");
                add = ui.button("Add Viewport").clicked();
                remove = ui
                    .add_enabled(
                        self.viewport_selected.is_some(),
                        egui::Button::new("Remove Selected"),
                    )
                    .clicked();
                ui.label(format!(
                    "{} / {} logical viewports",
                    self.doc.viewports.len(),
                    rastera_core::mapping::MAX_VIEWPORTS
                ));
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for v in self.doc.viewports.clone() {
                        ui.push_id(v.id, |ui| {
                            ui.horizontal(|ui| {
                                ui.colored_label(color(v.color), "■");
                                if ui
                                    .selectable_label(
                                        self.viewport_selected == Some(v.id),
                                        format!("{} · {}", v.name, v.id),
                                    )
                                    .clicked()
                                {
                                    self.select_viewport(v.id);
                                }
                            });
                            self.destinations.ui(ui,v.id,outputs);
                            ui.separator();
                        });
                    }
                });
                ui.label("Drag names/outlines. Option-click cycles overlaps, then drag normally.");
                ui.label(
                    "Outside the canvas samples black. Windows are separate from definitions.",
                );
            });
        if add {
            self.add_viewport();
        }
        if remove {
            if let Some(id) = self.viewport_selected {
                self.remove_viewport(id);
            }
        }
        let before = self.doc.clone();
        let mut ended = false;
        let mut apply = None;
        egui::SidePanel::right("viewport_inspector").default_width(280.).show(ctx,|ui|{
            if blocked{ui.disable();}
            ui.heading("Viewport inspector");
            egui::ScrollArea::vertical().show(ui,|ui|{
                if let Some(mut v)=self.viewport_selected.and_then(|id|self.doc.viewport(id)).cloned(){
                    ui.colored_label(color(v.color),format!("■ {} · ID {}",v.name,v.id));
                    ended|=ui.add(egui::TextEdit::singleline(&mut v.name).char_limit(32)).lost_focus();
                    if self.viewport_size.as_ref().is_none_or(|(id,original,_)|*id!=v.id||*original!=v.size){self.viewport_size=Some((v.id,v.size,v.size));}
                    let (_,_,draft)=self.viewport_size.as_mut().unwrap();
                    ui.label("Output resolution · pixels");
                    ui.small("Apply updates the clean window size and canvas footprint, preserving footprint scale.");
                    egui::ComboBox::from_id_salt("output_presets").selected_text("Preset / custom proportions").show_ui(ui,|ui|{for(name,size)in rastera_core::mapping::VIEWPORT_PRESETS{if ui.selectable_label(*draft==size,format!("{name} · {} × {}",size[0],size[1])).clicked(){*draft=size;}}});
                    for(label,n)in ["Width (px)","Height (px)"].into_iter().zip(draft.iter_mut()){ui.horizontal(|ui|{ui.label(label);ui.add(egui::DragValue::new(n).range(1..=8192));});}
                    ui.label(format!("Applied: {} × {} · {:.4}:1",v.size[0],v.size[1],v.size[0] as f64/v.size[1] as f64));
                    let valid=rastera_core::mapping::validate_output(*draft);
                    if let Err(error)=&valid{ui.colored_label(color(t::ELECTRIC_YELLOW),error);}
                    if ui.add_enabled(valid.is_ok()&&*draft!=v.size,egui::Button::new("Apply dimensions")).clicked(){apply=Some((v.id,*draft));}
                    ui.separator();
                    for(label,n)in ["Center X (canvas px)","Center Y (canvas px)"].into_iter().zip(v.center.iter_mut()){ui.label(label);let r=ui.add(egui::DragValue::new(n).range(-65536.0..=65536.0));ended|=r.drag_stopped()||r.lost_focus();}
                    ui.label("Canvas footprint scale · 100% = 1:1 canvas pixels");
                    let mut scale=100.0/v.zoom;
                    let r=ui.add(egui::DragValue::new(&mut scale).range((100.0/8192.0)..=(100.0/rastera_core::mapping::MIN_VIEWPORT_ZOOM)).speed(0.5).suffix("%"));
                    if r.changed(){v.zoom=100.0/scale;}
                    ended|=r.drag_stopped()||r.lost_focus();
                    ui.label(format!("Canvas footprint: {:.1} × {:.1} px",v.size[0] as f32/v.zoom,v.size[1] as f32/v.zoom));
                    ui.small("Drag footprint corners to scale the canvas selection; output resolution stays fixed.");
                    ui.label("Clockwise rotation");let r=ui.add(egui::DragValue::new(&mut v.rotation).range(-36000.0..=36000.).speed(0.25).suffix("°"));ended|=r.drag_stopped()||r.lost_focus();
                    ui.label("Identity color · RGBA");ui.horizontal(|ui|{for c in &mut v.color{let r=ui.add(egui::DragValue::new(c).range(0..=255));ended|=r.drag_stopped()||r.lost_focus();}});
                    ui.horizontal(|ui|{
                        if ui.button("Fit Canvas").on_hover_text("Contain the whole canvas; center and rotation reset").clicked(){v.fit_canvas(self.doc.canvas);ended=true;}
                        if ui.button("1:1 canvas pixels").on_hover_text("One logical output pixel samples one canvas pixel. Preserve center and rotation.").clicked(){v.zoom=1.;ended=true;}
                    });
                    let id=v.id;*self.doc.viewport_mut(id).unwrap()=v;
                }else{ui.label("Select a viewport or add one. Zero viewports is supported.");}
            });
        });
        if self.doc != before {
            if self.inspector_before.is_none() {
                self.inspector_before = Some(before);
            }
            self.publish();
        }
        if ended {
            self.finish_inspector();
        }
        if let Some((id, size)) = apply {
            if let Err(error) = self.apply_viewport_size(id, size) {
                self.error = Some(error);
            }
        }
    }
    pub(super) fn dialog_ui(&mut self, ctx: &egui::Context) {
        if let Some(mut size) = self.settings {
            let mut apply = false;
            let mut cancel = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("canvas_settings"),
                egui::ViewportBuilder::default()
                    .with_title("Rastera — Canvas settings")
                    .with_inner_size([460., 320.])
                    .with_active(self.probe.is_none())
                    .with_window_level(egui::WindowLevel::AlwaysOnTop),
                |ctx, _| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.heading("Canvas settings");
                        ui.label("Layers and viewport centers keep their canvas-pixel coordinates.");
                        egui::ComboBox::from_label("Presets")
                            .selected_text("Choose preset / custom")
                            .show_ui(ui, |ui| {
                                for (label, value) in [
                                    ("Dual HD", [3840, 1080]),
                                    ("HD", [1920, 1080]),
                                    ("UHD", [3840, 2160]),
                                    ("Portrait HD", [1080, 1920]),
                                    ("Square", [2048, 2048]),
                                ] {
                                    if ui.selectable_label(size == value, label).clicked() {
                                        size = value;
                                    }
                                }
                            });
                        for (label, n) in ["Width (px)", "Height (px)"].into_iter().zip(size.iter_mut()) {
                            ui.horizontal(|ui| {
                                ui.label(label);
                                ui.add(egui::DragValue::new(n).range(1..=8192));
                            });
                        }
                        let result = rastera_core::mapping::validate_canvas(size, self.device_limit);
                        if let Err(error) = &result {
                            ui.colored_label(color(t::ELECTRIC_YELLOW), error);
                        }
                        ui.label("RGBA texels: four textures ≤256 MiB; ≤2 generations / 512 MiB. Assets, drawables and driver overhead are additional.");
                        ui.horizontal(|ui| {
                            apply = ui.add_enabled(result.is_ok(), egui::Button::new("Apply")).clicked();
                            cancel = ui.button("Cancel").clicked();
                        });
                    });
                    cancel |= ctx.input(|i| i.viewport().close_requested());
                    if apply || cancel { ctx.send_viewport_cmd(egui::ViewportCommand::Close); }
                },
            );
            self.settings = if apply || cancel { None } else { Some(size) };
            if apply {
                self.finish_gesture();
                self.finish_view_gesture();
                self.finish_inspector();
                let mut candidate = self.doc.clone();
                candidate.canvas = size;
                self.stage_document(candidate, None);
            }
        }
        if self.font_dialog {
            let mut chosen = None;
            let mut close = false;
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("font_picker"),
                egui::ViewportBuilder::default()
                    .with_title("Rastera — Fonts")
                    .with_inner_size([560., 480.])
                    .with_active(self.probe.is_none())
                    .with_window_level(egui::WindowLevel::AlwaysOnTop),
                |ctx, _| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.heading("Font family and face");
                        ui.text_edit_singleline(&mut self.font_search);
                        if ui.button("Bundled Ubuntu Light + fallback fonts").clicked() {
                            chosen = Some(None);
                        }
                        if !self.library.ready.load(Ordering::Acquire) {
                            ui.label("Enumerating installed Mac fonts…");
                        }
                        let faces = self.library.faces.lock().unwrap();
                        egui::ScrollArea::vertical()
                            .max_height(340.)
                            .show(ui, |ui| {
                                let query = self.font_search.to_lowercase();
                                for f in faces.iter().filter(|f| {
                                    format!("{} {}", f.family, f.style)
                                        .to_lowercase()
                                        .contains(&query)
                                }) {
                                    if ui.button(format!("{} — {}", f.family, f.style)).clicked()
                                    {
                                        chosen = Some(Some(rastera_core::fonts::FontRef {
                                            family: f.family.clone(),
                                            style: f.style.clone(),
                                            postscript: f.postscript.clone(),
                                        }));
                                    }
                                }
                            });
                        close = ui.button("Cancel").clicked();
                    });
                    close |= ctx.input(|i| i.viewport().close_requested());
                    if chosen.is_some() || close {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                },
            );
            if let Some(font) = chosen {
                self.finish_inspector();
                let before = self.doc.clone();
                if let Some(Content::Text(id)) = self
                    .selected
                    .and_then(|id| self.doc.layer(id))
                    .map(|l| l.content.clone())
                {
                    self.doc.texts.get_mut(&id).unwrap().font = font;
                    self.history.record(before, &self.doc);
                    self.publish();
                }
                self.font_dialog = false;
            }
            if close {
                self.font_dialog = false;
            }
        }
    }
    pub fn settings_menu(&mut self, preview: &Consumer) {
        unsafe {
            if rastera_settings_menu(preview.handle.as_ptr()) == 1 {
                self.finish_gesture();
                self.finish_view_gesture();
                self.finish_inspector();
                self.settings = Some(self.doc.canvas);
            }
        }
    }
}
