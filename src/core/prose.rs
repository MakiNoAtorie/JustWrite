//! Prose formatting: `*italics*`, `**bold**`, scene breaks, and how a chunk of text is presented
//! in the editor with its markers concealed.
//!
//! The subset is deliberately small. There are no headings (chapter titles come from the project
//! tree), no lists and no links. Each non-empty line is a paragraph.

use super::notes::{self, Delim, Kind, Marker};
use ropey::Rope;
use std::ops::Range;

// ---- scene breaks ---------------------------------------------------------------------------

/// A line holding only `#`, `***`, `* * *`, `---` or similar.
pub fn is_scene_break(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() || t.len() > 16 {
        return false;
    }
    let compact: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    if compact == "⁂" {
        return true;
    }
    let first = compact.chars().next().unwrap();
    if !compact.chars().all(|c| c == first) {
        return false;
    }
    match first {
        '#' => compact.len() <= 3,
        '*' | '-' | '_' | '~' | '=' => compact.len() >= 3,
        _ => false,
    }
}

// ---- emphasis -------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Emphasis {
    pub bold: bool,
    pub italic: bool,
}

/// Result of parsing emphasis in one paragraph of note-free text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Parsed {
    /// Byte ranges of `*` delimiters that were consumed (concealed when rendering).
    pub delimiters: Vec<Range<usize>>,
    /// Byte ranges of backslashes that escape an asterisk or another backslash.
    pub escapes: Vec<Range<usize>>,
    /// Emphasis intervals in bytes, innermost last.
    pub spans: Vec<(Range<usize>, Emphasis)>,
}
impl Parsed {
    /// Emphasis in effect at each byte of `text`.
    pub fn per_byte(&self, len: usize) -> Vec<Emphasis> {
        let mut bold = vec![0i32; len + 1];
        let mut italic = vec![0i32; len + 1];
        for (range, e) in &self.spans {
            let (a, b) = (range.start.min(len), range.end.min(len));
            if e.bold {
                bold[a] += 1;
                bold[b] -= 1;
            }
            if e.italic {
                italic[a] += 1;
                italic[b] -= 1;
            }
        }
        let (mut b, mut i) = (0, 0);
        (0..len)
            .map(|k| {
                b += bold[k];
                i += italic[k];
                Emphasis { bold: b > 0, italic: i > 0 }
            })
            .collect()
    }
}

struct Run {
    start: usize,
    end: usize,
    can_open: bool,
    can_close: bool,
    open_used: usize,
    close_used: usize,
}
impl Run {
    fn left(&self) -> usize {
        self.end - self.start - self.open_used - self.close_used
    }
}

/// Parses `*` and `**` emphasis. A backslash before `*` (or before another backslash that is
/// followed by `*`) escapes it.
pub fn parse_emphasis(text: &str) -> Parsed {
    let bytes = text.as_bytes();
    let mut parsed = Parsed::default();
    let mut runs: Vec<Run> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if matches!(bytes.get(i + 1), Some(b'*') | Some(b'\\')) => {
                // `\*` is a literal asterisk; `\\` before an asterisk is a literal backslash.
                let mut j = i;
                while bytes.get(j) == Some(&b'\\') {
                    j += 1;
                }
                if bytes.get(j) == Some(&b'*') {
                    let n = j - i;
                    for k in 0..n / 2 {
                        parsed.escapes.push(i + 2 * k + 1..i + 2 * k + 2);
                    }
                    if n % 2 == 1 {
                        parsed.escapes.push(j - 1..j);
                        i = j + 1; // the asterisk is literal
                    } else {
                        i = j;
                    }
                } else {
                    i = j.max(i + 1);
                }
            }
            b'*' => {
                let start = i;
                while bytes.get(i) == Some(&b'*') {
                    i += 1;
                }
                let prev = text[..start].chars().next_back();
                let next = text[i..].chars().next();
                runs.push(Run {
                    start,
                    end: i,
                    can_open: next.is_some_and(|c| !c.is_whitespace()),
                    can_close: prev.is_some_and(|c| !c.is_whitespace()),
                    open_used: 0,
                    close_used: 0,
                });
            }
            _ => i += 1,
        }
    }
    let mut stack: Vec<usize> = Vec::new();
    for r in 0..runs.len() {
        if runs[r].can_close {
            while runs[r].left() > 0 {
                let Some(pos) = stack.iter().rposition(|&o| runs[o].left() > 0) else { break };
                let o = stack[pos];
                let n = if runs[o].left() >= 2 && runs[r].left() >= 2 { 2 } else { 1 };
                let open_end = runs[o].end - runs[o].open_used;
                let close_start = runs[r].start + runs[r].close_used;
                parsed.delimiters.push(open_end - n..open_end);
                parsed.delimiters.push(close_start..close_start + n);
                parsed.spans.push((
                    open_end..close_start,
                    Emphasis { bold: n == 2, italic: n == 1 },
                ));
                runs[o].open_used += n;
                runs[r].close_used += n;
                stack.truncate(pos + 1);
                if runs[o].left() == 0 {
                    stack.pop();
                }
            }
        }
        if runs[r].left() > 0 && runs[r].can_open {
            stack.push(r);
        }
    }
    parsed.delimiters.sort_by_key(|d| d.start);
    parsed.escapes.sort_by_key(|d| d.start);
    parsed
}

