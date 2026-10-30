//! File assets and independently addressable playback instances. No operator clock.
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub path: PathBuf,
    pub size: [u32; 2],
    pub duration: f64,
    pub gif: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum End {
    Hold,
    Black,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Playback {
    pub asset: u64,
    pub playing: bool,
    pub looping: Option<bool>,
    pub end: End,
    pub restart_on_activation: bool,
}
#[derive(Clone, Debug)]
pub struct State {
    pub position: f64,
    pub playing: bool,
    pub duration: f64,
    pub phase: String,
    pub seek: Option<f64>,
}
pub struct Control {
    pub state: Mutex<State>,
    pub slot: Arc<crate::capture::Slot>,
}
impl Control {
    pub fn new(slot: Arc<crate::capture::Slot>, playing: bool, duration: f64) -> Self {
        Self {
            slot,
            state: Mutex::new(State {
                position: 0.,
                playing,
                duration,
                phase: "Loading".into(),
                seek: None,
            }),
        }
    }
}
pub fn key(project: &str, id: u64) -> String {
    format!("media:{project}:{id}")
}
pub fn validate_asset(a: &Asset) -> Result<(), String> {
    if a.size.contains(&0)
        || a.size[0] > 1920
        || a.size[1] > 1920
        || u64::from(a.size[0]) * u64::from(a.size[1]) > 1920 * 1080
        || !a.duration.is_finite()
        || a.duration <= 0.
        || a.duration > 86400.
    {
        Err(
            "Media limit: SDR up to 1920×1080 (either orientation), finite duration ≤24 hours"
                .into(),
        )
    } else {
        Ok(())
    }
}
pub fn delay_seconds(delay: image::Delay) -> f64 {
    let (n, d) = delay.numer_denom_ms();
    (n as f64 / d as f64 / 1000.).max(0.01)
}
pub fn premultiplied_bgra(mut rgba: Vec<u8>) -> Vec<u8> {
    for p in rgba.as_chunks_mut::<4>().0 {
        let a = p[3] as u16;
        let r = (p[0] as u16 * a / 255) as u8;
        p[0] = (p[2] as u16 * a / 255) as u8;
        p[1] = (p[1] as u16 * a / 255) as u8;
        p[2] = r;
    }
    rgba
}
/// Monotonic accumulated playback clock; paused frames retain their content indefinitely.
pub struct Clock {
    position: f64,
    last: Instant,
}
impl Default for Clock {
    fn default() -> Self {
        Self {
            position: 0.,
            last: Instant::now(),
        }
    }
}
impl Clock {
    pub fn tick(&mut self, playing: bool, seek: Option<f64>) -> f64 {
        let now = Instant::now();
        if let Some(s) = seek {
            self.position = s.max(0.);
        } else if playing {
            self.position += now.duration_since(self.last).as_secs_f64();
        }
        self.last = now;
        self.position
    }
}
pub fn gif_decoder(
    path: &std::path::Path,
) -> Result<image::codecs::gif::GifDecoder<std::io::BufReader<std::fs::File>>, String> {
    use image::ImageDecoder;
    let f =
        std::fs::File::open(path).map_err(|e| format!("Missing GIF {}: {e}", path.display()))?;
    if f.metadata().map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
        return Err("GIF file limit: 64 MiB".into());
    }
    let mut d = image::codecs::gif::GifDecoder::new(std::io::BufReader::new(f))
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1920);
    limits.max_image_height = Some(1920);
    limits.max_alloc = Some(32 * 1024 * 1024);
    d.set_limits(limits).map_err(|e| e.to_string())?;
    Ok(d)
}
/// Streaming decode/composition via image-rs, with only current/next frames, never a whole clip cache.
pub fn run_gif(
    asset: Asset,
    control: Arc<Control>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    loop_override: Option<bool>,
    end: End,
) {
    use image::AnimationDecoder;
    use std::sync::atomic::Ordering;
    let epoch = control.slot.reset();
    let result = (|| -> Result<(), String> {
        let mut clock = Clock::default();
        let mut last = 0.;
        let mut cycle = 0u32;
        let decoder = gif_decoder(&asset.path)?;
        let limit = gif_plays(&asset.path)?;
        let mut frames = decoder.into_frames();
        let mut next_at = 0.;
        let mut done = false;
        while !stop.load(Ordering::Relaxed) {
            let (playing, seek) = {
                let mut s = control.state.lock().unwrap();
                (s.playing, s.seek.take())
            };
            let position = clock.tick(playing, seek);
            if position < last || seek.is_some() {
                frames = gif_decoder(&asset.path)?.into_frames();
                next_at = 0.;
                done = false;
                cycle = 0;
            }
            last = position;
            let mut decoded = 0;
            while !done && position >= next_at && decoded < 8 {
                decoded += 1;
                match frames.next() {
                    Some(frame) => {
                        let frame = frame.map_err(|e| e.to_string())?;
                        next_at += delay_seconds(frame.delay());
                        let b = frame.into_buffer();
                        if [b.width(), b.height()] != asset.size {
                            return Err("GIF dimensions changed; reimport".into());
                        }
                        control.slot.deliver_owned(
                            epoch,
                            b.width(),
                            b.height(),
                            premultiplied_bgra(b.into_raw()),
                        );
                    }
                    None => {
                        cycle += 1;
                        let repeats = loop_override.unwrap_or(limit.is_none_or(|n| cycle < n));
                        if repeats {
                            frames = gif_decoder(&asset.path)?.into_frames();
                        } else {
                            done = true;
                            control.state.lock().unwrap().playing = false;
                            if end == End::Black {
                                control.slot.phase(epoch, "Ended · Black");
                            }
                        }
                    }
                }
            }
            {
                let mut s = control.state.lock().unwrap();
                s.position = if done {
                    asset.duration
                } else {
                    position.rem_euclid(asset.duration)
                };
                s.phase = if done {
                    "Ended"
                } else if playing {
                    "Playing"
                } else {
                    "Paused"
                }
                .into();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(())
    })();
    if let Err(e) = result {
        control.state.lock().unwrap().phase = format!("Error: {e}");
        control.slot.phase(epoch, "Error");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn premultiplied_channels_and_delay() {
        assert_eq!(
            premultiplied_bgra(vec![200, 100, 50, 128]),
            vec![25, 50, 100, 128]
        );
        assert_eq!(delay_seconds(image::Delay::from_numer_denom_ms(0, 1)), 0.01);
    }
    #[test]
    fn paused_clock_and_seek() {
        let mut c = Clock::default();
        assert_eq!(c.tick(false, Some(12.)), 12.);
        assert_eq!(c.tick(false, None), 12.);
    }
    #[test]
    fn held_media_does_not_stale() {
        let hub = crate::capture::Hub::default();
        let s = hub.slot("test-media");
        let epoch = s.reset();
        s.deliver_owned(epoch, 1, 1, vec![0, 0, 0, 0]);
        assert!(s.persistent_snapshot().unwrap().is_some());
    }
}

pub fn inspect_gif(path: &std::path::Path) -> Result<Asset, String> {
    use image::{AnimationDecoder, ImageDecoder};
    let d = gif_decoder(path)?;
    let size = d.dimensions();
    let mut duration = 0.;
    let mut count = 0;
    for f in d.into_frames() {
        duration += delay_seconds(f.map_err(|e| e.to_string())?.delay());
        count += 1;
        if count > 10000 || duration > 86400. {
            return Err("GIF limit: 10000 frames / 24 hours".into());
        }
    }
    let a = Asset {
        path: path.to_path_buf(),
        size: [size.0, size.1],
        duration,
        gif: true,
    };
    validate_asset(&a)?;
    Ok(a)
}

#[cfg(test)]
mod gif_tests {
    use super::*;
    #[test]
    fn timing_loop_and_transparency_fixture() {
        use image::AnimationDecoder;
        let path = std::env::temp_dir().join(format!("rastera-gif-{}.gif", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        let mut e = image::codecs::gif::GifEncoder::new(file);
        e.set_repeat(image::codecs::gif::Repeat::Finite(2)).unwrap();
        let frames = [
            ([255, 0, 0, 255], 20),
            ([0, 255, 0, 128], 40),
            ([0, 0, 0, 0], 80),
        ]
        .into_iter()
        .map(|(c, ms)| {
            image::Frame::from_parts(
                image::RgbaImage::from_pixel(2, 2, image::Rgba(c)),
                0,
                0,
                image::Delay::from_numer_denom_ms(ms, 1),
            )
        });
        e.encode_frames(frames).unwrap();
        drop(e);
        let a = inspect_gif(&path).unwrap();
        assert!((a.duration - 0.14).abs() < 0.001);
        assert_eq!(gif_plays(&path).unwrap(), Some(3));
        let decoder = gif_decoder(&path).unwrap();
        assert!(matches!(decoder.loop_count(),image::metadata::LoopCount::Finite(n) if n.get()==2));
        let frames: Vec<_> = decoder.into_frames().map(Result::unwrap).collect();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].buffer().get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(frames[2].buffer().get_pixel(0, 0)[3], 0);
        std::fs::remove_file(path).unwrap();
    }
}

/// Netscape repeat extension counts repeats *after* the first play. No extension means once.
/// Parse bounded metadata blocks; image-rs remains responsible for actual pixels/disposal.
pub fn gif_plays(path: &std::path::Path) -> Result<Option<u32>, String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut r = std::io::BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?);
    fn parse(r: &mut std::io::BufReader<std::fs::File>) -> std::io::Result<Option<u32>> {
        let mut header = [0; 13];
        r.read_exact(&mut header)?;
        if &header[..3] != b"GIF" {
            return Err(std::io::Error::other("Not GIF"));
        }
        if header[10] & 128 != 0 {
            r.seek(SeekFrom::Current(3 * (1i64 << ((header[10] & 7) + 1))))?;
        }
        loop {
            let mut code = [0];
            r.read_exact(&mut code)?;
            match code[0] {
                0x3b => return Ok(Some(1)),
                0x21 => {
                    let mut label = [0];
                    r.read_exact(&mut label)?;
                    let mut app = false;
                    let mut first = true;
                    loop {
                        let mut n = [0];
                        r.read_exact(&mut n)?;
                        if n[0] == 0 {
                            break;
                        }
                        let mut b = vec![0; n[0] as usize];
                        r.read_exact(&mut b)?;
                        if first {
                            app = label[0] == 0xff && (b == b"NETSCAPE2.0" || b == b"ANIMEXTS1.0");
                            first = false;
                        } else if app && b.len() == 3 && b[0] == 1 {
                            let repeats = u16::from_le_bytes([b[1], b[2]]) as u32;
                            return Ok(if repeats == 0 {
                                None
                            } else {
                                Some(repeats + 1)
                            });
                        }
                    }
                }
                0x2c => {
                    let mut desc = [0; 9];
                    r.read_exact(&mut desc)?;
                    if desc[8] & 128 != 0 {
                        r.seek(SeekFrom::Current(3 * (1i64 << ((desc[8] & 7) + 1))))?;
                    }
                    let mut lzw = [0];
                    r.read_exact(&mut lzw)?;
                    loop {
                        let mut n = [0];
                        r.read_exact(&mut n)?;
                        if n[0] == 0 {
                            break;
                        }
                        r.seek(SeekFrom::Current(n[0] as i64))?;
                    }
                }
                _ => return Err(std::io::Error::other("Invalid GIF block")),
            }
        }
    }
    parse(&mut r).map_err(|e| e.to_string())
}
