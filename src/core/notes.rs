//! The notes layer.
//!
//! Notes live in the same text as the prose but are a separate layer that is stripped on export.
//! Two forms of the text exist:
//!
//! * **Neutral** text is what is stored in a `.jw` file, in snapshots and handed to exporters.
//!   Notes are marked with single private-use characters (see the constants below), so the
//!   marker the author prefers never reaches the file. A literal `$$` in prose is just `$$`.
//! * **Display** text is what the editor buffer holds. Notes are marked with the author's
//!   configurable marker (`$$` by default) and a literal marker in prose is written `\$$`.
//!
//! Changing the marker converts display text to neutral with the old marker and back with the new
//! one, so every existing note follows.

use std::ops::Range;

/// Line note: runs from here to the end of the line.
pub const LINE: char = '\u{E000}';
/// Inline note opens; closes at [`CLOSE`] on the same line.
pub const OPEN: char = '\u{E001}';
pub const CLOSE: char = '\u{E002}';
/// A line holding only this character opens a block note. The block ends at the next such line
/// holding [`BLOCK_CLOSE`].
pub const BLOCK_OPEN: char = '\u{E003}';
pub const BLOCK_CLOSE: char = '\u{E004}';

pub fn is_token(c: char) -> bool {
    ('\u{E000}'..='\u{E004}').contains(&c)
}

