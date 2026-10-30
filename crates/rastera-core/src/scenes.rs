//! Named, persistent scene instances. Geometry/sources/outputs remain project-level.
use crate::scene::{Document, Layer};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub id: u64,
    pub name: String,
    pub layers: Vec<Layer>,
    pub background: crate::background::Background,
    pub test_background: bool,
}
impl Default for Scene {
    fn default() -> Self {
        Self {
            id: 1,
            name: "Scene 1".into(),
            layers: vec![],
            background: crate::background::Background::default(),
            test_background: false,
        }
    }
}
impl std::ops::Deref for Document {
    type Target = Scene;
    fn deref(&self) -> &Scene {
        self.scenes
            .iter()
            .find(|s| s.id == self.active_scene)
            .expect("validated active scene")
    }
}
impl std::ops::DerefMut for Document {
    fn deref_mut(&mut self) -> &mut Scene {
        self.scenes
            .iter_mut()
            .find(|s| s.id == self.active_scene)
            .expect("validated active scene")
    }
}
pub fn validate(d: &Document) -> Result<(), String> {
    let mut ids = std::collections::BTreeSet::new();
    if d.scenes.is_empty()
        || d.scenes.len() > 16
        || d.scenes
            .iter()
            .any(|s| s.id == 0 || !ids.insert(s.id) || s.name.len() > 128)
        || !ids.contains(&d.active_scene)
        || d.next_scene_id <= ids.last().copied().unwrap_or(0)
        || d.next_scene_id > u64::MAX - 1024
    {
        return Err("Invalid scenes (1–16), active identity or scene allocator".into());
    }
    Ok(())
}
impl Document {
    pub fn add_scene(&mut self, duplicate: bool) -> Result<u64, String> {
        let mut candidate = self.clone();
        let id = candidate.allocate_scene(duplicate)?;
        candidate.validate()?;
        *self = candidate;
        Ok(id)
    }
    fn allocate_scene(&mut self, duplicate: bool) -> Result<u64, String> {
        if self.scenes.len() >= 16 {
            return Err("Scene limit reached (16)".into());
        }
        let mut scene = if duplicate {
            self.deref_scene().clone()
        } else {
            Scene::default()
        };
        scene.id = self.next_scene_id;
        self.next_scene_id += 1;
        scene.name = if duplicate {
            format!("{} copy", scene.name.chars().take(120).collect::<String>())
        } else {
            format!("Scene {}", scene.id)
        };
        // Placement identities and editable text are independent; capture/media references deliberately share.
        for l in &mut scene.layers {
            l.id = self.next_id;
            self.next_id += 1;
            if let crate::scene::Content::Text(id) = l.content {
                let new = self.next_id;
                self.next_id += 1;
                self.texts.insert(new, self.texts[&id].clone());
                l.content = crate::scene::Content::Text(new);
            }
        }
        let id = scene.id;
        self.scenes.push(scene);
        self.active_scene = id;
        Ok(id)
    }
    fn deref_scene(&self) -> &Scene {
        self
    }
    pub fn remove_scene(&mut self, id: u64) -> bool {
        if self.scenes.len() <= 1 {
            return false;
        }
        let n = self.scenes.len();
        self.scenes.retain(|s| s.id != id);
        if !self.scenes.iter().any(|s| s.id == self.active_scene) {
            self.active_scene = self.scenes[0].id;
        }
        self.prune_unused();
        n != self.scenes.len()
    }
    pub fn select_scene(&mut self, id: u64) -> bool {
        if self.scenes.iter().any(|s| s.id == id) {
            self.active_scene = id;
            true
        } else {
            false
        }
    }
}
#[cfg(test)]
pub fn legacy_value(d: &Document) -> serde_json::Value {
    let mut v = serde_json::to_value(d).unwrap();
    let o = v.as_object_mut().unwrap();
    let active = d.deref_scene();
    o.remove("scenes");
    o.remove("active_scene");
    o.remove("next_scene_id");
    o.insert(
        "layers".into(),
        serde_json::to_value(&active.layers).unwrap(),
    );
    o.insert(
        "background".into(),
        serde_json::to_value(&active.background).unwrap(),
    );
    o.insert("test_background".into(), active.test_background.into());
    v
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scene_instances_references_history() {
        let mut d = Document::default();
        let layer = d.add_text();
        let original = d.active_scene;
        let id = d.add_scene(true).unwrap();
        assert_ne!(d.layers[0].id, layer);
        assert_eq!(d.scenes.len(), 2);
        d.prune_unused();
        assert_eq!(d.texts.len(), 2);
        let before = d.clone();
        assert!(d.remove_scene(id));
        assert_eq!(d.active_scene, original);
        let mut h = crate::scene::History::default();
        h.record(before.clone(), &d);
        assert!(h.undo(&mut d));
        assert_eq!(d, before);
        assert!(d.validate().is_ok());
        assert!(d.remove_scene(original));
        assert!(!d.remove_scene(id));
    }
}
#[cfg(test)]
mod migration_tests {
    use super::*;
    #[test]
    fn schema5_preserves_ids_split_and_content() {
        let mut d = Document::default();
        d.add_text();
        d.test_background = true;
        let mut v = legacy_value(&d);
        v["version"] = 5.into();
        let o = v.as_object_mut().unwrap();
        o.remove("media_assets");
        o.remove("playbacks");
        o.remove("background");
        for l in o["layers"].as_array_mut().unwrap() {
            l.as_object_mut().unwrap().remove("crop");
        }
        let p = std::env::temp_dir().join(format!("rastera-schema5-{}.json", std::process::id()));
        std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
        let migrated = crate::scene::load_project(&p).unwrap();
        assert_eq!(migrated, d);
        assert_eq!(migrated.scenes.len(), 1);
        assert_eq!(migrated.project_id, d.project_id);
        std::fs::remove_file(p).unwrap();
    }
    #[test]
    fn inactive_scene_and_background_references_survive_prune() {
        let mut d = Document::default();
        let layer = d
            .add_media(crate::media::Asset {
                path: "missing.mp4".into(),
                size: [320, 180],
                duration: 2.,
                gif: false,
            })
            .unwrap();
        let crate::scene::Content::Media(source) = d.layer(layer).unwrap().content else {
            panic!()
        };
        d.background = crate::background::Background::Media {
            source,
            fit: crate::background::Fit::Fill,
        };
        d.layers.clear();
        d.add_scene(false).unwrap();
        d.prune_unused();
        assert_eq!(d.playbacks.len(), 1);
        assert_eq!(d.media_assets.len(), 1);
        assert!(d.validate().is_ok());
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;
    #[test]
    fn scene_limit_rejects_without_mutation() {
        let mut d = Document::default();
        while d.scenes.len() < 16 {
            d.add_scene(false).unwrap();
        }
        let before = d.clone();
        assert!(d.add_scene(true).is_err());
        assert_eq!(d, before);
    }
}
