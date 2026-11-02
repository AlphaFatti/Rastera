use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EqBand {
    pub hz: f32,
    pub db: f32,
    pub q: f32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compressor {
    pub bypass: bool,
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub knee_db: f32,
    pub makeup_db: f32,
}
impl Default for Compressor {
    fn default() -> Self {
        Self {
            bypass: true,
            threshold_db: -18.,
            ratio: 4.,
            attack_ms: 10.,
            release_ms: 100.,
            knee_db: 6.,
            makeup_db: 0.,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dsp {
    pub bypass: bool,
    pub trim_db: f32,
    pub high_pass_hz: f32,
    pub eq: [EqBand; 4],
    pub compressor: Compressor,
    pub fader_db: f32,
    pub mute: bool,
    pub limiter_db: Option<f32>,
}
impl Default for Dsp {
    fn default() -> Self {
        Self {
            bypass: false,
            trim_db: 0.,
            high_pass_hz: 0.,
            eq: [100., 500., 2500., 10000.].map(|hz| EqBand {
                hz,
                db: 0.,
                q: 0.707,
            }),
            compressor: Compressor::default(),
            fader_db: 0.,
            mute: false,
            limiter_db: None,
        }
    }
}
impl Dsp {
    pub fn valid(&self) -> bool {
        let range = |v: f32, lo, hi| v.is_finite() && (lo..=hi).contains(&v);
        range(self.trim_db, -96., 24.)
            && range(self.fader_db, -96., 12.)
            && range(self.high_pass_hz, 0., 20000.)
            && self
                .eq
                .iter()
                .all(|e| range(e.hz, 20., 20000.) && range(e.db, -24., 24.) && range(e.q, 0.1, 18.))
            && range(self.compressor.threshold_db, -80., 0.)
            && range(self.compressor.ratio, 1., 40.)
            && range(self.compressor.attack_ms, 0.1, 1000.)
            && range(self.compressor.release_ms, 1., 5000.)
            && range(self.compressor.knee_db, 0., 24.)
            && range(self.compressor.makeup_db, 0., 24.)
            && self.limiter_db.is_none_or(|x| range(x, -24., 0.))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Interpretation {
    Mono,
    Stereo,
    DualMono,
    DualStereo,
    QuadMono,
}
impl Interpretation {
    pub fn channels(self) -> usize {
        match self {
            Self::Mono => 1,
            Self::Stereo | Self::DualMono => 2,
            _ => 4,
        }
    }
}
/// Track ID and zero-based channel. Never an enumeration index or a downmixed channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub track: i32,
    pub channel: u32,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Strip {
    pub id: u64,
    pub name: String,
    pub mapping: Vec<Channel>,
    pub pan: f32,
    pub dsp: Dsp,
    pub sends: [f32; 10],
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Origin {
    Playback(u64),
    Capture(u64),
    File {
        path: std::path::PathBuf,
        duration: f64,
    },
    Input {
        uid: String,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub origin: Origin,
    pub name: String,
    pub always_on: bool,
    pub restart_on_activation: bool,
    pub scenes: Vec<u64>,
    pub playing: bool,
    pub looping: bool,
    pub resume_sample: i64,
    pub input_uid: String,
    pub enabled: bool,
    pub sync_ms: f32,
    pub interpretation: Interpretation,
    pub channels: Vec<Channel>,
    pub active: Vec<u64>,
    pub strips: Vec<Strip>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bus {
    pub id: u64,
    pub name: String,
    pub dsp: Dsp,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub sources: BTreeMap<u64, Source>,
    pub buses: [Bus; 10],
    pub next_id: u64,
}
impl Default for Model {
    fn default() -> Self {
        Self {
            sources: BTreeMap::new(),
            buses: std::array::from_fn(|i| Bus {
                id: i as u64 + 1,
                name: format!("Bus {}", i + 1),
                dsp: Dsp {
                    limiter_db: Some(-1.),
                    ..Dsp::default()
                },
            }),
            next_id: 11,
        }
    }
}
impl Model {
    fn allocate(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
    /// Transactional control-thread operation; failure retains all definitions and allocator state.
    pub fn add(
        &mut self,
        origin: Origin,
        name: String,
        channels: Vec<Channel>,
    ) -> Result<u64, String> {
        self.validate()?;
        if self.sources.len() >= 16 {
            return Err("Audio source limit: 16".into());
        }
        if matches!(origin, Origin::Playback(_) | Origin::Capture(_))
            && self.sources.values().any(|s| s.origin == origin)
        {
            return Err("This shared playback/capture already has an audio source".into());
        }
        let mode = match channels.len() {
            1 => Interpretation::Mono,
            2 => Interpretation::Stereo,
            4 => Interpretation::DualStereo,
            _ => return Err("Select 1, 2 or 4 discrete channels".into()),
        };
        let mut candidate = self.clone();
        let id = candidate.allocate();
        candidate.sources.insert(
            id,
            Source {
                origin,
                name,
                always_on: false,
                restart_on_activation: false,
                scenes: vec![],
                playing: false,
                looping: false,
                resume_sample: 0,
                input_uid: String::new(),
                enabled: false,
                sync_ms: 0.,
                interpretation: mode,
                channels: vec![],
                active: vec![],
                strips: vec![],
            },
        );
        candidate.interpret_inner(id, mode, channels)?;
        candidate.validate()?;
        *self = candidate;
        Ok(id)
    }
    /// Existing mappings recover their exact identity/settings. Newly split/merged mappings
    /// inherit the first overlapping strip's settings, with a new persistent identity.
    pub fn interpret(
        &mut self,
        id: u64,
        mode: Interpretation,
        channels: Vec<Channel>,
    ) -> Result<(), String> {
        self.validate()?;
        let mut candidate = self.clone();
        candidate.interpret_inner(id, mode, channels)?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    fn interpret_inner(
        &mut self,
        id: u64,
        mode: Interpretation,
        channels: Vec<Channel>,
    ) -> Result<(), String> {
        if !valid_channels(&channels) || channels.len() != mode.channels() {
            return Err(
                "Select distinct explicit track/channel mappings (1, 2 or 4 channels)".into(),
            );
        }
        let groups = groups(mode, &channels);
        let active_count = self.sources.values().map(|s| s.active.len()).sum::<usize>();
        let source = self.sources.get(&id).ok_or("Missing audio source")?;
        let missing = groups
            .iter()
            .filter(|g| !source.strips.iter().any(|s| s.mapping == **g))
            .count();
        if source.strips.len() + missing > 16 {
            return Err("16 retained interpretation strips per source; remove source to clear dormant mappings".into());
        }
        if active_count - source.active.len() + groups.len() > 64 {
            return Err("64 active strips maximum".into());
        }
        if self.next_id > u64::MAX - 512 {
            return Err("Audio identity allocator exhausted".into());
        }
        let source = self.sources.get_mut(&id).unwrap();
        // Retain the old active mappings for deterministic transfer, before activating a new set.
        let previous = source
            .active
            .iter()
            .filter_map(|id| source.strips.iter().find(|s| s.id == *id))
            .cloned()
            .collect::<Vec<_>>();
        source.active.clear();
        for group in groups {
            let sid = if let Some(s) = source.strips.iter().find(|s| s.mapping == group) {
                s.id
            } else {
                let sid = self.next_id;
                self.next_id += 1;
                let inherited = previous
                    .iter()
                    .find(|s| s.mapping.iter().any(|c| group.contains(c)));
                let label = group
                    .iter()
                    .map(|c| format!("T{} Ch{}", c.track, c.channel + 1))
                    .collect::<Vec<_>>()
                    .join("/");
                source.strips.push(Strip {
                    id: sid,
                    name: format!(
                        "{} — {}",
                        source.name.chars().take(48).collect::<String>(),
                        label
                    ),
                    mapping: group,
                    pan: inherited.map_or(0., |s| s.pan),
                    dsp: inherited.map_or_else(Dsp::default, |s| s.dsp.clone()),
                    sends: inherited.map_or_else(
                        || std::array::from_fn(|b| if b == 0 { 1. } else { 0. }),
                        |s| s.sends,
                    ),
                });
                sid
            };
            source.active.push(sid);
        }
        source.channels = channels;
        source.interpretation = mode;
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        let mut add = |id| id > 0 && id < self.next_id && ids.insert(id);
        if self.next_id < 11
            || self.next_id > u64::MAX - 512
            || self.sources.len() > 16
            || self.sources.values().map(|s| s.active.len()).sum::<usize>() > 64
            || self
                .buses
                .iter()
                .enumerate()
                .any(|(i, b)| b.id != i as u64 + 1 || b.name.len() > 256 || !b.dsp.valid())
        {
            return Err("Invalid audio buses/source count or allocator".into());
        }
        for bus in &self.buses {
            if !add(bus.id) {
                return Err("Audio identity collision".into());
            }
        }
        let mut shared_origins = Vec::new();
        for (&id, s) in &self.sources {
            let valid_origin = match &s.origin {
                Origin::Playback(id) | Origin::Capture(id) => *id > 0,
                Origin::File { path, duration } => {
                    !path.as_os_str().is_empty()
                        && path.as_os_str().as_encoded_bytes().len() <= 8192
                        && duration.is_finite()
                        && (0.0..=86400.).contains(duration)
                        && *duration > 0.
                }
                Origin::Input { uid } => !uid.is_empty() && uid.len() <= 1024,
            };
            if matches!(s.origin, Origin::Playback(_) | Origin::Capture(_)) {
                if shared_origins.contains(&s.origin) {
                    return Err("Duplicate shared audio source".into());
                }
                shared_origins.push(s.origin.clone());
            }
            if !add(id)
                || !valid_origin
                || s.name.len() > 256
                || s.input_uid.len() > 1024
                || !s.sync_ms.is_finite()
                || s.sync_ms.abs() > 500.
                || s.resume_sample < 0
                || s.resume_sample > 86400 * 48000
                || s.scenes.len() > 16
                || s.scenes
                    .iter()
                    .enumerate()
                    .any(|(i, id)| *id == 0 || s.scenes[..i].contains(id))
                || s.channels.len() != s.interpretation.channels()
                || !valid_channels(&s.channels)
                || s.strips.len() > 16
                || s.active.len() > 4
            {
                return Err("Invalid audio source".into());
            }
            for (index, st) in s.strips.iter().enumerate() {
                if !add(st.id)
                    || st.name.len() > 256
                    || !(1..=2).contains(&st.mapping.len())
                    || !valid_channels(&st.mapping)
                    || s.strips[..index]
                        .iter()
                        .any(|other| other.mapping == st.mapping)
                    || !st.pan.is_finite()
                    || st.pan.abs() > 1.
                    || !st.dsp.valid()
                    || st
                        .sends
                        .iter()
                        .any(|v| !v.is_finite() || !(0.0..=2.).contains(v))
                {
                    return Err("Invalid audio strip".into());
                }
            }
            let expected = groups(s.interpretation, &s.channels);
            if s.active.len() != expected.len()
                || s.active.iter().zip(expected).any(|(id, mapping)| {
                    !s.strips
                        .iter()
                        .any(|st| st.id == *id && st.mapping == mapping)
                })
            {
                return Err("Active strip mapping does not match channel interpretation".into());
            }
        }
        Ok(())
    }
}
fn groups(mode: Interpretation, channels: &[Channel]) -> Vec<Vec<Channel>> {
    let count = if matches!(mode, Interpretation::Stereo | Interpretation::DualStereo) {
        2
    } else {
        1
    };
    channels.chunks(count).map(|c| c.to_vec()).collect()
}
fn valid_channels(channels: &[Channel]) -> bool {
    !channels.is_empty()
        && channels
            .iter()
            .enumerate()
            .all(|(i, c)| c.track >= 0 && c.channel < 32 && !channels[..i].contains(c))
}
