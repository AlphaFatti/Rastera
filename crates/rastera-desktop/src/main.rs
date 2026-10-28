mod diagnostics;
use diagnostics::{output_id, Activity, Diagnostics, PaintProbe};
use eframe::egui;
use rastera_core::{
    fit,
    renderer::{CanvasRenderer, RenderStats},
    Slice,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

struct Rastera {
    _diagnostics: Option<Diagnostics>,
    activity: [Arc<Activity>; 3],
    canvas: CanvasRenderer,
    texture: egui::TextureId,
    outputs: [bool; 2],
    closed: Arc<[AtomicBool; 2]>,
    enabled: Arc<[AtomicBool; 2]>,
}
fn draw(
    ui: &mut egui::Ui,
    texture: egui::TextureId,
    slice: Slice,
    activity: Arc<Activity>,
    stats: Arc<RenderStats>,
) {
    let area = ui.max_rect();
    let [x, y, w, h] = fit([slice.width, slice.height], [area.width(), area.height()]);
    let rect = egui::Rect::from_min_size(area.min + egui::vec2(x, y), egui::vec2(w, h));
    let [min, max] = slice.uv();
    ui.painter()
        .add(eframe::egui_wgpu::Callback::new_paint_callback(
            rect,
            PaintProbe(activity, stats),
        ));
    ui.painter().image(
        texture,
        rect,
        egui::Rect::from_min_max(min.into(), max.into()),
        egui::Color32::WHITE,
    );
}
impl eframe::App for Rastera {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.activity[0].enter(ctx);
        for i in 0..2 {
            if self.closed[i].swap(false, Ordering::Relaxed) {
                self.outputs[i] = false;
            }
        }
        egui::TopBottomPanel::top("controls").show(ctx, |ui| {
            ui.heading("Rastera — first canvas");
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.outputs[0], "Left projection");
                ui.checkbox(&mut self.outputs[1], "Right projection");
                ui.label("3840 × 1080 · Metal · shared GPU canvas");
            });
        });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
            .show(ctx, |ui| {
                draw(
                    ui,
                    self.texture,
                    Slice::FULL,
                    self.activity[0].clone(),
                    self.canvas.stats.clone(),
                )
            });
        for i in 0..2 {
            self.enabled[i].store(self.outputs[i], Ordering::Relaxed);
        }
        for (i, slice) in [Slice::LEFT, Slice::RIGHT].into_iter().enumerate() {
            if !self.outputs[i] {
                continue;
            }
            let closed = self.closed.clone();
            let enabled = self.enabled.clone();
            let activity = self.activity[i + 1].clone();
            let texture = self.texture;
            let stats = self.canvas.stats.clone();
            ctx.show_viewport_deferred(
                output_id(i),
                egui::ViewportBuilder::default()
                    .with_title(if i == 0 {
                        "Rastera — Left"
                    } else {
                        "Rastera — Right"
                    })
                    .with_inner_size([960.0, 540.0]),
                move |ctx, _| {
                    activity.enter(ctx);
                    if ctx.input(|input| input.viewport().close_requested()) {
                        closed[i].store(true, Ordering::Relaxed);
                        enabled[i].store(false, Ordering::Relaxed);
                        ctx.request_repaint_of(egui::ViewportId::ROOT);
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
                        .show(ctx, |ui| {
                            draw(ui, texture, slice, activity.clone(), stats.clone())
                        });
                    activity.finish();
                },
            );
        }
        self.activity[0].finish();
    }
}
// A minimized viewport may not consume repaint requests. Keep only one pending
// request rather than growing egui's repaint cause list on every producer frame.
fn request_frame(ctx: &egui::Context, id: egui::ViewportId) {
    if !ctx.has_requested_repaint_for(&id) {
        ctx.request_repaint_of(id);
    }
}
fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 600.0]),
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: eframe::egui_wgpu::WgpuSetupCreateNew {
                instance_descriptor: wgpu::InstanceDescriptor {
                    backends: wgpu::Backends::METAL,
                    ..Default::default()
                },
                ..Default::default()
            }
            .into(),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "Rastera",
        options,
        Box::new(|cc| {
            let state = cc
                .wgpu_render_state
                .as_ref()
                .expect("wgpu renderer required");
            eprintln!("Rastera GPU: {:?}", state.adapter.get_info());
            let open_outputs = std::env::args()
                .any(|arg| arg == "--outputs" || arg.starts_with("--minimize-test="));
            let enabled = Arc::new(std::array::from_fn(|_| AtomicBool::new(open_outputs)));
            let wake_enabled: Arc<[AtomicBool; 2]> = enabled.clone();
            let repaint = cc.egui_ctx.clone();
            let canvas =
                CanvasRenderer::new(state.device.clone(), state.queue.clone(), move || {
                    request_frame(&repaint, egui::ViewportId::ROOT);
                    for i in 0..2 {
                        if wake_enabled[i].load(Ordering::Relaxed) {
                            request_frame(&repaint, output_id(i));
                        }
                    }
                });
            let texture = state.renderer.write().register_native_texture(
                &state.device,
                &canvas.view,
                wgpu::FilterMode::Nearest,
            );
            let activity = std::array::from_fn(|_| Arc::new(Activity::default()));
            let diagnostics =
                Diagnostics::start(cc.egui_ctx.clone(), canvas.stats.clone(), activity.clone());
            Ok(Box::new(Rastera {
                _diagnostics: diagnostics,
                activity,
                closed: Arc::new(std::array::from_fn(|_| AtomicBool::new(false))),
                canvas,
                texture,
                outputs: [open_outputs; 2],
                enabled,
            }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_notifications_remain_bounded_until_viewport_runs_again() {
        let ctx = egui::Context::default();
        // Consume startup repaint passes before simulating an inactive viewport.
        for _ in 0..3 {
            let _ = ctx.run(Default::default(), |_| {});
        }
        assert!(!ctx.has_requested_repaint_for(&egui::ViewportId::ROOT));
        for _ in 0..10_000 {
            request_frame(&ctx, egui::ViewportId::ROOT);
        }
        let mut causes = 0;
        let _ = ctx.run(Default::default(), |ctx| {
            causes = ctx.repaint_causes().len()
        });
        assert_eq!(
            causes, 1,
            "producer notifications must not queue per-frame causes"
        );
        // After consumption, new producer work can wake the viewport again.
        for _ in 0..2 {
            let _ = ctx.run(Default::default(), |_| {});
        }
        request_frame(&ctx, egui::ViewportId::ROOT);
        assert!(ctx.has_requested_repaint_for(&egui::ViewportId::ROOT));
    }
}
