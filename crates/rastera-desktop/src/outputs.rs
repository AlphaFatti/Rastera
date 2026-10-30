//! Machine-local destinations. No canvas geometry, GPU resources or enabled state are persisted.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    pub fullscreen: bool,
    pub display: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Display {
    pub id: String,
    pub name: String,
    pub pixels: [u32; 2],
    pub frame: [f64; 4],
    pub number: u32,
    pub builtin: bool,
}
impl Display {
    pub fn label(&self) -> String {
        format!(
            "{}{} · {}×{} · CG{} · {}",
            self.name,
            if self.builtin { " (built-in)" } else { "" },
            self.pixels[0],
            self.pixels[1],
            self.number,
            self.id.chars().take(8).collect::<String>()
        )
    }
}
#[derive(Default, Deserialize)]
pub struct Discovery {
    pub machine: String,
    pub displays: Vec<Display>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    version: u32,
    projects: BTreeMap<String, BTreeMap<u64, Destination>>,
}
fn namespace(machine: &str, project: &str, path: &Path) -> String {
    // Exact structured strings avoid hash collisions and project-ID aliasing across copied files.
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    let encoded: String = canonical
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    serde_json::to_string(&(machine, project, encoded)).unwrap()
}
pub fn resolve<'a>(
    destination: &Destination,
    displays: &'a [Display],
) -> Result<Option<&'a Display>, String> {
    if !destination.fullscreen {
        return Ok(None);
    }
    let target = destination
        .display
        .as_ref()
        .ok_or("Missing display: choose a target")?;
    let matches: Vec<_> = displays
        .iter()
        .filter(|d| !d.id.is_empty() && &d.id == target)
        .collect();
    match matches.as_slice() {
        [] => Err("Missing display: enable explicitly after reconnection".into()),
        [display] => Ok(Some(display)),
        _ => Err("Ambiguous display identity: reassign target".into()),
    }
}
pub fn plan(
    preferences: &BTreeMap<u64, Destination>,
    enabled: &BTreeSet<u64>,
    active: &BTreeMap<u64, Destination>,
    displays: &[Display],
) -> BTreeMap<u64, Result<Destination, String>> {
    let mut claimed = BTreeMap::<String, u64>::new();
    // Existing unchanged owners win over newly enabled/reassigned requests, regardless of ID order.
    for (&id, d) in active {
        if enabled.contains(&id)
            && preferences.get(&id) == Some(d)
            && resolve(d, displays).is_ok()
            && d.fullscreen
        {
            if let Some(display) = &d.display {
                claimed.insert(display.clone(), id);
            }
        }
    }
    enabled
        .iter()
        .map(|&id| {
            let d = preferences.get(&id).cloned().unwrap_or_default();
            let result = resolve(&d, displays).and_then(|target| {
                if let Some(target) = target {
                    if let Some(owner) = claimed.get(&target.id) {
                        if *owner != id {
                            return Err(format!(
                                "Conflicting assignment: viewport {owner} owns this display"
                            ));
                        }
                    }
                    claimed.insert(target.id.clone(), id);
                }
                Ok(d)
            });
            (id, result)
        })
        .collect()
}
pub struct Destinations {
    pub preferences: BTreeMap<u64, Destination>,
    pub status: BTreeMap<u64, String>,
    pub discovery: Discovery,
    pub error: Option<String>,
    pub identify: bool,
    pub refresh: bool,
    store: Store,
    path: PathBuf,
    key: Option<String>,
    project: String,
    writable: bool,
    context_tag: Option<(String, String, Option<PathBuf>)>,
}
impl Destinations {
    pub fn new() -> Self {
        let path = std::env::args()
            .find_map(|a| {
                a.strip_prefix("--session-dir=")
                    .map(|p| PathBuf::from(p).join("outputs.local.json"))
            })
            .or_else(|| {
                std::env::args().find_map(|a| {
                    a.strip_prefix("--editor-probe=")
                        .map(|p| PathBuf::from(p).join("outputs.local.json"))
                })
            })
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                    .join("Library/Application Support/Rastera/output-destinations.json")
            });
        Self::at_path(path)
    }
    fn at_path(path: PathBuf) -> Self {
        let loaded = read_store(&path);
        let error = loaded.as_ref().err().cloned();
        Self {
            preferences: BTreeMap::new(),
            status: BTreeMap::new(),
            discovery: Discovery::default(),
            error,
            identify: false,
            refresh: true,
            writable: loaded.is_ok(),
            context_tag: None,
            store: loaded.unwrap_or_else(|_| Store {
                version: 1,
                ..Default::default()
            }),
            path,
            key: None,
            project: String::new(),
        }
    }
    pub fn context(
        &mut self,
        project: &str,
        path: Option<&Path>,
        reopened: bool,
        enabled: &mut BTreeSet<u64>,
    ) {
        let tag = (
            self.discovery.machine.clone(),
            project.to_owned(),
            path.map(Path::to_owned),
        );
        if !reopened && self.context_tag.as_ref() == Some(&tag) {
            return;
        }
        let machine_changed = self
            .context_tag
            .as_ref()
            .is_some_and(|previous| previous.0 != self.discovery.machine);
        self.context_tag = Some(tag);
        let key = path
            .filter(|_| !self.discovery.machine.is_empty())
            .map(|p| namespace(&self.discovery.machine, project, p));
        if reopened || machine_changed || self.project != project {
            enabled.clear();
            self.status.clear();
            self.preferences = key
                .as_ref()
                .and_then(|k| self.store.projects.get(k))
                .cloned()
                .unwrap_or_default();
        }
        self.project = project.to_owned();
        if self.key != key {
            self.key = key;
            if !reopened {
                self.persist();
            }
        }
    }
    pub fn persist(&mut self) {
        if !self.writable {
            return;
        }
        if let Some(key) = &self.key {
            self.store
                .projects
                .insert(key.clone(), self.preferences.clone());
            if let Err(e) = write_store(&self.path, &self.store) {
                self.error = Some(e);
            }
        }
    }
    pub fn ui(&mut self, ui: &mut eframe::egui::Ui, id: u64, enabled: &mut BTreeSet<u64>) {
        use eframe::egui;
        let before = self.preferences.get(&id).cloned().unwrap_or_default();
        let mut d = before.clone();
        egui::ComboBox::from_id_salt(("presentation", id))
            .selected_text(if d.fullscreen {
                "Fullscreen"
            } else {
                "Windowed"
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut d.fullscreen, false, "Windowed");
                ui.selectable_value(&mut d.fullscreen, true, "Fullscreen");
            });
        if d.fullscreen {
            ui.label("Target display");
            if self.discovery.displays.is_empty() {
                ui.label("No connected displays");
            }
            // Inline wrapped targets stay outside the native canvas child view; an egui
            // popup extending over that surface would be hidden by AppKit view ordering.
            for display in &self.discovery.displays {
                let response = ui.add_enabled(
                    !display.id.is_empty(),
                    egui::Button::new(display.label())
                        .wrap()
                        .selected(d.display.as_ref() == Some(&display.id))
                        .min_size(egui::vec2(ui.available_width(), 0.)),
                );
                if response.clicked() {
                    d.display = Some(display.id.clone());
                }
                response.on_hover_text(format!(
                    "UUID {} · CG{} · origin ({:.0}, {:.0})",
                    display.id, display.number, display.frame[0], display.frame[1]
                ));
            }
        }
        if d != before {
            self.preferences.insert(id, d.clone());
            self.status.remove(&id);
            self.persist();
        }
        let mut open = enabled.contains(&id);
        if ui.checkbox(&mut open, "Enable output").changed() {
            self.status.remove(&id);
            if open {
                enabled.insert(id);
            } else {
                enabled.remove(&id);
            }
        }
        let status = self.status.get(&id).cloned().unwrap_or_else(|| {
            if let Err(e) = resolve(&d, &self.discovery.displays) {
                e
            } else if enabled.contains(&id) {
                "Starting…".into()
            } else {
                "Disabled".into()
            }
        });
        ui.label(status);
    }
}
fn read_store(path: &Path) -> Result<Store, String> {
    if !path.exists() {
        return Ok(Store {
            version: 1,
            ..Default::default()
        });
    }
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 2 * 1024 * 1024 {
        return Err("Destination settings exceed 2 MiB".into());
    }
    let store: Store = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Destination settings: {e}"))?;
    if store.version != 1 {
        return Err("Unsupported destination settings version".into());
    }
    for rows in store.projects.values() {
        for (&id, d) in rows {
            if id == 0
                || d.display.as_ref().is_some_and(|s| {
                    s.len() != 36
                        || !s.bytes().enumerate().all(|(i, b)| {
                            if [8, 13, 18, 23].contains(&i) {
                                b == b'-'
                            } else {
                                b.is_ascii_hexdigit()
                            }
                        })
                })
            {
                return Err("Invalid destination identity; settings left unchanged".into());
            }
        }
    }
    Ok(store)
}
fn write_store(path: &Path, store: &Store) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().ok_or("Invalid settings path")?)
        .map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(store).map_err(|e| e.to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("Destination settings exceed 2 MiB".into());
    }
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, bytes)
        .and_then(|()| std::fs::rename(&temporary, path))
        .map_err(|e| format!("Save destination settings: {e}"))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn screen(id: &str) -> Display {
        Display {
            id: id.into(),
            name: "Panel".into(),
            pixels: [1920, 1080],
            frame: [-1920., 0., 1920., 1080.],
            number: 1,
            builtin: false,
        }
    }
    #[test]
    fn resolution_missing_ambiguous_and_conflict() {
        let a = Destination {
            fullscreen: true,
            display: Some("a".into()),
        };
        assert!(resolve(&a, &[]).unwrap_err().contains("Missing"));
        assert!(resolve(&a, &[screen("a"), screen("a")])
            .unwrap_err()
            .contains("Ambiguous"));
        let p = [(1, a.clone()), (2, a.clone())].into();
        let result = plan(&p, &[1, 2].into(), &[(2, a)].into(), &[screen("a")]);
        assert!(result[&2].is_ok());
        assert!(result[&1].as_ref().unwrap_err().contains("2"));
        assert_eq!(
            resolve(&Destination::default(), &[])
                .unwrap()
                .map(|s| s.number),
            None
        );
    }
    #[test]
    fn local_persistence_has_no_enable_state_and_namespaces_projects() {
        let folder =
            std::env::temp_dir().join(format!("rastera-destinations-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("settings.json");
        let a = namespace("machine-a", "project-a", Path::new("/a"));
        assert_ne!(a, namespace("machine-b", "project-a", Path::new("/a")));
        assert_ne!(a, namespace("machine-a", "project-b", Path::new("/a")));
        assert_ne!(a, namespace("machine-a", "project-a", Path::new("/b")));
        let store = Store {
            version: 1,
            projects: [(
                a.clone(),
                [(
                    7,
                    Destination {
                        fullscreen: true,
                        display: Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into()),
                    },
                )]
                .into(),
            )]
            .into(),
        };
        write_store(&path, &store).unwrap();
        assert!(read_store(&path).unwrap().projects[&a][&7].fullscreen);
        let mut value = serde_json::to_value(store).unwrap();
        value["version"] = 2.into();
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(read_store(&path).is_err());
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn reopening_disables_and_machine_project_namespaces_are_isolated() {
        let folder =
            std::env::temp_dir().join(format!("rastera-route-reopen-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("store.json");
        let project = folder.join("show.json");
        std::fs::write(&project, "{}").unwrap();
        let mut a = Destinations::at_path(path.clone());
        a.discovery.machine = "mac-a".into();
        let mut enabled = [7].into();
        a.context("document-a", Some(&project), false, &mut enabled);
        assert!(enabled.is_empty());
        a.preferences.insert(
            7,
            Destination {
                fullscreen: true,
                display: Some("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into()),
            },
        );
        a.persist();
        enabled.insert(7);
        let mut b = Destinations::at_path(path.clone());
        b.discovery.machine = "mac-a".into();
        b.context("document-a", Some(&project), true, &mut enabled);
        assert!(enabled.is_empty());
        assert!(b.preferences[&7].fullscreen);
        b.context("unrelated-document", Some(&project), true, &mut enabled);
        assert!(b.preferences.is_empty());
        b.discovery.machine = "mac-b".into();
        b.context("document-a", Some(&project), true, &mut enabled);
        assert!(b.preferences.is_empty());
        assert!(!std::fs::read_to_string(path).unwrap().contains("enabled"));
        std::fs::remove_dir_all(folder).unwrap();
    }
    #[test]
    fn corrupt_or_future_settings_are_not_overwritten() {
        let path =
            std::env::temp_dir().join(format!("rastera-future-routes-{}.json", std::process::id()));
        let original = "{\"version\":99,\"projects\":{}}";
        std::fs::write(&path, original).unwrap();
        let mut d = Destinations::at_path(path.clone());
        d.key = Some("project".into());
        d.persist();
        assert!(d.error.is_some());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        std::fs::remove_file(path).unwrap();
    }
}
