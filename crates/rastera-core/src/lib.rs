//! Canvas geometry and GPU generator. No dependency on the operator UI.
pub mod renderer;
pub mod shared_frames;
pub mod timing;

pub const WIDTH: u32 = 3840;
pub const HEIGHT: u32 = 1080;

/// Integer pixels, top-left origin, half-open bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slice {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl Slice {
    pub const FULL: Self = Self {
        x: 0,
        y: 0,
        width: WIDTH,
        height: HEIGHT,
    };
    pub const LEFT: Self = Self {
        width: 1920,
        ..Self::FULL
    };
    pub const RIGHT: Self = Self {
        x: 1920,
        ..Self::LEFT
    };
    pub fn uv(self) -> [[f32; 2]; 2] {
        [
            [self.x as f32 / WIDTH as f32, self.y as f32 / HEIGHT as f32],
            [
                (self.x + self.width) as f32 / WIDTH as f32,
                (self.y + self.height) as f32 / HEIGHT as f32,
            ],
        ]
    }
}

/// Centered fit rectangle [x, y, width, height] in destination units.
/// The same mapping works in logical points or physical pixels.
pub fn fit(source: [u32; 2], destination: [f32; 2]) -> [f32; 4] {
    if source.contains(&0) || destination.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return [0.0; 4];
    }
    let scale = (destination[0] / source[0] as f32).min(destination[1] / source[1] as f32);
    let w = source[0] as f32 * scale;
    let h = source[1] as f32 * scale;
    [(destination[0] - w) / 2.0, (destination[1] - h) / 2.0, w, h]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slices_partition_every_pixel_once() {
        for x in 0..WIDTH {
            let contains = |s: Slice| x >= s.x && x < s.x + s.width;
            assert_ne!(contains(Slice::LEFT), contains(Slice::RIGHT));
        }
        assert_eq!(Slice::LEFT.x + Slice::LEFT.width, Slice::RIGHT.x);
        assert_eq!(Slice::RIGHT.x + Slice::RIGHT.width, WIDTH);
        assert_eq!(Slice::LEFT.height, HEIGHT);
        assert_eq!(Slice::RIGHT.height, HEIGHT);
        assert_eq!(Slice::LEFT.uv(), [[0.0, 0.0], [0.5, 1.0]]);
        assert_eq!(Slice::RIGHT.uv(), [[0.5, 0.0], [1.0, 1.0]]);
    }
    #[test]
    fn aspect_fit_letterbox_pillarbox_and_retina() {
        let close = |actual: [f32; 4], expected: [f32; 4]| {
            for (a, e) in actual.into_iter().zip(expected) {
                assert!((a - e).abs() < 0.001, "{actual:?} != {expected:?}");
            }
        };
        close(
            fit([1920, 1080], [1000.0, 1000.0]),
            [0.0, 218.75, 1000.0, 562.5],
        );
        close(
            fit([1920, 1080], [2000.0, 562.5]),
            [500.0, 0.0, 1000.0, 562.5],
        );
        close(
            fit([3840, 1080], [1920.0, 540.0]),
            [0.0, 0.0, 1920.0, 540.0],
        );
        assert_eq!(fit([1920, 1080], [0.0, 100.0]), [0.0; 4]);
        let a = fit([1920, 1080], [1000.0, 1000.0]);
        let b = fit([1920, 1080], [2000.0, 2000.0]);
        for i in 0..4 {
            assert_eq!(a[i] * 2.0, b[i]);
        }
    }
}

pub mod composition;
pub mod scene;

pub mod fonts;
pub mod mapping;

pub mod theme;

pub mod viewport_hit;

#[cfg(test)]
mod dynamic_tests;

pub mod capture;

pub mod crop;

pub mod media;

pub mod background;

pub mod scenes;

pub mod recovery;
