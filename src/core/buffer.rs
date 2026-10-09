use ropey::Rope;
use super::{
    comments::EditRec,
    typography::{Language, Typography, on_type},
};
use std::ops::Range;

#[derive(Clone, Debug)]
pub struct Edit {
    pub start: usize,
    pub removed: Rope,
    pub inserted: Rope,
    pub before: (usize, usize),
    pub after: (usize, usize),
    before_state: u64,
    after_state: u64,
}

pub struct Document {
    pub text: Rope,
    pub cursor: usize,
    pub anchor: usize,
    pub revision: u64,
    pub line_ending: &'static str,
    pub changed: Range<usize>,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    saved: Rope,
    saved_revision: u64,
    state: u64,
    saved_state: u64,
    states: im::OrdMap<u64, u64>,
    /// Changes since the last [`Document::take_edits`], for anchors that follow the text.
    edits: Vec<EditRec>,
    /// The previous edit was typing, so more typing may join its undo step.
    typing: bool,
    deleting: bool,
    last_edit: Option<std::time::Instant>,
}

impl Default for Document {
    fn default() -> Self {
        Self::new(Rope::new())
    }
}

impl Document {
    pub fn new(text: Rope) -> Self {
        let line_ending = if text.chars().take(8192).any(|c| c == '\r') {
            "\r\n"
        } else {
            "\n"
        };
        Self {
            saved: text.clone(),
            text,
            cursor: 0,
            anchor: 0,
            revision: 0,
            line_ending,
            changed: 0..1,
            undo: Vec::new(),
            redo: Vec::new(),
            saved_revision: 0,
            state: 0,
            saved_state: 0,
            states: [(0u64, 0u64)].into_iter().collect(),
            edits: Vec::new(),
            typing: false,
            deleting: false,
            last_edit: None,
        }
    }
    pub fn selection(&self) -> Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }
    pub fn dirty(&self) -> bool {
        self.state != self.saved_state
    }
    pub fn mark_saved(&mut self, revision: u64, snapshot: Rope) {
        self.saved_revision = revision;
        self.saved_state = self.states.get(&revision).copied().unwrap_or(revision);
        self.saved = snapshot;
    }
    pub fn saved_snapshot(&self) -> Rope {
        self.saved.clone()
    }
    /// The edits made since the last call, oldest first.
    pub fn take_edits(&mut self) -> Vec<EditRec> {
        std::mem::take(&mut self.edits)
    }
    pub fn append_loaded(&mut self, chunk: Rope) {
        self.saved.append(chunk.clone());
        self.text.append(chunk);
    }
    pub fn select(&mut self, cursor: usize, extend: bool) {
        self.cursor = cursor.min(self.text.len_chars());
        if !extend {
            self.anchor = self.cursor;
        }
    }
    pub fn replace(&mut self, range: Range<usize>, text: &str) {
        self.replace_rope(range, Rope::from_str(text));
    }
    pub fn replace_rope(&mut self, range: Range<usize>, inserted: Rope) {
        let changed_line = self.text.char_to_line(range.start);
        let before = (self.cursor, self.anchor);
        self.edits.push(EditRec { start: range.start, removed: range.len(), inserted: inserted.len_chars() });
        let removed = self.text.slice(range.clone()).into();
        let tail = self.text.split_off(range.end);
        self.text.remove(range.start..);
        self.text.append(inserted.clone());
        self.text.append(tail);
        self.cursor = range.start + inserted.len_chars();
        self.anchor = self.cursor;
        self.changed = changed_line..self.text.char_to_line(self.cursor) + 1;
        self.undo.push(Edit {
            start: range.start,
            removed,
            inserted,
            before,
            after: (self.cursor, self.anchor),
            before_state: self.state,
            after_state: self.revision + 1,
        });
        self.redo.clear();
        self.revision += 1;
        self.state = self.revision;
        self.states.insert(self.revision, self.state);
    }
    pub fn insert(&mut self, text: &str) {
        self.replace(self.selection(), text);
    }
    fn apply(&mut self, start: usize, count: usize, replacement: &Rope, caret: (usize, usize)) {
        self.edits.push(EditRec { start, removed: count, inserted: replacement.len_chars() });
        let changed_line = self.text.char_to_line(start);
        let tail = self.text.split_off(start + count);
        self.text.remove(start..);
        self.text.append(replacement.clone());
        self.text.append(tail);
        (self.cursor, self.anchor) = caret;
        self.changed = changed_line..self.text.char_to_line(start + replacement.len_chars()) + 1;
        self.revision += 1;
    }
    pub fn undo(&mut self) {
        if let Some(edit) = self.undo.pop() {
            self.apply(
                edit.start,
                edit.inserted.len_chars(),
                &edit.removed,
                edit.before,
            );
            self.state = edit.before_state;
            self.states.insert(self.revision, self.state);
            self.redo.push(edit);
        }
    }
    pub fn redo(&mut self) {
        if let Some(edit) = self.redo.pop() {
            self.apply(
                edit.start,
                edit.removed.len_chars(),
                &edit.inserted,
                edit.after,
            );
            self.state = edit.after_state;
            self.states.insert(self.revision, self.state);
            self.undo.push(edit);
        }
    }
    pub fn delete(&mut self, backwards: bool) {
        let mut range = self.selection();
        if range.is_empty() {
            if backwards {
                range.start = range.start.saturating_sub(1);
                if range.start > 0
                    && self.text.char(range.start) == '\n'
                    && self.text.char(range.start - 1) == '\r'
                {
                    range.start -= 1;
                }
            } else {
                range.end = (range.end + 1).min(self.text.len_chars());
                if range.end < self.text.len_chars()
                    && self.text.char(range.start) == '\r'
                    && self.text.char(range.end) == '\n'
                {
                    range.end += 1;
                }
            }
        }
        if !range.is_empty() {
            let single_back = backwards && range.len() == 1 && self.selection().is_empty();
            let fresh = self.last_edit.is_none_or(|t| t.elapsed() > std::time::Duration::from_millis(1500));
            let adjacent = self.undo.last().is_some_and(|e| e.inserted.len_chars() == 0 && e.start == range.end);
            self.replace(range, "");
            if single_back && adjacent && self.deleting && !fresh && self.undo.len() >= 2 {
                let last = self.undo.pop().unwrap();
                let previous = self.undo.last_mut().unwrap();
                let mut removed = last.removed;
                removed.append(std::mem::take(&mut previous.removed));
                previous.removed = removed;
                previous.start = last.start;
                previous.after = last.after;
                previous.after_state = last.after_state;
            }
            self.deleting = single_back;
            self.typing = false;
            self.last_edit = Some(std::time::Instant::now());
        }
    }
    pub fn horizontal(&mut self, right: bool, extend: bool, word: bool) {
        let range = self.selection();
        let mut pos = if !extend && !range.is_empty() {
            if right { range.end } else { range.start }
        } else {
            self.cursor
        };
        if range.is_empty() || extend {
            if right {
                pos = (pos + 1).min(self.text.len_chars());
                if pos < self.text.len_chars()
                    && pos > 0
                    && self.text.char(pos - 1) == '\r'
                    && self.text.char(pos) == '\n'
                {
                    pos += 1;
                }
                if word {
                    while pos < self.text.len_chars() && !self.text.char(pos).is_whitespace() {
                        pos += 1;
                    }
                }
            } else {
                pos = pos.saturating_sub(1);
                if pos > 0 && self.text.char(pos) == '\n' && self.text.char(pos - 1) == '\r' {
                    pos -= 1;
                }
                if word {
                    while pos > 0 && !self.text.char(pos - 1).is_whitespace() {
                        pos -= 1;
                    }
                }
            }
        }
        self.select(pos, extend);
    }
    /// A new paragraph. Paragraphs are lines, so nothing is carried over.
    pub fn enter(&mut self) {
        self.insert(&self.line_ending.to_string());
    }
    /// Types `text` as the author typed it, applying auto-typography when given: smart quotes,
    /// dashes and the like rewrite what is just before the caret.
    pub fn type_text(&mut self, text: &str, typography: Option<(&Typography, Language)>) {
        let mut text = text.to_owned();
        let mut delete_before = 0;
        if let (Some((options, language)), true) = (typography, text.chars().count() == 1) {
            let line_start = self.text.line_to_char(self.text.char_to_line(self.selection().start));
            let from = self.selection().start.saturating_sub(64).max(line_start);
            let before: String = self.text.slice(from..self.selection().start).into();
            if let Some(replacement) = on_type(&before, text.chars().next().unwrap(), language, options) {
                delete_before = replacement.delete_before.min(before.chars().count());
                text = replacement.insert;
            }
        }
        let range = self.selection();
        let word_start = text.chars().next().is_some_and(|c| !c.is_whitespace())
            && range.start > 0
            && self.text.char(range.start - 1).is_whitespace();
        let adjacent_to_last = self.undo.last().is_some_and(|e| e.removed.len_chars() == 0 && e.start + e.inserted.len_chars() == range.start);
        let fresh = self.last_edit.is_none_or(|t| t.elapsed() > std::time::Duration::from_millis(1500));
        let merge = range.is_empty() && delete_before == 0 && adjacent_to_last && !word_start && !fresh && self.typing;
        self.replace(range.start - delete_before..range.end, &text);
        if merge && self.undo.len() >= 2 {
            let last = self.undo.pop().unwrap();
            let previous = self.undo.last_mut().unwrap();
            previous.inserted.append(last.inserted);
            previous.after = last.after;
            previous.after_state = last.after_state;
        }
        self.typing = true;
        self.last_edit = Some(std::time::Instant::now());
    }
    /// Wraps the selection in `mark` (`*` or `**`), or takes the marks off if it already has them.
    /// With nothing selected the word at the caret is used, or an empty pair is inserted.
    /// Sets the alignment tag of every non-blank line the selection touches; a line already
    /// aligned that way goes back to the layout's default. The selection keeps its text.
    pub fn align_lines(&mut self, align: super::prose::Align) {
        let selection = self.selection();
        let first = self.text.char_to_line(selection.start);
        let mut last = self.text.char_to_line(selection.end);
        if last > first && self.text.line_to_char(last) == selection.end {
            last -= 1;
        }
        let (mut cursor, mut anchor) = (self.cursor, self.anchor);
        for line in (first..=last).rev() {
            let start = self.text.line_to_char(line);
            let head: String = self.text.line(line).chars().take(4).collect();
            if head.trim().is_empty() {
                continue;
            }
            let old = super::prose::align_tag(&head);
            let new = if old.is_some_and(|t| t.0 == align) { "" } else { align.tag() };
            // A tag is followed by one space, which a removed tag takes with it.
            let new = if new.is_empty() { String::new() } else { format!("{new} ") };
            let old_len = old.map_or(0, |t| head[..t.1].chars().count());
            self.replace(start..start + old_len, &new);
            let delta = new.chars().count() as isize - old_len as isize;
            let shift = |p: usize| if p >= start + old_len { (p as isize + delta) as usize } else if p > start { start } else { p };
            cursor = shift(cursor);
            anchor = shift(anchor);
        }
        self.cursor = cursor.min(self.text.len_chars());
        self.anchor = anchor.min(self.text.len_chars());
    }
    pub fn toggle_emphasis(&mut self, mark: &str) {
        self.typing = false;
        let m = mark.chars().count();
        let mut range = self.selection();
        if range.is_empty() {
            let word = |c: char| c.is_alphanumeric() || c == '\'' || c == '’';
            let (mut a, mut b) = (range.start, range.start);
            while a > 0 && word(self.text.char(a - 1)) {
                a -= 1;
            }
            while b < self.text.len_chars() && word(self.text.char(b)) {
                b += 1;
            }
            if a == b {
                self.insert(&format!("{mark}{mark}"));
                self.select(range.start + m, false);
                if let Some(edit) = self.undo.last_mut() {
                    edit.after = (self.cursor, self.anchor);
                }
                return;
            }
            range = a..b;
            (self.anchor, self.cursor) = (a, b);
        }
        let (a, b) = (range.start, range.end);
        let run = |from: usize, forward: bool, text: &Rope| -> usize {
            let mut n = 0;
            while n < 3
                && if forward { from + n < text.len_chars() && text.char(from + n) == '*' } else { from > n && text.char(from - n - 1) == '*' }
            {
                n += 1;
            }
            n
        };
        // `*` is italics when an odd number of asterisks hug the text, `**` bold when two or more do.
        let present = |n: usize| if m == 2 { n >= 2 } else { n % 2 == 1 };
        let outside = run(a, false, &self.text).min(run(b, true, &self.text));
        let inside = run(a, true, &self.text).min(run(b, false, &self.text));
        if present(outside) {
            let inner: Rope = self.text.slice(a..b).into();
            self.replace_rope(a - m..b + m, inner);
            self.anchor = a - m;
            self.cursor = b - m;
        } else if present(inside) && b - a > 2 * inside {
            let inner: Rope = self.text.slice(a + m..b - m).into();
            self.replace_rope(a..b, inner);
            self.anchor = a;
            self.cursor = b - 2 * m;
        } else {
            self.surround(mark, mark);
            self.anchor = a + m;
            self.cursor = b + m;
        }
        if let Some(edit) = self.undo.last_mut() {
            edit.after = (self.cursor, self.anchor);
        }
    }
    pub fn surround(&mut self, opening: &str, closing: &str) {
        let range = self.selection();
        let length = range.len();
        let mut inserted = Rope::from_str(opening);
        inserted.append(self.text.slice(range.clone()).into());
        inserted.append(Rope::from_str(closing));
        self.replace_rope(range.clone(), inserted);
        self.anchor = range.start + opening.chars().count();
        self.cursor = self.anchor + length;
        if let Some(edit) = self.undo.last_mut() {
            edit.after = (self.cursor, self.anchor);
        }
    }
}

