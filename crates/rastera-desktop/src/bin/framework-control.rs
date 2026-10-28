//! No rastera-core, source texture, or custom frame producer. Only egui animation.
#[path = "../activity.rs"]
mod activity;
use activity::{output_id, Activity};
use eframe::egui;
use std::{
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

struct Control {
    views: [Arc<Activity>; 3],
    origin: Instant,
    stop: mpsc::Sender<()>,
    monitor: Option<thread::JoinHandle<()>>,
}
fn animate(ctx: &egui::Context, origin: Instant, activity: &Arc<Activity>) {
    activity.enter(ctx);
    ctx.request_repaint_after(Duration::from_secs_f64(1.0 / 60.0));
    egui::CentralPanel::default().show(ctx, |ui| {
        let rect = ui.max_rect();
        ui.painter()
            .add(eframe::egui_wgpu::Callback::new_paint_callback(
                rect,
                activity::CpuPaintProbe(activity.clone()),
            ));
        let x = rect.left() + (origin.elapsed().as_secs_f32() * 200.0) % rect.width().max(1.0);
        ui.painter()
            .circle_filled(egui::pos2(x, rect.center().y), 20.0, egui::Color32::YELLOW);
    });
    activity.finish();
}
impl eframe::App for Control {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        animate(ctx, self.origin, &self.views[0]);
        for i in 0..2 {
            let activity = self.views[i + 1].clone();
            let origin = self.origin;
            ctx.show_viewport_deferred(
                output_id(i),
                egui::ViewportBuilder::default()
                    .with_title(if i == 0 {
                        "Rastera — Left"
                    } else {
                        "Rastera — Right"
                    })
                    .with_inner_size([960.0, 540.0]),
                move |ctx, _| animate(ctx, origin, &activity),
            );
        }
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.monitor.take() {
            let _ = thread.join();
        }
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
            eprintln!(
                "framework adapter={:?}",
                cc.wgpu_render_state.as_ref().unwrap().adapter.get_info()
            );
            let origin = Instant::now();
            eprintln!(
                "clock_anchor mono_us={} unix_us={}",
                origin.elapsed().as_micros(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_micros()
            );

            let views = std::array::from_fn(|_| Arc::new(Activity::new(origin)));
            let observed: [Arc<Activity>; 3] = views.clone();
            let ctx = cc.egui_ctx.clone();
            let driver = std::env::args().any(|a| a == "--minimize-test=operator");
            let (stop, rx) = mpsc::channel();
            // Monitoring/command driver only; animation is driven by egui repaint timers.
            let monitor = thread::spawn(move || {
                let mut previous = [[0; 3]; 3];
                for second in 1.. {
                    match rx.recv_timeout(Duration::from_secs(1)) {
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        _ => break,
                    }
                    for (i, name) in ["operator", "left", "right"].into_iter().enumerate() {
                        eprint!("{}", observed[i].report(name, &mut previous[i]));
                    }
                    if driver && second % 3 == 0 && second <= 18 {
                        let minimized = second % 6 == 3;
                        eprintln!(
                            "event t_us={} driver=egui case=operator minimized={minimized}",
                            origin.elapsed().as_micros()
                        );
                        ctx.send_viewport_cmd_to(
                            egui::ViewportId::ROOT,
                            egui::ViewportCommand::Minimized(minimized),
                        );
                    }
                    if driver && second == 21 {
                        ctx.send_viewport_cmd_to(
                            egui::ViewportId::ROOT,
                            egui::ViewportCommand::Close,
                        );
                        break;
                    }
                }
            });
            Ok(Box::new(Control {
                views,
                origin,
                stop,
                monitor: Some(monitor),
            }))
        }),
    )
}
