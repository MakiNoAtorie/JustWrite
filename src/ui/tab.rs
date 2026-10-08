use crate::core::{buffer::Document, markdown::BlockIndex, storage::Storage};
use crate::ui::editor::EditorView;
use ropey::Rope;
use std::{path::PathBuf, sync::Arc, time::Instant};

pub struct Tab {
    pub id: u64,
    pub document: Document,
    pub view: EditorView,
    pub blocks: BlockIndex,
    pub storage: Option<Arc<dyn Storage>>,
    pub path: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub watcher: Option<notify::RecommendedWatcher>,
    pub loading: bool,
    pub failed: bool,
    pub saving: bool,
    pub edited: Option<Instant>,
    pub external: Option<Rope>,
    pub restore_cursor: Option<usize>,
    pub guard_save: bool,
    pub disk_epoch: u64,
}
impl Tab {
    pub fn new(id: u64) -> Self {
        Self {
            id,
            document: Document::default(),
            view: EditorView::default(),
            blocks: BlockIndex::default(),
            storage: None,
            path: None,
            root: None,
            watcher: None,
            loading: false,
            failed: false,
            saving: false,
            edited: None,
            external: None,
            restore_cursor: None,
            guard_save: false,
            disk_epoch: 0,
        }
    }
    pub fn name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled.md".into())
    }
}