/// Whitespace-separated words, like `str::split_whitespace().count()` but without copying the rope.
pub fn word_count(text: ropey::RopeSlice) -> usize {
    let mut inside = false;
    text.chars()
        .filter(|c| {
            let word = !c.is_whitespace();
            let start = word && !inside;
            inside = word;
            start
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn typed(doc: &mut Document, text: &str) {
        for c in text.chars() {
            doc.type_text(&c.to_string(), None);
        }
    }
    #[test]
    fn edits_keep_unicode_and_original_endings() {
        let mut doc = Document::new(Rope::from_str("café\r\n日本語"));
        doc.select(4, false);
        doc.enter();
        doc.type_text("한글", None);
        assert_eq!(doc.text.to_string(), "café\r\n한글\r\n日本語");
        doc.undo();
        doc.undo();
        assert_eq!(doc.text.to_string(), "café\r\n日本語");
        doc.redo();
        doc.redo();
        assert!(doc.text.to_string().contains("한글"));
        // No bracket pairing in prose.
        doc.select(doc.text.len_chars(), false);
        doc.type_text("(", None);
        assert!(doc.text.to_string().ends_with("日本語("));
    }
    #[test]
    fn typing_is_undone_a_word_at_a_time_and_backspacing_in_one_step() {
        let mut doc = Document::new(Rope::new());
        typed(&mut doc, "hello world");
        assert_eq!(doc.text.to_string(), "hello world");
        doc.undo();
        assert_eq!(doc.text.to_string(), "hello ");
        doc.redo();
        assert_eq!(doc.text.to_string(), "hello world");
        doc.undo();
        doc.undo();
        assert_eq!(doc.text.to_string(), "");
        typed(&mut doc, "abcdef");
        for _ in 0..3 {
            doc.delete(true);
        }
        assert_eq!(doc.text.to_string(), "abc");
        doc.undo();
        assert_eq!(doc.text.to_string(), "abcdef");
        doc.undo();
        assert_eq!(doc.text.to_string(), "");
        assert!(!doc.dirty());
    }
    #[test]
    fn smart_typography_rewrites_what_is_typed() {
        let options = Typography::default();
        let mut doc = Document::new(Rope::new());
        for c in "He said \"hi\"... wait--it's \"fine\".".chars() {
            doc.type_text(&c.to_string(), Some((&options, Language::English)));
        }
        assert_eq!(doc.text.to_string(), "He said “hi”… wait—it’s “fine”.");
        // Undo takes back the rewritten character, not the straight one.
        doc.undo();
        assert!(doc.text.to_string().len() < 40);
        let mut es = Document::new(Rope::new());
        for c in "-Hola -dijo--. \"Adiós\"".chars() {
            es.type_text(&c.to_string(), Some((&options, Language::Spanish)));
        }
        assert_eq!(es.text.to_string(), "—Hola —dijo—. «Adiós»");
    }
    #[test]
    fn emphasis_toggles_on_selections_words_and_existing_marks() {
        let mut doc = Document::new(Rope::from_str("one two three"));
        doc.anchor = 4;
        doc.cursor = 7;
        doc.toggle_emphasis("*");
        assert_eq!(doc.text.to_string(), "one *two* three");
        assert_eq!(doc.selection(), 5..8);
        doc.toggle_emphasis("**");
        assert_eq!(doc.text.to_string(), "one ***two*** three");
        doc.toggle_emphasis("*");
        assert_eq!(doc.text.to_string(), "one **two** three");
        doc.toggle_emphasis("**");
        assert_eq!(doc.text.to_string(), "one two three");
        // No selection: the word at the caret.
        doc.select(9, false);
        assert_eq!((doc.anchor, doc.cursor), (9, 9));
        doc.toggle_emphasis("*");
        assert_eq!(doc.text.to_string(), "one two *three*");
        // Selecting the marks too also takes them off.
        doc.anchor = 8;
        doc.cursor = 15;
        doc.toggle_emphasis("*");
        assert_eq!(doc.text.to_string(), "one two three");
        // Nothing near the caret: an empty pair with the caret inside.
        doc.select(7, false);
        doc.insert(" ");
        doc.toggle_emphasis("*");
        assert_eq!(doc.text.to_string(), "one two ** three");
        assert_eq!(doc.cursor, 9);
    }
    #[test]
    fn deleted_selection_is_logged_without_flattening_the_file() {
        let mut doc = Document::new(Rope::from_str("αβγ\r\nδ"));
        doc.anchor = 1;
        doc.cursor = 5;
        doc.insert("é");
        assert_eq!(doc.text.to_string(), "αéδ");
        doc.undo();
        assert_eq!(doc.text.to_string(), "αβγ\r\nδ");
        assert_eq!(doc.selection(), 1..5);
    }
    #[test]
    fn edits_are_recorded_for_anchors() {
        let mut doc = Document::new(Rope::from_str("hello"));
        doc.select(5, false);
        doc.insert("!!");
        doc.anchor = 0;
        doc.cursor = 2;
        doc.insert("");
        doc.undo();
        let edits = doc.take_edits();
        assert_eq!(
            edits,
            [
                EditRec { start: 5, removed: 0, inserted: 2 },
                EditRec { start: 0, removed: 2, inserted: 0 },
                EditRec { start: 0, removed: 0, inserted: 2 },
            ]
        );
        assert!(doc.take_edits().is_empty());
    }
    #[test]
    fn saved_state_survives_undo_redo_branching_and_delayed_saves() {
        let mut doc = Document::new(Rope::from_str("hello"));
        doc.select(5, false);
        doc.insert("!");
        let revision = doc.revision;
        let saved = doc.text.clone();
        doc.insert("?");
        doc.mark_saved(revision, saved);
        assert!(doc.dirty());
        doc.undo();
        assert!(!doc.dirty());
        doc.redo();
        assert!(doc.dirty());
        doc.undo();
        doc.insert(".");
        assert!(doc.dirty());
        doc.anchor = 0;
        doc.cursor = 5;
        doc.surround("**", "**");
        let selected = doc.selection();
        doc.undo();
        doc.redo();
        assert_eq!(doc.selection(), selected);
    }
    #[test]
    fn counts_words_across_whitespace_and_chunks() {
        assert_eq!(word_count(Rope::from_str("").slice(..)), 0);
        assert_eq!(word_count(Rope::from_str(" a  b\r\nc\t").slice(..)), 3);
        let long = "word ".repeat(5000);
        assert_eq!(word_count(Rope::from_str(&long).slice(..)), 5000);
    }
}