/// Emphasis of every character of `text`; `None` for the `*` delimiters and escapes that are
/// not shown.
pub fn char_emphasis(text: &str) -> Vec<Option<Emphasis>> {
    let parsed = parse_emphasis(text);
    let per_byte = parsed.per_byte(text.len());
    let mut hidden = vec![false; text.len()];
    for r in parsed.delimiters.iter().chain(&parsed.escapes) {
        hidden[r.clone()].fill(true);
    }
    text.char_indices().map(|(b, _)| (!hidden[b]).then(|| per_byte[b])).collect()
}

/// A stretch of paragraph text with one style, for exporters. Delimiters are gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Piece {
    pub text: String,
    pub emphasis: Emphasis,
    /// Char offset of the first char within the parsed text.
    pub start: usize,
}
pub fn pieces(text: &str) -> Vec<Piece> {
    let parsed = parse_emphasis(text);
    let per_byte = parsed.per_byte(text.len());
    let mut hidden = vec![false; text.len()];
    for r in parsed.delimiters.iter().chain(&parsed.escapes) {
        hidden[r.clone()].fill(true);
    }
    let mut out: Vec<Piece> = Vec::new();
    for (char_index, (byte, c)) in text.char_indices().enumerate() {
        if hidden[byte] {
            continue;
        }
        match out.last_mut() {
            Some(p) if p.emphasis == per_byte[byte] && p.start + p.text.chars().count() == char_index => p.text.push(c),
            _ => out.push(Piece { text: c.to_string(), emphasis: per_byte[byte], start: char_index }),
        }
    }
    out
}

// ---- presentation ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    /// Part of a note: greyed out.
    pub note: bool,
    /// A concealed marker shown because its line is being edited.
    pub dim: bool,
    /// The scene-break symbol.
    pub scene: bool,
    /// Spelling mistake: drawn with a wavy underline.
    pub misspelled: bool,
}
#[derive(Clone, Debug)]
pub struct Span {
    pub bytes: Range<usize>,
    pub style: Style,
}

/// What a chunk of source looks like on screen. The source map is in characters; cosmic-text
/// hits are UTF-8 byte indices.
#[derive(Default)]
pub struct Presentation {
    pub text: String,
    pub spans: Vec<Span>,
    pub source: Vec<(usize, usize)>,
    /// The chunk is a scene break shown as a symbol: centre it.
    pub center: bool,
    /// Every character of the chunk is a hidden note: it takes no room.
    pub hidden: bool,
    pub start: usize,
    pub end: usize,
}
impl Presentation {
    pub fn source_at(&self, byte: usize) -> usize {
        let index = self.source.partition_point(|(b, _)| *b <= byte).saturating_sub(1);
        self.source.get(index).map_or(self.start, |(_, source)| *source)
    }
    pub fn byte_at(&self, source: usize) -> usize {
        let index = self.source.partition_point(|(_, c)| *c < source);
        self.source.get(index).map_or(self.text.len(), |(byte, _)| *byte)
    }
}

#[derive(Clone, Copy)]
pub struct Options<'a> {
    pub marker: &'a Marker,
    /// Keep `*` and note markers visible (dimmed) on every line.
    pub show_markers: bool,
    /// Do not show notes at all (except on the line being edited).
    pub hide_notes: bool,
    /// Shown for scene-break lines when they are not being edited.
    pub scene_symbol: &'a str,
}

/// State a chunk starts in, carried over from the lines and chunks before it.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Initial {
    pub in_block: bool,
    /// The chunk continues a line note begun in an earlier chunk of the same line.
    pub in_line_note: bool,
}

