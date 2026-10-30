//! One latest recovery snapshot, debounced writes on a dedicated worker.
use rastera_core::recovery::{self, Preferences, Snapshot};
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::Duration,
};
type Pending = Arc<(Mutex<(Option<(Preferences, Snapshot)>, bool)>, Condvar)>;
pub struct Session {
    pub prefs: Preferences,
    pub restored: Option<Snapshot>,
    pub warning: Option<String>,
    pending: Pending,
    worker: Option<thread::JoinHandle<()>>,
    pub last: std::time::Instant,
    pub remembered: Option<Snapshot>,
    error: Arc<Mutex<Option<String>>>,
}
impl Session {
    pub fn new() -> Self {
        let args: Vec<_> = std::env::args().collect();
        let root = args
            .iter()
            .find_map(|a| a.strip_prefix("--session-dir=").map(PathBuf::from))
            .or_else(|| {
                args.iter().find_map(|a| {
                    a.strip_prefix("--editor-probe=")
                        .map(|p| PathBuf::from(p).join("session"))
                })
            })
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                    .join("Library/Application Support/Rastera/session")
            });
        let prefs = std::fs::read(root.join("preferences.json"))
            .ok()
            .filter(|b| b.len() < 4096)
            .and_then(|b| serde_json::from_slice::<Preferences>(&b).ok())
            .filter(|p| p.version == 1)
            .unwrap_or_default();
        let skip = args.iter().any(|a| {
            a == "--no-restore" || a.starts_with("--project=") || a.starts_with("--editor-probe=")
        });
        let (restored, warning) = if !skip && prefs.restore_last {
            recovery::load(&root)
        } else {
            (None, None)
        };
        let pending: Pending = Arc::new((Mutex::new((None, false)), Condvar::new()));
        let job = pending.clone();
        let error = Arc::new(Mutex::new(None));
        let err = error.clone();
        let worker = thread::spawn(move || loop {
            let (lock, wake) = &*job;
            let mut state = lock.lock().unwrap();
            while state.0.is_none() && !state.1 {
                state = wake.wait(state).unwrap();
            }
            if state.0.is_none() && state.1 {
                break;
            }
            while !state.1 {
                let (s, timeout) = wake
                    .wait_timeout(state, Duration::from_millis(500))
                    .unwrap();
                state = s;
                if timeout.timed_out() {
                    break;
                }
            }
            let request = state.0.take();
            let stopping = state.1;
            drop(state);
            if let Some((p, s)) = request {
                *err.lock().unwrap() = recovery::store(&root, &p, &s).err();
            }
            if stopping {
                break;
            }
        });
        Self {
            prefs,
            restored,
            warning,
            pending,
            worker: Some(worker),
            last: std::time::Instant::now(),
            remembered: None,
            error,
        }
    }
    pub fn submit(&mut self, s: Snapshot) {
        self.remembered = Some(s.clone());
        self.last = std::time::Instant::now();
        let (lock, wake) = &*self.pending;
        lock.lock().unwrap().0 = Some((self.prefs.clone(), s));
        wake.notify_one();
    }
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let (lock, wake) = &*self.pending;
        lock.lock().unwrap().1 = true;
        wake.notify_one();
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}
