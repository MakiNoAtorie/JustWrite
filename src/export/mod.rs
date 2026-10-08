//! Exporters. A [`model::Doc`] is built once from the project text and a [`preset::Preset`];
//! each writer turns it into bytes. The PDF and the in-app preview come from the same
//! typesetting run.
pub mod docx;
pub mod epub;
pub mod model;
pub mod plain;
pub mod preset;
pub mod typeset;

use model::{Doc, Scope, Source, build};
use preset::{Format, Preset};

/// A finished export.
pub struct Output {
    pub bytes: Vec<u8>,
    pub extension: &'static str,
}

/// A file name for the export: the book's title and the format's extension.
pub fn file_name(doc: &Doc, preset: &Preset) -> String {
    let stem: String = doc
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '_' })
        .collect::<String>()
        .trim()
        .to_owned();
    format!("{}.{}", if stem.is_empty() { "manuscript" } else { &stem }, preset.format.extension())
}

/// Builds the document and writes it in the preset's format.
pub fn export(source: &Source, scope: &Scope, preset: &Preset) -> Result<(Doc, Output), String> {
    let mut preset = preset.clone();
    preset.sanitize();
    let doc = build(source, scope, &preset);
    if doc.chapters.is_empty() {
        return Err("There is nothing to export: choose at least one chapter".into());
    }
    let bytes = match preset.format {
        Format::PlainText | Format::Markdown => plain::write(&doc, &preset).into_bytes(),
        Format::Docx => docx::write(&doc, &preset).map_err(|e| e.to_string())?,
        Format::Epub => epub::write(&doc, &preset).map_err(|e| e.to_string())?,
        Format::Pdf => typeset::typeset(&doc, &preset)?.pdf()?,
    };
    Ok((doc, Output { bytes, extension: preset.format.extension() }))
}

/// Lays the book out for the in-app preview. Formats that are not paged are shown with the
/// layout values of the preset on the page size it carries.
pub fn preview(source: &Source, scope: &Scope, preset: &Preset) -> Result<typeset::Typeset, String> {
    let mut preset = preset.clone();
    preset.sanitize();
    let doc = build(source, scope, &preset);
    if doc.chapters.is_empty() {
        return Err("There is nothing to preview: choose at least one chapter".into());
    }
    typeset::typeset(&doc, &preset)
}

#[cfg(test)]
mod tests {
    use super::{model::testing::source, preset::builtins, *};

    #[test]
    fn every_built_in_preset_exports_in_its_format() {
        let src = source();
        for preset in builtins() {
            let (doc, out) = export(&src, &Scope::Whole, &preset).unwrap_or_else(|e| panic!("{}: {e}", preset.name));
            assert!(!out.bytes.is_empty(), "{}", preset.name);
            assert_eq!(out.extension, preset.format.extension());
            let name = file_name(&doc, &preset);
            assert!(name.starts_with("The Long Night.") && name.ends_with(out.extension), "{name}");
            match preset.format {
                Format::Pdf => assert!(out.bytes.starts_with(b"%PDF")),
                Format::Docx | Format::Epub => assert!(out.bytes.starts_with(b"PK")),
                _ => assert!(String::from_utf8(out.bytes).is_ok()),
            }
        }
        assert!(export(&src, &Scope::Chapters(vec![]), &builtins()[0]).is_err());
    }
}
