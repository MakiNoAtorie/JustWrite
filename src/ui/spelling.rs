//! Spell checking of the text on screen. The dictionary loads in the background; the words of
//! the visible paragraphs are checked on another thread, and the results come back as ranges
//! the editor underlines.
use super::*;
use crate::core::{
    notes::Marker,
    spell::{self, Speller},
    typography::Language,
};
use std::hash::{Hash, Hasher};

pub struct Request {
    tab: u64,
    signature: u64,
    chunks: Vec<(usize, String, bool)>,
    speller: Arc<Speller>,
    marker: Marker,
}

pub struct SpellState {
    pub speller: Option<Arc<Speller>>,
    pub loading: Option<Language>,
    pub worker: mpsc::Sender<Request>,
    /// Bumped when the dictionary or the personal list changes, so everything is checked again.
    pub generation: u64,
}

impl SpellState {
    pub fn new(proxy: EventLoopProxy<UserEvent>) -> Self {
        let (worker, requests) = mpsc::channel::<Request>();
        std::thread::spawn(move || {
            while let Ok(mut request) = requests.recv() {
                // Only the newest request of a tab matters.
                let mut pending = vec![];
                while let Ok(next) = requests.try_recv() {
                    pending.push(std::mem::replace(&mut request, next));
                }
                drop(pending);
                let mut ranges = Vec::new();
                for (start, text, in_block) in &request.chunks {
                    for r in spell::misspelled(&request.speller, text, &request.marker, *in_block) {
                        ranges.push((start + r.start, start + r.end));
                    }
                }
                ranges.sort_unstable();
                ranges.dedup();
                let _ = proxy.send_event(UserEvent::Spelling(request.tab, request.signature, ranges));
            }
        });
        Self { speller: None, loading: None, worker, generation: 0 }
    }
}

fn dictionary_path() -> PathBuf {
    platform::data_directory().join("dictionary.txt")
}

impl App {
    fn spell_language(&self) -> Option<Language> {
        let project = self.project.as_ref()?;
        self.settings.spellcheck.then_some(project.core.manifest.settings.language)
    }
    /// Called after the text of a tab was laid out: starts whatever checking is missing.
    pub(super) fn request_spelling(&mut self, tab_id: u64) {
        let Some(language) = self.spell_language() else {
            if let Some(tab) = self.tabs.get_mut(&tab_id) {
                if !tab.view.misspelled.is_empty() {
                    tab.view.misspelled.clear();
                }
            }
            return;
        };
        if self.spell.speller.as_ref().is_none_or(|s| s.language() != language) {
            if self.spell.loading != Some(language) {
                self.spell.loading = Some(language);
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let personal: Vec<String> = std::fs::read_to_string(dictionary_path()).map(|t| t.lines().map(str::to_owned).collect()).unwrap_or_default();
                    let _ = proxy.send_event(UserEvent::SpellerLoaded(Speller::load(language, &personal).map(Arc::new)));
                });
            }
            return;
        }
        let Some(speller) = self.spell.speller.clone() else { return };
        let marker = self.marker();
        let generation = self.spell.generation;
        let Some(tab) = self.tabs.get_mut(&tab_id) else { return };
        if tab.loading {
            return;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (tab.document.revision, tab.view.top, tab.view.rect.map(|r| r.width), generation, tab.disk_epoch).hash(&mut hasher);
        let signature = hasher.finish();
        if tab.spell_signature == signature {
            return;
        }
        tab.spell_signature = signature;
        let chunks = tab.view.spell_chunks();
        let _ = self.spell.worker.send(Request { tab: tab_id, signature, chunks, speller, marker });
    }
    pub(super) fn speller_loaded(&mut self, result: Result<Arc<Speller>, String>) {
        self.spell.loading = None;
        match result {
            Ok(speller) => {
                self.spell.speller = Some(speller);
                self.spell.generation += 1;
            }
            Err(error) => {
                self.status = error;
                self.settings.spellcheck = false;
            }
        }
        self.redraw();
    }
    pub(super) fn spelling_ready(&mut self, tab_id: u64, signature: u64, ranges: Vec<(usize, usize)>) {
        if let Some(tab) = self.tabs.get_mut(&tab_id).filter(|t| t.spell_signature == signature) {
            tab.view.misspelled = ranges;
            self.redraw();
        }
    }
    /// Adds the misspelled word at the caret to the author's dictionary.
    pub(super) fn add_word(&mut self) {
        let Some(tab) = self.active_id().and_then(|id| self.tabs.get(&id)) else { return };
        let cursor = tab.document.cursor;
        let hit = tab.view.misspelled.iter().find(|(a, b)| cursor >= *a && cursor <= *b).copied();
        let Some((start, end)) = hit else {
            self.status = "Put the caret in an underlined word first.".into();
            return;
        };
        let word: String = tab.document.text.slice(start..end.min(tab.document.text.len_chars())).to_string();
        if let Some(speller) = &self.spell.speller {
            speller.add(&word);
        }
        let path = dictionary_path();
        let line = format!("{word}\n");
        std::thread::spawn(move || {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = file.write_all(line.as_bytes());
            }
        });
        self.spell.generation += 1;
        self.status = format!("Added “{word}” to your dictionary.");
        self.redraw();
    }
}
