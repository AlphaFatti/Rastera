use crate::{HEIGHT, WIDTH};
use std::{
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

/// Producer progress, readable even when a GPU call or UI callback is blocked.
#[derive(Default)]
pub struct RenderStats {
    pub submitted: AtomicU64,
    pub max_submit_us: AtomicU64,
    pub max_wait_us: AtomicU64,
    pub completed: AtomicU64,
    /// 0: pacing, 1: encoding, 2: submitting, 3: waiting for GPU, 4: notifying views.
    pub phase: AtomicUsize,
}

/// Owns the shared texture and a paced GPU producer. Dropping stops and joins it.
pub struct CanvasRenderer {
    pub view: wgpu::TextureView,
    pub stats: Arc<RenderStats>,
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}
impl CanvasRenderer {
    /// `frame_ready` runs on the producer thread and should only signal consumers.
    /// Pass `|| {}` when no UI notification is needed.
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        frame_ready: impl Fn() + Send + 'static,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("3840x1080 shared canvas"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let target = texture.create_view(&Default::default());
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("clock"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("test pattern"),
            source: wgpu::ShaderSource::Wgsl(include_str!("pattern.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("canvas generator"),
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
                    blend: None,
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
        let (stop, rx) = mpsc::channel();
        let stats = Arc::new(RenderStats::default());
        let progress = stats.clone();
        let worker = thread::spawn(move || {
            let start = Instant::now();
            let period = Duration::from_nanos(1_000_000_000 / 60);
            let mut deadline = start;
            loop {
                progress.phase.store(1, Ordering::Relaxed);
                queue.write_buffer(&uniform, 0, &start.elapsed().as_secs_f32().to_ne_bytes());
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("canvas frame"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &target,
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
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &bind, &[]);
                    pass.draw(0..3, 0..1);
                }
                progress.phase.store(2, Ordering::Relaxed);
                let submit_start = Instant::now();
                let submission = queue.submit([encoder.finish()]);
                progress
                    .max_submit_us
                    .fetch_max(submit_start.elapsed().as_micros() as u64, Ordering::Relaxed);
                progress.submitted.fetch_add(1, Ordering::Relaxed);
                progress.phase.store(3, Ordering::Relaxed);
                // Bound producer work to one completed GPU frame before submitting another.
                let wait_start = Instant::now();
                // Wait only for this producer frame, not a later viewport submission.
                let _ = device.poll(wgpu::Maintain::WaitForSubmissionIndex(submission));
                progress
                    .max_wait_us
                    .fetch_max(wait_start.elapsed().as_micros() as u64, Ordering::Relaxed);
                progress.completed.fetch_add(1, Ordering::Relaxed);
                progress.phase.store(4, Ordering::Relaxed);
                frame_ready();
                progress.phase.store(0, Ordering::Relaxed);
                deadline += period;
                if deadline < Instant::now() {
                    deadline = Instant::now() + period;
                }
                match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => break,
                }
            }
        });
        Self {
            view,
            stats,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for CanvasRenderer {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
