//! The game's backdrops, drawn from the textures in the Growtopia install
//! folder: the title screen (sky, three hill layers, clouds, sun) and the
//! dimmed world behind the "connecting" screen.
//!
//! `.rttex`: `RTPACK` header (32 bytes) + zlib data. Inside: `RTTXTR` header
//! (124 bytes; height, width, format, original height, original width ...)
//! then RGBA rows stored bottom-up.

use std::io::Read;
use std::path::Path;

use eframe::egui::{self, Color32, ColorImage, Rect, TextureHandle, TextureOptions, pos2};

/// Sky colour behind the title screen.
const SKY: Color32 = Color32::from_rgb(96, 215, 242);

// The connecting screen, measured from the game.
const DIM_SKY: Color32 = Color32::from_rgb(23, 84, 111);
const DIM_CLOUD: Color32 = Color32::from_rgb(31, 113, 150);
const DIM_CLOUD_FAR: Color32 = Color32::from_rgb(26, 94, 125);
const GROUND: Color32 = Color32::from_rgb(10, 36, 47);
const GROUND_STRIPE: Color32 = Color32::from_rgb(8, 28, 36);
const GROUND_EDGE: Color32 = Color32::from_rgb(21, 76, 99);

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    Title,
    Connecting,
}
/// The title screen layout was measured at this width.
const REFERENCE_WIDTH: f32 = 1200.0;

pub struct Backdrop {
    hills: [TextureHandle; 3],
    cloud: TextureHandle,
    sun: TextureHandle,
}

impl Backdrop {
    /// `game_dir` is the folder holding Growtopia.exe; `None` if a texture is missing.
    pub fn load(ctx: &egui::Context, game_dir: &Path) -> Option<Self> {
        let tex = |name: &str| -> Option<TextureHandle> {
            let image = decode_rttex(&std::fs::read(game_dir.join("game").join(format!("{name}.rttex"))).ok()?)?;
            Some(ctx.load_texture(name, image, TextureOptions::LINEAR))
        };
        Some(Self { hills: [tex("hills3")?, tex("hills2")?, tex("hills1")?], cloud: tex("cloud")?, sun: tex("sun")? })
    }

    pub fn paint(&self, painter: &egui::Painter, rect: Rect, scene: Scene) {
        match scene {
            Scene::Title => self.paint_title(painter, rect),
            Scene::Connecting => self.paint_connecting(painter, rect),
        }
    }

    /// Big blue clouds over a dark teal sky, striped ground on the lower half.
    fn paint_connecting(&self, painter: &egui::Painter, rect: Rect) {
        painter.rect_filled(rect, 0.0, DIM_SKY);
        let k = rect.width() / REFERENCE_WIDTH;
        let full_uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        let cloud = self.cloud.size_vec2();
        // (x, y, scale, colour): a few large clouds, like the screenshot.
        for (x, y, s, tint) in [
            (-40.0, 20.0, 1.7, DIM_CLOUD),
            (110.0, 140.0, 0.9, DIM_CLOUD_FAR),
            (210.0, 40.0, 1.3, DIM_CLOUD_FAR),
            (280.0, 95.0, 1.1, DIM_CLOUD_FAR),
            (660.0, 150.0, 2.6, DIM_CLOUD),
        ] {
            let r = Rect::from_min_size(pos2(rect.left() + x * k, rect.top() + y * k), cloud * s * k);
            painter.image(self.cloud.id(), r, full_uv, tint);
        }

        let top = rect.top() + rect.height() * 0.53;
        let ground = Rect::from_min_max(pos2(rect.left(), top), rect.right_bottom());
        painter.rect_filled(ground, 0.0, GROUND);
        let clipped = painter.with_clip_rect(ground);
        let step = 44.0 * k.max(0.5);
        let mut x = rect.left() - ground.height();
        while x < rect.right() + ground.height() {
            clipped.line_segment(
                [pos2(x + ground.height(), top), pos2(x, rect.bottom())],
                egui::Stroke::new(14.0 * k.max(0.5), GROUND_STRIPE),
            );
            x += step;
        }
        painter.rect_filled(Rect::from_min_size(pos2(rect.left(), top - 3.0), egui::vec2(rect.width(), 6.0)), 0.0, GROUND_EDGE);
    }

    fn paint_title(&self, painter: &egui::Painter, rect: Rect) {
        painter.rect_filled(rect, 0.0, SKY);
        let k = rect.width() / REFERENCE_WIDTH;
        let full_uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

        let sun = self.sun.size_vec2() * k;
        let sun_rect = Rect::from_min_size(pos2(rect.right() - 260.0 * k, rect.top() + 60.0 * k), sun);
        painter.image(self.sun.id(), sun_rect, full_uv, Color32::WHITE);

        let cloud = self.cloud.size_vec2() * k;
        for (x, y) in [(100.0, 80.0), (500.0, 40.0), (800.0, 150.0)] {
            let r = Rect::from_min_size(pos2(rect.left() + x * k, rect.top() + y * k), cloud);
            painter.image(self.cloud.id(), r, full_uv, Color32::WHITE);
        }

        // Back to front, each stretched to the window width and sitting on the bottom edge.
        for hill in &self.hills {
            let size = hill.size_vec2();
            let height = size.y * rect.width() / size.x;
            let r = Rect::from_min_max(pos2(rect.left(), rect.bottom() - height), rect.right_bottom());
            painter.image(hill.id(), r, full_uv, Color32::WHITE);
        }
    }
}

fn decode_rttex(file: &[u8]) -> Option<ColorImage> {
    let data = if file.starts_with(b"RTPACK") {
        let mut out = Vec::new();
        flate2::read::ZlibDecoder::new(file.get(32..)?).read_to_end(&mut out).ok()?;
        out
    } else {
        file.to_vec()
    };
    if !data.starts_with(b"RTTXTR") {
        return None;
    }
    let i32_at = |o: usize| -> Option<usize> {
        Some(i32::from_le_bytes(data.get(o..o + 4)?.try_into().ok()?).max(0) as usize)
    };
    let (height, width) = (i32_at(8)?, i32_at(12)?);
    let (orig_height, orig_width) = (i32_at(20)?, i32_at(24)?);
    let pixels = data.get(124..124 + width * height * 4)?;
    // Rows are bottom-up and padded to a power of two; keep the original area.
    let (w, h) = (orig_width.min(width), orig_height.min(height));
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in 0..h {
        let src = (height - 1 - row) * width * 4;
        rgba.extend_from_slice(&pixels[src..src + w * 4]);
    }
    Some(ColorImage::from_rgba_unmultiplied([w, h], &rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_game_textures_if_installed() {
        let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|d| std::path::PathBuf::from(d).join(r"Growtopia\game")) else {
            return;
        };
        let Ok(file) = std::fs::read(dir.join("sun.rttex")) else { return };
        let img = decode_rttex(&file).unwrap();
        assert_eq!(img.size, [161, 159]);
        let hills = decode_rttex(&std::fs::read(dir.join("hills1.rttex")).unwrap()).unwrap();
        assert_eq!(hills.size, [512, 128]);
    }
}
