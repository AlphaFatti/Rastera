//! Independent native Metal output, with an eframe operator retained as a control.
#[cfg(target_os = "macos")]
// Reuse baseline CPU telemetry; deferred viewport IDs are unused in this executable.
#[allow(dead_code)]
#[path = "../../rastera-desktop/src/activity.rs"]
mod activity;
#[cfg(target_os = "macos")]
mod bridge;

#[cfg(target_os = "macos")]
mod app {
    use super::{
        activity,
        bridge::{log, NativeOutput},
    };
    use eframe::egui;
    use std::{
        sync::{mpsc, Arc},
        thread,
        time::{Duration, Instant},
    };
    struct Control {
        output: NativeOutput,
        activity: Arc<activity::Activity>,
        origin: Instant,
        callbacks: u64,
        stop: mpsc::Sender<()>,
        monitor: Option<thread::JoinHandle<()>>,
    }
    impl eframe::App for Control {
        fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
            self.activity.enter(ctx);
            self.callbacks += 1;
            let elapsed = self.origin.elapsed().as_secs_f64();
            let unix_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            ctx.request_repaint_after(Duration::from_secs_f64(1.0 / 60.0));
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.heading("Rastera — eframe / wgpu Metal controls (yellow circle)");
                ui.label(
                    egui::RichText::new(format!(
                        "Elapsed {elapsed:010.3} s   UI callback {}",
                        self.callbacks
                    ))
                    .monospace()
                    .size(28.0),
                );
                ui.monospace(format!(
                    "Unix timestamp: {unix_ms} ms — sampled in UI callback"
                ));
                ui.label("CAMetalDisplayLink runs on its own thread. No shared Rastera canvas.");
                ui.label(if self.output.is_open() {
                    "Output open"
                } else {
                    "Output closed; restart to reopen"
                });
                let rect = ui.max_rect();
                ui.painter()
                    .add(eframe::egui_wgpu::Callback::new_paint_callback(
                        rect,
                        activity::CpuPaintProbe(self.activity.clone()),
                    ));
                let x = rect.left()
                    + (self.origin.elapsed().as_secs_f32() * 200.0) % rect.width().max(1.0);
                ui.painter().circle_filled(
                    egui::pos2(x, rect.center().y),
                    20.0,
                    egui::Color32::YELLOW,
                );
            });
            self.activity.finish();
        }
    }
    impl Drop for Control {
        fn drop(&mut self) {
            let _ = self.stop.send(());
            if let Some(thread) = self.monitor.take() {
                let _ = thread.join();
            }
            // output then drops on this main thread; its native destructor drains
            // the renderer and prints the final telemetry after the logger has joined.
        }
    }
    pub fn run() -> eframe::Result {
        // Diagnostic only: change the native picture window's AppKit property.
        let mut disable_animations = false;
        for arg in std::env::args().skip(1) {
            if let Some(value) = arg.strip_prefix("--window-animation=") {
                disable_animations = match value {
                    "default" => false,
                    "none" => true,
                    _ => {
                        return Err(eframe::Error::AppCreation(Box::new(std::io::Error::other(
                            "--window-animation must be default or none",
                        ))))
                    }
                };
            }
        }
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
            "Rastera — Operator / UI preview (control)",
            options,
            Box::new(move |cc| {
                let (output, stats) = NativeOutput::new(disable_animations)?;
                let origin = Instant::now();
                log(format!(
                    "clock_anchor mono_us={} unix_us={}\n",
                    origin.elapsed().as_micros(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_micros()
                ));
                let activity = Arc::new(activity::Activity::new(origin));
                let observed = activity.clone();
                let ctx = cc.egui_ctx.clone();
                let driver = std::env::args().any(|a| a == "--minimize-test=operator");
                let (stop, rx) = mpsc::channel();
                let monitor = thread::spawn(move || {
                    let mut previous = [0; 3];
                    for second in 1.. {
                        match rx.recv_timeout(Duration::from_secs(1)) {
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            _ => break,
                        }
                        log(observed.report("operator", &mut previous));
                        stats.report();
                        if driver && second % 3 == 0 && second <= 18 {
                            let minimized = second % 6 == 3;
                            log(format!(
                                "event t_us={} driver=egui case=operator minimized={minimized}\n",
                                origin.elapsed().as_micros()
                            ));
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
                    output,
                    activity,
                    origin,
                    callbacks: 0,
                    stop,
                    monitor: Some(monitor),
                }))
            }),
        )
    }
}
#[cfg(target_os = "macos")]
fn main() -> eframe::Result {
    app::run()
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("native-output-control requires macOS 14+");
}
