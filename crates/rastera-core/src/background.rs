//! Canvas content below scene layers; unrelated to operator workspace colors.
use crate::{
    crop::Crop,
    scene::{Content, Document, Layer, Rect},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Fit {
    Fit,
    #[default]
    Fill,
    Stretch,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Background {
    Solid([u8; 4]),
    Image { source: u64, fit: Fit },
    Media { source: u64, fit: Fit },
}
impl Default for Background {
    fn default() -> Self {
        Self::Solid([0, 0, 0, 255])
    }
}
impl Background {
    pub fn layer(&self, d: &Document) -> Option<Layer> {
        let (content, size, fit) = match *self {
            Self::Solid(_) => return None,
            Self::Image { source, fit } => (
                Content::Image(source),
                [d.images[&source].width, d.images[&source].height],
                fit,
            ),
            Self::Media { source, fit } => (
                Content::Media(source),
                d.media_assets[&d.playbacks[&source].asset].size,
                fit,
            ),
        };
        let canvas = [d.canvas[0] as f32, d.canvas[1] as f32];
        let mut rect = Rect {
            x: 0.,
            y: 0.,
            width: canvas[0],
            height: canvas[1],
        };
        let mut crop = Crop::default();
        match fit {
            Fit::Stretch => {}
            Fit::Fit => {
                let s = (canvas[0] / size[0] as f32).min(canvas[1] / size[1] as f32);
                rect.width = size[0] as f32 * s;
                rect.height = size[1] as f32 * s;
                rect.x = (canvas[0] - rect.width) / 2.;
                rect.y = (canvas[1] - rect.height) / 2.;
            }
            Fit::Fill => {
                let aspect = canvas[0] / canvas[1] / (size[0] as f32 / size[1] as f32);
                if aspect < 1. {
                    crop.left = (1. - aspect) / 2.;
                    crop.right = 1. - crop.left;
                } else {
                    crop.top = (1. - 1. / aspect) / 2.;
                    crop.bottom = 1. - crop.top;
                }
            }
        }
        Some(Layer {
            id: 0,
            name: "Background".into(),
            rect,
            crop,
            visible: true,
            locked: true,
            opacity: 1.,
            aspect_lock: false,
            content,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fit_fill_stretch_geometry() {
        let mut d = Document::default();
        let id = d.add_image(crate::scene::ImageAsset {
            path: "test.png".into(),
            width: 1000,
            height: 1000,
        });
        let Content::Image(source) = d.layer(id).unwrap().content else {
            panic!()
        };
        let fill = Background::Image {
            source,
            fit: Fit::Fill,
        }
        .layer(&d)
        .unwrap();
        assert!(fill.crop.top > 0.);
        assert_eq!(fill.rect.width, 3840.);
        let fit = Background::Image {
            source,
            fit: Fit::Fit,
        }
        .layer(&d)
        .unwrap();
        assert_eq!(fit.rect.width, 1080.);
        let stretch = Background::Image {
            source,
            fit: Fit::Stretch,
        }
        .layer(&d)
        .unwrap();
        assert_eq!(stretch.crop, Crop::default());
    }
}
