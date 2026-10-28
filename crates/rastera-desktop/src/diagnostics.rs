//! Opt-in diagnostics and native minimize/restore driver. Never draws into outputs.
pub use crate::activity::{output_id, Activity};
use eframe::egui;
use rastera_core::renderer::RenderStats;
use std::{
    sync::{atomic::Ordering, mpsc, Arc},
    thread,
    time::Duration,
};

pub struct Diagnostics {
    stop: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Diagnostics {
    pub fn start(
        ctx: egui::Context,
        producer: Arc<RenderStats>,
        views: [Arc<Activity>; 3],
    ) -> Option<Self> {
        let case =
            std::env::args().find_map(|a| a.strip_prefix("--minimize-test=").map(str::to_owned));
        if case.is_none() && !std::env::args().any(|a| a == "--diagnostics") {
            return None;
        }
        if let Some(case) = &case {
            assert!(
                ["operator", "left", "right", "all"].contains(&case.as_str()),
                "--minimize-test must be operator, left, right, or all"
            );
        }
        eprintln!(
            "clock_anchor mono_us={} unix_us={}",
            producer.timings.origin.elapsed().as_micros(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_micros()
        );
        let (stop, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut previous = [[0; 3]; 3];
            for second in 1.. {
                match rx.recv_timeout(Duration::from_secs(1)) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => break,
                }
                eprintln!(
                    "producer t_us={} submitted={} completed={} phase={} phase_start_us={}",
                    producer.timings.origin.elapsed().as_micros(),
                    producer.submitted.load(Ordering::Relaxed),
                    producer.completed.load(Ordering::Relaxed),
                    producer.phase.load(Ordering::Relaxed),
                    producer.phase_since_us.load(Ordering::Relaxed)
                );
                eprint!("{}", producer.timings.report());
                for (i, name) in ["operator", "left", "right"].into_iter().enumerate() {
                    eprint!("{}", views[i].report(name, &mut previous[i]));
                }
                if let Some(case) = &case {
                    // Three separate minimize/restore cycles, three seconds per state.
                    if second % 3 == 0 && second <= 18 {
                        let minimized = second % 6 == 3;
                        eprintln!(
                            "event t_us={} driver=egui case={case} minimized={minimized}",
                            producer.timings.origin.elapsed().as_micros()
                        );
                        for (i, name) in ["operator", "left", "right"].into_iter().enumerate() {
                            if case == "all" || case == name {
                                let id = if i == 0 {
                                    egui::ViewportId::ROOT
                                } else {
                                    output_id(i - 1)
                                };
                                ctx.send_viewport_cmd_to(
                                    id,
                                    egui::ViewportCommand::Minimized(minimized),
                                );
                            }
                        }
                    }
                    if second == 21 {
                        eprintln!("TEST {case}: finished; closing operator");
                        ctx.send_viewport_cmd_to(
                            egui::ViewportId::ROOT,
                            egui::ViewportCommand::Close,
                        );
                        break;
                    }
                }
            }
        });
        Some(Self {
            stop,
            worker: Some(worker),
        })
    }
}
impl Drop for Diagnostics {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

// Records entry into eframe's GPU paint pass, rather than just UI execution.
// It does not draw pixels or prove that the OS displayed the resulting frame.
pub struct PaintProbe(pub Arc<Activity>, pub Arc<RenderStats>);
impl eframe::egui_wgpu::CallbackTrait for PaintProbe {
    fn paint(
        &self,
        _: egui::PaintCallbackInfo,
        _: &mut wgpu::RenderPass<'static>,
        _: &eframe::egui_wgpu::CallbackResources,
    ) {
        self.0.painted.fetch_add(1, Ordering::Relaxed);
        self.0
            .observed_frame
            .store(self.1.completed.load(Ordering::Relaxed), Ordering::Relaxed);
    }
}
