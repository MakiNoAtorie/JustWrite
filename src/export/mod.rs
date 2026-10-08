//! Exporters. A [`model::Doc`] is built once from the project text and a [`preset::Preset`];
//! each writer turns it into bytes. The PDF and the in-app preview come from the same
//! typesetting run.
pub mod docx;
pub mod epub;
pub mod model;
pub mod plain;
pub mod preset;
