#[derive(Clone, Copy)]
pub struct Palette {
    pub light: bool,
    pub background: u32,
    pub surface: u32,
    pub raised: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    accent_ink: u32,
    pub code: u32,
    pub selection: u32,
    pub border: u32,
}
impl Palette {
    pub fn new(light: bool, accent: u32) -> Self {
        let mut palette = Self::base(light, accent);
        palette.accent_ink = palette.readable_accent();
        palette
    }
    fn base(light: bool, accent: u32) -> Self {
        if light {
            Self {
                light: true,
                background: 0xfaf8f4,
                surface: 0xf1ede6,
                raised: 0xffffff,
                text: 0x302b26,
                muted: 0x6e6355,
                accent,
                accent_ink: accent,
                code: 0xeee7dd,
                selection: 0xe2cdb2,
                border: 0xe3dcd2,
            }
        } else {
            Self {
                light: false,
                background: 0x211e1b,
                surface: 0x29241f,
                raised: 0x332c25,
                text: 0xe9e2d6,
                muted: 0xa3988b,
                accent,
                accent_ink: accent,
                code: 0x302b26,
                selection: 0x554332,
                border: 0x3c342c,
            }
        }
    }
    /// The accent as a text or icon colour. The raw accent is a fill: a pale amber is about 1.8:1
    /// on the light theme's panels, so this walks it toward the text colour until it reads on
    /// every surface it can sit on, including the accent-washed selection fill.
    pub fn accent_text(&self) -> u32 {
        self.accent_ink
    }
    fn readable_accent(&self) -> u32 {
        let grounds = [
            self.background,
            self.surface,
            self.raised,
            mix(self.surface, self.accent, 0.3),
        ];
        let mut t = 0.;
        loop {
            let candidate = mix(self.accent, self.text, t);
            if t >= 1. || grounds.iter().all(|g| contrast(candidate, *g) >= MIN_TEXT_CONTRAST) {
                return candidate;
            }
            t += 0.05;
        }
    }
    /// Error text, at least 4.5:1 on the panels and dialogs in both themes.
    pub fn danger_text(&self) -> u32 {
        if self.light { 0xb8433f } else { 0xe8837f }
    }
    /// Warning text (an orphaned comment, a safety snapshot), at least 4.5:1 in both themes.
    pub fn warning_text(&self) -> u32 {
        if self.light { 0x9a5a1f } else { 0xe0a070 }
    }
    /// Secondary text that is switched off or only a hint (a placeholder, an unavailable
    /// command): quieter than `muted` but still at least 3:1 on `ground`.
    pub fn faint(&self, ground: u32) -> u32 {
        mix(self.muted, ground, 0.3)
    }
}
impl Default for Palette {
    fn default() -> Self {
        Self::new(false, 0xd9aa72)
    }
}
pub fn color(value: u32) -> cosmic_text::Color {
    cosmic_text::Color::rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// WCAG AA for body-sized text.
pub const MIN_TEXT_CONTRAST: f32 = 4.5;

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let channel = |shift: u32| {
        let (a, b) = (((a >> shift) & 255) as f32, ((b >> shift) & 255) as f32);
        (a * (1. - t) + b * t).round() as u32
    };
    channel(16) << 16 | channel(8) << 8 | channel(0)
}
fn luminance(color: u32) -> f32 {
    let linear = |shift: u32| {
        let v = ((color >> shift) & 255) as f32 / 255.;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0)
}
/// WCAG contrast ratio between two colours, 1.0 to 21.0.
pub fn contrast(a: u32, b: u32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;
    // The accent choices offered in the settings.
    const ACCENTS: [u32; 8] = [
        0xd9aa72, 0xe8864a, 0xd9534f, 0xd96aa0, 0x9b7ed8, 0x5b9bd5, 0x4fb3a5, 0x7fb069,
    ];

    #[test]
    fn muted_text_reads_on_every_surface() {
        for light in [false, true] {
            let p = Palette::new(light, ACCENTS[0]);
            for ground in [p.background, p.surface, p.raised] {
                assert!(
                    contrast(p.muted, ground) >= MIN_TEXT_CONTRAST,
                    "muted on {ground:06x} (light: {light}) is {:.2}",
                    contrast(p.muted, ground)
                );
            }
        }
    }
    #[test]
    fn accent_text_reads_for_every_accent_and_theme() {
        for light in [false, true] {
            for accent in ACCENTS {
                let p = Palette::new(light, accent);
                let text = p.accent_text();
                for ground in [p.background, p.surface, p.raised, mix(p.surface, accent, 0.3)] {
                    assert!(
                        contrast(text, ground) >= MIN_TEXT_CONTRAST,
                        "accent {accent:06x} (light: {light}) as text is {:.2} on {ground:06x}",
                        contrast(text, ground)
                    );
                }
            }
        }
    }
    #[test]
    fn status_text_reads_in_both_themes() {
        for light in [false, true] {
            let p = Palette::new(light, ACCENTS[0]);
            for ground in [p.background, p.surface, p.raised] {
                for (name, text) in [("danger", p.danger_text()), ("warning", p.warning_text())] {
                    assert!(
                        contrast(text, ground) >= MIN_TEXT_CONTRAST,
                        "{name} text on {ground:06x} (light: {light}) is {:.2}",
                        contrast(text, ground)
                    );
                }
            }
        }
    }
    #[test]
    fn faint_text_stays_legible() {
        for light in [false, true] {
            let p = Palette::new(light, ACCENTS[0]);
            for ground in [p.background, p.surface, p.raised] {
                assert!(contrast(p.faint(ground), ground) >= 3., "faint on {ground:06x} (light: {light})");
            }
        }
    }
}
