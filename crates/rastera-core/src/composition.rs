//! Background preparation and GPU composition of immutable, complete scene snapshots.
use crate::{
    renderer::PatternGenerator,
    scene::{Alignment, Content, Document, TextObject},
    HEIGHT, WIDTH,
};
use epaint::{
    text::{FontDefinitions, Fonts},
    Color32, FontId,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
    time::Duration,
};
use wgpu::util::DeviceExt;

struct GpuImage {
    view: wgpu::TextureView,
    _texture: wgpu::Texture,
}
struct Draw {
    bind: wgpu::BindGroup,
    _image: Arc<GpuImage>,
}
struct Prepared {
    revision: u64,
    background: bool,
    layers: Vec<Draw>,
}
struct Material {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}
struct Shared {
    request: Mutex<Option<(u64, Document)>>,
    wake: Condvar,
    ready: Mutex<Option<Arc<Prepared>>>,
    desired: AtomicU64,
    applied: AtomicU64,
    stop: AtomicBool,
    status: Mutex<Option<PreparationStatus>>,
}
#[derive(Debug)]
pub struct PreparationStatus {
    pub revision: u64,
    pub result: Result<(), String>,
}
pub struct SceneEngine {
    shared: Arc<Shared>,
    worker: Option<thread::JoinHandle<()>>,
    material: Arc<Material>,
}
pub struct SceneFeed {
    shared: Arc<Shared>,
    material: Arc<Material>,
    current: Option<Arc<Prepared>>,
}
impl SceneEngine {
    pub fn new(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let material = Arc::new(Material::new(&device));
        let shared = Arc::new(Shared {
            request: Mutex::new(None),
            wake: Condvar::new(),
            ready: Mutex::new(None),
            desired: AtomicU64::new(0),
            applied: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            status: Mutex::new(None),
        });
        let state = shared.clone();
        let gpu = material.clone();
        let worker = thread::spawn(move || {
            let fonts = Fonts::new(1., 8192, FontDefinitions::default());
            let mut cache = HashMap::new();
            // Keep retired snapshots until the producer releases them, so final
            // texture/bind-group destruction happens on this preparation thread.
            let mut retained: Vec<Arc<Prepared>> = vec![];
            loop {
                let request = {
                    let mut pending = state.request.lock().unwrap();
                    while pending.is_none() && !state.stop.load(Ordering::Relaxed) {
                        pending = state.wake.wait(pending).unwrap();
                    }
                    if state.stop.load(Ordering::Relaxed) {
                        break;
                    }
                    pending.take().unwrap()
                };
                // At most one uploaded ready snapshot plus one latest document request.
                while state.ready.lock().unwrap().is_some() && !state.stop.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(5));
                }
                if state.stop.load(Ordering::Relaxed) {
                    break;
                }
                retained.retain(|s| Arc::strong_count(s) > 1);
                let (revision, doc) = request;
                match prepare(&device, &queue, &gpu, &fonts, &doc, &mut cache) {
                    Ok(mut scene) => {
                        scene.revision = revision;
                        let scene = Arc::new(scene);
                        retained.push(scene.clone());
                        *state.ready.lock().unwrap() = Some(scene);
                        *state.status.lock().unwrap() = Some(PreparationStatus {
                            revision,
                            result: Ok(()),
                        });
                    }
                    Err(error) => {
                        cache.clear();
                        *state.status.lock().unwrap() = Some(PreparationStatus {
                            revision,
                            result: Err(error),
                        });
                    }
                }
            }
        });
        Self {
            shared,
            worker: Some(worker),
            material,
        }
    }
    pub fn feed(&self) -> SceneFeed {
        SceneFeed {
            shared: self.shared.clone(),
            material: self.material.clone(),
            current: None,
        }
    }
    pub fn submit(&self, doc: Document) -> u64 {
        let revision = self.shared.desired.fetch_add(1, Ordering::SeqCst) + 1;
        *self.shared.request.lock().unwrap() = Some((revision, doc));
        self.shared.wake.notify_one();
        revision
    }
    pub fn status(&self) -> Option<PreparationStatus> {
        self.shared.status.lock().unwrap().take()
    }
    pub fn applied(&self) -> u64 {
        self.shared.applied.load(Ordering::Relaxed)
    }
}
impl Drop for SceneEngine {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
impl SceneFeed {
    pub(crate) fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        seconds: f32,
        pattern: &PatternGenerator,
    ) -> wgpu::CommandEncoder {
        if let Ok(mut ready) = self.shared.ready.try_lock() {
            if let Some(scene) = ready.take() {
                // Show the most recent COMPLETE snapshot even if a newer document
                // is preparing. Discarding it whenever edits outrun 60 Hz starves
                // live dragging until mouse-up. One worker preserves revision order.
                self.current = Some(scene);
            }
        }
        let mut encoder = if self.current.as_ref().is_some_and(|s| s.background) {
            pattern.encode(device, queue, target, seconds)
        } else {
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("scene black background"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
            }
            encoder
        };
        if let Some(scene) = &self.current {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("ordered scene layers"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.material.pipeline);
            for draw in &scene.layers {
                pass.set_bind_group(0, &draw.bind, &[]);
                pass.draw(0..6, 0..1);
            }
        }
        encoder
    }
    pub(crate) fn frame_completed(&self) {
        if let Some(scene) = &self.current {
            self.shared.applied.store(scene.revision, Ordering::Relaxed);
        }
    }
}
impl Material {
    fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene layer layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader=device.create_shader_module(wgpu::ShaderModuleDescriptor {label:Some("scene compositor"),source:wgpu::ShaderSource::Wgsl(format!(r#"
struct Layer {{ rect:vec4f, properties:vec4f }}
@group(0) @binding(0) var<uniform> layer:Layer;
@group(0) @binding(1) var bitmap:texture_2d<f32>;
@group(0) @binding(2) var filtering:sampler;
struct Vertex {{@builtin(position) pos:vec4f,@location(0) uv:vec2f}}
@vertex fn vs(@builtin(vertex_index) i:u32)->Vertex {{
    var uv=array<vec2f,6>(vec2f(0,0),vec2f(1,0),vec2f(0,1),vec2f(0,1),vec2f(1,0),vec2f(1,1));
    let p=layer.rect.xy+uv[i]*layer.rect.zw;
    return Vertex(vec4f(p.x/{WIDTH}.0*2.0-1.0,1.0-p.y/{HEIGHT}.0*2.0,0,1),uv[i]*layer.properties.yz);
}}
@fragment fn fs(v:Vertex)->@location(0) vec4f {{let c=textureSample(bitmap,filtering,v.uv);return c*layer.properties.x;}}
"#).into())});
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("straight alpha scene composition"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("image/text filtering"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            sampler,
        }
    }
}
pub fn rasterize_text(
    text: &TextObject,
    size: [u32; 2],
    fonts: &Fonts,
) -> Result<image::RgbaImage, String> {
    if size[0] == 0 || size[1] == 0 || size[0] as u64 * size[1] as u64 > 32 * 1024 * 1024 {
        return Err("Text box too large".into());
    }
    fonts.begin_pass(1., 8192);
    let galley = fonts.layout(
        text.text.clone(),
        FontId::proportional(text.font_size),
        Color32::WHITE,
        size[0] as f32,
    );
    let atlas = fonts.image();
    let mut image = image::RgbaImage::new(size[0], size[1]);
    for row in &galley.rows {
        let factor = match text.alignment {
            Alignment::Left => 0.,
            Alignment::Center => 0.5,
            Alignment::Right => 1.,
        };
        let offset = (size[0] as f32 - row.rect.width()).max(0.) * factor - row.rect.left();
        for glyph in &row.glyphs {
            let uv = glyph.uv_rect;
            let x0 = (glyph.pos.x + uv.offset.x + offset).round() as i32;
            let y0 = (glyph.pos.y + uv.offset.y).round() as i32;
            for sy in uv.min[1]..uv.max[1] {
                for sx in uv.min[0]..uv.max[0] {
                    let x = x0 + (sx - uv.min[0]) as i32;
                    let y = y0 + (sy - uv.min[1]) as i32;
                    if x < 0 || y < 0 || x >= size[0] as i32 || y >= size[1] as i32 {
                        continue;
                    }
                    let alpha = atlas.pixels[sy as usize * atlas.size[0] + sx as usize].powf(0.55)
                        * text.color[3] as f32
                        / 255.;
                    let old = image.get_pixel(x as u32, y as u32)[3] as f32 / 255.;
                    image.put_pixel(
                        x as u32,
                        y as u32,
                        image::Rgba([
                            text.color[0],
                            text.color[1],
                            text.color[2],
                            ((alpha + old * (1. - alpha)) * 255.).round() as u8,
                        ]),
                    );
                }
            }
        }
    }
    let _ = fonts.font_image_delta();
    Ok(image)
}
fn prepare(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    material: &Material,
    fonts: &Fonts,
    doc: &Document,
    cache: &mut HashMap<String, Arc<GpuImage>>,
) -> Result<Prepared, String> {
    doc.validate()?;
    let mut layers = vec![];
    let mut used = HashSet::new();
    for layer in &doc.layers {
        if !layer.visible || layer.opacity == 0. {
            continue;
        }
        let r = layer.rect;
        let key = match layer.content {
            Content::Image(id) => {
                let a = &doc.images[&id];
                let m = std::fs::metadata(&a.path)
                    .map_err(|e| format!("Missing image {}: {e}", a.path.display()))?;
                format!("image:{:?}:{:?}:{}", a.path, m.modified(), m.len())
            }
            Content::Text(id) => format!(
                "text:{}:{}:{}",
                serde_json::to_string(&doc.texts[&id]).map_err(|e| e.to_string())?,
                r.width.ceil(),
                r.height.ceil()
            ),
        };
        used.insert(key.clone());
        let image = if let Some(image) = cache.get(&key) {
            image.clone()
        } else {
            let mut pixels = match layer.content {
                Content::Image(id) => {
                    let a = &doc.images[&id];
                    let checked = crate::scene::inspect_image(&a.path)?;
                    if checked.width != a.width || checked.height != a.height {
                        return Err(format!("Image dimensions changed: {}", a.path.display()));
                    }
                    image::ImageReader::open(&a.path)
                        .map_err(|e| e.to_string())?
                        .decode()
                        .map_err(|e| format!("Cannot decode {}: {e}", a.path.display()))?
                        .to_rgba8()
                }
                Content::Text(id) => rasterize_text(
                    &doc.texts[&id],
                    [r.width.ceil() as u32, r.height.ceil() as u32],
                    fonts,
                )?,
            };
            // Filter premultiplied texels to preserve transparent PNG/text edges.
            for pixel in pixels.pixels_mut() {
                for c in 0..3 {
                    pixel[c] = ((pixel[c] as u16 * pixel[3] as u16 + 127) / 255) as u8;
                }
            }
            let size = wgpu::Extent3d {
                width: pixels.width(),
                height: pixels.height(),
                depth_or_array_layers: 1,
            };
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("prepared layer bitmap"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &pixels,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(pixels.width() * 4),
                    rows_per_image: Some(pixels.height()),
                },
                size,
            );
            let image = Arc::new(GpuImage {
                view: texture.create_view(&Default::default()),
                _texture: texture,
            });
            cache.insert(key.clone(), image.clone());
            image
        };
        // Text tiles round allocation up to whole pixels. Crop the fractional
        // edge rather than stretching glyphs when only the wrapping box changes.
        let uv = if matches!(layer.content, Content::Text(_)) {
            [
                r.width / image._texture.width() as f32,
                r.height / image._texture.height() as f32,
            ]
        } else {
            [1., 1.]
        };
        let bytes: Vec<u8> = [r.x, r.y, r.width, r.height, layer.opacity, uv[0], uv[1], 0.]
            .iter()
            .flat_map(|v| v.to_ne_bytes())
            .collect();
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("immutable layer transform"),
            contents: &bytes,
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("prepared layer"),
            layout: &material.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&image.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&material.sampler),
                },
            ],
        });
        layers.push(Draw {
            bind,
            _image: image,
        });
    }
    cache.retain(|k, _| used.contains(k));
    Ok(Prepared {
        revision: 0,
        background: doc.test_background,
        layers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bounds(image: &image::RgbaImage) -> [u32; 4] {
        let mut b = [image.width(), image.height(), 0, 0];
        for (x, y, p) in image.enumerate_pixels() {
            if p[3] > 0 {
                b[0] = b[0].min(x);
                b[1] = b[1].min(y);
                b[2] = b[2].max(x);
                b[3] = b[3].max(y);
            }
        }
        b
    }
    #[test]
    fn text_wrap_alignment_alpha_and_box_clipping() {
        let fonts = Fonts::new(1., 8192, FontDefinitions::default());
        let mut text = TextObject {
            text: "Rastera editable multiline text".into(),
            font_size: 40.,
            color: [250, 80, 20, 128],
            alignment: Alignment::Left,
        };
        let wide = rasterize_text(&text, [900, 300], &fonts).unwrap();
        let narrow = rasterize_text(&text, [180, 300], &fonts).unwrap();
        assert!(bounds(&narrow)[3] > bounds(&wide)[3]);
        assert!(narrow.pixels().all(|p| p[3] <= 128));
        text.text = "A".into();
        let left = rasterize_text(&text, [300, 100], &fonts).unwrap();
        text.alignment = Alignment::Right;
        let right = rasterize_text(&text, [300, 100], &fonts).unwrap();
        assert!(bounds(&right)[0] > bounds(&left)[0] + 200);
        assert_eq!(
            bounds(&right)[3] - bounds(&right)[1],
            bounds(&left)[3] - bounds(&left)[1]
        );
        let clipped = rasterize_text(&text, [100, 5], &fonts).unwrap();
        assert_eq!(clipped.dimensions(), (100, 5));
    }
}
