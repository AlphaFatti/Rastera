use crate::{
    background::{Background, Fit},
    media, recovery,
    scene::{self, Content, Document, History},
};
use rastera_audio::model::{Channel, Interpretation, Model, Origin};
use std::collections::BTreeMap;
fn channels(n: u32) -> Vec<Channel> {
    (0..n)
        .map(|channel| Channel { track: 2, channel })
        .collect()
}
fn asset() -> media::Asset {
    media::Asset {
        path: "/missing/synthetic.mov".into(),
        size: [320, 180],
        duration: 6.,
        gif: false,
    }
}
#[test]
fn scene_participation_deduplicates_shared_visuals_and_retains_audio_definitions() {
    let mut doc = Document::default();
    let layer = doc.add_media(asset()).unwrap();
    let Content::Media(playback) = doc.layer(layer).unwrap().content else {
        panic!()
    };
    let audio = doc
        .audio
        .add(Origin::Playback(playback), "File".into(), channels(4))
        .unwrap();
    doc.audio.sources.get_mut(&audio).unwrap().enabled = true;
    doc.duplicate(layer).unwrap();
    doc.layer_mut(layer).unwrap().opacity = 0.;
    doc.layer_mut(layer).unwrap().visible = false;
    doc.background = Background::Media {
        source: playback,
        fit: Fit::Fill,
    };
    assert_eq!(
        doc.audio_participation().into_iter().collect::<Vec<_>>(),
        [audio]
    );
    let scene = doc.add_scene(false).unwrap();
    assert_eq!(doc.active_scene, scene);
    assert!(doc.audio_participation().is_empty());
    doc.audio.sources.get_mut(&audio).unwrap().always_on = true;
    assert!(doc.audio_participation().contains(&audio));
    let second_layer = doc.add_media(asset()).unwrap();
    let Content::Media(second_playback) = doc.layer(second_layer).unwrap().content else {
        panic!()
    };
    assert_ne!(playback, second_playback);
    let second_audio = doc
        .audio
        .add(
            Origin::Playback(second_playback),
            "Independent".into(),
            channels(2),
        )
        .unwrap();
    doc.audio.sources.get_mut(&second_audio).unwrap().enabled = true;
    assert_eq!(doc.audio_participation().len(), 2);
    for scene in &mut doc.scenes {
        scene.layers.clear();
        scene.background = Background::default();
    }
    doc.prune_unused();
    assert!(doc.playbacks.contains_key(&playback) && doc.playbacks.contains_key(&second_playback));
    assert_eq!(doc.media_assets.len(), 1);
    doc.audio.sources.remove(&second_audio);
    doc.prune_unused();
    assert!(!doc.playbacks.contains_key(&second_playback));
    assert!(doc.playbacks.contains_key(&playback));
    doc.validate().unwrap();
}
#[test]
fn capture_audio_reference_retains_picture_definition_without_an_instance() {
    let mut doc = Document::default();
    let layer = doc
        .add_capture(crate::capture::Source {
            device_id: "exact-video-device".into(),
            name: "Camera".into(),
            mode: crate::capture::Mode {
                width: 320,
                height: 180,
                fps: 30.,
            },
        })
        .unwrap();
    let Content::Capture(capture) = doc.layer(layer).unwrap().content else {
        panic!()
    };
    let id = doc
        .audio
        .add(Origin::Capture(capture), "Camera audio".into(), channels(2))
        .unwrap();
    let source = doc.audio.sources.get_mut(&id).unwrap();
    source.input_uid = "explicit-audio-device".into();
    source.enabled = true;
    source.always_on = true;
    doc.layers.clear();
    doc.prune_unused();
    assert!(doc.captures.contains_key(&capture));
    assert_eq!(
        doc.audio_participation().into_iter().collect::<Vec<_>>(),
        [id]
    );
    doc.validate().unwrap();
}
#[test]
fn schema_ten_migration_preserves_all_visual_fields_and_adds_silent_audio_defaults() {
    let mut doc = Document::default();
    let text = doc.add_text();
    doc.layer_mut(text).unwrap().pin.corners[0] = [0.1, 0.12];
    doc.layer_mut(text).unwrap().rect.x = -35.;
    doc.viewports[0].rotation = 12.;
    doc.viewports[1].size = [960, 192];
    doc.add_scene(true).unwrap();
    let expected = doc.clone();
    let mut v = serde_json::to_value(&doc).unwrap();
    v["version"] = 10.into();
    v.as_object_mut().unwrap().remove("audio");
    let migrated = scene::decode_document(v).unwrap();
    assert_eq!(migrated, expected);
    assert_eq!(migrated.version, 11);
    assert!(migrated.audio.sources.is_empty());
    assert_eq!(migrated.audio.buses.len(), 10);
}
#[test]
fn audio_interpretation_history_preserves_ids_and_monotonic_branch_allocation() {
    let mut doc = Document::default();
    let id = doc
        .audio
        .add(Origin::Playback(7), "File".into(), channels(2))
        .unwrap();
    let pair = doc.audio.sources[&id].active[0];
    let before = doc.clone();
    doc.audio
        .interpret(id, Interpretation::DualMono, channels(2))
        .unwrap();
    let mono = doc.audio.sources[&id].active.clone();
    let mut history = History::default();
    history.record(before, &doc);
    assert!(history.undo(&mut doc));
    assert_eq!(doc.audio.sources[&id].active, [pair]);
    assert!(history.redo(&mut doc));
    assert_eq!(doc.audio.sources[&id].active, mono);
    assert!(history.undo(&mut doc));
    let new_id = doc
        .audio
        .add(Origin::Playback(8), "Other".into(), channels(1))
        .unwrap();
    assert!(new_id > *mono.iter().max().unwrap());
    doc.validate().unwrap();
}
#[test]
fn audio_file_scene_duplication_project_and_recovery_round_trips() {
    let root = std::env::temp_dir().join(format!("rastera-audio-project-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    let mut doc = Document::default();
    let file = root.join("missing-but-retained.wav");
    let id = doc
        .audio
        .add(
            Origin::File {
                path: file.clone(),
                duration: 6.,
            },
            "Program".into(),
            channels(4),
        )
        .unwrap();
    let original_scene = doc.active_scene;
    let source = doc.audio.sources.get_mut(&id).unwrap();
    source.scenes = vec![original_scene];
    source.enabled = true;
    source.resume_sample = 144000;
    source.restart_on_activation = true;
    source.strips[1].sends[9] = 0.31;
    source.strips[1].dsp.eq[2].db = -7.;
    source.strips[1].dsp.compressor.ratio = 8.;
    doc.audio
        .interpret(id, Interpretation::QuadMono, channels(4))
        .unwrap();
    let duplicate = doc.add_scene(true).unwrap();
    assert!(doc.audio.sources[&id].scenes.contains(&duplicate));
    assert!(doc.audio_participation().contains(&id));
    let saved = doc.clone();
    let path = root.join("project.rastera.json");
    scene::save_project(&doc, &path).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        json["audio"]["sources"][id.to_string()]["origin"]["File"]["path"],
        "missing-but-retained.wav"
    );
    let loaded = scene::load_project(&path).unwrap();
    assert_eq!(loaded, doc);
    let snapshot = recovery::Snapshot {
        version: 1,
        document: doc.clone(),
        saved: Some(saved),
        project: Some(path.clone()),
        selected: None,
        viewport: None,
        viewports_mode: false,
        window: None,
        clean_windows: BTreeMap::new(),
        panel_rects: BTreeMap::new(),
        playheads: BTreeMap::new(),
        audio_playheads: BTreeMap::from([(
            id,
            recovery::Playhead {
                seconds: 3.5,
                playing: true,
            },
        )]),
        machine: "machine".into(),
        enabled: BTreeMap::new(),
    };
    recovery::store(&root, &recovery::Preferences::default(), &snapshot).unwrap();
    assert_eq!(recovery::load(&root).0, Some(snapshot.clone()));
    assert_eq!(snapshot.document, doc); // Runtime playhead does not dirty the editable document.
    let mut legacy = serde_json::to_value(&snapshot).unwrap();
    legacy.as_object_mut().unwrap().remove("audio_playheads");
    let old_path = root.join("old-journal.json");
    std::fs::write(&old_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(recovery::read_snapshot(&old_path)
        .unwrap()
        .audio_playheads
        .is_empty());
    let mut invalid = snapshot.clone();
    invalid.audio_playheads.get_mut(&id).unwrap().seconds = f64::NAN;
    assert!(invalid.validate().is_err());
    invalid = snapshot.clone();
    for extra in 0..17 {
        invalid.audio_playheads.insert(
            extra,
            recovery::Playhead {
                seconds: 0.,
                playing: false,
            },
        );
    }
    assert!(invalid.validate().is_err());
    let before = doc.clone();
    let mut history = History::default();
    assert!(doc.remove_scene(original_scene));
    history.record(before, &doc);
    assert_eq!(doc.audio.sources[&id].scenes, [duplicate]);
    assert!(history.undo(&mut doc));
    assert!(doc.audio.sources[&id].scenes.contains(&original_scene));
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn schema_eleven_rejects_invalid_audio_transactionally() {
    let original = Document::default();
    let mut value = serde_json::to_value(&original).unwrap();
    value["audio"]["buses"][9]["id"] = 1.into();
    assert!(scene::decode_document(value).is_err());
    assert_eq!(original.audio, Model::default());
}
