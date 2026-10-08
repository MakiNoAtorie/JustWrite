//! Bundled open-licensed book fonts (see `assets/fonts/NOTICE.md`). They are compiled into the
//! program as static data; a face is only parsed when a font database or the typesetter asks.
use cosmic_text::fontdb;
use std::sync::Arc;

pub struct Face {
    pub family: &'static str,
    pub bold: bool,
    pub italic: bool,
    pub data: &'static [u8],
}
macro_rules! face {
    ($family:literal, $file:literal, $bold:literal, $italic:literal) => {
        Face { family: $family, bold: $bold, italic: $italic, data: include_bytes!(concat!("../assets/fonts/", $file)) }
    };
}
pub const BUNDLED: &[Face] = &[
    face!("Libertinus Serif", "LibertinusSerif-Regular.otf", false, false),
    face!("Libertinus Serif", "LibertinusSerif-Italic.otf", false, true),
    face!("Libertinus Serif", "LibertinusSerif-Bold.otf", true, false),
    face!("Libertinus Serif", "LibertinusSerif-BoldItalic.otf", true, true),
    face!("Tinos", "Tinos-Regular.ttf", false, false),
    face!("Tinos", "Tinos-Italic.ttf", false, true),
    face!("Tinos", "Tinos-Bold.ttf", true, false),
    face!("Tinos", "Tinos-BoldItalic.ttf", true, true),
    face!("Courier Prime", "CourierPrime-Regular.ttf", false, false),
    face!("Courier Prime", "CourierPrime-Italic.ttf", false, true),
    face!("Courier Prime", "CourierPrime-Bold.ttf", true, false),
    face!("Courier Prime", "CourierPrime-BoldItalic.ttf", true, true),
];

/// The face the editor uses until the author picks another.
pub const DEFAULT_EDITOR_FONT: &str = "Libertinus Serif";

fn load(db: &mut fontdb::Database, face: &Face) {
    db.load_font_source(fontdb::Source::Binary(Arc::new(face.data)));
}
/// Only what the first frame needs: the default editor font.
pub fn load_bundled_interface(db: &mut fontdb::Database) {
    for face in BUNDLED.iter().filter(|f| f.family == DEFAULT_EDITOR_FONT) {
        load(db, face);
    }
}
/// The full set of bundled book fonts.
pub fn load_bundled(db: &mut fontdb::Database) {
    for face in BUNDLED {
        load(db, face);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_bundled_face_parses_with_the_right_family() {
        let mut db = fontdb::Database::new();
        load_bundled(&mut db);
        assert_eq!(db.faces().count(), BUNDLED.len());
        for family in ["Libertinus Serif", "Tinos", "Courier Prime"] {
            let faces: Vec<_> = db.faces().filter(|f| f.families.iter().any(|(n, _)| n == family)).collect();
            assert_eq!(faces.len(), 4, "{family}");
            assert!(faces.iter().any(|f| f.style == fontdb::Style::Italic));
            assert!(faces.iter().any(|f| f.weight == fontdb::Weight::BOLD));
        }
    }
}
