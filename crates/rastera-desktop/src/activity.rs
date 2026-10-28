//! CPU callback/paint timing only. No display-presentation timestamps.
use eframe::egui;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    time::Instant,
};

#[derive(Default)]
struct Gap {
    count: u64,
    max_us: u64,
    max_start_us: u64,
}
pub struct Activity {
    origin: Instant,
    last_ui_us: AtomicU64,
    gap: Mutex<Gap>,
    pub entered: AtomicU64,
    pub painted: AtomicU64,
    pub observed_frame: AtomicU64,
    pub finished: AtomicU64,
    pub minimized: AtomicBool,
}
impl Activity {
    pub fn new(origin: Instant) -> Self {
        Self {
            origin,
            last_ui_us: AtomicU64::new(0),
            gap: Mutex::new(Gap::default()),
            entered: AtomicU64::new(0),
            painted: AtomicU64::new(0),
            observed_frame: AtomicU64::new(0),
            finished: AtomicU64::new(0),
            minimized: AtomicBool::new(false),
        }
    }
    pub fn enter(&self, ctx: &egui::Context) {
        let now = self.origin.elapsed().as_micros() as u64;
        let previous = self.last_ui_us.swap(now, Ordering::Relaxed);
        if previous > 0 {
            let mut gap = self.gap.lock().unwrap();
            gap.count += 1;
            if now - previous >= gap.max_us {
                gap.max_us = now - previous;
                gap.max_start_us = previous;
            }
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
    pub fn report(&self, name: &str, previous: &mut [u64; 3]) -> String {
        let gap = std::mem::take(&mut *self.gap.lock().unwrap());
        let current = [
            self.entered.load(Ordering::Relaxed),
            self.finished.load(Ordering::Relaxed),
            self.painted.load(Ordering::Relaxed),
        ];
        let delta: [u64; 3] = std::array::from_fn(|i| current[i] - previous[i]);
        *previous = current;
        format!("viewport t_us={} name={name} ui_enter={} ui_exit={} cpu_paint={} gap_count={} max_gap_us={} max_gap_start_us={} observed_producer={} cached_minimized={}\n", self.origin.elapsed().as_micros(),delta[0],delta[1],delta[2],gap.count,gap.max_us,gap.max_start_us,self.observed_frame.load(Ordering::Relaxed),self.minimized.load(Ordering::Relaxed))
    }
}
pub fn output_id(i: usize) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("projection", i))
}

/// Marks CPU entry into a render-pass callback; does not measure GPU or display time.
#[allow(dead_code)]
pub struct CpuPaintProbe(pub std::sync::Arc<Activity>);
impl eframe::egui_wgpu::CallbackTrait for CpuPaintProbe {
    fn paint(
        &self,
        _: egui::PaintCallbackInfo,
        _: &mut wgpu::RenderPass<'static>,
        _: &eframe::egui_wgpu::CallbackResources,
    ) {
        self.0.painted.fetch_add(1, Ordering::Relaxed);
    }
}
