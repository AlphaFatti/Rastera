//! Opt-in diagnostics and native minimize/restore driver. Never draws into outputs.
use eframe::egui;
use rastera_core::renderer::RenderStats;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct Activity {
    started: OnceLock<Instant>,
    last_ui_us: AtomicU64,
    pub max_ui_gap_us: AtomicU64,
    pub entered: AtomicU64,
    pub painted: AtomicU64,
    pub observed_frame: AtomicU64,
    pub finished: AtomicU64,
    pub minimized: AtomicBool,
}
impl Activity {
    pub fn enter(&self, ctx: &egui::Context) {
        let now = self.started.get_or_init(Instant::now).elapsed().as_micros() as u64;
        let previous = self.last_ui_us.swap(now, Ordering::Relaxed);
        if previous > 0 {
            self.max_ui_gap_us
                .fetch_max(now - previous, Ordering::Relaxed);
        }
        self.entered.fetch_add(1, Ordering::Relaxed);
        self.minimized.store(
            ctx.input(|i| i.viewport().minimized.unwrap_or(false)),
            Ordering::Relaxed,
        );
    }
    pub fn finish(&self) {
        self.finished.fetch_add(1, Ordering::Relaxed);
    }
}
pub fn output_id(i: usize) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("projection", i))
}

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
        let (stop, rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            for second in 1.. {
                match rx.recv_timeout(Duration::from_secs(1)) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => break,
                }
                eprint!(
                    "t={second}s producer submitted={} completed={} phase={} max_submit_ms={} max_wait_ms={}",
                    producer.submitted.load(Ordering::Relaxed),
                    producer.completed.load(Ordering::Relaxed),
                    producer.phase.load(Ordering::Relaxed),
                    producer.max_submit_us.load(Ordering::Relaxed) / 1000,
                    producer.max_wait_us.load(Ordering::Relaxed) / 1000
                );
                for (name, v) in ["operator", "left", "right"].into_iter().zip(&views) {
                    eprint!(
                        " {name}={}/{} paint={} frame={} max_ui_gap_ms={} min={}",
                        v.entered.load(Ordering::Relaxed),
                        v.finished.load(Ordering::Relaxed),
                        v.painted.load(Ordering::Relaxed),
                        v.observed_frame.load(Ordering::Relaxed),
                        v.max_ui_gap_us.load(Ordering::Relaxed) / 1000,
                        v.minimized.load(Ordering::Relaxed)
                    );
                }
                eprintln!();
                if let Some(case) = &case {
                    // Three separate minimize/restore cycles, three seconds per state.
                    if second % 3 == 0 && second <= 18 {
                        let minimized = second % 6 == 3;
                        eprintln!("TEST {case}: minimized={minimized}");
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
