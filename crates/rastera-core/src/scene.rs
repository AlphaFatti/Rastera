//! Versioned reusable document objects, geometry and transactional history. No UI/GPU caches.
use crate::{fit, HEIGHT, WIDTH};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
pub type Id = u64;
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Rect {
    pub fn contains(self, p: [f32; 2]) -> bool {
        p[0] >= self.x
            && p[0] < self.x + self.width
            && p[1] >= self.y
            && p[1] < self.y + self.height
    }
    pub fn corners(self) -> [[f32; 2]; 4] {
        [
            [self.x, self.y],
            [self.x + self.width, self.y],
            [self.x, self.y + self.height],
            [self.x + self.width, self.y + self.height],
        ]
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageAsset {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Alignment {
    Left,
    Center,
    Right,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextObject {
    #[serde(default)]
    pub font: Option<crate::fonts::FontRef>,
    pub text: String,
    pub font_size: f32,
    pub color: [u8; 4],
    pub alignment: Alignment,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "source")]
pub enum Content {
    Image(Id),
    Text(Id),
    Capture(Id),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub id: Id,
    pub name: String,
    pub rect: Rect,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
    pub aspect_lock: bool,
    pub content: Content,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub version: u32,
    pub project_id: String,
    pub canvas: [u32; 2],
    pub viewports: Vec<crate::mapping::Viewport>,
    pub next_viewport_id: u64,
    pub next_id: Id,
    pub test_background: bool,
    pub images: BTreeMap<Id, ImageAsset>,
    pub texts: BTreeMap<Id, TextObject>,
    pub captures: BTreeMap<Id, crate::capture::Source>,
    /// Bottom to top. Layer IDs remain stable through edits/reordering/save/load.
    pub layers: Vec<Layer>,
}
/// Portable document identity; destinations themselves are machine-local, never in this model.
fn new_project_id() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("OS entropy required to allocate a unique project identity");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
impl Default for Document {
    fn default() -> Self {
        Self {
            version: 5,
            project_id: new_project_id(),
            canvas: [WIDTH, HEIGHT],
            viewports: crate::mapping::defaults(),
            next_viewport_id: 3,
            next_id: 1,
            test_background: false,
            images: BTreeMap::new(),
            texts: BTreeMap::new(),
            captures: BTreeMap::new(),
            layers: vec![],
        }
    }
}
impl Document {
    pub fn viewport(&self, id: u64) -> Option<&crate::mapping::Viewport> {
        self.viewports.iter().find(|v| v.id == id)
    }
    pub fn viewport_mut(&mut self, id: u64) -> Option<&mut crate::mapping::Viewport> {
        self.viewports.iter_mut().find(|v| v.id == id)
    }
    pub fn add_viewport(&mut self) -> Result<u64, String> {
        if self.viewports.len() >= crate::mapping::MAX_VIEWPORTS {
            return Err(format!(
                "Viewport limit reached (maximum {})",
                crate::mapping::MAX_VIEWPORTS
            ));
        }
        let id = self.next_viewport_id;
        self.next_viewport_id = id
            .checked_add(1)
            .filter(|&n| n < u64::MAX - 1024)
            .ok_or("Viewport ID allocator exhausted")?;
        let palette = [
            crate::theme::SIGNAL_MINT,
            crate::theme::ELECTRIC_YELLOW,
            crate::theme::HOT_MAGENTA,
            crate::theme::RETRO_LAVENDER,
            [80, 190, 255, 255],
            [255, 160, 70, 255],
            [170, 240, 210, 255],
            [240, 160, 220, 255],
        ];
        let mut name = format!("Viewport {id}");
        while self.viewports.iter().any(|v| v.name == name) {
            name.push('+');
        }
        let v = crate::mapping::Viewport {
            id,
            size: [1920, 1080],
            name,
            color: palette
                .into_iter()
                .find(|c| !self.viewports.iter().any(|v| v.color == *c))
                .unwrap_or(palette[((id - 1) % 8) as usize]),
            center: [self.canvas[0] as f32 / 2., self.canvas[1] as f32 / 2.],
            zoom: 1.,
            rotation: 0.,
        };
        self.viewports.push(v);
        Ok(id)
    }
    pub fn remove_viewport(&mut self, id: u64) -> bool {
        let n = self.viewports.len();
        self.viewports.retain(|v| v.id != id);
        n != self.viewports.len()
    }
    fn id(&mut self) -> Id {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    pub fn layer(&self, id: Id) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id)
    }
    pub fn layer_mut(&mut self, id: Id) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id)
    }
    pub fn add_text(&mut self) -> Id {
        let source = self.id();
        self.texts.insert(
            source,
            TextObject {
                font: None,
                text: "Editable text\nLive on the canvas".into(),
                font_size: 80.,
                color: [255; 4],
                alignment: Alignment::Left,
            },
        );
        self.add_layer(
            "Text".into(),
            Content::Text(source),
            Rect {
                x: self.canvas[0] as f32 * 0.3,
                y: self.canvas[1] as f32 * 0.25,
                width: (self.canvas[0] as f32 * 5. / 12.).max(1.),
                height: (self.canvas[1] as f32 * 10. / 27.).max(1.),
            },
            false,
        )
    }
    pub fn add_image(&mut self, asset: ImageAsset) -> Id {
        let rect = initial_image_rect_on([asset.width, asset.height], self.canvas);
        let name = asset
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let source = self.id();
        self.images.insert(source, asset);
        self.add_layer(name, Content::Image(source), rect, true)
    }
    pub fn add_capture(&mut self, source: crate::capture::Source) -> Result<Id, String> {
        source.validate()?;
        if self.captures.len() >= crate::capture::MAX_SOURCES {
            return Err("Capture source limit reached (two distinct definitions)".into());
        }
        let rect = initial_image_rect_on([source.mode.width, source.mode.height], self.canvas);
        let name = source.name.clone();
        let id = self.id();
        self.captures.insert(id, source);
        Ok(self.add_layer(name, Content::Capture(id), rect, true))
    }
    pub fn assign_capture(&mut self, id: Id, source: crate::capture::Source) -> Result<(), String> {
        source.validate()?;
        let aspect = source.mode.width as f32 / source.mode.height as f32;
        *self
            .captures
            .get_mut(&id)
            .ok_or("Capture definition no longer exists")? = source;
        for layer in &mut self.layers {
            if !layer.locked
                && layer.aspect_lock
                && matches!(layer.content,Content::Capture(source) if source==id)
            {
                let center = [
                    layer.rect.x + layer.rect.width / 2.,
                    layer.rect.y + layer.rect.height / 2.,
                ];
                layer.rect.width = layer.rect.width.min(layer.rect.height * aspect);
                layer.rect.height = layer.rect.width / aspect;
                layer.rect.x = center[0] - layer.rect.width / 2.;
                layer.rect.y = center[1] - layer.rect.height / 2.;
            }
        }
        Ok(())
    }
    fn add_layer(&mut self, name: String, content: Content, rect: Rect, aspect_lock: bool) -> Id {
        let id = self.id();
        self.layers.push(Layer {
            id,
            name,
            content,
            rect,
            visible: true,
            locked: false,
            opacity: 1.,
            aspect_lock,
        });
        id
    }
    pub fn duplicate(&mut self, id: Id) -> Option<Id> {
        let mut layer = self.layer(id)?.clone();
        if layer.locked {
            return None;
        }
        if let Content::Text(source) = layer.content {
            let text = self.texts.get(&source)?.clone();
            let new = self.id();
            self.texts.insert(new, text);
            layer.content = Content::Text(new);
        }
        layer.id = self.id();
        layer.name.push_str(" copy");
        layer.rect.x += 24.;
        layer.rect.y += 24.;
        let new = layer.id;
        let index = self.layers.iter().position(|l| l.id == id)?;
        self.layers.insert(index + 1, layer);
        Some(new)
    }
    pub fn prune_unused(&mut self) {
        let images: BTreeSet<_> = self
            .layers
            .iter()
            .filter_map(|l| {
                if let Content::Image(id) = l.content {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();
        let texts: BTreeSet<_> = self
            .layers
            .iter()
            .filter_map(|l| {
                if let Content::Text(id) = l.content {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();
        self.images.retain(|id, _| images.contains(id));
        self.texts.retain(|id, _| texts.contains(id));
        let captures: BTreeSet<_> = self
            .layers
            .iter()
            .filter_map(|l| {
                if let Content::Capture(id) = l.content {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();
        self.captures.retain(|id, _| captures.contains(id));
    }
    pub fn aspect(&self, id: Id) -> Option<f32> {
        let layer = self.layer(id)?;
        if !layer.aspect_lock {
            return None;
        }
        if let Content::Image(source) = layer.content {
            let asset = self.images.get(&source)?;
            Some(asset.width as f32 / asset.height as f32)
        } else if let Content::Capture(source) = layer.content {
            let c = self.captures.get(&source)?;
            Some(c.mode.width as f32 / c.mode.height as f32)
        } else {
            None
        }
    }
    pub fn hit_test(&self, p: [f32; 2]) -> Option<Id> {
        self.layers
            .iter()
            .rev()
            .find(|l| l.visible && !l.locked && l.rect.contains(p))
            .map(|l| l.id)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 5 {
            return Err("Unsupported project version (expected v5)".into());
        }
        if self.project_id.len() != 32 || !self.project_id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid project identity".into());
        }
        crate::mapping::validate_canvas(self.canvas, 8192)?;
        let mut viewport_ids = BTreeSet::new();
        if self.viewports.len() > crate::mapping::MAX_VIEWPORTS
            || self
                .viewports
                .iter()
                .any(|v| !v.validate() || !viewport_ids.insert(v.id))
            || self.next_viewport_id <= viewport_ids.last().copied().unwrap_or(0)
            || self.next_viewport_id > u64::MAX - 1024
        {
            return Err(
                "Invalid viewport identity, dimensions, transform, allocator or count (maximum 8)"
                    .into(),
            );
        }
        if self.layers.len() > 128 || self.images.len() > 256 || self.texts.len() > 256 {
            return Err("Project object limit exceeded".into());
        }
        let mut ids = BTreeSet::new();
        let mut pixels = 0u64;
        for (&id, a) in &self.images {
            if !ids.insert(id)
                || id == 0
                || a.width == 0
                || a.height == 0
                || a.width > 8192
                || a.height > 8192
            {
                return Err("Invalid image asset".into());
            }
        }
        for (&id, t) in &self.texts {
            if !ids.insert(id)
                || id == 0
                || !t.font_size.is_finite()
                || !(4.0..=512.).contains(&t.font_size)
                || t.text.len() > 16384
                || t.font.as_ref().is_some_and(|f| {
                    f.postscript.is_empty()
                        || f.postscript.len() > 256
                        || f.family.len() > 256
                        || f.style.len() > 256
                })
            {
                return Err("Invalid text object (font 4–512 px, text ≤16384 bytes)".into());
            }
        }
        if self.captures.len() > crate::capture::MAX_SOURCES {
            return Err("Capture limit: two source definitions".into());
        }
        for (&id, capture) in &self.captures {
            if id == 0 || !ids.insert(id) {
                return Err("Invalid capture source ID".into());
            }
            capture.validate()?;
        }
        let mut used_images = BTreeSet::new();
        for l in &self.layers {
            let r = l.rect;
            if !ids.insert(l.id)
                || l.id == 0
                || l.name.len() > 512
                || [r.x, r.y, r.width, r.height, l.opacity]
                    .iter()
                    .any(|v| !v.is_finite())
                || r.x.abs() > 32768.
                || r.y.abs() > 32768.
                || r.width <= 0.
                || r.height <= 0.
                || r.width > 8192.
                || r.height > 8192.
                || !(0.0..=1.).contains(&l.opacity)
            {
                return Err("Invalid layer geometry, ID, name or opacity".into());
            }
            match l.content {
                Content::Image(id) => {
                    let a = self
                        .images
                        .get(&id)
                        .ok_or("Missing image object reference")?;
                    if used_images.insert(id) {
                        pixels += a.width as u64 * a.height as u64;
                    }
                }
                Content::Capture(id) => {
                    if !self.captures.contains_key(&id) {
                        return Err("Missing capture source reference".into());
                    }
                }
                Content::Text(id) => {
                    if !self.texts.contains_key(&id) {
                        return Err("Missing text object reference".into());
                    }
                    pixels += r.width.ceil() as u64 * r.height.ceil() as u64;
                }
            }
        }
        if self.next_id <= ids.last().copied().unwrap_or(0) || self.next_id > u64::MAX - 1024 {
            return Err("Invalid ID allocator".into());
        }
        if pixels > 32 * 1024 * 1024 {
            return Err("Prepared image/text budget exceeded (32 million pixels)".into());
        }
        Ok(())
    }
}
pub fn initial_image_rect(size: [u32; 2]) -> Rect {
    initial_image_rect_on(size, [WIDTH, HEIGHT])
}
pub fn initial_image_rect_on(size: [u32; 2], canvas: [u32; 2]) -> Rect {
    let scale = (canvas[0] as f32 / size[0] as f32)
        .min(canvas[1] as f32 / size[1] as f32)
        .min(1.);
    let w = size[0] as f32 * scale;
    let h = size[1] as f32 * scale;
    Rect {
        x: (canvas[0] as f32 - w) / 2.,
        y: (canvas[1] as f32 - h) / 2.,
        width: w,
        height: h,
    }
}
/// Physical preview pixels (including its letterboxing), top-left origin.
#[derive(Clone, Copy, Debug)]
pub struct PreviewMap {
    pub fit: [f32; 4],
    pub canvas: [u32; 2],
}
impl PreviewMap {
    pub fn new(size: [f32; 2]) -> Self {
        Self::for_canvas([WIDTH, HEIGHT], size, 0.)
    }
    pub fn for_canvas(canvas: [u32; 2], size: [f32; 2], margin: f32) -> Self {
        let mut f = fit(
            canvas,
            [
                (size[0] - 2. * margin).max(0.),
                (size[1] - 2. * margin).max(0.),
            ],
        );
        f[0] += margin;
        f[1] += margin;
        Self { fit: f, canvas }
    }
    pub fn canvas(self, p: [f32; 2], allow_outside: bool) -> Option<[f32; 2]> {
        let [x, y, w, h] = self.fit;
        if w <= 0. || h <= 0. {
            return None;
        }
        if !allow_outside && (p[0] < x || p[1] < y || p[0] >= x + w || p[1] >= y + h) {
            return None;
        }
        Some([
            (p[0] - x) / w * self.canvas[0] as f32,
            (p[1] - y) / h * self.canvas[1] as f32,
        ])
    }
    pub fn handle(self, rect: Rect, p: [f32; 2], radius: f32) -> Option<usize> {
        rect.corners()
            .iter()
            .position(|q| (p[0] - q[0]).abs() <= radius && (p[1] - q[1]).abs() <= radius)
    }
}
pub fn resize_corner(original: Rect, corner: usize, p: [f32; 2], aspect: Option<f32>) -> Rect {
    let anchor = original.corners()[3 - corner];
    let left = corner.is_multiple_of(2);
    let top = corner < 2;
    let requested_w = (p[0] - anchor[0]) * if left { -1. } else { 1. };
    let requested_h = (p[1] - anchor[1]) * if top { -1. } else { 1. };
    let (w, h) = if let Some(r) = aspect.filter(|r| r.is_finite() && *r > 0.) {
        // Bound the longest side, preserving even very thin imported images.
        let h = (requested_w / r)
            .max(requested_h)
            .clamp(1. / r.max(1.), 8192. / r.max(1.));
        (h * r, h)
    } else {
        (requested_w.clamp(1., 8192.), requested_h.clamp(1., 8192.))
    };
    Rect {
        x: if left { anchor[0] - w } else { anchor[0] },
        y: if top { anchor[1] - h } else { anchor[1] },
        width: w,
        height: h,
    }
}
/// Resize from a handle grab, preserving its offset inside the hit target.
pub fn resize_drag(
    original: Rect,
    corner: usize,
    grab: [f32; 2],
    pointer: [f32; 2],
    aspect: Option<f32>,
) -> Rect {
    if pointer == grab {
        return original;
    }
    let c = original.corners()[corner];
    resize_corner(
        original,
        corner,
        [c[0] + (pointer[0] - grab[0]), c[1] + (pointer[1] - grab[1])],
        aspect,
    )
}
#[derive(Default, Clone, PartialEq)]
pub struct History {
    undo: Vec<Document>,
    redo: Vec<Document>,
}
impl History {
    pub fn record(&mut self, before: Document, after: &Document) {
        if before != *after {
            if self.undo.len() == 100 {
                self.undo.remove(0);
            }
            self.undo.push(before);
            self.redo.clear();
        }
    }
    pub fn undo(&mut self, doc: &mut Document) -> bool {
        if let Some(mut prev) = self.undo.pop() {
            prev.next_id = prev.next_id.max(doc.next_id);
            prev.next_viewport_id = prev.next_viewport_id.max(doc.next_viewport_id);
            self.redo.push(std::mem::replace(doc, prev));
            true
        } else {
            false
        }
    }
    pub fn redo(&mut self, doc: &mut Document) -> bool {
        if let Some(mut next) = self.redo.pop() {
            next.next_id = next.next_id.max(doc.next_id);
            next.next_viewport_id = next.next_viewport_id.max(doc.next_viewport_id);
            self.undo.push(std::mem::replace(doc, next));
            true
        } else {
            false
        }
    }
}
pub fn inspect_image(path: &Path) -> Result<ImageAsset, String> {
    let reader = image::ImageReader::open(path)
        .map_err(|e| format!("Cannot open {}: {e}", path.display()))?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg)
    ) {
        return Err("Only PNG and JPEG are supported".into());
    }
    let (width, height) = reader
        .into_dimensions()
        .map_err(|e| format!("Cannot decode {}: {e}", path.display()))?;
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || width as u64 * height as u64 > 32 * 1024 * 1024
    {
        return Err("Image exceeds 8192 px / 32 million pixel limit".into());
    }
    Ok(ImageAsset {
        path: std::fs::canonicalize(path).map_err(|e| e.to_string())?,
        width,
        height,
    })
}
fn relative(path: &Path, base: &Path) -> PathBuf {
    let a: Vec<_> = path.components().collect();
    let b: Vec<_> = base.components().collect();
    let n = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    if n == 0 {
        return path.to_path_buf();
    }
    let mut result = PathBuf::new();
    for _ in n..b.len() {
        result.push("..");
    }
    for c in &a[n..] {
        result.push(c.as_os_str());
    }
    result
}
pub fn save_project(doc: &Document, path: &Path) -> Result<(), String> {
    use std::io::Write;
    doc.validate()?;
    let base = std::fs::canonicalize(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )
    .map_err(|e| e.to_string())?;
    let mut stored = doc.clone();
    for asset in stored.images.values_mut() {
        asset.path = relative(&asset.path, &base);
    }
    let bytes = serde_json::to_vec_pretty(&stored).map_err(|e| e.to_string())?;
    let temporary = path.with_extension(format!("json.tmp-{}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| format!("Cannot create save file: {e}"))?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })()
    .map_err(|e: std::io::Error| e.to_string());
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}
pub fn load_project(path: &Path) -> Result<Document, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > 8 * 1024 * 1024 {
        return Err("Project exceeds 8 MiB".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid project: {e}"))?;
    match value.get("version").and_then(|v| v.as_u64()) {
        Some(1) => {
            if value.get("canvas") != Some(&serde_json::json!([3840, 1080])) {
                return Err("Version 1 requires original canvas dimensions".into());
            }
            value["version"] = 2.into();
            value["viewports"] = serde_json::to_value(crate::mapping::defaults()).unwrap();
        }
        Some(2) => {}
        Some(3) | Some(4) | Some(5) => {}
        _ => return Err("Unsupported project schema version".into()),
    }
    if value["version"].as_u64().is_some_and(|v| v < 3) {
        let viewports = value["viewports"]
            .as_array_mut()
            .ok_or("Missing legacy viewport list")?;
        let mut high = 0;
        for v in viewports {
            let v = v.as_object_mut().ok_or("Invalid legacy viewport object")?;
            v.insert("size".into(), serde_json::json!([1920, 1080]));
            high = high.max(
                v.get("id")
                    .and_then(serde_json::Value::as_u64)
                    .ok_or("Invalid legacy viewport ID")?,
            );
        }
        value["next_viewport_id"] = high
            .checked_add(1)
            .ok_or("Viewport ID allocator overflow")?
            .into();
        value["version"] = 3.into();
    }
    if value["version"].as_u64().is_some_and(|v| v < 4) {
        if value.get("project_id").is_none() {
            value["project_id"] = new_project_id().into();
        }
        value["version"] = 4.into();
    }
    if value["version"].as_u64() != Some(5) {
        value["captures"] = serde_json::json!({});
        value["version"] = 5.into();
    }
    let mut doc: Document = serde_json::from_value(value).map_err(|e| e.to_string())?;
    doc.validate()?;
    let base = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    for asset in doc.images.values_mut() {
        let resolved = if asset.path.is_absolute() {
            asset.path.clone()
        } else {
            base.join(&asset.path)
        };
        let checked = inspect_image(&resolved)?;
        if checked.width != asset.width || checked.height != asset.height {
            return Err(format!(
                "Image dimensions changed: {}. Reimport it.",
                resolved.display()
            ));
        }
        asset.path = checked.path;
    }
    Ok(doc)
}

pub fn inspect_decoded_image(path: &Path) -> Result<ImageAsset, String> {
    let asset = inspect_image(path)?;
    let _ = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .decode()
        .map_err(|e| format!("Cannot decode {}: {e}", path.display()))?;
    Ok(asset)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_mapping_letterboxes_retina_and_window_resize() {
        let a = PreviewMap::new([1000., 500.]);
        let b = PreviewMap::new([2000., 1000.]);
        assert!(a.canvas([500., 10.], false).is_none());
        assert_eq!(a.canvas([500., 250.], false), Some([1920., 540.]));
        assert_eq!(
            a.canvas([500., 250.], false),
            b.canvas([1000., 500.], false)
        );
        let c = PreviewMap::new([1800., 300.]);
        assert_eq!(c.canvas([900., 150.], false), Some([1920., 540.]));
        assert!(a.canvas([0., 0.], true).unwrap()[1] < 0.);
    }
    #[test]
    fn ordered_hit_testing_visibility_locking_and_off_canvas() {
        let mut doc = Document::default();
        let low = doc.add_text();
        let high = doc.add_text();
        for l in &mut doc.layers {
            l.rect = Rect {
                x: -100.,
                y: 0.,
                width: 200.,
                height: 200.,
            };
        }
        assert_eq!(doc.hit_test([20., 20.]), Some(high));
        doc.layer_mut(high).unwrap().locked = true;
        assert_eq!(doc.hit_test([20., 20.]), Some(low));
        doc.layer_mut(low).unwrap().visible = false;
        assert_eq!(doc.hit_test([20., 20.]), None);
        assert_eq!(doc.hit_test([100., 20.]), None); // Half-open source box.
    }
    #[test]
    fn resize_keeps_opposite_corner_and_image_ratio() {
        let r = Rect {
            x: 100.,
            y: 200.,
            width: 400.,
            height: 200.,
        };
        let a = resize_corner(r, 0, [-100., -100.], Some(2.));
        assert_eq!(a.x + a.width, r.x + r.width);
        assert_eq!(a.y + a.height, r.y + r.height);
        assert_eq!(a.width / a.height, 2.);
        let b = resize_corner(r, 3, [900., 350.], None);
        assert_eq!(b.x, r.x);
        assert_eq!(b.height, 150.);
        let fitted = initial_image_rect([8000, 4000]);
        assert!(fitted.width <= 3840. && fitted.height <= 1080.);
        assert_eq!(fitted.width / fitted.height, 2.);
    }
    #[test]
    fn one_gesture_one_history_entry_and_duplicate_text_is_independent() {
        let mut doc = Document::default();
        let id = doc.add_text();
        let original = doc.clone();
        let mut history = History::default();
        for _ in 0..100 {
            doc.layer_mut(id).unwrap().rect.x += 1.;
        }
        history.record(original.clone(), &doc);
        let moved = doc.clone();
        assert!(history.undo(&mut doc));
        assert_eq!(doc.layers, original.layers);
        assert!(!history.undo(&mut doc));
        assert!(history.redo(&mut doc));
        assert_eq!(doc.layers, moved.layers);
        let copy = doc.duplicate(id).unwrap();
        let Content::Text(a) = doc.layer(id).unwrap().content else {
            panic!()
        };
        let Content::Text(b) = doc.layer(copy).unwrap().content else {
            panic!()
        };
        assert_ne!(a, b);
        doc.texts.get_mut(&b).unwrap().text = "copy only".into();
        assert_ne!(doc.texts[&a].text, doc.texts[&b].text);
        let allocated = doc.next_id;
        history.record(moved, &doc);
        history.undo(&mut doc);
        assert!(doc.next_id >= allocated);
        assert!(doc.add_text() >= allocated);
    }
    #[test]
    fn resize_preserves_handle_grab_offset_and_thin_image_ratio() {
        let r = Rect {
            x: 100.,
            y: 200.,
            width: 400.,
            height: 200.,
        };
        let fractional = Rect {
            x: 1750.,
            y: 240.,
            width: 1657.5001,
            height: 828.75006,
        };
        assert_eq!(
            resize_drag(
                fractional,
                3,
                [3407.5, 1068.75],
                [3407.5, 1068.75],
                Some(2.)
            ),
            fractional
        );
        let next = resize_drag(r, 3, [498., 398.], [598., 448.], Some(2.));
        assert_eq!(
            next,
            Rect {
                x: 100.,
                y: 200.,
                width: 500.,
                height: 250.
            }
        );
        let thin = initial_image_rect([8192, 1]);
        let mut corner = thin.corners()[3];
        corner[0] += 100.;
        let resized = resize_corner(thin, 3, corner, Some(8192.));
        assert!((resized.width - (thin.width + 100.)).abs() < 0.01);
        assert_eq!(resized.width / resized.height, 8192.);
    }
    #[test]
    fn thin_images_fit_and_text_box_resize_preserves_editable_style() {
        let mut doc = Document::default();
        doc.add_image(ImageAsset {
            path: "/tmp/thin.png".into(),
            width: 8192,
            height: 1,
        });
        assert!(doc.layers[0].rect.height < 1.);
        assert!(doc.validate().is_ok());
        let id = doc.add_text();
        let Content::Text(source) = doc.layer(id).unwrap().content else {
            panic!()
        };
        let style = doc.texts[&source].clone();
        let r = doc.layer(id).unwrap().rect;
        for corner in 0..4 {
            let next = resize_corner(r, corner, [900., 700.], None);
            assert_eq!(next.corners()[3 - corner], r.corners()[3 - corner]);
        }
        doc.layer_mut(id).unwrap().rect = resize_corner(r, 3, [1700., 900.], None);
        assert_eq!(doc.texts[&source], style);
    }
    #[test]
    fn png_alpha_jpeg_import_and_missing_reference_validation() {
        let base = std::env::temp_dir().join(format!("rastera-formats-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let png = base.join("alpha.png");
        let jpg = base.join("photo.jpg");
        image::RgbaImage::from_pixel(3, 2, image::Rgba([200, 50, 10, 127]))
            .save(&png)
            .unwrap();
        image::RgbImage::from_pixel(7, 5, image::Rgb([20, 40, 60]))
            .save(&jpg)
            .unwrap();
        assert_eq!(
            image::open(&png).unwrap().to_rgba8().get_pixel(1, 1)[3],
            127
        );
        let a = inspect_decoded_image(&png).unwrap();
        let b = inspect_decoded_image(&jpg).unwrap();
        assert_eq!([a.width, a.height], [3, 2]);
        assert_eq!([b.width, b.height], [7, 5]);
        let mut doc = Document::default();
        let id = doc.add_image(a);
        doc.layer_mut(id).unwrap().content = Content::Image(999);
        assert!(doc.validate().is_err());
        std::fs::write(base.join("broken.png"), b"not a PNG").unwrap();
        assert!(inspect_decoded_image(&base.join("broken.png")).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn custom_canvas_initial_placement_and_retina_margin_mapping() {
        for canvas in [[1079, 1919], [2559, 1439], [1, 1]] {
            let mut doc = Document {
                canvas,
                ..Document::default()
            };
            let image = doc.add_image(ImageAsset {
                path: "test.png".into(),
                width: 1200,
                height: 600,
            });
            let r = doc.layer(image).unwrap().rect;
            assert!(r.width <= canvas[0] as f32 && r.height <= canvas[1] as f32);
            assert!((r.x + r.width / 2. - canvas[0] as f32 / 2.).abs() < 0.001);
            doc.add_text();
            assert!(doc.validate().is_ok());
            let a = PreviewMap::for_canvas(canvas, [1000., 600.], 20.);
            let b = PreviewMap::for_canvas(canvas, [2000., 1200.], 40.);
            for k in 0..4 {
                assert!((2. * a.fit[k] - b.fit[k]).abs() < 0.001);
            }
            assert!(a.canvas([a.fit[0] - 1., a.fit[1]], false).is_none());
            let p = a
                .canvas([a.fit[0] + a.fit[2] / 2., a.fit[1] + a.fit[3] / 2.], false)
                .unwrap();
            assert!((p[0] - canvas[0] as f32 / 2.).abs() < 0.001);
        }
    }
    #[test]
    fn schema_migration_custom_dimensions_and_history() {
        let dir = std::env::temp_dir().join(format!("rastera-migrate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("show.json");
        let mut original = Document::default();
        let id = original.add_text();
        let mut legacy = serde_json::to_value(&original).unwrap();
        legacy["version"] = 1.into();
        legacy.as_object_mut().unwrap().remove("viewports");
        legacy.as_object_mut().unwrap().remove("next_viewport_id");
        for t in legacy["texts"].as_object_mut().unwrap().values_mut() {
            t.as_object_mut().unwrap().remove("font");
        }
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let migrated = load_project(&path).unwrap();
        assert_eq!(migrated, original);
        let rect = original.layer(id).unwrap().rect;
        let mut history = History::default();
        let before = original.clone();
        original.canvas = [1079, 1919];
        original.viewports[1].center = [800., 1234.];
        original.viewports[1].zoom = 2.5;
        original.viewports[1].rotation = 73.;
        original.viewports[1].name = "Portrait detail".into();
        original.viewports[1].color = [255, 0, 128, 255];
        let Content::Text(source) = original.layer(id).unwrap().content else {
            panic!()
        };
        original.texts.get_mut(&source).unwrap().font = Some(crate::fonts::FontRef {
            family: "Unavailable".into(),
            style: "Bold".into(),
            postscript: "Definitely-Missing-XYZ".into(),
        });
        history.record(before.clone(), &original);
        save_project(&original, &path).unwrap();
        assert_eq!(load_project(&path).unwrap(), original);
        assert_eq!(original.layer(id).unwrap().rect, rect);
        assert!(history.undo(&mut original));
        assert_eq!(original, before);
        assert!(history.redo(&mut original));
        assert_eq!(original.canvas, [1079, 1919]);
        assert_eq!(original.viewports[1].rotation, 73.);
        let map = PreviewMap::for_canvas([1079, 1919], [2000., 1000.], 40.);
        assert!(map.fit[0] >= 40. && map.fit[1] >= 40.);
        assert_eq!(map.canvas([1000., 500.], false), Some([539.5, 959.5]));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn project_roundtrip_relative_media_and_validation_failures() {
        let base = std::env::temp_dir().join(format!("rastera-roundtrip-{}", std::process::id()));
        std::fs::create_dir_all(base.join("projects")).unwrap();
        let media = base.join("alpha.png");
        image::RgbaImage::from_pixel(4, 2, image::Rgba([240, 60, 20, 128]))
            .save(&media)
            .unwrap();
        let mut doc = Document::default();
        doc.add_image(inspect_decoded_image(&media).unwrap());
        let text = doc.add_text();
        doc.layer_mut(text).unwrap().rect.x = -300.;
        doc.layer_mut(text).unwrap().opacity = 0.5;
        let path = base.join("projects/show.json");
        save_project(&doc, &path).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(saved.contains("../alpha.png"));
        assert_eq!(load_project(&path).unwrap(), doc);
        let bad = base.join("bad.json");
        std::fs::write(&bad, saved.replace("\"version\": 5", "\"version\": 99")).unwrap();
        assert!(load_project(&bad).is_err());
        std::fs::remove_file(&media).unwrap();
        assert!(load_project(&path).unwrap_err().contains("alpha.png"));
        let mut broken = doc.clone();
        broken.layers[0].rect.width = f32::NAN;
        assert!(broken.validate().is_err());
        std::fs::remove_dir_all(base).unwrap();
    }
}
