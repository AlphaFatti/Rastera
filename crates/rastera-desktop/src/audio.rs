// Audio control, transport/decode, rendering and HAL callbacks have separate owners.
// Graphs/decoders/devices retire on control after the renderer releases their last use.
#[path = "audio_ui.rs"]
mod ui;
use super::audio_native as native;
use rastera_audio::{
    clock::{Clock, Observation},
    dsp::{Meter, Smooth},
    model::{Model, Origin, Source},
    queue,
    timing::{Command, Transport},
    Mixer, BLOCK, MAX_STRIPS,
};
use rastera_core::scene::Document;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
const DECODE_AHEAD: i64 = 6000;
const FADE_SAMPLES: usize = 480;
const MAX_ERRORS: usize = 16;
#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
pub struct Patch {
    pub uid: String,
    pub first: u32,
}
#[derive(Clone, PartialEq)]
pub struct Desired {
    pub model: Model,
    pub patches: BTreeMap<u64, Patch>,
    pub participating: BTreeSet<u64>,
    pub active: bool,
    pub namespace: String,
    pub runtime: String,
    pub clocks: BTreeMap<u64, usize>,
}
#[derive(Clone)]
struct Request {
    desired: Desired,
    files: BTreeMap<u64, (PathBuf, Arc<Clock>)>,
    clocks: BTreeMap<u64, Arc<Clock>>,
}
#[derive(Clone, Copy)]
struct Packet {
    start: i64,
    frames: usize,
    channels: usize,
    generation: u64,
    data: [f32; BLOCK * 32],
}
impl Default for Packet {
    fn default() -> Self {
        Self {
            start: 0,
            frames: 0,
            channels: 0,
            generation: 0,
            data: [0.; BLOCK * 32],
        }
    }
}
#[derive(Default)]
struct SourceCounters {
    missing_samples: AtomicU64,
    backpressure: AtomicU64,
    decoded_frames: AtomicU64,
    discontinuities: AtomicU64,
}
struct Cursor {
    rx: queue::Receiver<Packet>,
    current: Packet,
    counters: Arc<SourceCounters>,
}
impl Cursor {
    fn synchronize(&mut self, generation: u64) {
        while self.current.frames == 0 || self.current.generation != generation {
            match self.rx.pop() {
                Some(packet) => self.current = packet,
                None => {
                    self.current.frames = 0;
                    break;
                }
            }
        }
    }
    fn at(&mut self, position: i64, channel: u32, generation: u64) -> f32 {
        if position < 0 {
            return 0.;
        }
        // A generation includes transport and sync-offset changes, so a backwards seek
        // cannot leave a future packet permanently latched as the current packet.
        while self.current.frames == 0
            || self.current.generation != generation
            || self.current.start + self.current.frames as i64 <= position
        {
            match self.rx.pop() {
                Some(packet) => self.current = packet,
                None => {
                    self.counters
                        .missing_samples
                        .fetch_add(1, Ordering::Relaxed);
                    return 0.;
                }
            }
        }
        if channel as usize >= self.current.channels || position < self.current.start {
            self.counters
                .missing_samples
                .fetch_add(1, Ordering::Relaxed);
            return 0.;
        }
        self.current.data
            [(position - self.current.start) as usize * self.current.channels + channel as usize]
    }
}
fn offset_samples(ms: f32) -> i32 {
    (ms * 48.).round() as i32
}
fn packet_generation(generation: u64, offset: i32) -> u64 {
    generation.rotate_left(32) ^ u64::from(offset as u32)
}
struct FileInput {
    token: u64,
    offset: Arc<AtomicI32>,
    counters: Arc<SourceCounters>,
    tracks: BTreeMap<i32, Cursor>,
}
struct Decoder {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Drop for Decoder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn decoder(
    path: PathBuf,
    source: &Source,
    clock: Arc<Clock>,
    token: u64,
    error: mpsc::SyncSender<String>,
) -> (Decoder, FileInput) {
    let stop = Arc::new(AtomicBool::new(false));
    let offset = Arc::new(AtomicI32::new(offset_samples(source.sync_ms)));
    let counters = Arc::new(SourceCounters::default());
    let mut senders = Vec::with_capacity(4);
    let mut tracks = BTreeMap::new();
    for track in source
        .channels
        .iter()
        .map(|channel| channel.track)
        .collect::<BTreeSet<_>>()
    {
        let (tx, rx) = queue::bounded(64, Packet::default());
        senders.push((track, tx));
        tracks.insert(
            track,
            Cursor {
                rx,
                current: Packet::default(),
                counters: counters.clone(),
            },
        );
    }
    let done = stop.clone();
    let observed = clock.clone();
    let reader_offset = offset.clone();
    let reader_counters = counters.clone();
    let worker = thread::spawn(move || {
        let mut readers = Vec::with_capacity(4);
        for (track, sender) in senders {
            match native::Reader::new(&path, track) {
                Ok(reader) => readers.push((reader, sender, u64::MAX, 0i64, false)),
                Err(message) => {
                    let _ = error.try_send(message);
                    return;
                }
            }
        }
        let mut previous = Observation::default();
        while !done.load(Ordering::Acquire) {
            let observation = observed.load().unwrap_or(previous);
            previous = observation;
            let offset = reader_offset.load(Ordering::Relaxed);
            let generation = packet_generation(observation.generation, offset);
            let target = (observation.at(native::host()) - i64::from(offset)).max(0);
            let mut progressed = false;
            for (reader, sender, epoch, end, ended) in &mut readers {
                if *epoch != generation || *end < target - 4800 || *end > target + 48000 {
                    if !reader.seek(target as f64 / 48000.) {
                        let _ = error.try_send("Audio reader seek failed".into());
                        *ended = true;
                        continue;
                    }
                    *epoch = generation;
                    *end = target;
                    *ended = false;
                    reader_counters
                        .discontinuities
                        .fetch_add(1, Ordering::Relaxed);
                }
                if *ended || *end > target + DECODE_AHEAD {
                    continue;
                }
                let mut packet = Packet {
                    generation,
                    ..Packet::default()
                };
                match reader.read(&mut packet.data) {
                    Ok((frames, channels, pts)) if frames > 0 => {
                        packet.start = (pts * 48000.).round() as i64;
                        packet.frames = frames;
                        packet.channels = channels;
                        *end = packet.start + frames as i64;
                        let mut waiting = false;
                        loop {
                            if done.load(Ordering::Acquire) {
                                return;
                            }
                            let latest = observed.load().unwrap_or(observation);
                            if packet_generation(
                                latest.generation,
                                reader_offset.load(Ordering::Relaxed),
                            ) != generation
                            {
                                break;
                            }
                            if sender.push(packet) {
                                reader_counters
                                    .decoded_frames
                                    .fetch_add(frames as u64, Ordering::Relaxed);
                                break;
                            }
                            if !waiting {
                                reader_counters.backpressure.fetch_add(1, Ordering::Relaxed);
                                waiting = true;
                            }
                            thread::sleep(Duration::from_millis(1));
                        }
                        progressed = true;
                    }
                    Ok(_) => *ended = true,
                    Err(message) => {
                        let _ = error.try_send(message);
                        *ended = true;
                    }
                }
            }
            if !progressed {
                thread::sleep(Duration::from_millis(2));
            }
        }
    });
    (
        Decoder {
            stop,
            worker: Some(worker),
        },
        FileInput {
            token,
            offset,
            counters,
            tracks,
        },
    )
}
#[derive(Clone, Copy, PartialEq)]
struct FileTransportSettings {
    playing: bool,
    looping: bool,
    resume: i64,
    duration: i64,
}
impl FileTransportSettings {
    fn from_source(source: &Source) -> Self {
        Self {
            playing: source.playing,
            looping: source.looping,
            resume: source.resume_sample,
            duration: match source.origin {
                Origin::File { duration, .. } => (duration * 48000.).round() as i64,
                _ => 0,
            },
        }
    }
}
enum FileCommand {
    Settings {
        settings: FileTransportSettings,
        restart: bool,
    },
    Transport(Command),
}
struct FileTransport {
    clock: Arc<Clock>,
    stop: Arc<AtomicBool>,
    tx: mpsc::SyncSender<FileCommand>,
    worker: Option<thread::JoinHandle<()>>,
    requested: FileTransportSettings,
}
impl FileTransport {
    fn new(source: &Source, clock: Arc<Clock>) -> Self {
        let requested = FileTransportSettings::from_source(source);
        let owned_clock = clock.clone();
        let (tx, commands) = mpsc::sync_channel(32);
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            let now = native::host();
            let mut state = Transport::default();
            state.position = requested.resume;
            state.host = now;
            state.playing = requested.playing;
            state.generation = clock
                .load()
                .map_or(1, |observation| observation.generation.wrapping_add(1));
            let mut settings = requested;
            while !done.load(Ordering::Acquire) {
                let now = native::host();
                for _ in 0..32 {
                    match commands.try_recv() {
                        Ok(FileCommand::Settings {
                            settings: next,
                            restart,
                        }) => {
                            if next.resume != settings.resume {
                                state.command(Command::Seek(next.resume), now);
                            }
                            if next.playing != settings.playing {
                                state.command(
                                    if next.playing {
                                        Command::Play
                                    } else {
                                        Command::Pause
                                    },
                                    now,
                                );
                            }
                            if restart {
                                state.command(
                                    if next.playing {
                                        Command::Restart
                                    } else {
                                        Command::Seek(0)
                                    },
                                    now,
                                );
                            }
                            settings = next;
                        }
                        Ok(FileCommand::Transport(Command::Gain { at, value })) => {
                            clock.schedule_gain(at, value);
                        }
                        Ok(FileCommand::Transport(command)) => state.command(command, now),
                        Err(_) => break,
                    }
                }
                let sample = state.at(now);
                if settings.duration > 0 && sample >= settings.duration {
                    state.position = if settings.looping {
                        sample % settings.duration
                    } else {
                        settings.duration
                    };
                    state.host = now;
                    if settings.looping {
                        state.generation = state.generation.wrapping_add(1);
                    } else {
                        state.playing = false;
                    }
                }
                clock.store(Observation {
                    sample: state.at(now),
                    host: now,
                    playing: state.playing,
                    generation: state.generation,
                });
                thread::sleep(Duration::from_millis(1));
            }
            // A stopped owner no longer advances a borrowed observation during retirement.
            let now = native::host();
            clock.store(Observation {
                sample: state.at(now),
                host: now,
                playing: false,
                generation: state.generation,
            });
        });
        Self {
            stop,
            tx,
            worker: Some(worker),
            clock: owned_clock,
            requested,
        }
    }
    fn configure(&mut self, source: &Source, restart: bool) {
        let settings = FileTransportSettings::from_source(source);
        if (settings != self.requested || restart)
            && self
                .tx
                .try_send(FileCommand::Settings { settings, restart })
                .is_ok()
        {
            self.requested = settings;
        }
    }
}
impl Drop for FileTransport {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[derive(Clone, Default)]
pub struct Status {
    pub devices: Vec<native::DeviceInfo>,
    pub errors: Vec<String>,
    pub stats: BTreeMap<String, [u64; 6]>,
    pub meters: BTreeMap<u64, Meter>,
    pub source_stats: BTreeMap<u64, [u64; 4]>,
    pub source_formats: BTreeMap<u64, String>,
    pub details: BTreeMap<String, native::DeviceDetails>,
    pub samples: u64,
    pub timeline_sample: u64,
    pub skipped_samples: u64,
    pub source_queues: BTreeMap<u64, (usize, usize)>,
    pub late_blocks: u64,
}
struct Update {
    version: u64,
    desired: Desired,
    clocks: BTreeMap<u64, Arc<Clock>>,
    devices: BTreeMap<String, Arc<native::Device>>,
    input_uids: BTreeSet<String>,
    add: BTreeMap<u64, FileInput>,
    remove: Vec<u64>,
}
struct Retired {
    version: u64,
    inputs: Vec<(u64, FileInput)>,
    update: Option<Box<Update>>,
}
type ProbeResult = Result<(PathBuf, Origin, String, native::Probe, Option<u64>), String>;
struct Pending {
    path: PathBuf,
    origin: Origin,
    name: String,
    probe: native::Probe,
    selected: Vec<rastera_audio::model::Channel>,
    edit: Option<u64>,
}
#[derive(Default, Serialize, Deserialize)]
struct PatchStore {
    version: u32,
    projects: BTreeMap<String, BTreeMap<u64, Patch>>,
}
pub struct Audio {
    probe_tx: mpsc::SyncSender<(PathBuf, Origin, String, Option<u64>)>,
    probe_rx: mpsc::Receiver<ProbeResult>,
    pending: Option<Pending>,
    tx: mpsc::SyncSender<Request>,
    commands: mpsc::SyncSender<(u64, Command)>,
    restore_tx: mpsc::SyncSender<BTreeMap<u64, rastera_core::recovery::Playhead>>,
    pending_restore: Option<BTreeMap<u64, rastera_core::recovery::Playhead>>,
    rx: mpsc::Receiver<Status>,
    stop: Arc<AtomicBool>,
    control: Option<thread::JoinHandle<()>>,
    last: Option<Desired>,
    pub status: Status,
    pub active: bool,
    pub patches: BTreeMap<u64, Patch>,
    pub open: bool,
    clocks: BTreeMap<u64, Arc<Clock>>,
    source_clocks: BTreeMap<u64, Arc<Clock>>,
    video_controls: BTreeMap<u64, Arc<rastera_core::media::Control>>,
    namespace: String,
    project_identity: String,
    runtime_generation: u64,
    patch_file: PathBuf,
    store: BTreeMap<String, BTreeMap<u64, Patch>>,
}
fn error(status: &mut Status, message: String) {
    if status.errors.last() == Some(&message) {
        return;
    }
    if status.errors.len() >= MAX_ERRORS {
        status.errors.remove(0);
    }
    status.errors.push(message);
}
fn device_signature(info: &native::DeviceInfo, input: bool) -> (u64, u32, u32, u32, bool) {
    (
        info.rate.to_bits(),
        info.buffer_frames,
        info.input_channels,
        info.output_channels,
        input,
    )
}
fn metadata(source: &Source, probe: &native::Probe) -> String {
    let mut missing = Vec::new();
    for channel in &source.channels {
        if !probe
            .tracks
            .iter()
            .any(|track| track.id == channel.track && channel.channel < track.channels)
        {
            missing.push(format!("T{} Ch{}", channel.track, channel.channel + 1));
        }
    }
    let mut description = probe
        .tracks
        .iter()
        .filter(|track| {
            source
                .channels
                .iter()
                .any(|channel| channel.track == track.id)
        })
        .map(|track| {
            format!(
                "Track {}: {} ch, {} Hz, start {:.6}s, {}",
                track.id, track.channels, track.rate, track.start, track.language
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    if !missing.is_empty() {
        description.push_str(&format!(
            "; unavailable mappings (silenced): {}",
            missing.join(", ")
        ));
    }
    description
}
impl Audio {
    pub fn new() -> Self {
        native::shortcut();
        let (probe_tx, probe_requests) =
            mpsc::sync_channel::<(PathBuf, Origin, String, Option<u64>)>(1);
        let (probe_results, probe_rx) = mpsc::sync_channel(1);
        let (tx, requests) = mpsc::sync_channel::<Request>(1);
        let (commands, command_rx) = mpsc::sync_channel::<(u64, Command)>(32);
        let (restore_tx, restore_rx) =
            mpsc::sync_channel::<BTreeMap<u64, rastera_core::recovery::Playhead>>(1);
        let (status_tx, rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let (control_tx, updates) = mpsc::sync_channel::<Box<Update>>(1);
        let (retire_tx, retired) = mpsc::sync_channel::<Retired>(4);
        let (meter_tx, meters) = mpsc::sync_channel(1);
        let control = thread::spawn(move || {
            let render_stop = done.clone();
            let render = thread::spawn(move || render(updates, retire_tx, meter_tx, render_stop));
            let mut devices = BTreeMap::<String, Arc<native::Device>>::new();
            let mut device_signatures = BTreeMap::new();
            let mut old_devices = Vec::<(u64, Arc<native::Device>)>::with_capacity(24);
            let mut deferred_devices = BTreeMap::<String, u64>::new();
            let mut decoders = BTreeMap::<u64, Decoder>::new();
            let mut transports = BTreeMap::<u64, FileTransport>::new();
            let mut old_transports = Vec::<(u64, FileTransport)>::with_capacity(32);
            let mut signatures = BTreeMap::<u64, (Origin, Vec<i32>, usize, i32)>::new();
            let mut pending_restore = BTreeMap::<u64, rastera_core::recovery::Playhead>::new();
            let mut cached_probes = BTreeMap::<u64, native::Probe>::new();
            let mut status = Status::default();
            let mut version = 0u64;
            let mut token = 0u64;
            let mut latest = None;
            let mut last_request: Option<Request> = None;
            let mut poll = Instant::now() - Duration::from_secs(5);
            let (error_tx, errors) = mpsc::sync_channel(16);
            loop {
                while let Ok(retirement) = retired.try_recv() {
                    for (_, input) in retirement.inputs {
                        decoders.remove(&input.token);
                        drop(input);
                    }
                    drop(retirement.update);
                    old_devices.retain(|(used, _)| *used > retirement.version);
                    old_transports.retain(|(used, _)| *used > retirement.version);
                    let completed: Vec<_> = deferred_devices
                        .iter()
                        .filter(|(_, used)| **used <= retirement.version)
                        .map(|(uid, _)| uid.clone())
                        .collect();
                    for uid in completed {
                        deferred_devices.remove(&uid);
                        if latest.is_none() {
                            latest = last_request.clone();
                        }
                    }
                }
                while let Ok(message) = errors.try_recv() {
                    error(&mut status, message);
                }
                while let Ok(restored) = restore_rx.try_recv() {
                    pending_restore = restored;
                }
                let mut restored = Vec::with_capacity(16);
                for (&id, playhead) in &pending_restore {
                    if let Some(actor) = transports.get(&id) {
                        if actor
                            .tx
                            .try_send(FileCommand::Transport(Command::Seek(
                                (playhead.seconds * 48000.).round() as i64,
                            )))
                            .is_ok()
                            && actor
                                .tx
                                .try_send(FileCommand::Transport(if playhead.playing {
                                    Command::Play
                                } else {
                                    Command::Pause
                                }))
                                .is_ok()
                        {
                            restored.push(id);
                        }
                    }
                }
                for id in restored {
                    pending_restore.remove(&id);
                }
                while let Ok((id, command)) = command_rx.try_recv() {
                    if let Some(actor) = transports.get(&id) {
                        if actor.tx.try_send(FileCommand::Transport(command)).is_err() {
                            error(
                                &mut status,
                                format!("Source {id} transport command queue full"),
                            );
                        }
                    } else if let Command::Gain { at, value } = command {
                        if let Some(clock) = last_request
                            .as_ref()
                            .and_then(|request| request.clocks.get(&id))
                        {
                            clock.schedule_gain(at, value);
                        }
                    } else {
                        error(
                            &mut status,
                            format!("Source {id}: use its shared video transport controls"),
                        );
                    }
                }
                if let Ok(mut snapshot) = meters.try_recv() {
                    snapshot.details = devices
                        .iter()
                        .filter_map(|(uid, device)| {
                            device.details().ok().map(|details| (uid.clone(), details))
                        })
                        .collect();
                    snapshot.devices = status.devices.clone();
                    snapshot.errors = status.errors.clone();
                    snapshot.source_formats = status.source_formats.clone();
                    let _ = status_tx.try_send(snapshot);
                }
                if done.load(Ordering::Acquire) {
                    break;
                }
                if let Ok((path, origin, name, edit)) = probe_requests.try_recv() {
                    let result = native::probe(&path).and_then(|probe| {
                        if probe.tracks.is_empty()
                            || !probe.duration.is_finite()
                            || !(0.0..=86400.).contains(&probe.duration)
                            || probe.duration == 0.
                        {
                            Err("No supported finite audio tracks (maximum 24 hours)".into())
                        } else {
                            Ok((path, origin, name, probe, edit))
                        }
                    });
                    let _ = probe_results.try_send(result);
                }
                while let Ok(request) = requests.try_recv() {
                    latest = Some(request);
                }
                if poll.elapsed() >= Duration::from_secs(2) {
                    match native::devices() {
                        Ok(discovered) => {
                            if discovered != status.devices {
                                status.devices = discovered;
                                if latest.is_none() {
                                    latest = last_request.clone();
                                }
                            }
                        }
                        Err(message) => error(&mut status, message),
                    }
                    // The native listener catches changes not reflected in the list's public fields.
                    if devices
                        .values()
                        .any(|device| device.details().is_ok_and(|details| details.format_changed))
                        && latest.is_none()
                    {
                        latest = last_request.clone();
                    }
                    poll = Instant::now();
                }
                let Some(request) = latest.take() else {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                };
                if request.desired.model.validate().is_err() {
                    error(
                        &mut status,
                        "Invalid audio graph ignored; last valid graph retained".into(),
                    );
                    continue;
                }
                version += 1;
                let mut needed = BTreeMap::<String, bool>::new();
                if request.desired.active {
                    for patch in request.desired.patches.values() {
                        if !patch.uid.is_empty() {
                            needed.entry(patch.uid.clone()).or_insert(false);
                        }
                    }
                    for (&id, source) in &request.desired.model.sources {
                        if !source.enabled || !request.desired.participating.contains(&id) {
                            continue;
                        }
                        let uid = match &source.origin {
                            Origin::Input { uid } => uid.as_str(),
                            Origin::Capture(_) => &source.input_uid,
                            _ => "",
                        };
                        if !uid.is_empty() {
                            needed.insert(uid.to_owned(), true);
                        }
                    }
                }
                let retire: Vec<_> = devices
                    .iter()
                    .filter_map(|(uid, device)| {
                        let rows: Vec<_> = status
                            .devices
                            .iter()
                            .filter(|info| info.uid == *uid)
                            .collect();
                        let invalid = match (needed.get(uid), rows.as_slice()) {
                            (Some(input), [info]) => {
                                device_signatures.get(uid) != Some(&device_signature(info, *input))
                                    || device.details().is_ok_and(|details| details.format_changed)
                            }
                            _ => true,
                        };
                        invalid.then(|| uid.clone())
                    })
                    .collect();
                for uid in retire {
                    old_devices.push((version - 1, devices.remove(&uid).unwrap()));
                    device_signatures.remove(&uid);
                    if needed.contains_key(&uid) {
                        deferred_devices.insert(uid, version - 1);
                    }
                }
                for (uid, input) in &needed {
                    if devices.contains_key(uid) || deferred_devices.contains_key(uid) {
                        continue;
                    }
                    let rows: Vec<_> = status
                        .devices
                        .iter()
                        .filter(|info| info.uid == *uid)
                        .collect();
                    if rows.len() != 1 {
                        error(
                            &mut status,
                            format!("Missing/ambiguous audio device (assignment retained): {uid}"),
                        );
                        continue;
                    }
                    if devices.len() >= 12 {
                        error(&mut status, "Audio device limit: 12".into());
                        continue;
                    }
                    match native::Device::new(rows[0].clone(), *input) {
                        Ok(device) => {
                            device_signatures
                                .insert(uid.clone(), device_signature(&device.info, *input));
                            devices.insert(uid.clone(), Arc::new(device));
                        }
                        Err(message) => error(&mut status, message),
                    }
                }
                for (&bus, patch) in &request.desired.patches {
                    let unavailable=status.devices.iter().find(|info|info.uid==patch.uid).filter(|info|patch.first.saturating_add(1)>=info.output_channels).map(|info|format!("Bus {bus}: requested channels {}/{} unavailable on {} (assignment retained)",patch.first.saturating_add(1),patch.first.saturating_add(2),info.name));
                    if let Some(message) = unavailable {
                        error(&mut status, message);
                    }
                }
                let mut add = BTreeMap::new();
                let mut remove = Vec::new();
                for (&id, source) in &request.desired.model.sources {
                    let activated = request.desired.participating.contains(&id)
                        && !last_request
                            .as_ref()
                            .is_some_and(|old| old.desired.participating.contains(&id));
                    if let Origin::File { .. } = source.origin {
                        if transports.get(&id).is_some_and(|actor| {
                            request
                                .clocks
                                .get(&id)
                                .is_none_or(|clock| !Arc::ptr_eq(&actor.clock, clock))
                        }) {
                            old_transports.push((version - 1, transports.remove(&id).unwrap()));
                        }
                        if let Some(actor) = transports.get_mut(&id) {
                            actor.configure(source, activated && source.restart_on_activation);
                        } else if let Some(clock) = request.clocks.get(&id) {
                            transports.insert(id, FileTransport::new(source, clock.clone()));
                        }
                    }
                    if let Some((path, clock)) = request.files.get(&id) {
                        let signature = (
                            source.origin.clone(),
                            source
                                .channels
                                .iter()
                                .map(|channel| channel.track)
                                .collect::<BTreeSet<_>>()
                                .into_iter()
                                .collect(),
                            Arc::as_ptr(clock) as usize,
                            offset_samples(source.sync_ms),
                        );
                        if signatures.get(&id) != Some(&signature) {
                            match native::probe(path) {
                                Ok(probe) => {
                                    status.source_formats.insert(id, metadata(source, &probe));
                                    cached_probes.insert(id, probe);
                                }
                                Err(message) => {
                                    status
                                        .source_formats
                                        .insert(id, format!("Unavailable: {message}"));
                                    error(&mut status, message);
                                }
                            }
                            token += 1;
                            let (owner, endpoint) = decoder(
                                path.clone(),
                                source,
                                clock.clone(),
                                token,
                                error_tx.clone(),
                            );
                            decoders.insert(token, owner);
                            signatures.insert(id, signature);
                            add.insert(id, endpoint);
                        } else if let Some(probe) = cached_probes.get(&id) {
                            status.source_formats.insert(id, metadata(source, probe));
                        }
                    } else {
                        let uid = match &source.origin {
                            Origin::Input { uid } => uid.as_str(),
                            Origin::Capture(_) => &source.input_uid,
                            _ => "",
                        };
                        if !uid.is_empty() {
                            let description = if let Some(info) =
                                status.devices.iter().find(|info| info.uid == uid)
                            {
                                let unavailable = source
                                    .channels
                                    .iter()
                                    .any(|channel| channel.channel >= info.input_channels);
                                format!(
                                    "{}: {} channels, {} Hz{}{}",
                                    info.name,
                                    info.input_channels,
                                    info.rate,
                                    if unavailable {
                                        "; unavailable requested channels are silenced"
                                    } else {
                                        ""
                                    },
                                    if source.sync_ms < -20. {
                                        "; live offset below −20 ms unavailable (silenced)"
                                    } else {
                                        ""
                                    }
                                )
                            } else {
                                format!(
                                    "Missing input {uid}; requested mapping retained and silenced"
                                )
                            };
                            status.source_formats.insert(id, description);
                        }
                    }
                }
                let removed: Vec<_> = signatures
                    .keys()
                    .filter(|id| !request.files.contains_key(*id))
                    .copied()
                    .collect();
                for id in removed {
                    signatures.remove(&id);
                    cached_probes.remove(&id);
                    remove.push(id);
                }
                let removed_actors: Vec<_> = transports
                    .keys()
                    .filter(|id| {
                        !request
                            .desired
                            .model
                            .sources
                            .get(*id)
                            .is_some_and(|source| matches!(source.origin, Origin::File { .. }))
                    })
                    .copied()
                    .collect();
                for id in removed_actors {
                    old_transports.push((version - 1, transports.remove(&id).unwrap()));
                }
                status
                    .source_formats
                    .retain(|id, _| request.desired.model.sources.contains_key(id));
                last_request = Some(request.clone());
                let mut batch = Box::new(Update {
                    version,
                    desired: request.desired,
                    clocks: request.clocks,
                    input_uids: needed
                        .iter()
                        .filter(|(_, input)| **input)
                        .map(|(uid, _)| uid.clone())
                        .collect(),
                    devices: devices.clone(),
                    add,
                    remove,
                });
                loop {
                    match control_tx.try_send(batch) {
                        Ok(()) => break,
                        Err(mpsc::TrySendError::Full(returned)) => {
                            batch = returned;
                            if done.load(Ordering::Acquire) {
                                break;
                            }
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(_) => break,
                    }
                }
            }
            // The renderer must be allowed to return its final leases while being joined.
            while !render.is_finished() {
                while let Ok(retirement) = retired.try_recv() {
                    drop(retirement);
                }
                thread::sleep(Duration::from_millis(1));
            }
            let _ = render.join();
            while let Ok(retirement) = retired.try_recv() {
                drop(retirement);
            }
            drop(decoders);
            drop(transports);
            drop(old_transports);
            drop(devices);
            drop(old_devices);
        });
        let patch_file = patch_file();
        let store = std::fs::read(&patch_file)
            .ok()
            .filter(|bytes| bytes.len() <= 1024 * 1024)
            .and_then(|bytes| {
                serde_json::from_slice::<PatchStore>(&bytes)
                    .ok()
                    .filter(|store| store.version == 1)
                    .map(|store| store.projects)
                    .or_else(|| {
                        serde_json::from_slice::<BTreeMap<String, BTreeMap<u64, Patch>>>(&bytes)
                            .ok()
                    })
            })
            .unwrap_or_default();
        Self {
            probe_tx,
            probe_rx,
            pending: None,
            tx,
            commands,
            restore_tx,
            pending_restore: None,
            rx,
            stop,
            control: Some(control),
            last: None,
            status: Status::default(),
            active: false,
            patches: BTreeMap::new(),
            open: std::env::args().any(|a| a == "--audio-mixer"),
            clocks: BTreeMap::new(),
            source_clocks: BTreeMap::new(),
            video_controls: BTreeMap::new(),
            namespace: String::new(),
            project_identity: String::new(),
            runtime_generation: 0,
            patch_file,
            store,
        }
    }
    pub fn reconcile(
        &mut self,
        doc: &Document,
        media: &super::media::Media,
        machine: &str,
        project: Option<&Path>,
    ) {
        if let Some(restore) = self.pending_restore.take() {
            if let Err(mpsc::TrySendError::Full(restore)) = self.restore_tx.try_send(restore) {
                self.pending_restore = Some(restore);
            }
        }
        while let Ok(status) = self.rx.try_recv() {
            self.status = status;
        }
        let namespace = namespace(machine, &doc.project_id, project);
        let identity = format!("{machine}:{}", doc.project_id);
        if self.project_identity != identity {
            self.project_identity = identity;
            self.runtime_generation += 1;
            self.active = false;
            native::emergency(true);
            self.clocks.clear();
            self.source_clocks.clear();
            self.namespace = namespace.clone();
            self.patches = self.store.get(&namespace).cloned().unwrap_or_default();
        } else if self.namespace != namespace {
            // Save As changes patch persistence scope, not the running transport or activation.
            self.namespace = namespace.clone();
            self.save_patches();
        }
        if self.active && native::is_muted() {
            self.active = false;
        }
        self.clocks
            .retain(|id, _| doc.audio.sources.contains_key(id));
        self.source_clocks
            .retain(|id, _| doc.audio.sources.contains_key(id));
        let mut files = BTreeMap::new();
        self.video_controls.clear();
        for (&id, source) in &doc.audio.sources {
            match &source.origin {
                Origin::Playback(playback) => {
                    if let (Some(settings), Some(control)) =
                        (doc.playbacks.get(playback), media.control(doc, *playback))
                    {
                        files.insert(
                            id,
                            (
                                doc.media_assets[&settings.asset].path.clone(),
                                control.audio_clock.clone(),
                            ),
                        );
                        self.source_clocks.insert(id, control.audio_clock.clone());
                        self.video_controls.insert(id, control.clone());
                    }
                }
                Origin::File { path, .. } => {
                    let clock = self
                        .clocks
                        .entry(id)
                        .or_insert_with(|| Arc::new(Clock::default()))
                        .clone();
                    files.insert(id, (path.clone(), clock.clone()));
                    self.source_clocks.insert(id, clock);
                }
                _ => {
                    self.source_clocks.entry(id).or_insert_with(|| {
                        let clock = Arc::new(Clock::default());
                        let host = native::host();
                        clock.store(Observation {
                            sample: (host * 48000.).round() as i64,
                            host,
                            playing: true,
                            generation: 0,
                        });
                        clock
                    });
                }
            }
        }
        let desired = Desired {
            model: doc.audio.clone(),
            patches: self.patches.clone(),
            participating: doc.audio_participation(),
            active: self.active,
            namespace,
            runtime: format!("{}:{}", self.project_identity, self.runtime_generation),
            clocks: self
                .source_clocks
                .iter()
                .map(|(&id, clock)| (id, Arc::as_ptr(clock) as usize))
                .collect(),
        };
        if self.last.as_ref() == Some(&desired) {
            return;
        }
        if self
            .tx
            .try_send(Request {
                desired: desired.clone(),
                files,
                clocks: self.source_clocks.clone(),
            })
            .is_ok()
        {
            self.last = Some(desired);
        }
    }
    pub fn project_opened(&mut self) {
        self.active = false;
        native::emergency(true);
        self.project_identity.clear();
        self.last = None;
    }
    pub fn inspect(&mut self, path: PathBuf, origin: Origin, name: String, edit: Option<u64>) {
        if self.probe_tx.try_send((path, origin, name, edit)).is_err() {
            error(
                &mut self.status,
                "Audio metadata worker busy; try again".into(),
            );
        }
    }
    /// Bounded future-producer control path; existing video uses its already-shared transport UI.
    pub fn command_source(&self, id: u64, command: Command) -> Result<(), String> {
        if let Some(control) = self.video_controls.get(&id) {
            return video_command(control, command);
        }
        self.commands
            .try_send((id, command))
            .map_err(|_| "Audio transport queue full; command not applied".into())
    }
    pub fn schedule_gain(&self, id: u64, at: i64, gain: f32) -> Result<(), String> {
        let clock = self.source_clocks.get(&id).ok_or("Missing source clock")?;
        if clock.schedule_gain(at, gain) {
            Ok(())
        } else {
            Err("Invalid scheduled gain".into())
        }
    }
    pub fn audio_playheads(&self) -> BTreeMap<u64, rastera_core::recovery::Playhead> {
        self.clocks
            .iter()
            .filter_map(|(&id, clock)| {
                clock.load().map(|observation| {
                    (
                        id,
                        rastera_core::recovery::Playhead {
                            seconds: observation.at(native::host()).max(0) as f64 / 48000.,
                            playing: observation.playing,
                        },
                    )
                })
            })
            .collect()
    }
    pub fn restore_playheads(
        &mut self,
        playheads: BTreeMap<u64, rastera_core::recovery::Playhead>,
    ) {
        self.pending_restore = Some(
            playheads
                .into_iter()
                .filter(|(_, p)| p.seconds.is_finite() && (0.0..=86400.).contains(&p.seconds))
                .take(16)
                .collect(),
        );
        self.last = None;
    }
    /// Recovery copy only; publishing playheads must not edit the live graph or trigger seeks.
    pub fn snapshot_document(&self, document: &mut Document) {
        for (&id, source) in &mut document.audio.sources {
            if matches!(source.origin, Origin::File { .. }) {
                if let Some(observation) = self.clocks.get(&id).and_then(|clock| clock.load()) {
                    source.resume_sample = observation.at(native::host()).max(0);
                    source.playing = observation.playing;
                }
            }
        }
    }
    pub fn save_patches(&mut self) {
        self.store
            .insert(self.namespace.clone(), self.patches.clone());
        if let Some(parent) = self.patch_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&PatchStore {
            version: 1,
            projects: self.store.clone(),
        }) {
            if bytes.len() > 1024 * 1024 {
                error(&mut self.status, "Audio patch store exceeds 1 MiB".into());
                return;
            }
            let temporary = self.patch_file.with_extension("tmp");
            if let Err(message) = std::fs::write(&temporary, bytes)
                .and_then(|()| std::fs::rename(&temporary, &self.patch_file))
            {
                error(
                    &mut self.status,
                    format!("Could not save audio patches: {message}"),
                );
            }
        }
    }
}
fn namespace(machine: &str, project: &str, path: Option<&Path>) -> String {
    let path = path
        .map(|path| path.canonicalize().unwrap_or_else(|_| path.to_owned()))
        .unwrap_or_else(|| PathBuf::from("<unsaved-session>"));
    serde_json::to_string(&(machine, project, path)).unwrap()
}
fn patch_file() -> PathBuf {
    let args: Vec<_> = std::env::args().collect();
    if let Some(root) = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--session-dir="))
    {
        return PathBuf::from(root).join("audio-patches.json");
    }
    if let Some(root) = args
        .iter()
        .find_map(|arg| arg.strip_prefix("--editor-probe="))
    {
        return PathBuf::from(root).join("session/audio-patches.json");
    }
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join("Library/Application Support/Rastera/audio-patches.json")
}
impl Drop for Audio {
    fn drop(&mut self) {
        native::emergency(true);
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.control.take() {
            let _ = worker.join();
        }
    }
}
struct GainState {
    clock: Arc<Clock>,
    smooth: Smooth,
    revision: u64,
    pending: Option<rastera_audio::clock::ScheduledGain>,
    value: f32,
}
impl GainState {
    fn sample(&mut self, host: f64) {
        let observation = self.clock.load();
        if let Some(event) = self.clock.gain_event() {
            if event.revision != self.revision {
                self.revision = event.revision;
                self.pending = Some(event);
            }
        }
        if let (Some(event), Some(observation)) = (self.pending, observation) {
            if event.generation != observation.generation {
                self.pending = None;
            } else if observation.at(host) >= event.at {
                self.smooth.target(event.gain);
                self.pending = None;
            }
        }
        self.value = self.smooth.next_sample();
    }
}
struct Fade {
    mixer: Box<Mixer>,
    update: Box<Update>,
    positions: BTreeMap<u64, usize>,
    inputs: BTreeMap<u64, FileInput>,
    previous: [[f32; 2]; MAX_STRIPS],
    remaining: usize,
}
fn layout(model: &Model) -> BTreeMap<u64, Vec<(u64, Vec<rastera_audio::model::Channel>)>> {
    model
        .sources
        .iter()
        .map(|(&id, source)| {
            (
                id,
                source
                    .active
                    .iter()
                    .filter_map(|sid| {
                        source
                            .strips
                            .iter()
                            .find(|strip| strip.id == *sid)
                            .map(|strip| (*sid, strip.mapping.clone()))
                    })
                    .collect(),
            )
        })
        .collect()
}
fn configure_mutes(mixer: &mut Mixer, positions: &BTreeMap<u64, usize>, update: &Update) {
    for (&id, source) in &update.desired.model.sources {
        let transport_plays = !matches!(source.origin, Origin::File { .. } | Origin::Playback(_))
            || update
                .clocks
                .get(&id)
                .and_then(|clock| clock.load())
                .is_some_and(|clock| clock.playing);
        for sid in &source.active {
            if let (Some(&slot), Some(strip)) = (
                positions.get(sid),
                source.strips.iter().find(|strip| strip.id == *sid),
            ) {
                let mut dsp = strip.dsp.clone();
                dsp.mute |= !source.enabled
                    || !update.desired.participating.contains(&id)
                    || !transport_plays
                    || !update.desired.active;
                mixer.sources[slot].configure(&dsp);
            }
        }
    }
}
fn source_values(
    update: &Update,
    positions: &BTreeMap<u64, usize>,
    banks: (&mut BTreeMap<u64, FileInput>, &mut BTreeMap<u64, FileInput>),
    live: &BTreeMap<String, LiveHistory>,
    gains: &BTreeMap<u64, GainState>,
    previous: &mut [[f32; 2]; MAX_STRIPS],
    host: f64,
) -> [[f32; 2]; MAX_STRIPS] {
    let (inputs, alternate) = banks;
    let mut values = [[0.; 2]; MAX_STRIPS];
    for (&id, source) in &update.desired.model.sources {
        let clock = update.clocks.get(&id).and_then(|clock| clock.load());
        let playing = !matches!(source.origin, Origin::File { .. } | Origin::Playback(_))
            || clock.is_some_and(|clock| clock.playing);
        let file = if inputs.contains_key(&id) {
            inputs.get_mut(&id)
        } else {
            alternate.get_mut(&id)
        };
        let mut file = file;
        let gain = gains.get(&id).map_or(1., |state| state.value);
        for sid in &source.active {
            let Some(&slot) = positions.get(sid) else {
                continue;
            };
            let Some(strip) = source.strips.iter().find(|strip| strip.id == *sid) else {
                continue;
            };
            if !playing {
                // Keep a short click-suppression tail, never hold a DC sample indefinitely.
                previous[slot] = previous[slot].map(|sample| {
                    let value = sample * 0.99;
                    if value.abs() < 1e-8 {
                        0.
                    } else {
                        value
                    }
                });
                values[slot] = previous[slot];
                continue;
            }
            for (channel, mapping) in strip.mapping.iter().enumerate() {
                let value = if let (Some(observation), Some(input)) = (clock, file.as_mut()) {
                    let offset = input.offset.load(Ordering::Relaxed);
                    let position = observation.at(host) - i64::from(offset);
                    let generation = packet_generation(observation.generation, offset);
                    input.tracks.get_mut(&mapping.track).map_or(0., |cursor| {
                        cursor.at(position, mapping.channel, generation)
                    })
                } else if matches!(source.origin, Origin::Input { .. } | Origin::Capture(_)) {
                    let uid = match &source.origin {
                        Origin::Input { uid } => uid.as_str(),
                        _ => &source.input_uid,
                    };
                    if source.sync_ms < -20. {
                        0.
                    } else {
                        live.get(uid).map_or(0., |history| {
                            history.at(
                                host - 0.12 - f64::from(source.sync_ms) / 1000.,
                                mapping.channel as usize,
                            )
                        })
                    }
                } else {
                    0.
                };
                values[slot][channel] = value * gain;
            }
            if strip.mapping.len() == 1 {
                values[slot][1] = values[slot][0];
            }
            previous[slot] = values[slot];
        }
    }
    values
}
fn render(
    updates: mpsc::Receiver<Box<Update>>,
    retired: mpsc::SyncSender<Retired>,
    meters: mpsc::SyncSender<Status>,
    stop: Arc<AtomicBool>,
) {
    let mut current: Option<Box<Update>> = None;
    let mut inputs = BTreeMap::<u64, FileInput>::new();
    let mut mixer = Box::new(Mixer::default());
    let mut positions = BTreeMap::<u64, usize>::new();
    let mut previous = [[0.; 2]; MAX_STRIPS];
    let mut gains = BTreeMap::<u64, GainState>::new();
    let mut fade: Option<Fade> = None;
    let mut returned = VecDeque::<Retired>::with_capacity(4);
    let mut status = Status::default();
    let mut last_meter = Instant::now();
    let start = native::host() + 0.08;
    let mut sample = 0u64;
    let mut physical = [0f32; BLOCK * 32];
    let mut live = BTreeMap::<String, LiveHistory>::new();
    let mut empty_inputs = BTreeMap::new();
    while !stop.load(Ordering::Acquire) {
        while let Some(retirement) = returned.pop_front() {
            match retired.try_send(retirement) {
                Ok(()) => {}
                Err(mpsc::TrySendError::Full(retirement)) => {
                    returned.push_front(retirement);
                    break;
                }
                Err(_) => break,
            }
        }
        // One 10 ms transition at a time. The bounded update queue coalesces control-side work.
        if fade.is_none() && returned.len() < 3 {
            if let Ok(mut next) = updates.try_recv() {
                let structural = current.as_ref().is_some_and(|old| {
                    old.desired.runtime != next.desired.runtime
                        || layout(&old.desired.model) != layout(&next.desired.model)
                        || !next.add.is_empty()
                        || !next.remove.is_empty()
                });
                let old_mixer = structural.then(|| Box::new((*mixer).clone()));
                let old_positions = structural.then(|| positions.clone());
                let mut old_inputs = BTreeMap::new();
                for (id, input) in std::mem::take(&mut next.add) {
                    if let Some(old) = inputs.insert(id, input) {
                        old_inputs.insert(id, old);
                    }
                }
                for id in &next.remove {
                    if let Some(old) = inputs.remove(id) {
                        old_inputs.insert(*id, old);
                    }
                }
                let new_namespace = current
                    .as_ref()
                    .is_some_and(|old| old.desired.runtime != next.desired.runtime);
                let active_ids: BTreeSet<_> = next
                    .desired
                    .model
                    .sources
                    .values()
                    .flat_map(|source| source.active.iter().copied())
                    .collect();
                let removed_slots: Vec<_> = positions
                    .iter()
                    .filter(|(id, _)| new_namespace || !active_ids.contains(*id))
                    .map(|(_, &slot)| slot)
                    .collect();
                for slot in removed_slots {
                    mixer.clear_slot(slot);
                    previous[slot] = [0.; 2];
                }
                positions.retain(|id, _| !new_namespace && active_ids.contains(id));
                if new_namespace {
                    gains.clear();
                }
                for (&id, source) in &next.desired.model.sources {
                    if let Some(clock) = next.clocks.get(&id) {
                        gains
                            .entry(id)
                            .and_modify(|state| state.clock = clock.clone())
                            .or_insert_with(|| GainState {
                                clock: clock.clone(),
                                smooth: Smooth::new(1.),
                                revision: 0,
                                pending: None,
                                value: 1.,
                            });
                    }
                    for sid in &source.active {
                        let slot = if let Some(&slot) = positions.get(sid) {
                            slot
                        } else {
                            let slot = (0..MAX_STRIPS)
                                .find(|slot| !positions.values().any(|used| used == slot))
                                .expect("validated strip count");
                            mixer.clear_slot(slot);
                            positions.insert(*sid, slot);
                            slot
                        };
                        if let Some(strip) = source.strips.iter().find(|strip| strip.id == *sid) {
                            mixer.configure(slot, strip);
                        }
                    }
                }
                for (index, bus) in next.desired.model.buses.iter().enumerate() {
                    mixer.buses[index].configure(&bus.dsp);
                }
                configure_mutes(&mut mixer, &positions, &next);
                // Keep a live device's history during a fader edit; rebuilding it would cause a gap.
                for (uid, device) in &next.devices {
                    if next.input_uids.contains(uid) && device.info.input_channels > 0 {
                        let replace = live
                            .get(uid)
                            .is_none_or(|history| history.device != Arc::as_ptr(device) as usize);
                        if replace {
                            live.insert(uid.clone(), LiveHistory::new(device));
                        }
                    }
                }
                let old = current.replace(next);
                match (old, old_mixer, old_positions) {
                    (Some(old), Some(old_mixer), Some(old_positions)) => {
                        fade = Some(Fade {
                            mixer: old_mixer,
                            update: old,
                            positions: old_positions,
                            inputs: old_inputs,
                            previous,
                            remaining: FADE_SAMPLES,
                        });
                    }
                    (Some(old), _, _) => returned.push_back(Retired {
                        version: old.version,
                        inputs: old_inputs.into_iter().collect(),
                        update: Some(old),
                    }),
                    _ => {}
                }
                live.retain(|uid, _| {
                    current
                        .as_ref()
                        .is_some_and(|config| config.devices.contains_key(uid))
                        || fade
                            .as_ref()
                            .is_some_and(|fade| fade.update.devices.contains_key(uid))
                });
            }
        }
        let target = start + sample as f64 / 48000.;
        let now = native::host();
        if target - now > 0.08 {
            thread::sleep(Duration::from_micros(500));
            continue;
        }
        if target < now {
            status.late_blocks += 1;
            let next = ((now + 0.02 - start) * 48000.).max(0.) as u64;
            status.skipped_samples += next.saturating_sub(sample);
            sample = next;
            continue;
        }
        let Some(config) = current.as_ref() else {
            thread::sleep(Duration::from_millis(2));
            continue;
        };
        for (&id, input) in &mut inputs {
            if let Some(observation) = config
                .clocks
                .get(&id)
                .and_then(|clock| clock.load())
                .filter(|o| !o.playing)
            {
                let generation =
                    packet_generation(observation.generation, input.offset.load(Ordering::Relaxed));
                for cursor in input.tracks.values_mut() {
                    cursor.synchronize(generation);
                }
            }
        }
        configure_mutes(&mut mixer, &positions, config);
        if let Some(fade) = &mut fade {
            configure_mutes(&mut fade.mixer, &fade.positions, &fade.update);
        }
        for (uid, history) in &mut live {
            if let Some(device) = config
                .devices
                .get(uid)
                .or_else(|| fade.as_ref().and_then(|fade| fade.update.devices.get(uid)))
            {
                history.update(device);
            }
        }
        let mut buses = [[[0f32; 2]; 10]; BLOCK];
        for (index, frame) in buses.iter_mut().enumerate() {
            let host = target + index as f64 / 48000.;
            for gain in gains.values_mut() {
                gain.sample(host);
            }
            let values = source_values(
                config,
                &positions,
                (&mut inputs, &mut empty_inputs),
                &live,
                &gains,
                &mut previous,
                host,
            );
            *frame = mixer.process(&values);
            if let Some(old) = &mut fade {
                let old_values = source_values(
                    &old.update,
                    &old.positions,
                    (&mut old.inputs, &mut inputs),
                    &live,
                    &gains,
                    &mut old.previous,
                    host,
                );
                let old_frame = old.mixer.process(&old_values);
                let mix = 1. - old.remaining as f32 / FADE_SAMPLES as f32;
                for bus in 0..10 {
                    for channel in 0..2 {
                        frame[bus][channel] =
                            old_frame[bus][channel] * (1. - mix) + frame[bus][channel] * mix;
                    }
                }
                old.remaining = old.remaining.saturating_sub(1);
            }
        }
        for (uid, device) in &config.devices {
            let channels = device.info.output_channels as usize;
            if channels == 0 || channels > 32 {
                continue;
            }
            physical.fill(0.);
            for (&bus, patch) in &config.desired.patches {
                if patch.uid != *uid
                    || !(1..=10).contains(&bus)
                    || patch.first as usize + 1 >= channels
                {
                    continue;
                }
                for index in 0..BLOCK {
                    for channel in 0..2 {
                        physical[index * channels + patch.first as usize + channel] +=
                            buses[index][bus as usize - 1][channel];
                    }
                }
            }
            device.write(target, &physical[..BLOCK * channels], channels);
        }
        if fade.as_ref().is_some_and(|fade| fade.remaining == 0) {
            let old = fade.take().unwrap();
            returned.push_back(Retired {
                version: old.update.version,
                inputs: old.inputs.into_iter().collect(),
                update: Some(old.update),
            });
            gains.retain(|id, _| config.desired.model.sources.contains_key(id));
            live.retain(|uid, _| config.devices.contains_key(uid));
        }
        sample += BLOCK as u64;
        status.samples += BLOCK as u64;
        status.timeline_sample = sample;
        if last_meter.elapsed() >= Duration::from_millis(100) {
            status.meters.clear();
            for (&id, &slot) in &positions {
                status.meters.insert(id, mixer.sources[slot].meter);
                mixer.sources[slot].clear_meters();
            }
            for (index, bus) in mixer.buses.iter_mut().enumerate() {
                status.meters.insert(index as u64 + 1, bus.meter);
                bus.clear_meters();
            }
            status.stats = config
                .devices
                .iter()
                .map(|(uid, device)| (uid.clone(), device.stats()))
                .collect();
            status.source_queues = inputs
                .iter()
                .map(|(&id, input)| {
                    (
                        id,
                        (
                            input.tracks.values().map(|cursor| cursor.rx.queued()).sum(),
                            input
                                .tracks
                                .values()
                                .map(|cursor| cursor.rx.capacity())
                                .sum(),
                        ),
                    )
                })
                .collect();
            status.source_stats = inputs
                .iter()
                .map(|(&id, input)| {
                    (
                        id,
                        [
                            input.counters.missing_samples.load(Ordering::Relaxed),
                            input.counters.backpressure.load(Ordering::Relaxed),
                            input.counters.decoded_frames.load(Ordering::Relaxed),
                            input.counters.discontinuities.load(Ordering::Relaxed),
                        ],
                    )
                })
                .collect();
            let _ = meters.try_send(status.clone());
            last_meter = Instant::now();
        }
    }
    if let Some(fade) = fade {
        returned.push_back(Retired {
            version: fade.update.version,
            inputs: fade.inputs.into_iter().collect(),
            update: Some(fade.update),
        });
    }
    returned.push_back(Retired {
        version: u64::MAX,
        inputs: inputs.into_iter().collect(),
        update: current,
    });
    // Control drains until this worker finishes, so final sends cannot deadlock shutdown.
    for retirement in returned {
        if retired.send(retirement).is_err() {
            break;
        }
    }
}
struct LiveHistory {
    device: usize,
    data: Box<[[f32; 32]]>,
    times: Box<[f64]>,
    written: u64,
    rate: f64,
    channels: usize,
    scratch: [f32; BLOCK * 32],
}
impl LiveHistory {
    fn new(device: &Arc<native::Device>) -> Self {
        Self {
            device: Arc::as_ptr(device) as usize,
            data: vec![[0.; 32]; 32768].into_boxed_slice(),
            times: vec![0.; 32768].into_boxed_slice(),
            written: 0,
            rate: 48000.,
            channels: device.info.input_channels as usize,
            scratch: [0.; BLOCK * 32],
        }
    }
    fn update(&mut self, device: &native::Device) {
        for _ in 0..16 {
            let (frames, host) =
                device.read(&mut self.scratch[..BLOCK * self.channels], self.channels);
            if frames == 0 {
                break;
            }
            for index in 0..frames {
                self.push(host + index as f64 / self.rate, index);
            }
        }
    }
    fn push(&mut self, host: f64, source: usize) {
        let slot = self.written as usize % self.data.len();
        self.data[slot][..self.channels]
            .copy_from_slice(&self.scratch[source * self.channels..(source + 1) * self.channels]);
        self.times[slot] = host;
        self.written += 1;
    }
    fn at(&self, host: f64, channel: usize) -> f32 {
        if self.written < 2 || channel >= self.channels {
            return 0.;
        }
        let last = (self.written - 1) as usize % self.data.len();
        let behind = (self.times[last] - host) * self.rate;
        if behind < 0. || behind > self.data.len() as f64 - 2. || behind > self.written as f64 - 1.
        {
            return 0.;
        }
        let offset = behind.floor() as u64;
        let a = (self.written - 1 - offset) as usize % self.data.len();
        let b = (self.written - 2 - offset) as usize % self.data.len();
        if (self.times[a] - self.times[b] - 1. / self.rate).abs() > 0.5 / self.rate {
            return 0.;
        }
        self.data[a][channel]
            + (self.data[b][channel] - self.data[a][channel]) * behind.fract() as f32
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_packets_are_discarded_on_seek_without_poisoning_future_positions() {
        let (mut tx, rx) = queue::bounded(4, Packet::default());
        let mut packet = Packet {
            start: 1000,
            frames: 2,
            channels: 2,
            generation: 5,
            ..Packet::default()
        };
        packet.data[..4].copy_from_slice(&[0.1, 0.2, 0.3, 0.4]);
        assert!(tx.push(packet));
        packet.start = 0;
        packet.generation = 6;
        packet.data[..4].copy_from_slice(&[0.5, 0.6, 0.7, 0.8]);
        assert!(tx.push(packet));
        let mut cursor = Cursor {
            rx,
            current: Packet::default(),
            counters: Arc::new(SourceCounters::default()),
        };
        assert_eq!(cursor.at(1000, 0, 5), 0.1);
        cursor.synchronize(6);
        assert_eq!(cursor.at(0, 0, 6), 0.5);
        assert_eq!(cursor.at(0, 1, 6), 0.6);
        assert_eq!(cursor.at(1, 0, 6), 0.7);
        assert_eq!(cursor.at(1, 1, 6), 0.8);
        assert_eq!(cursor.at(1, 3, 6), 0.);
    }
    #[test]
    fn sync_offset_is_part_of_packet_epoch_even_when_transport_does_not_restart() {
        assert_ne!(packet_generation(8, -24000), packet_generation(8, 0));
        assert_ne!(packet_generation(8, 0), packet_generation(8, 24000));
        assert_ne!(packet_generation(8, 12), packet_generation(9, 12));
        assert_eq!(offset_samples(0.5), 24);
        assert_eq!(offset_samples(-0.5), -24);
    }
    #[test]
    fn physical_patches_are_scoped_to_project_path_machine_and_document_identity() {
        let a = namespace(
            "machine-a",
            "project-a",
            Some(Path::new("/tmp/show-a.rastera.json")),
        );
        assert_ne!(
            a,
            namespace(
                "machine-a",
                "project-a",
                Some(Path::new("/tmp/show-b.rastera.json"))
            )
        );
        assert_ne!(
            a,
            namespace(
                "machine-b",
                "project-a",
                Some(Path::new("/tmp/show-a.rastera.json"))
            )
        );
        assert_ne!(
            a,
            namespace(
                "machine-a",
                "project-b",
                Some(Path::new("/tmp/show-a.rastera.json"))
            )
        );
        assert_ne!(a, namespace("machine-a", "project-a", None));
    }
    #[test]
    fn status_errors_are_bounded_and_repeated_latest_error_is_coalesced() {
        let mut status = Status::default();
        for i in 0..100 {
            error(&mut status, format!("Missing {i}"));
        }
        assert_eq!(status.errors.len(), MAX_ERRORS);
        error(&mut status, "Missing 99".into());
        assert_eq!(status.errors.len(), MAX_ERRORS);
    }
    #[test]
    fn file_only_transport_preserves_pause_and_applies_seek_stop_restart() {
        let mut model = Model::default();
        let id = model
            .add(
                Origin::File {
                    path: PathBuf::from("/tmp/transport-test.wav"),
                    duration: 20.,
                },
                "Test".into(),
                vec![rastera_audio::model::Channel {
                    track: 1,
                    channel: 0,
                }],
            )
            .unwrap();
        let mut source = model.sources[&id].clone();
        let clock = Arc::new(Clock::default());
        let mut actor = FileTransport::new(&source, clock.clone());
        let wait = |predicate: &dyn Fn(Observation) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(1);
            loop {
                if let Some(observation) = clock.load() {
                    if predicate(observation) {
                        return observation;
                    }
                }
                assert!(Instant::now() < deadline, "transport actor timed out");
                thread::sleep(Duration::from_millis(1));
            }
        };
        wait(&|o| o.generation > 0 && !o.playing);
        source.playing = true;
        actor.configure(&source, false);
        wait(&|o| o.playing && o.sample > 0);
        source.playing = false;
        actor.configure(&source, false);
        let paused = wait(&|o| !o.playing);
        thread::sleep(Duration::from_millis(15));
        assert_eq!(clock.load().unwrap().sample, paused.sample);
        source.resume_sample = 48000;
        actor.configure(&source, false);
        wait(&|o| o.sample == 48000 && !o.playing);
        actor
            .tx
            .send(FileCommand::Transport(Command::Restart))
            .unwrap();
        wait(&|o| o.playing && o.sample < 4800);
        actor
            .tx
            .send(FileCommand::Transport(Command::Stop))
            .unwrap();
        wait(&|o| !o.playing && o.sample == 0);
    }
}

// Control-plane latest-state transport; native media worker remains the sole clock writer.
fn video_command(control: &rastera_core::media::Control, command: Command) -> Result<(), String> {
    if let Command::Gain { at, value } = command {
        return if control.audio_clock.schedule_gain(at, value) {
            Ok(())
        } else {
            Err("Invalid scheduled gain".into())
        };
    }
    let mut state = control
        .state
        .lock()
        .map_err(|_| "Video transport unavailable")?;
    match command {
        Command::Play => state.playing = true,
        Command::Pause => state.playing = false,
        Command::Stop => {
            state.playing = false;
            state.seek = Some(0.);
        }
        Command::Restart => {
            state.playing = true;
            state.seek = Some(0.);
        }
        Command::Seek(sample) => {
            if sample < 0 || sample as f64 > state.duration * 48000. {
                return Err("Seek outside source duration".into());
            }
            state.seek = Some(sample as f64 / 48000.);
        }
        Command::Gain { .. } => unreachable!(),
    }
    Ok(())
}
#[cfg(test)]
mod video_transport_tests {
    use super::*;
    #[test]
    fn shared_video_commands_keep_one_transport_and_validate_seek() {
        let hub = rastera_core::capture::Hub::default();
        let c = rastera_core::media::Control::new(hub.slot("test"), true, 6.);
        video_command(&c, Command::Pause).unwrap();
        assert!(!c.state.lock().unwrap().playing);
        video_command(&c, Command::Seek(48000)).unwrap();
        assert_eq!(c.state.lock().unwrap().seek, Some(1.));
        assert!(video_command(&c, Command::Seek(48000 * 7)).is_err());
        video_command(&c, Command::Restart).unwrap();
        {
            let s = c.state.lock().unwrap();
            assert!(s.playing);
            assert_eq!(s.seek, Some(0.));
        }
        video_command(&c, Command::Stop).unwrap();
        assert!(!c.state.lock().unwrap().playing);
    }
}
