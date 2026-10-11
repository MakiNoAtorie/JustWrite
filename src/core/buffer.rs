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
    /// Edits that touch a scene's name (and its break) are refused; the Scenes panel renames.
    pub protect_names: bool,
    /// An edit was refused because of `protect_names`; the program says why and clears this.
    pub name_blocked: bool,
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
            protect_names: true,
            name_blocked: false,
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
    fn name_pair_line(&self, l: usize) -> bool {
        locked_line(&self.text, l)
    }
    /// Whether `range` changes part of a name line or of its break. Covering the whole line is fine.
    fn touches_scene_name(&self, range: &Range<usize>) -> bool {
        let len = self.text.len_chars();
        let end = range.end.min(len);
        let check = |pos: usize| {
            let l = self.text.char_to_line(pos.min(len));
            if !self.name_pair_line(l) {
                return false;
            }
            let last = l + 1 >= self.text.len_lines();
            let (s, e) = (self.text.line_to_char(l), if last { len } else { self.text.line_to_char(l + 1) });
            let overlaps = if range.start == end { s <= range.start && (range.start < e || last) } else { range.start < e && end > s };
            let covered = range.start < end && range.start <= s && end >= e;
            overlaps && !covered
        };
        check(range.start) || (end > range.start && check(end))
    }
    /// Whether typing `inserted` over `range` turns its line into a scene name (`## x ##`).
    fn creates_scene_name(&self, range: &Range<usize>, inserted: &Rope) -> bool {
        let text: String = inserted.to_string();
        if !text.contains('#') || text.contains('\n') {
            return false;
        }
        let start = range.start.min(self.text.len_chars());
        let line = self.text.char_to_line(start);
        let prefix: String = self.text.slice(self.text.line_to_char(line)..start).into();
        let end = range.end.min(self.text.len_chars());
        let tail: String = self.text.slice(end..self.text.line_to_char((self.text.char_to_line(end) + 1).min(self.text.len_lines())).max(end)).into();
        let tail = tail.split(['\r', '\n']).next().unwrap_or("");
        let merged = format!("{prefix}{text}{tail}");
        super::prose::scene_name(&merged).is_some() || super::prose::is_hash_break(&merged)
    }
    pub fn replace_rope(&mut self, range: Range<usize>, inserted: Rope) {
        if self.protect_names && (self.touches_scene_name(&range) || self.creates_scene_name(&range, &inserted)) {
            self.name_blocked = true;
            return;
        }
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
    /// The lines of the separator group at `line` (a hash break with its name, or a name with its
    /// break), as a char range including line endings.
    fn separator_group(&self, line: usize) -> Option<Range<usize>> {
        if !self.name_pair_line(line) {
            return None;
        }
        let text = |l: usize| -> String { self.text.line(l).chars().take(512).collect() };
        let is_name = |l: usize| super::prose::scene_name(&text(l)).is_some();
        let (mut first, mut last) = (line, line);
        if is_name(line) {
            if line > 0 && super::prose::is_scene_break(text(line - 1).trim()) {
                first = line - 1;
            }
        } else if line + 1 < self.text.len_lines() && is_name(line + 1) {
            last = line + 1;
        }
        let end = if last + 1 < self.text.len_lines() { self.text.line_to_char(last + 1) } else { self.text.len_chars() };
        Some(self.text.line_to_char(first)..end)
    }
    pub fn delete(&mut self, backwards: bool) {
        let mut range = self.selection();
        if range.is_empty() && self.protect_names {
            // The caret is on a separator shown as a symbol: Delete removes the whole separator.
            let line = self.text.char_to_line(self.cursor.min(self.text.len_chars()));
            if let Some(group) = self.separator_group(line) {
                self.protect_names = false;
                self.replace(group, "");
                self.protect_names = true;
                self.typing = false;
                return;
            }
        }
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
            let before = self.revision;
            self.replace(range, "");
            if self.revision == before {
                return;
            }
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
        let before = self.revision;
        self.replace(range.start - delete_before..range.end, &text);
        if self.revision == before {
            return;
        }
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
    /// Puts a scene break (`###newscene###`) with a blank line on each side at the caret, as one undo step.
    /// The paragraph is split at the caret; the caret ends on the line after the break. Returns
    /// false, changing nothing, when the caret is already on a break line.
    pub fn insert_scene_break(&mut self) -> bool {
        self.typing = false;
        let le = self.line_ending;
        let mark = super::prose::SCENE_BREAK;
        let line = self.text.char_to_line(self.cursor.min(self.text.len_chars()));
        let start = self.text.line_to_char(line);
        let content: String = self.text.line(line).to_string().trim_end_matches(['\r', '\n']).to_owned();
        if super::prose::is_scene_break(&content) || (self.protect_names && self.name_pair_line(line)) {
            return false;
        }
        let end = start + content.chars().count();
        let column = (self.cursor - start).min(content.chars().count());
        let split = content.char_indices().nth(column).map_or(content.len(), |(i, _)| i);
        let (pre, post) = (content[..split].trim_end(), content[split..].trim_start());
        let prev_blank = line == 0 || self.text.line(line - 1).to_string().trim().is_empty();
        let (insert, back) = if content.trim().is_empty() {
            let text = if prev_blank { format!("{mark}{le}") } else { format!("{le}{mark}{le}") };
            (text, 0)
        } else {
            let head = match (pre.is_empty(), prev_blank) {
                (false, _) => format!("{pre}{le}{le}{mark}"),
                (true, true) => mark.to_owned(),
                (true, false) => format!("{le}{mark}"),
            };
            let back = post.chars().count();
            (format!("{head}{le}{le}{post}"), back)
        };
        let caret = start + insert.chars().count() - back;
        self.replace(start..end, &insert);
        self.select(caret, false);
        if let Some(edit) = self.undo.last_mut() {
            edit.after = (self.cursor, self.anchor);
        }
        true
    }
    /// Puts an empty epigraph at the head of the chapter (under the name of scene 1), as one undo
    /// step, with its placeholder selected. Returns false, only moving the caret, when the chapter
    /// already starts with one.
    pub fn insert_epigraph(&mut self) -> bool {
        self.typing = false;
        let le = self.line_ending;
        let lines = self.text.len_lines();
        let text = |l: usize| self.text.line(l).to_string();
        let line = usize::from(super::prose::scene_name(&text(0)).is_some());
        if let Some(found) = (line..lines).find(|l| !text(*l).trim().is_empty()) {
            let own = text(found);
            if super::prose::opens_quote(&own) {
                let content = super::prose::quote_text(&own, true, true);
                let start = self.text.line_to_char(found) + own[..own.find(content).unwrap_or(0)].chars().count();
                let end = start + content.chars().count();
                self.select(start, false);
                self.select(end, true);
                return false;
            }
        }
        let (quote, author) = ("Quote", "Author");
        let body = format!("> {quote}{le}— {author} <{le}");
        let (start, insert, lead) = if line < lines {
            let gap = if text(line).trim().is_empty() { "" } else { le };
            (self.text.line_to_char(line), format!("{body}{gap}"), 0)
        } else {
            (self.text.len_chars(), format!("{le}{body}"), le.len())
        };
        // A whole-line insert next to the name of scene 1 does not change the name.
        let protect = std::mem::take(&mut self.protect_names);
        self.replace(start..start, &insert);
        self.protect_names = protect;
        self.select(start + lead + 2, false);
        self.select(start + lead + 2 + quote.len(), true);
        if let Some(edit) = self.undo.last_mut() {
            edit.after = (self.cursor, self.anchor);
        }
        true
    }
    /// Wraps the selection in `mark` (`*` or `**`), or takes the marks off if it already has them.
    /// With nothing selected the word at the caret is used, or an empty pair is inserted.
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
        // Marks go in as two edits; a locked line would take only one of them.
        if self.protect_names && (self.text.char_to_line(a)..=self.text.char_to_line(b)).any(|l| self.name_pair_line(l)) {
            self.name_blocked = true;
            return;
        }
        let run =|from: usize, forward: bool, text: &Rope| -> usize {
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

/// Whether line `l` is a scene name, a hash break, or the break above a name: the lines the
/// editor locks.
pub fn locked_line(text: &Rope, l: usize) -> bool {
    let lines = text.len_lines();
    if l >= lines {
        return false;
    }
    // A name line can be longer than a break, but not without limit.
    let get = |i: usize| -> String { text.line(i).chars().take(512).collect() };
    let line = get(l);
    super::prose::is_hash_break(&line)
        || super::prose::scene_name(&line).is_some()
        || (l + 1 < lines && super::prose::is_scene_break(line.trim()) && super::prose::scene_name(&get(l + 1)).is_some())
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
    fn an_epigraph_goes_to_the_head_of_the_chapter_once() {
        let mut doc = Document::new(Rope::from_str("## One ##\nProse.\n"));
        doc.protect_names = true;
        doc.select(12, false);
        assert!(doc.insert_epigraph());
        assert_eq!(doc.text.to_string(), "## One ##\n> Quote\n— Author <\n\nProse.\n");
        assert_eq!(doc.text.slice(doc.selection()).to_string(), "Quote");
        assert!(!doc.insert_epigraph(), "the one that is there is selected");
        assert_eq!(doc.text.slice(doc.selection()).to_string(), "Quote");
        doc.undo();
        assert_eq!(doc.text.to_string(), "## One ##\nProse.\n");
        let mut bare = Document::new(Rope::from_str("## One ##"));
        bare.protect_names = true;
        assert!(bare.insert_epigraph());
        assert_eq!(bare.text.to_string(), "## One ##\n> Quote\n— Author <\n");
        let mut empty = Document::new(Rope::from_str(""));
        assert!(empty.insert_epigraph());
        assert_eq!(empty.text.to_string(), "> Quote\n— Author <\n");
    }

    #[test]
    fn scene_break_splits_a_paragraph_and_undoes_in_one_step() {
        let mut doc = Document::new(Rope::from_str("one two\nnext"));
        doc.select(4, false);
        assert!(doc.insert_scene_break());
        assert_eq!(doc.text.to_string(), "one\n\n###newscene###\n\ntwo\nnext");
        assert_eq!(doc.text.char_to_line(doc.cursor), 4);
        doc.select(doc.text.line_to_char(2), false);
        assert!(!doc.insert_scene_break());
        doc.undo();
        assert_eq!(doc.text.to_string(), "one two\nnext");
    }
    #[test]
    fn scene_break_at_line_end_and_on_empty_line() {
        let mut doc = Document::new(Rope::from_str("end"));
        doc.select(3, false);
        doc.insert_scene_break();
        assert_eq!(doc.text.to_string(), "end\n\n###newscene###\n\n");
        let mut doc = Document::new(Rope::from_str("a\n\nb"));
        doc.select(2, false);
        doc.insert_scene_break();
        assert_eq!(doc.text.to_string(), "a\n\n###newscene###\n\nb");
    }
    #[test]
    fn separators_and_names_resist_edits_unless_unprotected() {
        let text = "para\n\n###newscene###\n## The storm ##\n\nnext";
        let mut doc = Document::new(Rope::from_str(text));
        let name_end = text.find("storm").unwrap() + 5;
        doc.select(name_end, false);
        doc.type_text("x", None);
        let mark = text.find("###newscene").unwrap();
        doc.select(mark + 3, false);
        doc.type_text("q", None);
        assert_eq!(doc.text.to_string(), text);
        assert!(doc.name_blocked);
        // Text around them still edits, and a selection covering both lines goes whole.
        doc.select(0, false);
        doc.type_text("A", None);
        assert!(doc.text.to_string().starts_with("Apara"));
        let now = doc.text.to_string();
        doc.anchor = now.find("###newscene").unwrap();
        doc.cursor = now.find("\n\nnext").unwrap() + 1;
        doc.delete(false);
        assert_eq!(doc.text.to_string(), "Apara\n\n\nnext");
        doc.undo();
        // Delete on the separator's own line removes the separator with its name.
        doc.select(doc.text.to_string().find("###newscene").unwrap() + 2, false);
        doc.delete(false);
        assert_eq!(doc.text.to_string(), "Apara\n\n\nnext");
        doc.undo();
        // A name away from a break is locked too; Bold over it does nothing; Delete takes that line only.
        let lone_text = "a\n## Lone ##\n## Two ##\nb";
        let mut lone = Document::new(Rope::from_str(lone_text));
        lone.select(4, false);
        lone.type_text("x", None);
        (lone.anchor, lone.cursor) = (0, 6);
        lone.toggle_emphasis("**");
        assert_eq!(lone.text.to_string(), lone_text);
        lone.select(4, false);
        lone.delete(false);
        assert_eq!(lone.text.to_string(), "a\n## Two ##\nb");
        // Typing a separator or a name by hand is refused.
        let mut plain = Document::new(Rope::new());
        for c in "###".chars() {
            plain.type_text(&c.to_string(), None);
        }
        assert_eq!(plain.text.to_string(), "");
        // Unprotected, the name edits like text.
        doc.protect_names = false;
        doc.select(doc.text.to_string().find("storm").unwrap() + 5, false);
        doc.type_text("y", None);
        assert!(doc.text.to_string().contains("## The stormy ##"));
    }
    #[test]
    fn a_half_typed_break_has_a_ghost() {
        assert_eq!(super::super::prose::scene_ghost("###"), Some("newscene###"));
        assert_eq!(super::super::prose::scene_ghost("###new"), Some("scene###"));
        assert_eq!(super::super::prose::scene_ghost("##"), None);
        assert_eq!(super::super::prose::scene_ghost("###newscene###"), None);
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
