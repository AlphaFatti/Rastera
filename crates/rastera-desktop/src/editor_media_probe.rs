use super::*;
use std::time::Instant;
pub(super) struct MediaProbe {
    phase: u32,
    start: Instant,
    source: Id,
    sequence: u64,
    folder: PathBuf,
}
impl MediaProbe {
    pub fn new(folder: PathBuf) -> Self {
        Self {
            phase: 0,
            start: Instant::now(),
            source: 0,
            sequence: 0,
            folder,
        }
    }
    pub fn tick(
        &mut self,
        e: &mut Editor,
        enabled: &mut std::collections::BTreeSet<u64>,
    ) -> Result<bool, String> {
        match self.phase {
            0 => {
                let asset = crate::shared_native::media::inspect(&self.folder.join("test.mp4"))?;
                let layer = e.doc.add_media(asset)?;
                e.doc.layer_mut(layer).unwrap().rect = Rect {
                    x: 100.,
                    y: 100.,
                    width: 960.,
                    height: 540.,
                };
                if let Content::Media(id) = e.doc.layer(layer).unwrap().content {
                    self.source = id;
                }
                let gif = crate::shared_native::media::inspect(&self.folder.join("test.gif"))?;
                let id = e.doc.add_media(gif)?;
                e.doc.layer_mut(id).unwrap().rect = Rect {
                    x: 1200.,
                    y: 100.,
                    width: 640.,
                    height: 360.,
                };
                e.doc.add_text();
                e.publish();
                enabled.insert(1);
                self.phase = 1;
                self.start = Instant::now();
            }
            1 => {
                if self.start.elapsed().as_secs() < 3 {
                    return Ok(false);
                }
                let c = e
                    .media
                    .control(&e.doc, self.source)
                    .ok_or("No video worker")?;
                let f =
                    c.slot.persistent_snapshot().flatten().ok_or_else(|| {
                        format!("Video has no frame: {:?}", c.state.lock().unwrap())
                    })?;
                if f.sequence < 10 {
                    return Err(format!("Video cadence did not advance: {}", f.sequence));
                }
                c.state.lock().unwrap().playing = false;
                self.sequence = f.sequence;
                self.phase = 2;
                self.start = Instant::now();
            }
            2 => {
                if self.start.elapsed().as_secs() < 3 {
                    return Ok(false);
                }
                let c = e.media.control(&e.doc, self.source).unwrap();
                let f = c
                    .slot
                    .persistent_snapshot()
                    .flatten()
                    .ok_or("Paused video incorrectly became stale")?;
                if f.sequence > self.sequence + 3 {
                    return Err("Paused video kept decoding".into());
                }
                c.state.lock().unwrap().seek = Some(0.5);
                c.state.lock().unwrap().playing = true;
                self.phase = 3;
                self.start = Instant::now();
            }
            3 => {
                if self.start.elapsed().as_secs() < 2 {
                    return Ok(false);
                }
                scene::save_project(&e.doc, &self.folder.join("media.rastera.json"))?;
                let d = scene::load_project(&self.folder.join("media.rastera.json"))?;
                if d != e.doc {
                    std::fs::write(
                        self.folder.join("before.json"),
                        serde_json::to_vec_pretty(&e.doc).unwrap(),
                    )
                    .unwrap();
                    std::fs::write(
                        self.folder.join("after.json"),
                        serde_json::to_vec_pretty(&d).unwrap(),
                    )
                    .unwrap();
                    return Err("Media project round trip differs".into());
                }
                log("media_probe passed=1 video_advances=1 paused_frame_valid=1 gif_prepared=1 save_reopen=1\n".into());
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
}
