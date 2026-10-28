//! The existing generator, without surfaces, windowing, or egui.
use rastera_core::renderer::CanvasRenderer;
use std::{
    future::Future,
    sync::{atomic::Ordering, Arc},
    task::{Context, Poll, Wake, Waker},
    thread,
    time::Duration,
};

struct ThreadWake(thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}
fn block_on<T>(future: impl Future<Output = T>) -> T {
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::park(),
        }
    }
}
fn main() {
    let seconds: u64 = std::env::args()
        .find_map(|a| {
            a.strip_prefix("--seconds=")
                .map(|n| n.parse().expect("seconds must be an integer"))
        })
        .unwrap_or(24);
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..Default::default()
    });
    let adapter = block_on(instance.request_adapter(&Default::default())).expect("Metal adapter");
    eprintln!("headless adapter={:?}", adapter.get_info());
    let (device, queue) = block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits {
                max_texture_dimension_2d: 8192,
                ..Default::default()
            },
            ..Default::default()
        },
        None,
    ))
    .expect("GPU device");
    let canvas = CanvasRenderer::new(device, queue, || {});
    eprintln!(
        "clock_anchor mono_us={} unix_us={}",
        canvas.stats.timings.origin.elapsed().as_micros(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros()
    );
    for _ in 0..seconds {
        thread::sleep(Duration::from_secs(1));
        eprintln!(
            "producer t_us={} submitted={} completed={} phase={} phase_start_us={}",
            canvas.stats.timings.origin.elapsed().as_micros(),
            canvas.stats.submitted.load(Ordering::Relaxed),
            canvas.stats.completed.load(Ordering::Relaxed),
            canvas.stats.phase.load(Ordering::Relaxed),
            canvas.stats.phase_since_us.load(Ordering::Relaxed)
        );
        eprint!("{}", canvas.stats.timings.report());
    }
}
