//! Operator hit geometry in screen points, derived from the shared sampling transform.
use crate::{mapping::Viewport, scene::PreviewMap};

pub const OPTION_MODIFIER: u64 = 1 << 19; // NSEventModifierFlagOption, bridge preserves native flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    Pan,
    Resize,
    Rotate,
    Cycle,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub id: u64,
    pub kind: HitKind,
}
#[derive(Clone, Debug)]
pub struct Footprint {
    pub corners: [[f32; 2]; 4],
    pub rotation: [f32; 2],
    pub label: [f32; 4],
}
fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let d = [b[0] - a[0], b[1] - a[1]];
    let length = d[0] * d[0] + d[1] * d[1];
    let t = if length > 0. {
        ((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / length
    } else {
        0.
    }
    .clamp(0., 1.);
    distance(p, [a[0] + t * d[0], a[1] + t * d[1]])
}
impl Footprint {
    /// `map.fit` and pointer events use backing pixels; all returned geometry uses points.
    /// Label size is measured by the native renderer with its actual font.
    pub fn new(v: &Viewport, map: PreviewMap, scale: f32, label_size: [f32; 2]) -> Self {
        let screen = |p: [f32; 2]| {
            [
                (map.fit[0] + p[0] / map.canvas[0] as f32 * map.fit[2]) / scale,
                (map.fit[1] + p[1] / map.canvas[1] as f32 * map.fit[3]) / scale,
            ]
        };
        let corners = v.corners().map(screen);
        let top = screen(v.canvas_point([v.size[0] as f32 / 2., 0.]));
        // Derive orientation analytically, even when a tiny footprint rounds to its center.
        let (sin, cos) = v.rotation.to_radians().sin_cos();
        let rotation = [top[0] + sin * 24., top[1] - cos * 24.];
        Self {
            corners,
            rotation,
            label: [
                corners[0][0] + 6.,
                corners[0][1] + 6.,
                label_size[0],
                label_size[1],
            ],
        }
    }
    pub fn outline_distance(&self, p: [f32; 2]) -> f32 {
        (0..4)
            .map(|i| segment(p, self.corners[i], self.corners[(i + 1) % 4]))
            .fold(f32::INFINITY, f32::min)
    }
    fn label_hit(&self, p: [f32; 2]) -> bool {
        let [x, y, w, h] = self.label;
        p[0] >= x - 2. && p[1] >= y - 2. && p[0] <= x + w + 2. && p[1] <= y + h + 2.
    }
    fn handle(&self, p: [f32; 2]) -> Option<HitKind> {
        if distance(p, self.rotation) <= 9. {
            Some(HitKind::Rotate)
        } else if self.corners.iter().any(|&q| distance(p, q) <= 9.) {
            Some(HitKind::Resize)
        } else {
            None
        }
    }
}
/// Ordinary click: selected handles, all labels, closest outline, selected body, other bodies.
/// Equal label/outline hits prefer the selection, then reverse paint order. Option overrides
/// handles and cycles all hit footprints in stable ID order, creating no drag gesture.
pub fn pick(
    viewports: &[Viewport],
    map: PreviewMap,
    scale: f32,
    label_sizes: &[[f32; 2]],
    selected: Option<u64>,
    pixel: [f32; 2],
    option: bool,
) -> Option<Hit> {
    if scale <= 0. || map.fit[2] <= 0. || map.fit[3] <= 0. {
        return None;
    }
    let canvas = map.canvas(pixel, true)?;
    let p = [pixel[0] / scale, pixel[1] / scale];
    let geometry: Vec<_> = viewports
        .iter()
        .enumerate()
        .map(|(i, v)| Footprint::new(v, map, scale, label_sizes[i]))
        .collect();
    let selected = selected.and_then(|id| viewports.iter().position(|v| v.id == id));
    if option {
        let mut candidates: Vec<_> = (0..viewports.len())
            .filter(|&i| {
                viewports[i].contains(canvas)
                    || geometry[i].label_hit(p)
                    || geometry[i].outline_distance(p) <= 6.
                    || (selected == Some(i) && geometry[i].handle(p).is_some())
            })
            .collect();
        candidates.sort_by_key(|&i| viewports[i].id);
        if candidates.is_empty() {
            return None;
        }
        let next = selected
            .and_then(|i| candidates.iter().position(|&c| c == i))
            .map_or(0, |n| (n + 1) % candidates.len());
        return Some(Hit {
            id: viewports[candidates[next]].id,
            kind: HitKind::Cycle,
        });
    }
    if let Some(i) = selected {
        if let Some(kind) = geometry[i].handle(p) {
            return Some(Hit {
                id: viewports[i].id,
                kind,
            });
        }
    }
    let order: Vec<_> = selected
        .into_iter()
        .chain((0..viewports.len()).rev().filter(|i| Some(*i) != selected))
        .collect();
    if let Some(&i) = order.iter().find(|&&i| geometry[i].label_hit(p)) {
        return Some(Hit {
            id: viewports[i].id,
            kind: HitKind::Pan,
        });
    }
    let mut closest = None;
    let mut best = 6.001;
    for &i in &order {
        let d = geometry[i].outline_distance(p);
        if d <= 6. && d < best - 0.0001 {
            closest = Some(i);
            best = d;
        }
    }
    if let Some(index) = closest {
        return Some(Hit {
            id: viewports[index].id,
            kind: HitKind::Pan,
        });
    }
    order
        .into_iter()
        .find(|&i| viewports[i].contains(canvas))
        .map(|index| Hit {
            id: viewports[index].id,
            kind: HitKind::Pan,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::defaults;
    fn map(scale: f32) -> PreviewMap {
        PreviewMap::for_canvas([3840, 1080], [1200. * scale, 600. * scale], 20. * scale)
    }
    fn pixel(map: PreviewMap, p: [f32; 2]) -> [f32; 2] {
        [
            map.fit[0] + p[0] / 3840. * map.fit[2],
            map.fit[1] + p[1] / 1080. * map.fit[3],
        ]
    }
    fn sizes() -> [[f32; 2]; 2] {
        [[28., 15.], [36., 15.]]
    }
    #[test]
    fn nested_labels_outlines_and_sidebar_body_priority() {
        let mut v = defaults();
        v[1].center = v[0].center;
        v[1].zoom = 0.5;
        let m = map(1.);
        assert_eq!(
            pick(&v, m, 1., &sizes(), Some(2), pixel(m, [960., 540.]), true)
                .unwrap()
                .id,
            1
        );
        let f = Footprint::new(&v[0], m, 1., sizes()[0]);
        for p in [
            [f.label[0] + 10., f.label[1] + 8.],
            [
                (f.corners[1][0] + f.corners[2][0]) / 2.,
                (f.corners[1][1] + f.corners[2][1]) / 2.,
            ],
        ] {
            assert_eq!(
                pick(&v, m, 1., &sizes(), Some(2), p, false),
                Some(Hit {
                    id: 1,
                    kind: HitKind::Pan
                })
            );
        }
        assert_eq!(
            pick(&v, m, 1., &sizes(), Some(1), pixel(m, [960., 540.]), false)
                .unwrap()
                .id,
            1
        );
        assert_eq!(
            pick(&v, m, 1., &sizes(), Some(2), pixel(m, [960., 540.]), false)
                .unwrap()
                .id,
            2
        );
    }
    #[test]
    fn partial_overlap_and_rotated_outline_tolerance_is_in_points() {
        let mut v = defaults();
        v[1].center = [1500., 600.];
        v[0].rotation = 31.;
        for scale in [1., 2., 3.] {
            let m = map(scale);
            let f = Footprint::new(&v[0], m, scale, sizes()[0]);
            let a = f.corners[1];
            let b = f.corners[2];
            let length = distance(a, b);
            let mid = [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.];
            let p = [
                (mid[0] + (b[1] - a[1]) / length * 5.) * scale,
                (mid[1] - (b[0] - a[0]) / length * 5.) * scale,
            ];
            assert_eq!(
                pick(&v, m, scale, &sizes(), Some(2), p, false).unwrap().id,
                1
            );
            assert!((f.outline_distance([p[0] / scale, p[1] / scale]) - 5.).abs() < 0.001);
            let outside = [
                (mid[0] + (b[1] - a[1]) / length * 8.) * scale,
                (mid[1] - (b[0] - a[0]) / length * 8.) * scale,
            ];
            assert_eq!(
                pick(&v, m, scale, &sizes(), Some(2), outside, false)
                    .unwrap()
                    .id,
                2
            );
            assert_eq!(
                pick(
                    &v,
                    m,
                    scale,
                    &sizes(),
                    Some(1),
                    pixel(m, [1000., 550.]),
                    true
                )
                .unwrap()
                .id,
                2
            );
        }
    }
    #[test]
    fn coincident_option_cycles_by_id_and_normal_drag_keeps_choice() {
        let mut v = defaults();
        v[1].center = v[0].center;
        let m = map(2.);
        let p = pixel(m, [960., 540.]);
        let mut selected = None;
        for expected in [1, 2, 1, 2] {
            let h = pick(&v, m, 2., &sizes(), selected, p, true).unwrap();
            assert_eq!(
                h,
                Hit {
                    id: expected,
                    kind: HitKind::Cycle
                }
            );
            selected = Some(h.id);
            assert_eq!(
                pick(&v, m, 2., &sizes(), selected, p, false).unwrap(),
                Hit {
                    id: expected,
                    kind: HitKind::Pan
                }
            );
        }
        let f = Footprint::new(&v[0], m, 2., sizes()[0]);
        let label = [(f.label[0] + 5.) * 2., (f.label[1] + 5.) * 2.];
        assert_eq!(
            pick(&v, m, 2., &sizes(), Some(1), label, false).unwrap(),
            Hit {
                id: 1,
                kind: HitKind::Pan
            }
        );
        assert_eq!(
            pick(&v, m, 2., &sizes(), Some(2), label, false).unwrap(),
            Hit {
                id: 2,
                kind: HitKind::Pan
            }
        );
        v[0].id = 20;
        v[1].id = 10;
        assert_eq!(pick(&v, m, 2., &sizes(), None, p, true).unwrap().id, 10);

        assert_eq!(
            pick(&v, m, 2., &sizes(), Some(20), label, true).unwrap().id,
            10
        );
        assert_eq!(pick(&v, m, 2., &sizes(), Some(1), [0., 0.], true), None);
    }
    #[test]
    fn selected_handles_precede_other_labels_and_outlines() {
        let mut v = defaults();
        v[1].center = v[0].center;
        let m = map(2.);
        let f = Footprint::new(&v[0], m, 2., sizes()[0]);
        for (p, kind) in [
            (f.corners[0], HitKind::Resize),
            (f.rotation, HitKind::Rotate),
        ] {
            assert_eq!(
                pick(&v, m, 2., &sizes(), Some(1), p.map(|x| x * 2.), false),
                Some(Hit { id: 1, kind })
            );
            assert_eq!(
                pick(&v, m, 2., &sizes(), Some(1), p.map(|x| x * 2.), true)
                    .unwrap()
                    .kind,
                HitKind::Cycle
            );
        }
    }
}