/// Presents a bounded chunk of display-form text. `cursor_line` is the index (within `source`)
/// of the line being edited; its markers are shown.
pub fn present(source: &str, start: usize, cursor_line: Option<usize>, initial: Initial, options: Options) -> Presentation {
    #[derive(Clone, Copy, PartialEq)]
    enum Class {
        Prose,
        Note,
        Marker,
        Escape,
        Ending,
    }
    let mut out = Presentation { start, end: start + source.chars().count(), ..Default::default() };
    let mut char_offset = start;
    let mut in_block = initial.in_block;
    let mut hid_something = false;
    for (line_i, line) in source.split_inclusive('\n').enumerate() {
        let raw = options.show_markers || cursor_line == Some(line_i);
        let body_end = line.trim_end_matches(['\r', '\n']).len();
        let mut scan = notes::scan_line(line, options.marker, in_block);
        if line_i == 0 && initial.in_line_note {
            scan.segs = vec![notes::Seg { range: 0..body_end, kind: Kind::Note }];
            scan.segs.retain(|s| !s.range.is_empty());
        }
        let started_in_block = in_block;
        in_block = scan.block_after;
        // Classify every byte of the line.
        let mut class = vec![Class::Prose; line.len()];
        for seg in &scan.segs {
            class[seg.range.clone()].fill(match seg.kind {
                Kind::Prose => Class::Prose,
                Kind::Note => Class::Note,
                Kind::Delim(_) => Class::Marker,
                Kind::Escape => Class::Escape,
            });
        }
        class[body_end..].fill(Class::Ending);
        let block_marker_line = scan
            .segs
            .iter()
            .any(|s| matches!(s.kind, Kind::Delim(Delim::BlockOpen | Delim::BlockClose)));
        // The space padding a note away from its marker belongs to the marker.
        for (k, seg) in scan.segs.iter().enumerate() {
            let pad = |range: Range<usize>, class: &mut Vec<Class>| {
                if range.len() == 1 && matches!(line.as_bytes()[range.start], b' ' | b'\t') {
                    class[range.start] = Class::Marker;
                }
            };
            match seg.kind {
                Kind::Delim(Delim::Open | Delim::Line) => {
                    if let Some(next) = scan.segs.get(k + 1).filter(|n| n.kind == Kind::Note) {
                        pad(next.range.start..(next.range.start + 1).min(next.range.end), &mut class);
                    }
                }
                Kind::Delim(Delim::Close) if k > 0 && scan.segs[k - 1].kind == Kind::Note => {
                    let prev = &scan.segs[k - 1];
                    pad(prev.range.end.saturating_sub(1).max(prev.range.start)..prev.range.end, &mut class);
                }
                _ => {}
            }
        }

        // Scene break shown as a symbol.
        if !raw && !scan.has_notes() && !started_in_block && !initial.in_line_note && is_scene_break(&line[..body_end]) {
            let position = out.text.len();
            out.source.push((position, char_offset));
            out.text.push_str(options.scene_symbol);
            out.spans.push(Span { bytes: position..out.text.len(), style: Style { scene: true, ..Default::default() } });
            out.center = true;
            let ending = &line[body_end..];
            if !ending.is_empty() {
                let at = out.text.len();
                out.source.push((at, char_offset + line[..body_end].chars().count()));
                out.text.push_str(ending);
                out.spans.push(Span { bytes: at..out.text.len(), style: Style::default() });
            }
            char_offset += line.chars().count();
            continue;
        }

        // Emphasis runs over the prose characters only, so a note in the middle of italics
        // does not break the pair.
        let prose: String = line.char_indices().filter(|&(b, _)| class[b] == Class::Prose).map(|(_, c)| c).collect();
        let parsed = parse_emphasis(&prose);
        let emphasis = parsed.per_byte(prose.len());
        let mut conceal = vec![false; prose.len()];
        for r in parsed.delimiters.iter().chain(&parsed.escapes) {
            conceal[r.clone()].fill(true);
        }
        let mut prose_index = 0usize; // byte index into `prose`
        let hide_notes_here = options.hide_notes && !raw;
        let mut collapsing = false;
        for (byte, character) in line.char_indices() {
            let mut style = Style::default();
            let mut visible = true;
            match class[byte] {
                Class::Prose => {
                    let e = emphasis[prose_index];
                    style.bold = e.bold;
                    style.italic = e.italic;
                    if conceal[prose_index] {
                        visible = raw;
                        style.dim = true;
                    }
                    prose_index += character.len_utf8();
                }
                Class::Note => {
                    style.note = true;
                    visible = !hide_notes_here;
                }
                Class::Marker => {
                    style.note = true;
                    style.dim = true;
                    visible = if block_marker_line { !hide_notes_here } else { raw };
                }
                Class::Escape => {
                    style.dim = true;
                    visible = raw;
                }
                Class::Ending => {}
            }
            if hide_notes_here && matches!(class[byte], Class::Note | Class::Marker) {
                hid_something = true;
                collapsing = true;
            }
            // Hiding a note between two spaces would leave a double space.
            if visible && class[byte] == Class::Prose && collapsing {
                if character == ' ' && out.text.ends_with(' ') {
                    visible = false;
                }
                collapsing = false;
            }
            if visible {
                let position = out.text.len();
                out.source.push((position, char_offset));
                out.text.push(character);
                match out.spans.last_mut() {
                    Some(span) if span.style == style && span.bytes.end == position => span.bytes.end = out.text.len(),
                    _ => out.spans.push(Span { bytes: position..out.text.len(), style }),
                }
            }
            char_offset += 1;
        }
    }
    out.hidden = hid_something && out.text.trim().is_empty();
    out.source.push((out.text.len(), out.end));
    out
}

