//! The `.jw` project file: a standard ZIP holding text files, JSON metadata and the history.
//!
//! A project is edited as an unpacked *working copy* in the application data folder; the `.jw`
//! file is repacked atomically from it.
pub mod archive;
pub mod book;
pub mod lock;
pub mod manifest;
pub mod project;
pub mod time;
pub mod workdir;

pub use manifest::{Manifest, Node, Section, new_id};
pub use project::Project;
