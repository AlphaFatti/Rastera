//! Full-source unit plane -> normalized destination homography. Order: TL, TR, BL, BR.
//! The ordinary visible rect and crop reconstruct the stable uncropped plane.
use crate::scene::{Layer, Rect};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pin {
    pub corners: [[f64; 2]; 4],
}
impl Default for Pin {
    fn default() -> Self {
        Self {
            corners: [[0., 0.], [1., 0.], [0., 1.], [1., 1.]],
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Homography {
    pub rows: [[f64; 3]; 3],
}
impl Homography {
    pub fn map(self, p: [f64; 2]) -> Option<[f64; 2]> {
        let m = self.rows;
        let w = m[2][0] * p[0] + m[2][1] * p[1] + m[2][2];
        if !w.is_finite() || w.abs() < 1e-10 {
            return None;
        }
        let q = [
            (m[0][0] * p[0] + m[0][1] * p[1] + m[0][2]) / w,
            (m[1][0] * p[0] + m[1][1] * p[1] + m[1][2]) / w,
        ];
        q.iter().all(|x| x.is_finite()).then_some(q)
    }
    pub fn inverse(self) -> Option<Self> {
        let m = self.rows;
        let [a, b, c] = m[0];
        let [d, e, f] = m[1];
        let [g, h, i] = m[2];
        let cof = [
            [e * i - f * h, c * h - b * i, b * f - c * e],
            [f * g - d * i, a * i - c * g, c * d - a * f],
            [d * h - e * g, b * g - a * h, a * e - b * d],
        ];
        let det = a * cof[0][0] + b * cof[1][0] + c * cof[2][0];
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        Some(Self {
            rows: cof.map(|r| r.map(|v| v / det)),
        })
    }
}
impl Pin {
    pub fn homography(self) -> Result<Homography, String> {
        let q = self.corners;
        let order = [0, 1, 3, 2];
        let err = || {
            "Corner Pin requires a stable, clockwise convex quadrilateral (TL, TR, BL, BR)"
                .to_string()
        };
        if q.iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() > 10000.)
        {
            return Err(err());
        }
        let span = (q.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max)
            - q.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min))
        .max(
            q.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max)
                - q.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min),
        );
        for n in 0..4 {
            let a = q[order[n]];
            let b = q[order[(n + 1) % 4]];
            let c = q[order[(n + 2) % 4]];
            let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
            if cross <= 1e-6 * span * span || span < 1e-5 {
                return Err(err());
            }
        }
        let dx1 = q[1][0] - q[3][0];
        let dx2 = q[2][0] - q[3][0];
        let dx3 = q[0][0] - q[1][0] - q[2][0] + q[3][0];
        let dy1 = q[1][1] - q[3][1];
        let dy2 = q[2][1] - q[3][1];
        let dy3 = q[0][1] - q[1][1] - q[2][1] + q[3][1];
        let det = dx1 * dy2 - dx2 * dy1;
        if det.abs() < 1e-12 {
            return Err(err());
        }
        let g = (dx3 * dy2 - dx2 * dy3) / det;
        let h = (dx1 * dy3 - dx3 * dy1) / det;
        let m = Homography {
            rows: [
                [
                    q[1][0] - q[0][0] + g * q[1][0],
                    q[2][0] - q[0][0] + h * q[2][0],
                    q[0][0],
                ],
                [
                    q[1][1] - q[0][1] + g * q[1][1],
                    q[2][1] - q[0][1] + h * q[2][1],
                    q[0][1],
                ],
                [g, h, 1.],
            ],
        };
        let w = [1., 1. + g, 1. + h, 1. + g + h];
        if w.iter().any(|v| !v.is_finite() || *v < 1e-4)
            || w.iter().copied().fold(0., f64::max)
                / w.iter().copied().fold(f64::INFINITY, f64::min)
                > 10000.
        {
            return Err(err());
        }
        let inv = m.inverse().ok_or_else(err)?;
        let norm = |a: Homography| {
            a.rows
                .iter()
                .map(|r| r.iter().map(|x| x.abs()).sum::<f64>())
                .fold(0., f64::max)
        };
        if norm(m) * norm(inv) > 1e8 {
            return Err(err());
        }
        Ok(m)
    }
}
impl Layer {
    pub fn source_canvas(&self, uv: [f64; 2]) -> Option<[f32; 2]> {
        let f = self.crop.full_rect(self.rect);
        let q = self.pin.homography().ok()?.map(uv)?;
        Some([f.x + q[0] as f32 * f.width, f.y + q[1] as f32 * f.height])
    }
    pub fn source_point(&self, p: [f32; 2]) -> Option<[f64; 2]> {
        let f = self.crop.full_rect(self.rect);
        self.pin.homography().ok()?.inverse()?.map([
            ((p[0] - f.x) / f.width) as f64,
            ((p[1] - f.y) / f.height) as f64,
        ])
    }
    pub fn visible_corners(&self) -> [[f32; 2]; 4] {
        let c = self.crop;
        [
            [c.left, c.top],
            [c.right, c.top],
            [c.left, c.bottom],
            [c.right, c.bottom],
        ]
        .map(|p| {
            self.source_canvas([p[0] as f64, p[1] as f64])
                .unwrap_or([f32::NAN; 2])
        })
    }
    pub fn contains(&self, p: [f32; 2]) -> bool {
        self.source_point(p).is_some_and(|q| {
            q[0] >= self.crop.left as f64
                && q[0] < self.crop.right as f64
                && q[1] >= self.crop.top as f64
                && q[1] < self.crop.bottom as f64
        })
    }
    pub fn set_visible_corners(&mut self, canvas: [[f32; 2]; 4]) -> Result<(), String> {
        let full = self.crop.full_rect(self.rect);
        let q = canvas.map(|p| {
            [
                ((p[0] - full.x) / full.width) as f64,
                ((p[1] - full.y) / full.height) as f64,
            ]
        });
        // First solve visible unit square -> requested visible quad, then compose crop inverse.
        let visible = Pin { corners: q }.homography()?;
        let c = self.crop;
        let corners = Pin::default().corners.map(|p| {
            visible
                .map([
                    (p[0] - c.left as f64) / (c.right - c.left) as f64,
                    (p[1] - c.top as f64) / (c.bottom - c.top) as f64,
                ])
                .unwrap_or([f64::NAN; 2])
        });
        let pin = Pin { corners };
        pin.homography()?;
        let mut next = self.clone();
        next.pin = pin;
        next.validate_pin()?;
        self.pin = pin;
        Ok(())
    }
    pub fn validate_pin(&self) -> Result<(), String> {
        self.pin.homography()?;
        let r = self.rect;
        if [r.x, r.y, r.width, r.height].iter().any(|n| !n.is_finite())
            || r.x.abs() > 32768.
            || r.y.abs() > 32768.
            || r.width <= 0.
            || r.height <= 0.
            || r.width > 8192.
            || r.height > 8192.
        {
            return Err("Layer geometry exceeds ordinary placement limits".into());
        }
        if self
            .visible_corners()
            .iter()
            .flatten()
            .any(|n| !n.is_finite() || n.abs() > 65536.)
        {
            return Err("Warped visible corners must remain within ±65536 canvas pixels".into());
        }
        Ok(())
    }
    pub fn resize_pinned(
        &self,
        corner: usize,
        grab: [f32; 2],
        pointer: [f32; 2],
        aspect: Option<f32>,
    ) -> Result<Rect, String> {
        let anchor = self.visible_corners()[3 - corner];
        let mut next = self.clone();
        next.rect = crate::scene::resize_drag(self.rect, corner, grab, pointer, aspect);
        let moved = next.visible_corners()[3 - corner];
        next.rect.x += anchor[0] - moved[0];
        next.rect.y += anchor[1] - moved[1];
        next.validate_pin()?;
        Ok(next.rect)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn layer() -> Layer {
        let mut d = crate::scene::Document::default();
        let id = d.add_text();
        d.layer(id).unwrap().clone()
    }
    #[test]
    fn identity_perspective_inverse_and_grid_continuity() {
        let id = Pin::default().homography().unwrap();
        assert_eq!(id.map([0.2, 0.7]), Some([0.2, 0.7]));
        let m = Pin {
            corners: [[0., 0.], [1.2, 0.2], [0.2, 1.], [0.8, 0.8]],
        }
        .homography()
        .unwrap();
        let inv = m.inverse().unwrap();
        for y in 0..=20 {
            for x in 0..=20 {
                let p = [x as f64 / 20., y as f64 / 20.];
                let q = inv.map(m.map(p).unwrap()).unwrap();
                assert!((p[0] - q[0]).abs() < 1e-10 && (p[1] - q[1]).abs() < 1e-10);
            }
        }
    }
    #[test]
    fn rejects_crossed_concave_collapsed_and_poles() {
        for q in [
            [[0., 0.], [1., 1.], [0., 1.], [1., 0.]],
            [[0., 0.], [1., 0.], [0., 1.], [0.1, 0.1]],
            [[0., 0.]; 4],
            [[f64::NAN, 0.], [1., 0.], [0., 1.], [1., 1.]],
        ] {
            assert!(Pin { corners: q }.homography().is_err());
        }
    }
    #[test]
    fn warped_crop_preserves_surviving_points_and_reset() {
        let mut l = layer();
        l.pin.corners = [[0., 0.], [1.1, 0.1], [0.2, 1.], [0.9, 0.8]];
        let p = l.source_canvas([0.4, 0.6]).unwrap();
        l.set_crop(crate::crop::Crop {
            left: 0.2,
            top: 0.1,
            right: 0.8,
            bottom: 0.9,
        })
        .unwrap();
        let q = l.source_canvas([0.4, 0.6]).unwrap();
        assert!((p[0] - q[0]).abs() < 0.001 && (p[1] - q[1]).abs() < 0.001);
        let crop = l.crop;
        let rect = l.rect;
        l.pin = Pin::default();
        assert_eq!(l.crop, crop);
        assert_eq!(l.rect, rect);
    }
    #[test]
    fn hit_resize_move_and_edit_cropped_visible_corners() {
        let mut l = layer();
        l.pin.corners = [[0.2, 0.], [0.8, 0.], [0., 1.], [1., 1.]];
        assert!(!l.contains([l.rect.x + 1., l.rect.y + 1.]));
        let p = l.source_canvas([0.5, 0.5]).unwrap();
        assert!(l.contains(p));
        let anchor = l.visible_corners()[0];
        l.rect = l
            .resize_pinned(
                3,
                l.visible_corners()[3],
                [
                    l.visible_corners()[3][0] + 100.,
                    l.visible_corners()[3][1] + 50.,
                ],
                None,
            )
            .unwrap();
        assert!((l.visible_corners()[0][0] - anchor[0]).abs() < 0.001);
        let q = l.visible_corners();
        l.rect.x -= 1000.;
        assert!((l.visible_corners()[1][0] - q[1][0] + 1000.).abs() < 0.001);
        let mut corners = l.visible_corners();
        corners[0][0] += 25.;
        l.set_visible_corners(corners).unwrap();
        assert!((l.visible_corners()[0][0] - corners[0][0]).abs() < 0.01);
    }
}
#[cfg(test)]
mod persistence_tests {
    use super::*;
    #[test]
    fn independent_instances_scene_history_and_legacy_migration() {
        let mut d = crate::scene::Document::default();
        let id = d.add_image(crate::scene::ImageAsset {
            path: "missing.png".into(),
            width: 640,
            height: 480,
        });
        d.layer_mut(id).unwrap().pin.corners[0] = [0.15, 0.1];
        let copy = d.duplicate(id).unwrap();
        assert_eq!(d.layer(id).unwrap().pin, d.layer(copy).unwrap().pin);
        let before = d.clone();
        d.layer_mut(copy).unwrap().pin.corners[0] = [0.25, 0.2];
        assert_ne!(d.layer(id).unwrap().pin, d.layer(copy).unwrap().pin);
        let mut history = crate::scene::History::default();
        history.record(before.clone(), &d);
        history.undo(&mut d);
        assert_eq!(d, before);
        d.add_scene(true).unwrap();
        assert_eq!(d.scenes[0].layers[0].pin, d.scenes[1].layers[0].pin);
        let mut v = serde_json::to_value(&d).unwrap();
        v["version"] = 9.into();
        for s in v["scenes"].as_array_mut().unwrap() {
            for l in s["layers"].as_array_mut().unwrap() {
                l.as_object_mut().unwrap().remove("pin");
            }
        }
        let migrated = crate::scene::decode_document(v).unwrap();
        assert_eq!(migrated.project_id, d.project_id);
        assert_eq!(migrated.viewports, d.viewports);
        assert!(migrated.all_layers().all(|l| l.pin == Pin::default()));
    }
}
#[cfg(test)]
mod interpolation_tests {
    use super::*;
    #[test]
    fn gpu_perspective_interpolation_matches_one_homography_across_diagonal() {
        let m = Pin {
            corners: [[0., 0.], [1.2, 0.2], [0.2, 1.], [0.8, 0.8]],
        }
        .homography()
        .unwrap();
        let interpolate = |triangle: [[f64; 2]; 3], uv: [f64; 2]| {
            let s = triangle.map(|v| m.map(v).unwrap());
            let p = m.map(uv).unwrap();
            let det = (s[1][1] - s[2][1]) * (s[0][0] - s[2][0])
                + (s[2][0] - s[1][0]) * (s[0][1] - s[2][1]);
            let a = ((s[1][1] - s[2][1]) * (p[0] - s[2][0])
                + (s[2][0] - s[1][0]) * (p[1] - s[2][1]))
                / det;
            let b = ((s[2][1] - s[0][1]) * (p[0] - s[2][0])
                + (s[0][0] - s[2][0]) * (p[1] - s[2][1]))
                / det;
            let weights = [a, b, 1. - a - b];
            let iw = triangle.map(|v| 1. / (m.rows[2][0] * v[0] + m.rows[2][1] * v[1] + 1.));
            let denominator = (0..3).map(|i| weights[i] * iw[i]).sum::<f64>();
            [0, 1].map(|axis| {
                (0..3)
                    .map(|i| weights[i] * iw[i] * triangle[i][axis])
                    .sum::<f64>()
                    / denominator
            })
        };
        let left = [[0., 0.], [1., 0.], [0., 1.]];
        let right = [[0., 1.], [1., 0.], [1., 1.]];
        for i in 1..20 {
            let p = [i as f64 / 20., 1. - i as f64 / 20.];
            for q in [interpolate(left, p), interpolate(right, p)] {
                assert!((p[0] - q[0]).abs() < 1e-10 && (p[1] - q[1]).abs() < 1e-10);
            }
        }
    }
}