// ---- block index ----------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockLine {
    /// Starts inside a block note.
    pub before: bool,
    pub after: bool,
    /// Begins a line note that continues past the first 512 characters.
    pub scene: bool,
    hash: u64,
}
/// Per-line state for the whole document: which lines sit inside block notes and where the scene
/// breaks are. Updated incrementally; only the first 512 characters of a line are inspected, so a
/// single huge line is never flattened.
#[derive(Clone, Default)]
pub struct BlockIndex {
    pub lines: im::Vector<BlockLine>,
    pub scenes: im::OrdSet<usize>,
}
fn fnv(text: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for b in text.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
impl BlockIndex {
    pub fn update(&mut self, rope: &Rope, from_line: usize, marker: &Marker) {
        self.update_range(rope, from_line..from_line + 1, marker);
    }
    pub fn update_range(&mut self, rope: &Rope, dirty: Range<usize>, marker: &Marker) {
        let count = rope.len_lines();
        let initial = self.lines.is_empty();
        let from = dirty.start.min(count.saturating_sub(1));
        let structural = count != self.lines.len();
        if structural && !initial {
            let delta = count as isize - self.lines.len() as isize;
            self.scenes = self
                .scenes
                .iter()
                .filter_map(|&line| {
                    if line < from {
                        Some(line)
                    } else if delta < 0 && line < from + (-delta as usize) + 1 {
                        None
                    } else {
                        Some((line as isize + delta).max(0) as usize)
                    }
                })
                .collect();
        }
        if self.lines.is_empty() {
            self.lines = std::iter::repeat_n(BlockLine::default(), count).collect();
        } else if structural {
            let position = (from + 1).min(self.lines.len());
            if count > self.lines.len() {
                for _ in 0..count - self.lines.len() {
                    self.lines.insert(position, BlockLine::default());
                }
            } else {
                for _ in 0..self.lines.len() - count {
                    self.lines.remove(position.min(self.lines.len() - 1));
                }
            }
        }
        let mut state = if from == 0 { false } else { self.lines.get(from - 1).is_some_and(|l| l.after) };
        for line in from..count {
            let prefix: String = rope.line(line).chars().take(512).collect();
            let before = state;
            if notes::is_block_line(&prefix, marker) {
                state = !state;
            }
            let scene = !before && is_scene_break(&prefix);
            let entry = BlockLine { before, after: state, scene, hash: fnv(&prefix) };
            if !initial && line >= dirty.end && self.lines.get(line) == Some(&entry) {
                break;
            }
            if scene {
                self.scenes.insert(line);
            } else {
                self.scenes.remove(&line);
            }
            self.lines.set(line, entry);
        }
    }
    pub fn in_block(&self, line: usize) -> bool {
        self.lines.get(line).is_some_and(|l| l.before)
    }
}

// ---- toolbar state --------------------------------------------------------------------------

/// Formatting that applies at the cursor, used to light up toolbar buttons.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct Format {
    pub bold: bool,
    pub italic: bool,
    pub note: bool,
}
/// `line` is the cursor's line (possibly a window of a very long one) and `col` the cursor column
/// in it, in characters.
pub fn format_at(line: &str, col: usize, marker: &Marker) -> Format {
    let scan = notes::scan_line(line, marker, false);
    let mut format = Format::default();
    let byte = line.char_indices().nth(col).map_or(line.len(), |(b, _)| b);
    let mut prose = String::new();
    let mut at = None;
    for seg in &scan.segs {
        if seg.range.start <= byte && byte <= seg.range.end && matches!(seg.kind, Kind::Note) {
            format.note = true;
        }
        if matches!(seg.kind, Kind::Prose) {
            if seg.range.start <= byte && byte <= seg.range.end {
                at = Some(prose.len() + (byte - seg.range.start));
            }
            prose.push_str(&line[seg.range.clone()]);
        }
    }
    if let Some(at) = at {
        let parsed = parse_emphasis(&prose);
        let per = parsed.per_byte(prose.len());
        let probe = at.saturating_sub(1).min(prose.len().saturating_sub(1));
        if !prose.is_empty() {
            format.bold = per[probe].bold && at > 0;
            format.italic = per[probe].italic && at > 0;
        }
    }
    format
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options(marker: &Marker) -> Options<'_> {
        Options { marker, show_markers: false, hide_notes: false, scene_symbol: "* * *" }
    }

    #[test]
    fn scene_breaks() {
        for yes in ["#", "  #  ", "***", "* * *", "---", "___", "~~~~", "⁂", "##"] {
            assert!(is_scene_break(yes), "{yes:?}");
        }
        for no in ["", "# Title", "**", "*", "word", "- item", "#hash", "*****************", "—", "####"] {
            assert!(!is_scene_break(no), "{no:?}");
        }
    }

    fn render(text: &str) -> String {
        pieces(text)
            .iter()
            .map(|p| {
                let (b, i) = (p.emphasis.bold, p.emphasis.italic);
                match (b, i) {
                    (true, true) => format!("<bi>{}</bi>", p.text),
                    (true, false) => format!("<b>{}</b>", p.text),
                    (false, true) => format!("<i>{}</i>", p.text),
                    _ => p.text.clone(),
                }
            })
            .collect()
    }
    #[test]
    fn emphasis_follows_markdown_rules() {
        assert_eq!(render("a *b* c"), "a <i>b</i> c");
        assert_eq!(render("a **b** c"), "a <b>b</b> c");
        assert_eq!(render("***both***"), "<bi>both</bi>");
        assert_eq!(render("*a **b** c*"), "<i>a </i><bi>b</bi><i> c</i>");
        assert_eq!(render("2 * 3 * 4"), "2 * 3 * 4");
        assert_eq!(render("lonely * star"), "lonely * star");
        assert_eq!(render("*unclosed"), "*unclosed");
        assert_eq!(render(r"\*not italic\*"), "*not italic*");
        assert_eq!(render(r"\\*is italic*"), r"\<i>is italic</i>");
        assert_eq!(render("*é 日本語*"), "<i>é 日本語</i>");
        assert_eq!(render("**bold *and italic* end**"), "<b>bold </b><bi>and italic</bi><b> end</b>");
        assert_eq!(render(""), "");
    }
    #[test]
    fn emphasis_never_panics_on_hostile_input() {
        let alphabet = ["*", "**", "\\", " ", "a", "é", "日", "\\*", "***"];
        let mut seed = 5u64;
        for _ in 0..20000 {
            let mut text = String::new();
            for _ in 0..(seed % 14) {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                text.push_str(alphabet[(seed >> 33) as usize % alphabet.len()]);
            }
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let visible: String = pieces(&text).iter().map(|p| p.text.as_str()).collect();
            // Only delimiters and escapes may disappear.
            assert!(visible.chars().all(|c| text.contains(c)), "{text:?} -> {visible:?}");
            let _ = parse_emphasis(&text).per_byte(text.len());
        }
    }

    #[test]
    fn markers_are_concealed_off_the_cursor_line_and_dimmed_on_it() {
        let marker = Marker::default();
        let hidden = present("a *b* **c**\n", 0, None, Initial::default(), options(&marker));
        assert_eq!(hidden.text, "a b c\n");
        assert!(hidden.spans.iter().any(|s| s.style.italic) && hidden.spans.iter().any(|s| s.style.bold));
        let shown = present("a *b* **c**\n", 0, Some(0), Initial::default(), options(&marker));
        assert_eq!(shown.text, "a *b* **c**\n");
        assert!(shown.spans.iter().any(|s| s.style.dim));
        // Source offsets survive concealment, also for multi-byte characters.
        let p = present("é *日本* x", 10, None, Initial::default(), options(&marker));
        assert_eq!(p.text, "é 日本 x");
        let byte = p.text.find('本').unwrap();
        assert_eq!(p.source_at(byte), 10 + "é *日".chars().count());
        assert_eq!(p.byte_at(10 + 3), p.text.find('日').unwrap());
    }

    #[test]
    fn notes_are_greyed_hidden_or_revealed() {
        let marker = Marker::default();
        let p = present("real $$ aside $$ more\n", 0, None, Initial::default(), options(&marker));
        assert_eq!(p.text, "real aside more\n");
        assert!(p.spans.iter().any(|s| s.style.note && p.text[s.bytes.clone()].contains("aside")));
        let mut opts = options(&marker);
        opts.hide_notes = true;
        let hidden = present("real $$ aside $$ more\n", 0, None, Initial::default(), opts);
        assert_eq!(hidden.text, "real more\n");
        assert!(!hidden.hidden);
        let line_note = present("$$ only a note\n", 0, None, Initial::default(), opts);
        assert!(line_note.hidden);
        let editing = present("$$ only a note\n", 0, Some(0), Initial::default(), opts);
        assert!(!editing.hidden && editing.text.contains("only a note"));
        // A block note line keeps its marker visible as a fence.
        let fence = present("$$$\n", 0, None, Initial::default(), options(&marker));
        assert_eq!(fence.text, "$$$\n");
        let inside = present("inside\n", 0, None, Initial { in_block: true, in_line_note: false }, options(&marker));
        assert!(inside.spans.iter().filter(|s| inside.text[s.bytes.clone()].contains("inside")).all(|s| s.style.note));
        let continued = present("rest of a long note", 0, None, Initial { in_block: false, in_line_note: true }, options(&marker));
        assert!(continued.spans.iter().all(|s| s.style.note || continued.text[s.bytes.clone()].trim().is_empty()));
        // Escaped marker: literal text, backslash concealed.
        let literal = present("cost \\$$ five\n", 0, None, Initial::default(), options(&marker));
        assert_eq!(literal.text, "cost $$ five\n");
        assert!(literal.spans.iter().all(|s| !s.style.note));
    }

    #[test]
    fn scene_breaks_render_as_a_symbol() {
        let marker = Marker::default();
        let p = present("#\n", 0, None, Initial::default(), options(&marker));
        assert_eq!(p.text, "* * *\n");
        assert!(p.center);
        assert_eq!(present("#\n", 0, Some(0), Initial::default(), options(&marker)).text, "#\n");
        // Emphasis may not leak across a note.
        let q = present("*one $$ n $$ two*\n", 0, None, Initial::default(), options(&marker));
        assert!(q.spans.iter().filter(|s| s.style.italic).count() >= 2);
    }

    #[test]
    fn incremental_index_matches_a_fresh_scan() {
        let marker = Marker::default();
        let mut rope = Rope::from_str("one\n#\n$$$\nnote\n$$$\ntwo\n***\nthree\n");
        let mut index = BlockIndex::default();
        index.update(&rope, 0, &marker);
        assert!(index.lines[3].before && !index.lines[5].before);
        assert_eq!(index.scenes.iter().copied().collect::<Vec<_>>(), vec![1, 6]);
        for (start, end, value) in [(4, 4, "x\n"), (0, 0, "new\n"), (8, 12, ""), (2, 3, "")] {
            let line = rope.char_to_line(start);
            rope.remove(start..end);
            rope.insert(start, value);
            let to = rope.char_to_line(start + value.chars().count()) + 1;
            index.update_range(&rope, line..to, &marker);
            let mut fresh = BlockIndex::default();
            fresh.update(&rope, 0, &marker);
            assert_eq!(index.lines, fresh.lines, "after editing {start}..{end} {value:?}");
            assert_eq!(index.scenes, fresh.scenes);
        }
        let long = Rope::from_str(&"x".repeat(1024 * 1024));
        index.update(&long, 0, &marker);
        assert_eq!(index.lines.len(), 1);
    }

    #[test]
    fn toolbar_format_at_cursor() {
        let marker = Marker::default();
        assert!(format_at("a *b c* d", 5, &marker).italic);
        assert!(!format_at("a *b c* d", 8, &marker).italic);
        assert!(format_at("**bold**", 4, &marker).bold);
        assert!(format_at("text $$ in note", 12, &marker).note);
        assert_eq!(format_at("plain", 2, &marker), Format::default());
    }
}
