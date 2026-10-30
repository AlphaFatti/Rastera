//! Regression tests for document identities, custom outputs and immutable frame snapshots.
use crate::{
    mapping::*,
    scene::{self, Document, History, PreviewMap},
    viewport_hit,
};

#[test]
fn zero_one_many_ids_history_and_limit() {
    let mut doc = Document::default();
    let mut history = History::default();
    let old = doc.clone();
    doc.remove_viewport(1);
    history.record(old, &doc);
    assert_eq!(doc.viewports.len(), 1);
    assert_eq!(doc.viewports[0].id, 2);
    assert!(history.undo(&mut doc));
    assert!(doc.viewport(1).is_some());
    assert!(history.redo(&mut doc));
    assert!(doc.viewport(1).is_none());
    assert!(doc.viewport(2).is_some());
    assert!(history.undo(&mut doc));
    assert!(doc.viewport(1).is_some());
    doc.viewports.clear();
    assert!(doc.validate().is_ok());
    let before = doc.clone();
    let id = doc.add_viewport().unwrap();
    history.record(before, &doc);
    assert_eq!(id, 3);
    assert_eq!(doc.viewports.len(), 1);
    assert!(history.undo(&mut doc));
    assert!(doc.viewports.is_empty());
    let next = doc.add_viewport().unwrap();
    assert!(next > id); // New undo branch never reuses the old identity.
    while doc.viewports.len() < MAX_VIEWPORTS {
        doc.add_viewport().unwrap();
    }
    let full = doc.clone();
    assert!(doc.add_viewport().unwrap_err().contains("limit"));
    assert_eq!(doc, full);
    assert!(doc.validate().is_ok());
    let names: std::collections::BTreeSet<_> = doc.viewports.iter().map(|v| &v.name).collect();
    assert_eq!(names.len(), MAX_VIEWPORTS);
}

#[test]
fn custom_aspects_transform_inverse_fit_and_size_preserves_transform() {
    let mut v = defaults().remove(0);
    v.center = [1333., 777.];
    v.zoom = 1.75;
    v.rotation = 37.;
    for (_, size) in VIEWPORT_PRESETS
        .into_iter()
        .chain([("tiny", [1, 1]), ("thin", [1, 8192])])
    {
        assert!(validate_output(size).is_ok());
        v.size = size;
        assert!(v.validate());
        let c = v.canvas_point([size[0] as f32 / 2., size[1] as f32 / 2.]);
        assert!((c[0] - 1333.).abs() < 0.01 && (c[1] - 777.).abs() < 0.01);
        for p in [[0., 0.], [size[0] as f32 * 0.7, size[1] as f32 * 0.6]] {
            let q = v.output_point(v.canvas_point(p));
            assert!((q[0] - p[0]).abs() < 0.01 && (q[1] - p[1]).abs() < 0.01);
        }
        let mut fitted = v.clone();
        fitted.fit_canvas([3840, 1080]);
        assert!(fitted.validate());
        let rect = crate::fit(size, [1280., 720.]);
        assert!((rect[2] / rect[3] - size[0] as f32 / size[1] as f32).abs() < 0.001);
    }
    for size in [[0, 100], [8193, 1], [8192, 8192]] {
        assert!(validate_output(size).is_err());
    }
    v.size = [4096, 4096];
    v.fit_canvas([1, 1]);
    assert_eq!(v.zoom, 4096.); // Fit maximum only if dimensions validate separately.
}

