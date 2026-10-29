//! Reusable output sampling transforms. Canvas origin is top-left; +rotation is clockwise.
pub use crate::theme::{ELECTRIC_YELLOW, SIGNAL_MINT};
use serde::{Deserialize, Serialize};

pub const MIN_VIEWPORT_ZOOM: f32 = 1.0 / 8192.0;
pub const MAX_VIEWPORTS: usize = 8;
pub const VIEWPORT_PRESETS: [(&str, [u32; 2]); 7] = [
    ("HD", [1920, 1080]),
    ("WUXGA", [1920, 1200]),
    ("XGA", [1024, 768]),
    ("Portrait", [1080, 1920]),
    ("Square", [1080, 1080]),
    ("Ultrawide", [3440, 1440]),
    ("Banner", [960, 192]),
];
pub fn validate_output(size: [u32; 2]) -> Result<(), String> {
    if size.contains(&0)
        || size.iter().any(|&n| n > 8192)
        || u64::from(size[0]) * u64::from(size[1]) > 16 * 1024 * 1024
    {
        Err("Output axes must be 1–8192 pixels, with at most 16 Mi-pixels; any aspect ratio is allowed".into())
    } else {
        Ok(())
    }
}
pub const MAX_GENERATION_BYTES: u64 = 256 * 1024 * 1024;
/// Four textures per generation, at most two live generations (512 MiB canvas storage).
pub fn validate_canvas(size: [u32; 2], device_limit: u32) -> Result<u64, String> {
    if size.contains(&0) || size.iter().any(|&n| n > device_limit.min(8192)) {
        return Err(format!(
            "Canvas axes must be 1–{} pixels",
            device_limit.min(8192)
        ));
    }
    let bytes = (size[0] as u64)
        .checked_mul(size[1] as u64)
        .and_then(|p| p.checked_mul(16))
        .ok_or("Canvas allocation overflow")?;
    if bytes > MAX_GENERATION_BYTES {
        return Err("Canvas exceeds 256 MiB four-texture generation budget (16 Mi-pixels); two generations are limited to 512 MiB".into());
    }
    Ok(bytes)
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Viewport {
    pub id: u64,
    pub size: [u32; 2],
    pub name: String,
    pub color: [u8; 4],
    pub center: [f32; 2],
    pub zoom: f32,
    pub rotation: f32,
}
impl Viewport {
    pub fn matrix(&self) -> [f32; 8] {
        let (s, c) = self.rotation.to_radians().sin_cos();
        let a = c / self.zoom;
        let b = -s / self.zoom;
        let d = s / self.zoom;
        let e = c / self.zoom;
        [
            a,
            b,
            self.center[0] - a * self.size[0] as f32 / 2. - b * self.size[1] as f32 / 2.,
            0.,
            d,
            e,
            self.center[1] - d * self.size[0] as f32 / 2. - e * self.size[1] as f32 / 2.,
            0.,
        ]
    }
    pub fn canvas_point(&self, output: [f32; 2]) -> [f32; 2] {
        let m = self.matrix();
        [
            m[0] * output[0] + m[1] * output[1] + m[2],
            m[4] * output[0] + m[5] * output[1] + m[6],
        ]
    }
    pub fn output_point(&self, p: [f32; 2]) -> [f32; 2] {
        let (s, c) = self.rotation.to_radians().sin_cos();
        let x = p[0] - self.center[0];
        let y = p[1] - self.center[1];
        [
            self.size[0] as f32 / 2. + self.zoom * (c * x + s * y),
            self.size[1] as f32 / 2. + self.zoom * (-s * x + c * y),
        ]
    }
    pub fn corners(&self) -> [[f32; 2]; 4] {
        [
            [0., 0.],
            [self.size[0] as f32, 0.],
            [self.size[0] as f32, self.size[1] as f32],
            [0., self.size[1] as f32],
        ]
        .map(|p| self.canvas_point(p))
    }
    pub fn contains(&self, p: [f32; 2]) -> bool {
        let q = self.output_point(p);
        q[0] >= 0. && q[1] >= 0. && q[0] < self.size[0] as f32 && q[1] < self.size[1] as f32
    }
    pub fn sample(&self, output: [f32; 2], canvas: [u32; 2]) -> Option<[u32; 2]> {
        let p = self.canvas_point(output);
        if p[0] >= 0. && p[1] >= 0. && p[0] < canvas[0] as f32 && p[1] < canvas[1] as f32 {
            Some([p[0].floor() as u32, p[1].floor() as u32])
        } else {
            None
        }
    }
    pub fn fit_canvas(&mut self, size: [u32; 2]) {
        self.center = [size[0] as f32 / 2., size[1] as f32 / 2.];
        self.zoom =
            (self.size[0] as f32 / size[0] as f32).min(self.size[1] as f32 / size[1] as f32);
        self.rotation = 0.;
    }
    pub fn validate(&self) -> bool {
        self.id > 0
            && validate_output(self.size).is_ok()
            && !self.name.trim().is_empty()
            && !self.name.chars().any(char::is_control)
            && self.name.len() <= 128
            && self
                .center
                .iter()
                .all(|x| x.is_finite() && x.abs() <= 65536.)
            && self.zoom.is_finite()
            && (MIN_VIEWPORT_ZOOM..=8192.).contains(&self.zoom)
            && self.rotation.is_finite()
            && self.rotation.abs() <= 36000.
    }
}
pub fn defaults() -> Vec<Viewport> {
    vec![
        Viewport {
            id: 1,
            size: [1920, 1080],
            name: "Left".into(),
            color: SIGNAL_MINT,
            center: [960., 540.],
            zoom: 1.,
            rotation: 0.,
        },
        Viewport {
            id: 2,
            size: [1920, 1080],
            name: "Right".into(),
            color: ELECTRIC_YELLOW,
            center: [2880., 540.],
            zoom: 1.,
            rotation: 0.,
        },
    ]
}
/// Immutable metadata owned by the same lease as the completed canvas texture.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameViewport {
    pub id: u64,
    pub size: [u32; 2],
    pub matrix: [f32; 8],
}
const _: [(); 48] = [(); std::mem::size_of::<FrameViewport>()];
impl From<&Viewport> for FrameViewport {
    fn from(v: &Viewport) -> Self {
        Self {
            id: v.id,
            size: v.size,
            matrix: v.matrix(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct FrameMeta {
    pub canvas: [u32; 2],
    pub viewports: std::sync::Arc<[FrameViewport]>,
}
impl FrameMeta {
    pub fn viewport(&self, id: u64) -> Option<&FrameViewport> {
        self.viewports.iter().find(|v| v.id == id)
    }
}
impl Default for FrameMeta {
    fn default() -> Self {
        Self {
            canvas: [3840, 1080],
            viewports: defaults()
                .iter()
                .map(FrameViewport::from)
                .collect::<Vec<_>>()
                .into(),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_seam_is_exact() {
        let v = defaults();
        for y in 0..1080 {
            for x in 0..1920 {
                assert_eq!(
                    v[0].sample([x as f32 + 0.5, y as f32 + 0.5], [3840, 1080]),
                    Some([x, y])
                );
                assert_eq!(
                    v[1].sample([x as f32 + 0.5, y as f32 + 0.5], [3840, 1080]),
                    Some([x + 1920, y])
                );
            }
        }
    }
    #[test]
    fn rotate_zoom_inverse_and_black() {
        let mut v = defaults()[0].clone();
        v.rotation = 90.;
        v.zoom = 2.;
        let p = v.canvas_point([1060., 540.]);
        assert!((p[0] - 960.).abs() < 0.001 && (p[1] - 590.).abs() < 0.001);
        for r in [-179., -30., 0., 45., 179.] {
            v.rotation = r;
            for q in [[0., 0.], [960., 540.], [1919., 1079.]] {
                let a = v.output_point(v.canvas_point(q));
                assert!((a[0] - q[0]).abs() < 0.002 && (a[1] - q[1]).abs() < 0.002);
            }
        }
        v.center = [-10000., 0.];
        assert_eq!(v.sample([960., 540.], [3840, 1080]), None);
    }
    #[test]
    fn canvas_sizes_and_fit() {
        for s in [[1, 1], [1080, 1920], [3839, 1079], [4096, 4096]] {
            assert!(validate_canvas(s, 8192).is_ok());
            let mut v = defaults()[0].clone();
            v.fit_canvas(s);
            assert!(v.validate());
            assert_eq!(v.center, [s[0] as f32 / 2., s[1] as f32 / 2.]);
        }
        for s in [[0, 1080], [8193, 1], [8192, 8192], [u32::MAX, u32::MAX]] {
            assert!(validate_canvas(s, 8192).is_err());
        }
        assert!(validate_canvas([3840, 1080], 2048).is_err());
    }
}
