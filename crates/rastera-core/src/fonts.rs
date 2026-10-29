//! Installed-font references, never embedded font bytes in projects.
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontRef {
    pub family: String,
    pub style: String,
    pub postscript: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct InstalledFace {
    pub family: String,
    pub style: String,
    pub postscript: String,
    pub path: PathBuf,
}
pub struct FontLibrary {
    pub faces: Mutex<Vec<InstalledFace>>,
    pub ready: std::sync::atomic::AtomicBool,
    failed: Mutex<std::collections::HashSet<String>>,
    loaded: Mutex<std::collections::HashMap<String, std::sync::Arc<epaint::text::FontData>>>,
}
impl Default for FontLibrary {
    fn default() -> Self {
        Self {
            faces: Mutex::new(vec![]),
            ready: std::sync::atomic::AtomicBool::new(true),
            failed: Mutex::new(Default::default()),
            loaded: Mutex::new(Default::default()),
        }
    }
}
impl FontLibrary {
    pub fn available(&self, r: &FontRef) -> bool {
        !self.failed.lock().unwrap().contains(&r.postscript)
            && self
                .faces
                .lock()
                .unwrap()
                .iter()
                .any(|f| f.postscript == r.postscript)
    }
    pub fn definitions(&self, r: Option<&FontRef>) -> epaint::text::FontDefinitions {
        let mut defs = epaint::text::FontDefinitions::default();
        let Some(r) = r else {
            return defs;
        };
        let cached = self.loaded.lock().unwrap().get(&r.postscript).cloned();
        if let Some(data) = cached {
            defs.font_data.insert("installed".into(), data);
            defs.families
                .get_mut(&epaint::FontFamily::Proportional)
                .unwrap()
                .insert(0, "installed".into());
            return defs;
        }
        self.failed.lock().unwrap().insert(r.postscript.clone());
        let path = self
            .faces
            .lock()
            .unwrap()
            .iter()
            .find(|f| f.postscript == r.postscript)
            .map(|f| f.path.clone());
        let Some(path) = path else {
            return defs;
        };
        let Ok(meta) = std::fs::metadata(&path) else {
            return defs;
        };
        if meta.len() > 32 * 1024 * 1024 {
            return defs;
        }
        let Ok(bytes) = std::fs::read(path) else {
            return defs;
        };
        let count = ttf_parser::fonts_in_collection(&bytes).unwrap_or(1);
        for index in 0..count {
            if let Ok(face) = ttf_parser::Face::parse(&bytes, index) {
                if face.names().into_iter().any(|n| {
                    n.name_id == ttf_parser::name_id::POST_SCRIPT_NAME
                        && (n.to_string().as_deref() == Some(&r.postscript)
                            // Legacy Mac name records store ASCII PostScript names.
                            || n.name == r.postscript.as_bytes())
                }) {
                    self.failed.lock().unwrap().remove(&r.postscript);
                    let mut data = epaint::text::FontData::from_owned(bytes);
                    data.index = index;
                    let data = std::sync::Arc::new(data);
                    let mut loaded = self.loaded.lock().unwrap();
                    if loaded.values().map(|d| d.font.len()).sum::<usize>() + data.font.len()
                        > 64 * 1024 * 1024
                    {
                        loaded.clear(); // Active preparations retain their own immutable bytes.
                    }
                    loaded.insert(r.postscript.clone(), data.clone());
                    defs.font_data.insert("installed".into(), data);
                    defs.families
                        .get_mut(&epaint::FontFamily::Proportional)
                        .unwrap()
                        .insert(0, "installed".into());
                    break;
                }
            }
        }
        defs
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn installed_collection_selects_legacy_mac_named_face() {
        let path = PathBuf::from("/System/Library/Fonts/Menlo.ttc");
        if !path.is_file() {
            return;
        } // Optional host resource; no system font is bundled.
        let lib = FontLibrary::default();
        lib.faces.lock().unwrap().push(InstalledFace {
            family: "Menlo".into(),
            style: "Bold".into(),
            postscript: "Menlo-Bold".into(),
            path,
        });
        let r = FontRef {
            family: "Menlo".into(),
            style: "Bold".into(),
            postscript: "Menlo-Bold".into(),
        };
        let defs = lib.definitions(Some(&r));
        assert_eq!(defs.font_data["installed"].index, 1);
        assert_eq!(
            defs.families[&epaint::FontFamily::Proportional][0],
            "installed"
        );
        assert!(lib.available(&r));
        let again = lib.definitions(Some(&r));
        assert!(std::sync::Arc::ptr_eq(
            &defs.font_data["installed"],
            &again.font_data["installed"]
        ));
        let custom = epaint::text::Fonts::new(1., 4096, defs);
        let fallback = epaint::text::Fonts::new(1., 4096, lib.definitions(None));
        let width = |f: &epaint::text::Fonts| {
            f.layout_no_wrap(
                "iiiiWWWW".into(),
                epaint::FontId::proportional(40.),
                epaint::Color32::WHITE,
            )
            .size()
            .x
        };
        assert!((width(&custom) - width(&fallback)).abs() > 1.);
    }
    #[test]
    fn missing_font_reference_retained_with_fallback() {
        let r = FontRef {
            family: "Unavailable family".into(),
            style: "Bold".into(),
            postscript: "Missing-12345".into(),
        };
        let lib = FontLibrary::default();
        assert!(!lib.available(&r));
        let a = lib.definitions(Some(&r));
        let b = lib.definitions(None);
        assert_eq!(a.families, b.families);
        assert_eq!(r.postscript, "Missing-12345");
    }
}