#[test]
fn any_count_aspect_hit_testing_uses_stable_ids() {
    let mut doc = Document::default();
    doc.viewports.clear();
    let map = PreviewMap::for_canvas(doc.canvas, [1600., 900.], 20.);
    assert_eq!(
        viewport_hit::pick(&doc.viewports, map, 1., &[], None, [100., 100.], true),
        None
    );
    for (_, size) in VIEWPORT_PRESETS.iter().take(5) {
        let id = doc.add_viewport().unwrap();
        doc.viewport_mut(id).unwrap().size = *size;
    }
    for v in &mut doc.viewports {
        v.center = [1920., 540.];
        v.zoom = 1.;
        v.rotation = 24.;
    }
    let labels = vec![[50., 15.]; doc.viewports.len()];
    let pixel = [map.fit[0] + map.fit[2] / 2., map.fit[1] + map.fit[3] / 2.];
    let mut selected = None;
    for v in &doc.viewports {
        let hit =
            viewport_hit::pick(&doc.viewports, map, 1., &labels, selected, pixel, true).unwrap();
        assert_eq!(hit.id, v.id);
        selected = Some(hit.id);
    }
    let chosen = doc.viewports[3].id;
    doc.remove_viewport(doc.viewports[0].id);
    assert_eq!(
        viewport_hit::pick(
            &doc.viewports,
            map,
            1.,
            &labels[..doc.viewports.len()],
            Some(chosen),
            pixel,
            false
        )
        .unwrap()
        .id,
        chosen
    );
    let mut tiny = doc.viewports[0].clone();
    tiny.size = [8192, 1];
    tiny.zoom = 8192.;
    tiny.center = [65536.; 2];
    assert!(viewport_hit::Footprint::new(&tiny, map, 1., [50., 15.])
        .rotation
        .into_iter()
        .all(f32::is_finite));
}

#[test]
fn leased_frame_metadata_never_reassigns_removed_or_resized_id() {
    let doc = Document::default();
    let old = FrameMeta {
        canvas: doc.canvas,
        viewports: doc
            .viewports
            .iter()
            .map(FrameViewport::from)
            .collect::<Vec<_>>()
            .into(),
    };
    let lease = old.clone();
    let right = *lease.viewport(2).unwrap();
    let mut next = doc.clone();
    next.remove_viewport(1);
    next.viewport_mut(2).unwrap().size = [960, 192];
    let new = FrameMeta {
        canvas: next.canvas,
        viewports: next
            .viewports
            .iter()
            .map(FrameViewport::from)
            .collect::<Vec<_>>()
            .into(),
    };
    assert!(new.viewport(1).is_none());
    assert_eq!(new.viewport(2).unwrap().id, 2);
    assert_eq!(new.viewport(2).unwrap().size, [960, 192]);
    assert_eq!(*lease.viewport(2).unwrap(), right);
    assert_eq!(lease.viewport(1).unwrap().size, [1920, 1080]);
}

