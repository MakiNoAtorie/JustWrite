//! Bundled open-licensed fonts. Faces are only parsed when a database asks for them.
use cosmic_text::fontdb;

/// Fonts needed to draw the interface on any system.
pub fn load_bundled_interface(_db: &mut fontdb::Database) {}
/// The full set of bundled book fonts.
pub fn load_bundled(_db: &mut fontdb::Database) {}
