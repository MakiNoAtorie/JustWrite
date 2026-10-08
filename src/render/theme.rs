#[derive(Clone, Copy)]
pub struct Palette {
    pub light: bool,
    pub background: u32,
    pub surface: u32,
    pub raised: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub code: u32,
    pub selection: u32,
    pub border: u32,
}
impl Palette {
    pub fn new(light: bool, accent: u32) -> Self {
        if light {
            Self {
                light: true,
                background: 0xfaf8f4,
                surface: 0xf1ede6,
                raised: 0xffffff,
                text: 0x302b26,
                muted: 0x827669,
                accent,
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
                muted: 0x92877b,
                accent,
                code: 0x302b26,
                selection: 0x554332,
                border: 0x3c342c,
            }
        }
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
