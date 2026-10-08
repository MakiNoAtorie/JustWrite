use cosmic_text::{Buffer, Color, FontSystem, SwashCache};
pub mod theme;

pub const BACKGROUND: u32 = 0x211e1b;
pub const TEXT: u32 = 0xe9e2d6;
pub const MUTED: u32 = 0x92877b;
pub const ACCENT: u32 = 0xd9aa72;
pub const CODE: u32 = 0x302b26;
pub const SELECTION: u32 = 0x554332;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Rect {
    pub fn intersect(self, other: Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (self.x + self.width as i32).min(other.x + other.width as i32);
        let bottom = (self.y + self.height as i32).min(other.y + other.height as i32);
        Rect {
            x,
            y,
            width: right.saturating_sub(x).max(0) as u32,
            height: bottom.saturating_sub(y).max(0) as u32,
        }
    }
}
pub trait Renderer {
    fn resize(&mut self, width: u32, height: u32);
    fn fill(&mut self, rect: Rect, color: u32);
    fn text(&mut self, fonts: &mut FontSystem, buffer: &mut Buffer, x: i32, y: i32, clip: Rect);
    fn pixels(&self) -> &[u32];
    fn damage(&mut self) -> Vec<Rect>;
    /// Blends `color` over `rect` (alpha 0-255); used to dim the window behind dialogs.
    fn shade(&mut self, rect: Rect, color: u32, alpha: u32);
    fn fill_round(&mut self, rect: Rect, radius: u32, color: u32) {
        let r = radius.min(rect.width / 2).min(rect.height / 2) as i32;
        if r == 0 {
            return self.fill(rect, color);
        }
        let (w, h) = (rect.width as i32, rect.height as i32);
        self.fill(
            Rect {
                y: rect.y + r,
                height: (h - 2 * r) as u32,
                ..rect
            },
            color,
        );
        for dy in 0..r {
            let d = (r - dy) as f32 - 0.5;
            let inset = r - ((r * r) as f32 - d * d).max(0.).sqrt().round() as i32;
            let row = Rect {
                x: rect.x + inset,
                y: rect.y + dy,
                width: (w - 2 * inset).max(0) as u32,
                height: 1,
            };
            self.fill(row, color);
            self.fill(
                Rect {
                    y: rect.y + h - 1 - dy,
                    ..row
                },
                color,
            );
        }
    }
    /// A rounded box with a one-pixel border.
    fn framed(&mut self, rect: Rect, radius: u32, fill: u32, border: u32) {
        self.fill_round(rect, radius, border);
        self.fill_round(
            Rect {
                x: rect.x + 1,
                y: rect.y + 1,
                width: rect.width.saturating_sub(2),
                height: rect.height.saturating_sub(2),
            },
            radius.saturating_sub(1),
            fill,
        );
    }
}
pub struct CpuRenderer {
    pub width: u32,
    pub height: u32,
    pixels: Vec<u32>,
    glyphs: SwashCache,
    dirty: Vec<Rect>,
}
impl Default for CpuRenderer {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            pixels: Vec::new(),
            glyphs: SwashCache::new(),
            dirty: Vec::new(),
        }
    }
}
impl CpuRenderer {
    /// Blends one pixel; `coverage` is 0.0-1.0.
    fn blend(&mut self, x: i32, y: i32, color: u32, coverage: f32) {
        if coverage <= 0. || x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let pixel = &mut self.pixels[y as usize * self.width as usize + x as usize];
        let mix = |shift: u32| {
            let (c, p) = (((color >> shift) & 255) as f32, ((*pixel >> shift) & 255) as f32);
            (c * coverage + p * (1. - coverage)).round() as u32
        };
        *pixel = mix(16) << 16 | mix(8) << 8 | mix(0);
    }
}
impl Renderer for CpuRenderer {
    fn resize(&mut self, width: u32, height: u32) {
        if self.width != width || self.height != height {
            self.width = width;
            self.height = height;
            self.pixels
                .resize(width as usize * height as usize, BACKGROUND);
            self.dirty.push(Rect {
                x: 0,
                y: 0,
                width,
                height,
            });
        }
    }
    fn fill(&mut self, rect: Rect, color: u32) {
        let left = rect.x.max(0).min(self.width as i32) as usize;
        let right = (rect.x + rect.width as i32).max(0).min(self.width as i32) as usize;
        let top = rect.y.max(0).min(self.height as i32) as usize;
        let bottom = (rect.y + rect.height as i32).max(0).min(self.height as i32) as usize;
        for row in top..bottom {
            if left < right {
                self.pixels[row * self.width as usize + left..row * self.width as usize + right]
                    .fill(color);
            }
        }
        self.dirty.push(rect);
    }
    fn text(&mut self, fonts: &mut FontSystem, buffer: &mut Buffer, x: i32, y: i32, clip: Rect) {
        self.dirty.push(clip);
        let width = self.width as i32;
        let height = self.height as i32;
        let pixels = &mut self.pixels;
        buffer.draw(
            fonts,
            &mut self.glyphs,
            Color::rgb(233, 226, 214),
            |gx, gy, gw, gh, color| {
                let alpha = color.a() as u32;
                if alpha == 0 {
                    return;
                }
                for row in y + gy..y + gy + gh as i32 {
                    if row < clip.y
                        || row >= clip.y + clip.height as i32
                        || row < 0
                        || row >= height
                    {
                        continue;
                    }
                    for column in x + gx..x + gx + gw as i32 {
                        if column < clip.x
                            || column >= clip.x + clip.width as i32
                            || column < 0
                            || column >= width
                        {
                            continue;
                        }
                        let pixel = &mut pixels[(row * width + column) as usize];
                        let red = ((color.r() as u32 * alpha)
                            + ((*pixel >> 16) & 255) * (255 - alpha))
                            / 255;
                        let green = ((color.g() as u32 * alpha)
                            + ((*pixel >> 8) & 255) * (255 - alpha))
                            / 255;
                        let blue =
                            ((color.b() as u32 * alpha) + (*pixel & 255) * (255 - alpha)) / 255;
                        *pixel = red << 16 | green << 8 | blue;
                    }
                }
            },
        );
        // ponytail: bounded glyph cache; add LRU eviction if cache clears cause measured stalls.
        if self.glyphs.image_cache.len() > 16384 {
            self.glyphs.image_cache.clear();
        }
    }
    fn pixels(&self) -> &[u32] {
        &self.pixels
    }
    fn fill_round(&mut self, rect: Rect, radius: u32, color: u32) {
        let radius = radius.min(rect.width / 2).min(rect.height / 2) as f32;
        let edge = radius.floor() as i32;
        if edge < 1 {
            return self.fill(rect, color);
        }
        let (w, h) = (rect.width as i32, rect.height as i32);
        self.fill(
            Rect {
                y: rect.y + edge,
                height: (h - 2 * edge).max(0) as u32,
                ..rect
            },
            color,
        );
        for y in [rect.y, rect.y + h - edge] {
            self.fill(
                Rect {
                    x: rect.x + edge,
                    y,
                    width: (w - 2 * edge).max(0) as u32,
                    height: edge as u32,
                },
                color,
            );
        }
        for dy in 0..edge {
            for dx in 0..edge {
                // Distance from this pixel's centre to the corner circle's centre.
                let (px, py) = (dx as f32 + 0.5 - radius, dy as f32 + 0.5 - radius);
                let coverage = (radius - (px * px + py * py).sqrt() + 0.5).clamp(0., 1.);
                for (x, y) in [
                    (rect.x + dx, rect.y + dy),
                    (rect.x + w - 1 - dx, rect.y + dy),
                    (rect.x + dx, rect.y + h - 1 - dy),
                    (rect.x + w - 1 - dx, rect.y + h - 1 - dy),
                ] {
                    self.blend(x, y, color, coverage);
                }
            }
        }
        self.dirty.push(rect);
    }
    fn shade(&mut self, rect: Rect, color: u32, alpha: u32) {
        let clipped = rect.intersect(Rect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        });
        for row in clipped.y.max(0)..clipped.y.max(0) + clipped.height as i32 {
            let start = row as usize * self.width as usize + clipped.x.max(0) as usize;
            for pixel in &mut self.pixels[start..start + clipped.width as usize] {
                let mix = |shift: u32| {
                    (((color >> shift) & 255) * alpha + ((*pixel >> shift) & 255) * (255 - alpha))
                        / 255
                };
                *pixel = mix(16) << 16 | mix(8) << 8 | mix(0);
            }
        }
        self.dirty.push(rect);
    }
    fn damage(&mut self) -> Vec<Rect> {
        let full = Rect {
            x: 0,
            y: 0,
            width: self.width,
            height: self.height,
        };
        let mut dirty = std::mem::take(&mut self.dirty)
            .into_iter()
            .map(|r| r.intersect(full))
            .filter(|r| r.width > 0 && r.height > 0);
        let Some(mut bounds) = dirty.next() else {
            return Vec::new();
        };
        for rect in dirty {
            let right = (bounds.x + bounds.width as i32).max(rect.x + rect.width as i32);
            let bottom = (bounds.y + bounds.height as i32).max(rect.y + rect.height as i32);
            bounds.x = bounds.x.min(rect.x);
            bounds.y = bounds.y.min(rect.y);
            bounds.width = (right - bounds.x) as u32;
            bounds.height = (bottom - bounds.y) as u32;
        }
        vec![bounds]
    }
}
