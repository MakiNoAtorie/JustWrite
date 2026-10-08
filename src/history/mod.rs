//! Snapshots: the history of a project.
//!
//! Text is stored once per distinct content, in blobs named by their hash, so a chapter that did
//! not change between two snapshots costs nothing. Restoring never rewrites history: it takes a
//! safety snapshot of what is there, applies the old state, and records the restore as a new
//! snapshot. Named *lines* ("Draft 2", "Alt ending") are parallel sequences of snapshots to
//! switch between and compare; text moves between them by hand.
pub mod diff;
pub mod prune;
pub mod restore;
pub mod store;

pub use store::{History, ItemEntry, Kind, SnapshotDetail, SnapshotMeta, State};
