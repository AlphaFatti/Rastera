//! Bounded recovery journals, separate from user project saves. No GPU or platform calls.
use crate::scene::{Document, Id};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
pub const MAX_BYTES: u64 = 16 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub restore_last: bool,
    pub restore_outputs: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            restore_last: true,
            restore_outputs: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Playhead {
    pub seconds: f64,
    pub playing: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Output {
    pub fullscreen: bool,
    pub display: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: u32,
    pub document: Document,
    pub saved: Option<Document>,
    pub project: Option<PathBuf>,
    pub selected: Option<Id>,
    pub viewport: Option<u64>,
    pub viewports_mode: bool,
    pub window: Option<[f32; 4]>,
    #[serde(default)]
    pub clean_windows: BTreeMap<u64, [f64; 4]>,
    #[serde(default)]
    pub panel_rects: BTreeMap<String, [f32; 4]>,
    pub playheads: BTreeMap<Id, Playhead>,
    pub machine: String,
    pub enabled: BTreeMap<u64, Output>,
}
impl Snapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("Unsupported recovery schema".into());
        }
        self.document.validate()?;
        if let Some(d) = &self.saved {
            d.validate()?;
        }
        if self.clean_windows.len() > 8
            || self.clean_windows.values().any(|r| {
                r.iter().any(|n| !n.is_finite())
                    || r[2] <= 0.
                    || r[3] <= 0.
                    || r[2] > 32768.
                    || r[3] > 32768.
            })
            || self.panel_rects.len() > 4
            || self.panel_rects.iter().any(|(key, r)| {
                ![
                    "editor_layers",
                    "editor_inspector",
                    "viewport_list",
                    "viewport_inspector",
                ]
                .contains(&key.as_str())
                    || r.iter().any(|n| !n.is_finite())
            })
        {
            return Err("Invalid recovered window/panel layout".into());
        }
        if self.window.is_some_and(|r| {
            r.iter().any(|x| !x.is_finite())
                || r[2] < 200.
                || r[3] < 200.
                || r[2] > 32768.
                || r[3] > 32768.
        }) || self.playheads.len() > 4
            || self
                .playheads
                .values()
                .any(|p| !p.seconds.is_finite() || !(0.0..=86400.).contains(&p.seconds))
            || self.enabled.len() > 8
            || self.project.as_ref().is_some_and(|p| !p.is_absolute())
        {
            return Err("Invalid recovery metadata".into());
        }
        Ok(())
    }
}
pub fn read_snapshot(path: &Path) -> Result<Snapshot, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_BYTES {
        return Err("Recovery exceeds 16 MiB".into());
    }
    let s: Snapshot = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    s.validate()?;
    Ok(s)
}
pub fn load(root: &Path) -> (Option<Snapshot>, Option<String>) {
    let current = root.join("current.json");
    let previous = root.join("previous.json");
    if !current.exists() && !previous.exists() {
        return (None, None);
    }
    match read_snapshot(&current) {
        Ok(s) => (Some(s), None),
        Err(e) => match read_snapshot(&previous) {
            Ok(s) => (
                Some(s),
                Some(format!(
                    "Current recovery invalid ({e}); recovered previous snapshot"
                )),
            ),
            Err(_) => (
                None,
                Some(format!(
                    "Recovery unavailable ({e}); starting an empty session"
                )),
            ),
        },
    }
}
fn atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut f = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
    f.write_all(bytes)
        .and_then(|_| f.sync_all())
        .map_err(|e| e.to_string())?;
    std::fs::rename(&temp, path).map_err(|e| e.to_string())
}
pub fn store(root: &Path, prefs: &Preferences, snapshot: &Snapshot) -> Result<(), String> {
    snapshot.validate()?;
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    atomic(
        &root.join("preferences.json"),
        &serde_json::to_vec(prefs).map_err(|e| e.to_string())?,
    )?;
    if !prefs.restore_last {
        return Ok(());
    }
    let bytes = serde_json::to_vec(snapshot).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Recovery exceeds 16 MiB; project remains unchanged".into());
    }
    let current = root.join("current.json");
    if read_snapshot(&current).is_ok() {
        let old = std::fs::read(&current).map_err(|e| e.to_string())?;
        atomic(&root.join("previous.json"), &old)?;
    }
    atomic(&current, &bytes)
}
pub fn eligible_outputs(
    snapshot: &Snapshot,
    machine: &str,
    current: &BTreeMap<u64, Output>,
) -> Vec<u64> {
    if machine.is_empty() || snapshot.machine != machine {
        return vec![];
    }
    snapshot
        .enabled
        .iter()
        .filter_map(|(&id, target)| {
            let preference = current.get(&id).cloned().unwrap_or(Output {
                fullscreen: false,
                display: None,
            });
            (snapshot.document.viewport(id).is_some() && preference == *target).then_some(id)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_preserves_identity_unsaved_edits_and_fallback() {
        let root = std::env::temp_dir().join(format!("rastera-recovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut d = Document::default();
        let saved = d.clone();
        d.add_text();
        let s = Snapshot {
            version: 1,
            document: d,
            saved: Some(saved),
            project: Some(root.join("show.json")),
            selected: None,
            viewport: Some(1),
            viewports_mode: true,
            window: Some([10., 20., 1200., 800.]),
            clean_windows: Default::default(),
            panel_rects: Default::default(),
            playheads: BTreeMap::new(),
            machine: "test".into(),
            enabled: BTreeMap::new(),
        };
        let mut targets = s.clone();
        targets.enabled.insert(
            1,
            Output {
                fullscreen: true,
                display: Some("exact".into()),
            },
        );
        assert!(eligible_outputs(&targets, "other", &targets.enabled).is_empty());
        assert_eq!(
            eligible_outputs(&targets, "test", &targets.enabled),
            vec![1]
        );
        let wrong = BTreeMap::from([(
            1,
            Output {
                fullscreen: true,
                display: Some("different".into()),
            },
        )]);
        assert!(eligible_outputs(&targets, "test", &wrong).is_empty());
        store(&root, &Preferences::default(), &s).unwrap();
        store(&root, &Preferences::default(), &s).unwrap();
        assert_eq!(load(&root).0, Some(s.clone()));
        std::fs::write(root.join("current.json"), b"broken").unwrap();
        let (recovered, warning) = load(&root);
        assert_eq!(recovered, Some(s));
        assert!(warning.is_some());
        std::fs::write(root.join("previous.json"), b"broken").unwrap();
        assert!(load(&root).0.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }
}
