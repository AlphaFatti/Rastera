//! Per-instance source UV bounds. Layer rectangles describe the visible region.
use crate::scene::{Layer, Rect};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Crop {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}
impl Default for Crop {
    fn default() -> Self {
        Self {
            left: 0.,
            top: 0.,
            right: 1.,
            bottom: 1.,
        }
    }
}
impl Crop {
    pub fn validate(self) -> Result<(), String> {
        if [self.left, self.top, self.right, self.bottom]
            .iter()
            .all(|x| x.is_finite() && (0.0..=1.).contains(x))
            && self.right - self.left >= 0.0001
            && self.bottom - self.top >= 0.0001
        {
            Ok(())
        } else {
            Err("Crop must be finite, inside the source and nonempty".into())
        }
    }
    pub fn aspect(self) -> f32 {
        (self.right - self.left) / (self.bottom - self.top)
    }
    pub fn full_rect(self, visible: Rect) -> Rect {
        let width = visible.width / (self.right - self.left);
        let height = visible.height / (self.bottom - self.top);
        Rect {
            x: visible.x - self.left * width,
            y: visible.y - self.top * height,
            width,
            height,
        }
    }
    pub fn rect(self, full: Rect) -> Rect {
        Rect {
            x: full.x + self.left * full.width,
            y: full.y + self.top * full.height,
            width: full.width * (self.right - self.left),
            height: full.height * (self.bottom - self.top),
        }
    }
    pub fn corner(self, corner: usize, full: Rect, delta: [f32; 2]) -> Self {
        let mut c = self;
        if corner.is_multiple_of(2) {
            c.left = (c.left + delta[0] / full.width).clamp(0., c.right - 0.0001);
        } else {
            c.right = (c.right + delta[0] / full.width).clamp(c.left + 0.0001, 1.);
        }
        if corner < 2 {
            c.top = (c.top + delta[1] / full.height).clamp(0., c.bottom - 0.0001);
        } else {
            c.bottom = (c.bottom + delta[1] / full.height).clamp(c.top + 0.0001, 1.);
        }
        c
    }
}
impl Layer {
    /// Keep source scale and full-source position; never stretch the surviving pixels.
    pub fn set_crop(&mut self, crop: Crop) -> Result<(), String> {
        crop.validate()?;
        let rect = crop.rect(self.crop.full_rect(self.rect));
        if rect.width > 8192.
            || rect.height > 8192.
            || rect.x.abs() > 32768.
            || rect.y.abs() > 32768.
        {
            return Err(
                "Reset crop would exceed layer geometry limits; reduce layer size first".into(),
            );
        }
        self.rect = rect;
        self.crop = crop;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_preserves_source_pixel_mapping_and_reset() {
        let mut d = crate::scene::Document::default();
        let id = d.add_text();
        let l = d.layer_mut(id).unwrap();
        let original = l.rect;
        let c = Crop {
            left: 0.25,
            top: 0.1,
            right: 0.75,
            bottom: 0.9,
        };
        l.set_crop(c).unwrap();
        assert_eq!(l.rect.width, original.width * 0.5);
        assert_eq!(l.rect.x, original.x + original.width * 0.25);
        let p = c.full_rect(l.rect);
        assert!((p.height - original.height).abs() < 0.01);
        l.set_crop(Crop::default()).unwrap();
        assert!((l.rect.width - original.width).abs() < 0.01);
    }
    #[test]
    fn invalid_and_crossing_handles() {
        assert!(Crop {
            right: 0.,
            ..Crop::default()
        }
        .validate()
        .is_err());
        assert!(Crop {
            top: f32::NAN,
            ..Crop::default()
        }
        .validate()
        .is_err());
        let r = Rect {
            x: 0.,
            y: 0.,
            width: 1000.,
            height: 500.,
        };
        let c = Crop::default().corner(0, r, [2000., 2000.]);
        assert!(c.validate().is_ok());
        assert!(c.left < c.right);
    }
    #[test]
    fn independent_crops_shared_camera_and_history() {
        let mut d = crate::scene::Document::default();
        let id = d
            .add_capture(crate::capture::Source {
                device_id: "missing".into(),
                name: "Camera".into(),
                mode: crate::capture::Mode {
                    width: 1920,
                    height: 1080,
                    fps: 30.,
                },
            })
            .unwrap();
        let other = d.duplicate(id).unwrap();
        let before = d.clone();
        d.layer_mut(id)
            .unwrap()
            .set_crop(Crop {
                right: 0.5,
                ..Crop::default()
            })
            .unwrap();
        assert_eq!(d.captures.len(), 1);
        assert_eq!(d.layer(other).unwrap().crop, Crop::default());
        assert_eq!(d.aspect(id), Some(960. / 1080.));
        let mut h = crate::scene::History::default();
        h.record(before.clone(), &d);
        h.undo(&mut d);
        assert_eq!(d, before);
    }
}