/// The author's note marker, such as `$$`. Block notes use the marker plus its last character
/// again (`$$$`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    text: String,
    block: String,
}
impl Default for Marker {
    fn default() -> Self {
        Self::new("$$").unwrap()
    }
}
impl Marker {
    pub fn new(text: &str) -> Result<Self, String> {
        let count = text.chars().count();
        if !(2..=4).contains(&count) {
            return Err("The note marker must be two to four characters".into());
        }
        if text.chars().any(|c| c.is_alphanumeric() || c.is_whitespace() || c == '\\' || is_token(c) || c.is_control()) {
            return Err("The note marker cannot contain letters, digits, spaces or backslashes".into());
        }
        let mut block = text.to_owned();
        block.push(text.chars().last().unwrap());
        Ok(Self { text: text.to_owned(), block })
    }
    pub fn as_str(&self) -> &str {
        &self.text
    }
    pub fn block(&self) -> &str {
        &self.block
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delim {
    /// Starts a note that runs to the end of the line.
    Line,
    Open,
    Close,
    BlockOpen,
    BlockClose,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Prose,
    /// Text inside a note.
    Note,
    /// A marker.
    Delim(Delim),
    /// The backslash of `\$$`; the marker after it is prose.
    Escape,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seg {
    /// Byte range within the line.
    pub range: Range<usize>,
    pub kind: Kind,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineScan {
    pub segs: Vec<Seg>,
    /// Whether the next line starts inside a block note.
    pub block_after: bool,
}
impl LineScan {
    pub fn has_notes(&self) -> bool {
        self.segs.iter().any(|s| !matches!(s.kind, Kind::Prose | Kind::Escape))
    }
}

fn push(segs: &mut Vec<Seg>, range: Range<usize>, kind: Kind) {
    if range.is_empty() {
        return;
    }
    if let Some(last) = segs.last_mut() {
        if last.kind == kind && last.range.end == range.start && !matches!(kind, Kind::Delim(_) | Kind::Escape) {
            last.range.end = range.end;
            return;
        }
    }
    segs.push(Seg { range, kind });
}

/// A run of backslashes starting at `i` that is directly followed by the marker's first
/// character. Returns the run length. Pairs of backslashes stand for one literal backslash; an odd
/// run also escapes the marker character after it.
fn backslash_run(body: &str, i: usize, m: &str) -> Option<usize> {
    let bytes = body.as_bytes();
    if bytes[i] != b'\\' {
        return None;
    }
    let mut n = 1;
    while bytes.get(i + n) == Some(&b'\\') {
        n += 1;
    }
    let first = m.chars().next()?;
    body[i + n..].starts_with(first).then_some(n)
}
/// Pushes the pieces of a backslash run; returns the position after what was consumed. When the
/// run is even the marker character after it is left for the caller to read.
fn push_run(segs: &mut Vec<Seg>, body: &str, i: usize, n: usize, kind: Kind, m: &str) -> usize {
    for k in 0..n / 2 {
        push(segs, i + 2 * k..i + 2 * k + 1, kind);
        push(segs, i + 2 * k + 1..i + 2 * k + 2, Kind::Escape);
    }
    if n % 2 == 1 {
        let first = m.chars().next().unwrap();
        push(segs, i + n - 1..i + n, Kind::Escape);
        push(segs, i + n..i + n + first.len_utf8(), kind);
        let _ = body;
        i + n + first.len_utf8()
    } else {
        i + n
    }
}
/// Pushes `line[range]` as `kind`, resolving backslash escapes.
fn push_escaped(segs: &mut Vec<Seg>, line: &str, range: Range<usize>, kind: Kind, m: &str) {
    let body = &line[..range.end];
    let mut start = range.start;
    let mut i = range.start;
    while i < range.end {
        if let Some(n) = backslash_run(body, i, m) {
            push(segs, start..i, kind);
            i = push_run(segs, body, i, n, kind, m);
            if n % 2 == 0 {
                // A live marker cannot occur here; its first character is plain text.
                let first = m.chars().next().unwrap();
                push(segs, i..i + first.len_utf8(), kind);
                i += first.len_utf8();
            }
            start = i;
        } else {
            i += line[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    push(segs, start..range.end, kind);
}

/// Splits one display-form line (with or without its line ending) into prose, note text and
/// markers. `in_block` says whether the line starts inside a block note.
pub fn scan_line(line: &str, marker: &Marker, in_block: bool) -> LineScan {
    let body_end = line.trim_end_matches(['\r', '\n']).len();
    let body = &line[..body_end];
    let trimmed = body.trim();
    let lead = body.len() - body.trim_start().len();
    let mut segs = Vec::new();
    if in_block || trimmed == marker.block() {
        if trimmed == marker.block() {
            push(&mut segs, 0..lead, if in_block { Kind::Note } else { Kind::Prose });
            let delim = if in_block { Delim::BlockClose } else { Delim::BlockOpen };
            push(&mut segs, lead..lead + trimmed.len(), Kind::Delim(delim));
            push(&mut segs, lead + trimmed.len()..body_end, if in_block { Kind::Note } else { Kind::Prose });
            push(&mut segs, body_end..line.len(), Kind::Prose);
            return LineScan { segs, block_after: !in_block };
        }
        push_escaped(&mut segs, line, 0..body_end, Kind::Note, marker.as_str());
        push(&mut segs, body_end..line.len(), Kind::Prose);
        return LineScan { segs, block_after: true };
    }
    let m = marker.as_str();
    let mut i = 0;
    let mut start = 0;
    let mut inline = false;
    while i < body.len() {
        if let Some(n) = backslash_run(body, i, m) {
            let kind = if inline { Kind::Note } else { Kind::Prose };
            push(&mut segs, start..i, kind);
            i = push_run(&mut segs, body, i, n, kind, m);
            start = i;
        } else if body[i..].starts_with(m) {
            if inline {
                push(&mut segs, start..i, Kind::Note);
                push(&mut segs, i..i + m.len(), Kind::Delim(Delim::Close));
                inline = false;
                i += m.len();
                start = i;
            } else {
                push(&mut segs, start..i, Kind::Prose);
                if next_marker(body, i + m.len(), m).is_some() {
                    push(&mut segs, i..i + m.len(), Kind::Delim(Delim::Open));
                    inline = true;
                    i += m.len();
                    start = i;
                } else {
                    push(&mut segs, i..i + m.len(), Kind::Delim(Delim::Line));
                    push_escaped(&mut segs, line, i + m.len()..body_end, Kind::Note, m);
                    start = body_end;
                    i = body_end;
                }
            }
        } else {
            i += body[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    push(&mut segs, start..body_end, if inline { Kind::Note } else { Kind::Prose });
    push(&mut segs, body_end..line.len(), Kind::Prose);
    LineScan { segs, block_after: false }
}
/// Next live marker at or after `from`.
fn next_marker(body: &str, from: usize, m: &str) -> Option<usize> {
    let mut i = from;
    while i < body.len() {
        if let Some(n) = backslash_run(body, i, m) {
            i += n;
            if n % 2 == 1 {
                i += m.chars().next().map_or(0, char::len_utf8);
            }
        } else if body[i..].starts_with(m) {
            return Some(i);
        } else {
            i += body[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    None
}

/// Cheap check used by the incremental block index: does this line open or close a block note?
pub fn is_block_line(prefix: &str, marker: &Marker) -> bool {
    prefix.trim_end_matches(['\r', '\n']).trim() == marker.block()
}

/// Character-offset conversion between the two forms of the same text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Remap {
    /// (source start, source length, destination start, destination length), in source order, for
    /// every span that changed length.
    spans: Vec<(usize, usize, usize, usize)>,
}
impl Remap {
    pub fn is_identity(&self) -> bool {
        self.spans.is_empty()
    }
    /// A position inside a replaced span lands at the start of its replacement.
    pub fn map(&self, pos: usize) -> usize {
        let index = self.spans.partition_point(|s| s.0 + s.1 <= pos);
        if let Some(&(src, _, dst, _)) = self.spans.get(index) {
            if pos >= src {
                return dst;
            }
        }
        match index.checked_sub(1).map(|i| self.spans[i]) {
            Some((src, len, dst, dlen)) => dst + dlen + (pos - (src + len)),
            None => pos,
        }
    }
    /// The conversion in the other direction.
    pub fn inverse(&self) -> Remap {
        Remap { spans: self.spans.iter().map(|&(src, len, dst, dlen)| (dst, dlen, src, len)).collect() }
    }
    pub fn map_range(&self, range: Range<usize>) -> Range<usize> {
        self.map(range.start)..self.map(range.end)
    }
}

/// Display text (the author's marker) to neutral text.
pub fn to_neutral(display: &str, marker: &Marker) -> (String, Remap) {
    let mut out = String::with_capacity(display.len());
    let mut remap = Remap::default();
    let mut in_block = false;
    let (mut src_chars, mut dst_chars) = (0usize, 0usize);
    for line in display.split_inclusive('\n') {
        let scan = scan_line(line, marker, in_block);
        for seg in &scan.segs {
            let text = &line[seg.range.clone()];
            let count = text.chars().count();
            let replacement = match seg.kind {
                Kind::Prose | Kind::Note => None,
                Kind::Escape => Some(""),
                Kind::Delim(Delim::Line) => Some("\u{E000}"),
                Kind::Delim(Delim::Open) => Some("\u{E001}"),
                Kind::Delim(Delim::Close) => Some("\u{E002}"),
                Kind::Delim(Delim::BlockOpen) => Some("\u{E003}"),
                Kind::Delim(Delim::BlockClose) => Some("\u{E004}"),
            };
            match replacement {
                None => {
                    out.push_str(text);
                    dst_chars += count;
                }
                Some(r) => {
                    let dlen = r.chars().count();
                    if count != dlen {
                        remap.spans.push((src_chars, count, dst_chars, dlen));
                    }
                    out.push_str(r);
                    dst_chars += dlen;
                }
            }
            src_chars += count;
        }
        in_block = scan.block_after;
    }
    (out, remap)
}

/// Neutral text to display text with the author's marker.
///
/// A literal character that would merge with a marker (the start of a literal marker, or a
/// marker character touching a note marker) is written with a backslash, and backslashes in
/// front of a marker are doubled.
pub fn to_display(neutral: &str, marker: &Marker) -> (String, Remap) {
    let m = marker.as_str();
    let first = m.chars().next().unwrap();
    let mut out = String::with_capacity(neutral.len() + 16);
    let mut remap = Remap::default();
    let (mut src_chars, mut dst_chars) = (0usize, 0usize);
    let mut previous: Option<char> = None;
    let mut iter = neutral.char_indices().peekable();
    while let Some((at, c)) = iter.next() {
        let next = iter.peek().map(|&(_, n)| n);
        let after_run = {
            // The character after the backslash run that `c` belongs to.
            let mut rest = neutral[at..].chars().skip_while(|r| *r == '\\');
            rest.next()
        };
        if is_token(c) {
            let r = match c {
                BLOCK_OPEN | BLOCK_CLOSE => marker.block(),
                _ => m,
            };
            let dlen = r.chars().count();
            if dlen != 1 {
                remap.spans.push((src_chars, 1, dst_chars, dlen));
            }
            out.push_str(r);
            src_chars += 1;
            dst_chars += dlen;
        } else if c == '\\' && after_run.is_some_and(|r| r == first || is_token(r)) {
            remap.spans.push((src_chars, 1, dst_chars, 2));
            out.push_str("\\\\");
            src_chars += 1;
            dst_chars += 2;
        } else if c == first
            && (neutral[at..].starts_with(m)
                || previous.is_some_and(|p| p == '\\' || is_token(p))
                || next.is_some_and(is_token))
        {
            remap.spans.push((src_chars, 1, dst_chars, 2));
            out.push('\\');
            out.push(c);
            src_chars += 1;
            dst_chars += 2;
        } else {
            out.push(c);
            src_chars += 1;
            dst_chars += 1;
        }
        previous = Some(c);
    }
    (out, remap)
}

/// Re-expresses display text written with `old` using `new`.
pub fn change_marker(display: &str, old: &Marker, new: &Marker) -> String {
    to_display(&to_neutral(display, old).0, new).0
}

pub fn has_notes(neutral: &str) -> bool {
    neutral.chars().any(|c| matches!(c, LINE | OPEN | BLOCK_OPEN))
}

/// Neutral text with every note removed and the whitespace they leave behind cleaned up: a line
/// that held only a note disappears, trailing spaces before a line note go, and two spaces
/// around an inline note become one. Runs of blank lines left behind collapse to one.
pub fn strip(neutral: &str) -> String {
    if !has_notes(neutral) {
        return neutral.to_owned();
    }
    let mut out = String::with_capacity(neutral.len());
    let mut in_block = false;
    let mut blank_run = 0;
    for line in neutral.split_inclusive('\n') {
        let ending = &line[line.trim_end_matches(['\r', '\n']).len()..];
        let body = line.trim_end_matches(['\r', '\n']);
        let (kept, noted, still_in_block) = strip_line(body, in_block);
        in_block = still_in_block;
        if noted && kept.trim().is_empty() {
            continue;
        }
        if kept.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(&kept);
        out.push_str(ending);
    }
    out
}
/// (text without notes, whether the line had any, whether a block note continues)
fn strip_line(body: &str, mut in_block: bool) -> (String, bool, bool) {
    let mut kept = String::with_capacity(body.len());
    let mut noted = in_block;
    let mut inline = false;
    // After an inline note that follows a space, the next spaces would double up.
    let mut skip_space = false;
    for c in body.chars() {
        match c {
            BLOCK_OPEN => {
                in_block = true;
                noted = true;
            }
            BLOCK_CLOSE => {
                in_block = false;
                noted = true;
            }
            _ if in_block => {}
            LINE => {
                while kept.ends_with([' ', '\t']) {
                    kept.pop();
                }
                return (kept, true, in_block);
            }
            OPEN => {
                noted = true;
                inline = true;
            }
            CLOSE => {
                inline = false;
                skip_space = kept.is_empty() || kept.ends_with([' ', '\t']);
            }
            _ if inline => {}
            ' ' | '\t' if skip_space => {}
            _ => {
                skip_space = false;
                kept.push(c);
            }
        }
    }
    if noted {
        while kept.ends_with([' ', '\t']) {
            kept.pop();
        }
    }
    (kept, noted, in_block)
}

/// Words outside notes: whitespace-separated runs containing a letter or digit. Scene-break lines,
/// epigraphs and bare punctuation do not count.
pub fn word_count(neutral: &str) -> usize {
    let mut count = 0;
    let mut in_word = false;
    let mut alnum = false;
    let mut inline = false;
    let mut line_note = false;
    let mut block = false;
    // Only blanks so far on this line, which starts at byte `line_at`.
    let mut lead = true;
    let mut line_at = 0;
    // Only set-off blocks and blank lines since the start of the chapter or a scene's name: a
    // block that opens here is an epigraph. `mark` is the count to go back to when it closes.
    // ponytail: only hash breaks end an open block here; the export also ends it at `***`.
    let mut head = true;
    let mut content = false;
    let mut mark: Option<usize> = None;
    let mut last = ' ';
    for (i, c) in neutral.char_indices() {
        let at_lead = lead;
        lead = c == '\n' || (lead && c.is_whitespace());
        if c == '\n' {
            line_at = i + 1;
        }
        match c {
            BLOCK_OPEN => block = true,
            BLOCK_CLOSE => block = false,
            _ if block => {}
            // A scene's name (`## The storm ##`) and the `###newscene###` break are not prose.
            '#' if at_lead && {
                let line = neutral[line_at..].lines().next().unwrap_or("");
                super::prose::scene_name(line).is_some() || super::prose::is_hash_break(line)
            } =>
            {
                line_note = true;
                mark = None;
                head = super::prose::scene_name(neutral[line_at..].lines().next().unwrap_or("")).is_some();
            }
            '>' if i == line_at => {
                // An earlier block that was never closed is prose.
                head &= mark.take().is_none();
                mark = head.then_some(count);
                in_word = true;
            }
            '\n' => {
                line_note = false;
                inline = false;
                if in_word && alnum {
                    count += 1;
                }
                in_word = false;
                alnum = false;
                match mark {
                    Some(before) if last == '<' => (count, mark) = (before, None),
                    None => head &= !content,
                    _ => {}
                }
                (content, last) = (false, ' ');
            }
            _ if line_note => {}
            LINE => {
                line_note = true;
                if in_word && alnum {
                    count += 1;
                }
                in_word = false;
                alnum = false;
            }
            OPEN => {
                inline = true;
                if in_word && alnum {
                    count += 1;
                }
                in_word = false;
                alnum = false;
            }
            CLOSE => inline = false,
            _ if inline => {}
            c if c.is_whitespace() => {
                if in_word && alnum {
                    count += 1;
                }
                in_word = false;
                alnum = false;
            }
            c => {
                in_word = true;
                (content, last) = (true, c);
                alnum |= c.is_alphanumeric();
            }
        }
    }
    if in_word && alnum {
        count += 1;
    }
    match mark {
        Some(before) if last == '<' => before,
        _ => count,
    }
}

/// Result of [`toggle`]: replace `range` (display char offsets) with `text`, then select `select`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toggle {
    pub range: Range<usize>,
    pub text: String,
    pub select: Range<usize>,
}

/// Turns the selection or the cursor's line into a note, or turns it back into prose.
///
/// * A selection inside one line becomes an inline note (or loses its markers).
/// * A selection across lines is wrapped in a block note, or unwrapped if it already is one.
/// * With no selection, the whole line gets (or loses) a line-note marker.
///
/// `text` is the full display text, `sel` the selection in chars.
pub fn toggle(text: &ropey::Rope, sel: Range<usize>, marker: &Marker) -> Option<Toggle> {
    let m = marker.as_str();
    let mlen = m.chars().count();
    let len = text.len_chars();
    let sel = sel.start.min(len)..sel.end.min(len);
    let line_start = |l: usize| text.line_to_char(l);
    let line_end = |l: usize| {
        let mut end = if l + 1 < text.len_lines() { text.line_to_char(l + 1) } else { len };
        while end > line_start(l) && matches!(text.char(end - 1), '\n' | '\r') {
            end -= 1;
        }
        end
    };
    let eol = if text.slice(..len.min(4096)).chars().any(|c| c == '\r') { "\r\n" } else { "\n" };
    let cursor_after = |range: &Range<usize>, inserted: usize, cursor: usize| {
        if cursor >= range.end {
            cursor - range.len() + inserted
        } else if cursor > range.start {
            range.start + inserted
        } else {
            cursor
        }
    };
    if sel.is_empty() {
        let l = text.char_to_line(sel.start);
        let (a, b) = (line_start(l), line_end(l));
        let line: String = text.slice(a..b).into();
        if line.trim() == marker.block() {
            return None;
        }
        let scan = scan_line(&line, marker, false);
        let chars_before = |byte: usize| line[..byte].chars().count();
        // Already a line note: drop the marker and one following space.
        if let Some(seg) = scan.segs.iter().find(|s| s.kind == Kind::Delim(Delim::Line)) {
            let from = a + chars_before(seg.range.start);
            let mut to = from + mlen;
            if to < b && text.char(to) == ' ' {
                to += 1;
            }
            let range = from..to;
            let at = cursor_after(&range, 0, sel.start);
            return Some(Toggle { range, text: String::new(), select: at..at });
        }
        // A line holding a single inline note: unwrap it.
        let delims: Vec<&Seg> = scan.segs.iter().filter(|s| matches!(s.kind, Kind::Delim(_))).collect();
        let only_note = delims.len() == 2
            && delims[0].kind == Kind::Delim(Delim::Open)
            && scan.segs.iter().filter(|s| s.kind == Kind::Prose).all(|s| line[s.range.clone()].trim().is_empty());
        if only_note {
            let lead = &line[..line.len() - line.trim_start().len()];
            let inner = line[delims[0].range.end..delims[1].range.start].trim();
            let out = format!("{lead}{inner}");
            let end = a + out.chars().count();
            return Some(Toggle { range: a..b, text: out, select: end..end });
        }
        let lead = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        let at = a + lead;
        let range = at..at;
        let cursor = sel.start.max(at) + mlen + 1;
        return Some(Toggle { range, text: format!("{m} "), select: cursor..cursor });
    }
    let first = text.char_to_line(sel.start);
    let ends_on_break = sel.end > sel.start && matches!(text.char(sel.end - 1), '\n');
    let last = text.char_to_line(if ends_on_break { sel.end - 1 } else { sel.end });
    if first == last {
        let selected: String = text.slice(sel.clone()).into();
        let scan = scan_line(&selected, marker, false);
        let kinds: Vec<Kind> = scan
            .segs
            .iter()
            .filter(|s| s.kind != Kind::Prose || !selected[s.range.clone()].trim().is_empty())
            .map(|s| s.kind)
            .collect();
        if kinds == [Kind::Delim(Delim::Open), Kind::Note, Kind::Delim(Delim::Close)] {
            let t = selected.trim();
            let inner = t[m.len()..t.len() - m.len()].trim().to_owned();
            let lead = &selected[..selected.len() - selected.trim_start().len()];
            let trail = &selected[selected.trim_end().len()..];
            let n = inner.chars().count();
            let start = sel.start + lead.chars().count();
            return Some(Toggle { range: sel, text: format!("{lead}{inner}{trail}"), select: start..start + n });
        }
        let core = selected.trim();
        if core.is_empty() {
            return None;
        }
        let lead = &selected[..selected.len() - selected.trim_start().len()];
        let trail = &selected[selected.trim_end().len()..];
        let note = format!("{m} {core} {m}");
        let start = sel.start + lead.chars().count();
        let n = note.chars().count();
        return Some(Toggle { range: sel, text: format!("{lead}{note}{trail}"), select: start..start + n });
    }
    let (a, b) = (line_start(first), line_end(last));
    let block = marker.block();
    let first_line: String = text.slice(a..line_end(first)).into();
    let last_line: String = text.slice(line_start(last)..b).into();
    if first_line.trim() == block && last_line.trim() == block {
        let inner: String = text.slice(line_start(first + 1)..line_start(last)).into();
        let inner = inner.trim_end_matches(['\r', '\n']).to_owned();
        let n = inner.chars().count();
        return Some(Toggle { range: a..b, text: inner, select: a..a + n });
    }
    let body: String = text.slice(a..b).into();
    let out = format!("{block}{eol}{body}{eol}{block}");
    let n = out.chars().count();
    Some(Toggle { range: a..b, text: out, select: a..a + n })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn m() -> Marker {
        Marker::default()
    }

    #[test]
    fn epigraphs_are_not_counted_as_words_but_extracts_are() {
        assert_eq!(word_count("> Dreams are made.\n— W. S. <\n\nOne two.\n> a letter <\n"), 4);
        assert_eq!(word_count("###newscene###\n## Two ##\n> Quoted here. <\nOne.\n###newscene###\n> A sign. <\n"), 3);
        assert_eq!(word_count("> never closed\nOne.\n> a b <\n"), 5, "an open block is prose, and so is what follows it");
        assert_eq!(word_count("> a b <"), 0);
    }

    #[test]
    fn the_count_leaves_out_the_blocks_the_export_sets_as_epigraphs() {
        let pieces = ["> a b", "c d", "e <", "> f g <", "", "###newscene###", "## n ##", ">", "<"];
        let mut seed = 0x9e3779b97f4a7c15u64;
        let mut pick = |n: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize % n
        };
        for round in 0..500 {
            let lines: Vec<&str> = (0..10).map(|_| pieces[pick(pieces.len())]).collect();
            let export = super::super::prose::quote_lines(&lines);
            let expected: usize = lines
                .iter()
                .zip(&export)
                .filter(|(line, quote)| !line.starts_with('#') && !quote.is_some_and(|q| q.epigraph))
                .map(|(line, _)| line.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count())
                .sum();
            let ending = if round % 2 == 0 { "\n" } else { "" };
            assert_eq!(word_count(&(lines.join("\n") + ending)), expected, "round {round}: {lines:?}");
        }
    }

    #[test]
    fn scene_name_lines_are_not_counted_as_words() {
        assert_eq!(word_count("one two\n## The storm ##\nthree"), 3);
        assert_eq!(word_count("one\n# not a name\n"), 4);
        assert_eq!(word_count("a # b"), 2);
        assert_eq!(word_count("one\n  ## The storm ##\r\n## open heading\n"), 3);
        assert_eq!(word_count("one\n###newscene###\ntwo"), 2);
    }
    #[test]
    fn markers_are_validated() {
        assert!(Marker::new("$$").is_ok() && Marker::new("%%").is_ok() && Marker::new("//").is_ok());
        assert!(Marker::new("$").is_err() && Marker::new("ab").is_err() && Marker::new("$ $").is_err());
        assert!(Marker::new("\\\\").is_err() && Marker::new("$$$$$").is_err());
        assert_eq!(m().block(), "$$$");
        assert_eq!(Marker::new("//").unwrap().block(), "///");
    }

    #[test]
    fn scans_line_inline_and_block_notes() {
        let kinds = |line: &str, block| -> Vec<(String, Kind)> {
            scan_line(line, &m(), block).segs.iter().map(|s| (line[s.range.clone()].to_owned(), s.kind)).collect()
        };
        assert_eq!(
            kinds("Text $$ remember this\n", false),
            vec![
                ("Text ".into(), Kind::Prose),
                ("$$".into(), Kind::Delim(Delim::Line)),
                (" remember this".into(), Kind::Note),
                ("\n".into(), Kind::Prose)
            ]
        );
        assert_eq!(
            kinds("real $$ note $$ more", false),
            vec![
                ("real ".into(), Kind::Prose),
                ("$$".into(), Kind::Delim(Delim::Open)),
                (" note ".into(), Kind::Note),
                ("$$".into(), Kind::Delim(Delim::Close)),
                (" more".into(), Kind::Prose)
            ]
        );
        let escaped = kinds(r"costs \$$ five", false);
        assert!(escaped.iter().all(|(_, k)| matches!(k, Kind::Prose | Kind::Escape)));
        assert!(scan_line("$$$", &m(), false).block_after);
        assert!(!scan_line("$$$", &m(), true).block_after);
        assert!(scan_line("inside", &m(), true).block_after);
        assert!(scan_line("inside", &m(), true).has_notes());
        // A block marker with other text on the line is a line note, not a block.
        assert!(!scan_line("$$$ text", &m(), false).block_after);
    }

    #[test]
    fn neutral_form_is_independent_of_the_marker() {
        let display = "Hello $$ todo\nreal $$ aside $$ more\n$$$\nblock\n$$$\nafter \\$$ literal\n";
        let (neutral, remap) = to_neutral(display, &m());
        assert_eq!(
            neutral,
            "Hello \u{E000} todo\nreal \u{E001} aside \u{E002} more\n\u{E003}\nblock\n\u{E004}\nafter $$ literal\n"
        );
        assert!(!remap.is_identity());
        let other = Marker::new("%%").unwrap();
        let moved = to_display(&neutral, &other).0;
        assert_eq!(moved, "Hello %% todo\nreal %% aside %% more\n%%%\nblock\n%%%\nafter $$ literal\n");
        assert_eq!(change_marker(display, &m(), &other), moved);
        assert_eq!(to_display(&neutral, &m()).0, display);
        // A literal marker in prose is escaped on the way back out.
        assert_eq!(to_display("price $$ five", &m()).0, "price \\$$ five");
    }

    #[test]
    fn offsets_map_between_forms() {
        let display = "ab $$ cd $$ ef \\$$ gh";
        let (neutral, remap) = to_neutral(display, &m());
        // "ab " is unchanged; the first marker collapses to one char.
        assert_eq!(remap.map(0), 0);
        assert_eq!(remap.map(3), 3);
        assert_eq!(remap.map(5), 4); // after the marker
        let ef = display.chars().position(|c| c == 'e').unwrap();
        let mapped = remap.map(ef);
        assert_eq!(neutral.chars().nth(mapped), Some('e'));
        let gh = display.chars().position(|c| c == 'g').unwrap();
        assert_eq!(neutral.chars().nth(remap.map(gh)), Some('g'));
        assert_eq!(remap.map(display.chars().count()), neutral.chars().count());
        let (_, back) = to_display(&neutral, &m());
        let ne = neutral.chars().position(|c| c == 'e').unwrap();
        assert_eq!(display.chars().nth(back.map(ne)), Some('e'));
    }

    #[test]
    fn inverse_maps_back_positions_outside_markers() {
        let display = "ab $$ cd $$ ef \\$$ gh";
        let (neutral, forward) = to_neutral(display, &m());
        let back = forward.inverse();
        for (i, c) in display.chars().enumerate() {
            if c.is_alphabetic() {
                let there = forward.map(i);
                assert_eq!(neutral.chars().nth(there), Some(c));
                assert_eq!(back.map(there), i);
            }
        }
        assert_eq!(back.map(neutral.chars().count()), display.chars().count());
    }

    #[test]
    fn round_trips_hostile_text() {
        let marker = m();
        let alphabet = ["$", "$$", "\\", "\\$$", "a", " ", "\n", "$$$", "é", "\r\n", "$ "];
        let mut seed = 99u64;
        for _ in 0..4000 {
            let mut text = String::new();
            for _ in 0..(seed % 12) {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                text.push_str(alphabet[(seed >> 33) as usize % alphabet.len()]);
            }
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            // display -> neutral -> display is stable once normalised.
            let (neutral, _) = to_neutral(&text, &marker);
            let (display, _) = to_display(&neutral, &marker);
            let (again, _) = to_neutral(&display, &marker);
            assert_eq!(neutral, again, "text {text:?}");
        }
    }

    #[test]
    fn well_formed_neutral_text_round_trips_for_any_marker() {
        let literal = ["$", "$$", "\\", "a", " ", "é", "%", "%%", "/", "//", "x"];
        let mut seed = 7u64;
        let mut next = move |n: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize % n
        };
        for marker in ["$$", "%%", "//", "$%"] {
            let marker = Marker::new(marker).unwrap();
            for _ in 0..3000 {
                let mut text = String::new();
                for _ in 0..next(8) {
                    let piece = |n: &mut dyn FnMut(usize) -> usize, len: usize| -> String {
                        (0..len).map(|_| literal[n(literal.len())]).collect()
                    };
                    match next(6) {
                        0 => text.push_str(&format!("{}{LINE}{}\n", piece(&mut next, 3), piece(&mut next, 3))),
                        1 => text.push_str(&format!("{}{OPEN}{}{CLOSE}{}", piece(&mut next, 2), piece(&mut next, 3), piece(&mut next, 2))),
                        2 => text.push_str(&format!("\n{BLOCK_OPEN}\n{}\n{BLOCK_CLOSE}\n", piece(&mut next, 4))),
                        _ => text.push_str(&piece(&mut next, 4)),
                    }
                    if next(3) == 0 {
                        text.push('\n');
                    }
                }
                let (display, _) = to_display(&text, &marker);
                let (back, _) = to_neutral(&display, &marker);
                assert_eq!(back, text, "marker {:?} display {display:?}", marker.as_str());
            }
        }
    }

    #[test]
    fn strip_removes_notes_and_cleans_whitespace() {
        let marker = m();
        let n = |s: &str| to_neutral(s, &marker).0;
        assert_eq!(strip(&n("Hello world. $$ trim this\nNext line\n")), "Hello world.\nNext line\n");
        assert_eq!(strip(&n("real text $$ note $$ more real text\n")), "real text more real text\n");
        assert_eq!(strip(&n("start $$ note $$\n")), "start\n");
        assert_eq!(strip(&n("$$ only a note\nkept\n")), "kept\n");
        assert_eq!(strip(&n("a\n$$$\nblock\nof notes\n$$$\nb\n")), "a\nb\n");
        assert_eq!(strip(&n("a\n\n$$ gone\n\nb\n")), "a\n\nb\n");
        assert_eq!(strip(&n("no notes here\r\n")), "no notes here\r\n");
        assert_eq!(strip(&n("Word$$ glued $$word\n")), "Wordword\n");
    }

    #[test]
    fn word_count_skips_notes_and_symbols() {
        let marker = m();
        let n = |s: &str| to_neutral(s, &marker).0;
        assert_eq!(word_count(&n("one two three\n")), 3);
        assert_eq!(word_count(&n("one $$ two three\nfour\n")), 2);
        assert_eq!(word_count(&n("one $$ two $$ three")), 2);
        assert_eq!(word_count(&n("a\n$$$\nlots of hidden words\n$$$\nb")), 2);
        assert_eq!(word_count(&n("— Hola, dijo.\n#\n***\n—Hola\n")), 3);
        assert_eq!(word_count(""), 0);
        assert_eq!(word_count("café—noir 3.5 x"), 3);
    }

    #[test]
    fn toggle_line_selection_and_paragraph() {
        use ropey::Rope;
        let marker = m();
        let apply = |text: &str, sel: Range<usize>| -> (String, Range<usize>) {
            let rope = Rope::from_str(text);
            let t = toggle(&rope, sel, &marker).unwrap();
            let mut s: Vec<char> = text.chars().collect();
            s.splice(t.range.clone(), t.text.chars());
            (s.into_iter().collect(), t.select)
        };
        // Line note on and off.
        let (on, sel) = apply("first line\nsecond", 3..3);
        assert_eq!(on, "$$ first line\nsecond");
        assert_eq!(sel, 6..6);
        let (off, _) = apply(&on, 8..8);
        assert_eq!(off, "first line\nsecond");
        // Inline selection on and off.
        let (on, sel) = apply("say hello there", 4..10);
        assert_eq!(on, "say $$ hello $$ there");
        assert_eq!(&on[sel.start..sel.end], "$$ hello $$");
        let (off, _) = apply(&on, sel);
        assert_eq!(off, "say hello there");
        // Multi-line selection becomes a block note and back.
        let (on, sel) = apply("a\nb\nc\nd", 2..5);
        assert_eq!(on, "a\n$$$\nb\nc\n$$$\nd");
        let (off, _) = apply(&on, sel);
        assert_eq!(off, "a\nb\nc\nd");
        // Indented line keeps its indentation before the marker.
        assert_eq!(apply("  indented", 4..4).0, "  $$ indented");
    }
}