#[test]
fn v1_v2_migration_and_empty_custom_round_trips() {
    let folder =
        std::env::temp_dir().join(format!("rastera-dynamic-project-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("project.json");
    let mut doc = Document::default();
    doc.add_text();
    for version in [1, 2] {
        let mut legacy = serde_json::to_value(&doc).unwrap();
        legacy["version"] = version.into();
        legacy.as_object_mut().unwrap().remove("next_viewport_id");
        if version == 1 {
            legacy.as_object_mut().unwrap().remove("viewports");
        } else {
            legacy["viewports"][0]["name"] = "Legacy detail".into();
            legacy["viewports"][0]["rotation"] = 27.into();
            legacy["viewports"][0]["zoom"] = 2.into();
            for v in legacy["viewports"].as_array_mut().unwrap() {
                v.as_object_mut().unwrap().remove("size");
            }
        }
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let loaded = scene::load_project(&path).unwrap();
        assert_eq!(loaded.version, 7);
        assert_eq!(loaded.next_viewport_id, 3);
        assert_eq!(loaded.viewports[0].size, [1920, 1080]);
        assert_eq!(loaded.layers, doc.layers);
        if version == 1 {
            assert_eq!(loaded.viewports, defaults());
        } else {
            assert_eq!(loaded.viewports[0].rotation, 27.);
            assert_eq!(loaded.viewports[0].name, "Legacy detail");
        }
    }
    doc.viewports.clear();
    for (_, size) in VIEWPORT_PRESETS {
        let id = doc.add_viewport().unwrap();
        doc.viewport_mut(id).unwrap().size = size;
    }
    scene::save_project(&doc, &path).unwrap();
    assert_eq!(scene::load_project(&path).unwrap(), doc);
    doc.viewports.clear();
    let high = doc.next_viewport_id;
    scene::save_project(&doc, &path).unwrap();
    let empty = scene::load_project(&path).unwrap();
    assert!(empty.viewports.is_empty());
    assert_eq!(empty.next_viewport_id, high);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn malformed_legacy_viewports_reject_without_panicking() {
    let folder = std::env::temp_dir().join(format!("rastera-bad-migration-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("invalid.json");
    for rows in [
        serde_json::json!([42]),
        serde_json::json!([null]),
        serde_json::json!([{}]),
        serde_json::json!("invalid"),
    ] {
        let mut value = serde_json::to_value(Document::default()).unwrap();
        value["version"] = 2.into();
        value["viewports"] = rows;
        value.as_object_mut().unwrap().remove("next_viewport_id");
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(scene::load_project(&path).is_err());
    }
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn new_viewport_is_pixel_exact_and_dimensions_scale_footprint() {
    let mut doc = Document::default();
    let id = doc.add_viewport().unwrap();
    let v = doc.viewport_mut(id).unwrap();
    assert_eq!(v.zoom, 1.);
    assert_eq!(v.center, [1920., 540.]);
    for (_, size) in VIEWPORT_PRESETS {
        v.size = size;
        let corners = v.corners();
        assert_eq!(corners[1][0] - corners[0][0], size[0] as f32);
        assert_eq!(corners[3][1] - corners[0][1], size[1] as f32);
        assert_eq!(
            v.canvas_point([0.5, 0.5]),
            [
                v.center[0] - size[0] as f32 / 2. + 0.5,
                v.center[1] - size[1] as f32 / 2. + 0.5
            ]
        );
    }
}

#[test]
fn footprint_scale_changes_coverage_without_changing_resolution() {
    let mut v = defaults().remove(0);
    v.zoom = 0.5; // UI footprint scale = 200%.
    let q = v.corners();
    assert_eq!(v.size, [1920, 1080]);
    assert_eq!(q[1][0] - q[0][0], 3840.);
    assert_eq!(q[3][1] - q[0][1], 2160.);
    v.zoom = 2.; // UI footprint scale = 50%.
    let q = v.corners();
    assert_eq!(v.size, [1920, 1080]);
    assert_eq!(q[1][0] - q[0][0], 960.);
    assert_eq!(q[3][1] - q[0][1], 540.);
}

#[test]
fn v3_identity_migration_preserves_sampling_and_v4_identity_roundtrips() {
    let folder = std::env::temp_dir().join(format!("rastera-v4-migration-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("show.json");
    let mut original = Document::default();
    original.viewports[0].size = [960, 192];
    original.viewports[0].zoom = 0.75;
    original.viewports[0].rotation = 30.;
    assert_ne!(original.project_id, Document::default().project_id);
    let mut value = serde_json::to_value(&original).unwrap();
    value["version"] = 3.into();
    value.as_object_mut().unwrap().remove("project_id");
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    let migrated = scene::load_project(&path).unwrap();
    assert_eq!(migrated.version, 7);
    assert_eq!(migrated.viewports, original.viewports);
    assert_eq!(migrated.next_viewport_id, original.next_viewport_id);
    assert_eq!(migrated.project_id.len(), 32);
    scene::save_project(&migrated, &path).unwrap();
    assert_eq!(scene::load_project(&path).unwrap(), migrated);
    value = serde_json::to_value(migrated).unwrap();
    value["project_id"] = "invalid".into();
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(scene::load_project(&path).is_err());
    std::fs::remove_dir_all(folder).unwrap();
}
