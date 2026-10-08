use crate::{
    core::{
        buffer::Document,
        config::Settings,
        notes::{self, Delim, Kind as NoteKind, Marker},
        prose::{self, BlockIndex, Initial, Presentation, Style},
    },
    render::{self, Rect, Renderer},
};
use cosmic_text::{
    Align, Attrs, Buffer, Cursor, Family, FontSystem, Metrics, Motion, Shaping, Weight, Wrap,
};
use std::collections::VecDeque;

const CHUNK: usize = 1024;
/// A number that depends only on the six characters ending at `at`.
fn fingerprint(text: &ropey::Rope, at: usize) -> u32 {
    text.slice(at.saturating_sub(5)..=at).chars().fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32))
}
pub struct Tile {
    pub start: usize,
    pub y: f32,
    pub height: f32,
    pub preview: Presentation,
    pub buffer: Buffer,
    source: String,
    raw: bool,
    initial: Initial,
    hide: bool,
    width: f32,
    size: f32,
    line_bytes: Vec<usize>,
}
impl Tile {
    fn display_cursor(&self, source: usize) -> Cursor {
        let byte = self.preview.byte_at(source.saturating_sub(self.start));
        let line = self
            .line_bytes
            .partition_point(|b| *b <= byte)
            .saturating_sub(1);
        Cursor::new(line, byte - self.line_bytes[line])
    }
    fn source_cursor(&self, cursor: Cursor) -> usize {
        self.start
            + self
                .preview
                .source_at(self.line_bytes.get(cursor.line).copied().unwrap_or(0) + cursor.index)
    }
    pub fn caret(&self, source: usize) -> Option<(f32, f32)> {
        self.buffer
            .cursor_position(&self.display_cursor(source))
            .map(|(x, y)| (x, self.y + y))
    }
}

/// A range of the text drawn with a tint behind it: a review comment, say.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub start: usize,
    pub end: usize,
    pub orphaned: bool,
    pub active: bool,
}

#[derive(Default)]
pub struct EditorView {
    pub top: usize,
    pub tiles: Vec<Tile>,
    pub rect: Option<Rect>,
    cache: VecDeque<Tile>,
    desired_x: Option<i32>,
    palette: render::theme::Palette,
    history: Vec<usize>,
    /// Sorted char ranges to tint, e.g. every match of the find bar.
    pub highlights: Vec<(usize, usize)>,
    /// Review comments, sorted by start.
    pub marks: Vec<Mark>,
    /// Ranges of misspelled words, sorted.
    pub misspelled: Vec<(usize, usize)>,
    pub marker: Marker,
    /// Shown for scene-break lines.
    pub scene_symbol: String,
    /// Scroll so the caret shows at the next layout.
    pub reveal: bool,
}
fn blend(a: u32, b: u32, t: f32) -> u32 {
    let channel = |s: u32| {
        ((((a >> s) & 255) as f32) * (1. - t) + (((b >> s) & 255) as f32) * t).round() as u32
    };
    channel(16) << 16 | channel(8) << 8 | channel(0)
}
fn attrs<'a>(style: Style, size: f32, settings: &'a Settings) -> Attrs<'a> {
    let palette = render::theme::Palette::new(settings.light(), settings.accent_color());
    let color = render::theme::color(if style.note {
        let base = settings.note_colour().unwrap_or(palette.muted);
        if style.dim { blend(base, palette.background, 0.4) } else { base }
    } else if style.dim || style.scene {
        palette.muted
    } else {
        palette.text
    });
    let italic = style.italic || (style.note && settings.note_italic);
    Attrs::new()
        .family(Family::Name(&settings.editor_font))
        .weight(if style.bold { Weight::BOLD } else { Weight::NORMAL })
        .style(if italic { cosmic_text::Style::Italic } else { cosmic_text::Style::Normal })
        .color(color)
        .metadata(usize::from(style.misspelled))
        .metrics(Metrics::new(size, size * settings.line_height))
}
fn shape(preview: &Presentation, wrap: Wrap, width: f32, settings: &Settings, fonts: &mut FontSystem) -> Buffer {
    let mut buffer = Buffer::new_empty(Metrics::new(settings.font_size, settings.font_size * settings.line_height));
    buffer.set_size(Some(width), None);
    buffer.set_wrap(wrap);
    buffer.set_rich_text(
        preview.spans.iter().map(|span| {
            (
                &preview.text[span.bytes.clone()],
                attrs(span.style, settings.font_size, settings),
            )
        }),
        &attrs(Style::default(), settings.font_size, settings),
        Shaping::Advanced,
        None,
    );
    if preview.center {
        for line in &mut buffer.lines {
            line.set_align(Some(Align::Center));
        }
    }
    buffer.shape_until_scroll(fonts, false);
    buffer
}
impl EditorView {
    pub fn configure(&mut self, settings: &Settings, scene_symbol: &str) {
        let marker = settings.marker();
        if marker != self.marker || scene_symbol != self.scene_symbol {
            self.marker = marker;
            self.scene_symbol = scene_symbol.to_owned();
            self.clear_cache();
        }
    }
    pub fn clear_cache(&mut self) {
        self.tiles.clear();
        self.cache.clear();
    }
    pub fn layout(
        &mut self,
        doc: &Document,
        blocks: &BlockIndex,
        settings: &Settings,
        fonts: &mut FontSystem,
        rect: Rect,
    ) {
        self.layout_pass(doc, blocks, settings, fonts, rect);
        if !std::mem::take(&mut self.reveal) {
            return;
        }
        let line_height = settings.font_size * settings.line_height;
        let gap = settings.font_size * settings.paragraph_gap;
        let visible = |view: &Self| {
            let found = view.caret_at(doc.cursor);
            match found {
                Some((_, y)) => y >= -1.0 && y + line_height <= rect.height as f32 + 1.0,
                // The empty line after a final newline has no tile of its own.
                None => doc.cursor == doc.text.len_chars() && view.tiles.last().is_some_and(|t| t.start + t.preview.end == doc.cursor && t.y + t.height + line_height <= rect.height as f32 + 1.0),
            }
        };
        for attempt in 0..2 {
            if visible(self) {
                return;
            }
            let line = doc.text.char_to_line(doc.cursor.min(doc.text.len_chars()));
            let line_start = doc.text.line_to_char(line);
            self.top = if doc.cursor - line_start > CHUNK {
                doc.cursor.saturating_sub(160)
            } else if attempt == 0 && doc.cursor >= self.top {
                // Below the window: show some of what comes before the caret.
                let above = ((rect.height as f32 * 0.45) / (line_height + gap)).floor() as usize;
                doc.text.line_to_char(line.saturating_sub(above))
            } else {
                line_start
            };
            self.history.clear();
            self.layout_pass(doc, blocks, settings, fonts, rect);
        }
    }
    fn layout_pass(
        &mut self,
        doc: &Document,
        blocks: &BlockIndex,
        settings: &Settings,
        fonts: &mut FontSystem,
        rect: Rect,
    ) {
        self.palette = render::theme::Palette::new(settings.light(), settings.accent_color());
        self.rect = Some(rect);
        self.cache.extend(self.tiles.drain(..));
        let cursor_line = doc.text.char_to_line(doc.cursor);
        let mut start = self.top.min(doc.text.len_chars());
        let mut y = 0.0;
        let line_height = settings.font_size * settings.line_height;
        let gap = settings.font_size * settings.paragraph_gap;
        while y < rect.height as f32 + settings.font_size * 2.0 {
            let line = doc.text.char_to_line(start);
            let line_start = doc.text.line_to_char(line);
            let line_end = if line + 1 < doc.text.len_lines() {
                doc.text.line_to_char(line + 1)
            } else {
                doc.text.len_chars()
            };
            let mut end = (start + CHUNK).min(line_end);
            // Long lines are cut after a space, so words are not split between chunks. The cut
            // is chosen from the words around it, not from the distance to the chunk's start, so
            // typing moves the cuts only near the caret and the chunks after it stay cached.
            if end < line_end {
                let mut cut = None;
                for (i, c) in doc.text.slice(start..end).chars().enumerate() {
                    if i >= CHUNK / 2 && c == ' ' && fingerprint(&doc.text, start + i) % 24 == 0 {
                        cut = Some(start + i + 1);
                        break;
                    }
                }
                if let Some(at) = cut {
                    end = at;
                } else {
                    let floor = end.saturating_sub(80).max(start + 1);
                    let mut at = end;
                    while at > floor && doc.text.char(at - 1) != ' ' {
                        at -= 1;
                    }
                    if at > floor {
                        end = at;
                    }
                }
            }
            // Don't split a CRLF pair between virtual chunks.
            if end > start
                && end < doc.text.len_chars()
                && doc.text.char(end - 1) == '\r'
                && doc.text.char(end) == '\n'
            {
                end += 1;
            }
            let source = doc.text.slice(start..end).to_string();
            let raw = settings.show_markers || line == cursor_line;
            let initial = Initial {
                in_block: blocks.in_block(line),
                in_line_note: start > line_start && self.line_note_before(doc, line_start, start),
            };
            let width = rect.width as f32;
            let hide = settings.hide_notes;
            let cached = self.cache.iter().position(|tile| {
                tile.source == source
                    && tile.raw == raw
                    && tile.initial == initial
                    && tile.hide == hide
                    && tile.width == width
                    && tile.size == settings.font_size
            });
            let mut tile = if let Some(index) = cached {
                self.cache.remove(index).unwrap()
            } else {
                let options = prose::Options {
                    marker: &self.marker,
                    show_markers: settings.show_markers,
                    hide_notes: hide,
                    scene_symbol: &self.scene_symbol,
                };
                let preview = prose::present(&source, 0, if raw { Some(0) } else { None }, initial, options);
                let buffer = shape(&preview, Wrap::WordOrGlyph, width, settings, fonts);
                let height = buffer
                    .layout_runs()
                    .map(|run| run.line_top + run.line_height)
                    .last()
                    .unwrap_or(line_height);
                let mut line_bytes = vec![0];
                for (b, c) in preview.text.char_indices() {
                    if c == '\n' {
                        line_bytes.push(b + 1);
                    }
                }
                Tile { start, y, height, preview, buffer, source, raw, initial, hide, width, size: settings.font_size, line_bytes }
            };
            tile.start = start;
            tile.y = y;
            // A terminating newline belongs to the next tile, not an extra blank layout row.
            let height = if tile.source.ends_with('\n') && tile.buffer.lines.len() > 1 {
                tile.height - line_height
            } else {
                tile.height
            };
            if tile.preview.hidden {
                // A hidden note takes no room.
                tile.height = 0.;
            } else {
                y += height.max(line_height);
                if tile.source.ends_with('\n') {
                    y += gap;
                }
            }
            self.tiles.push(tile);
            if end == doc.text.len_chars() {
                break;
            }
            start = end;
        }
        while self.cache.len() > 128 {
            self.cache.pop_front();
        }
        if settings.typewriter {
            if let Some((_, y)) = self.caret_at(doc.cursor) {
                let shift = rect.height as f32 * 0.5 - y;
                for tile in &mut self.tiles {
                    tile.y += shift;
                }
            }
        }
    }
    /// Whether the line starting at `line_start` has already opened a note that runs to its end
    /// by `at`; only the first 512 characters of the line are inspected.
    fn line_note_before(&self, doc: &Document, line_start: usize, at: usize) -> bool {
        let prefix: String = doc.text.slice(line_start..at.min(line_start + 512)).chars().collect();
        notes::scan_line(&prefix, &self.marker, false)
            .segs
            .iter()
            .any(|s| matches!(s.kind, NoteKind::Delim(Delim::Line | Delim::Open)))
    }
    pub fn paint(&mut self, doc: &Document, fonts: &mut FontSystem, renderer: &mut impl Renderer) {
        let Some(rect) = self.rect else {
            return;
        };
        renderer.fill(Rect { x: rect.x - 8, width: rect.width + 8, ..rect }, self.palette.background);
        let selection = doc.selection();
        let comment_tint = blend(self.palette.background, 0xe2b93b, if self.palette.light { 0.40 } else { 0.30 });
        let comment_active = blend(self.palette.background, 0xe2b93b, if self.palette.light { 0.65 } else { 0.55 });
        let orphan_tint = blend(self.palette.background, 0x9a8f86, 0.35);
        for tile in &mut self.tiles {
            if tile.preview.hidden {
                continue;
            }
            // Most tiles carry no selection, match, comment or misspelling: skip the per-glyph pass.
            let span = tile.start..tile.start + tile.preview.end + 1;
            let near = |ranges: &[(usize, usize)]| {
                let hi = ranges.partition_point(|r| r.0 < span.end);
                ranges[hi.saturating_sub(8)..hi].iter().any(|r| r.1 > span.start)
            };
            let decorated = (!selection.is_empty() && selection.start < span.end && selection.end > span.start)
                || near(&self.highlights)
                || near(&self.misspelled)
                || {
                    let hi = self.marks.partition_point(|m| m.start < span.end);
                    self.marks[hi.saturating_sub(8)..hi].iter().any(|m| m.end > span.start)
                };
            for run in tile.buffer.layout_runs().filter(|_| decorated) {
                let y = rect.y + (tile.y + run.line_top) as i32;
                if y >= rect.y + rect.height as i32 {
                    break;
                }
                for glyph in run.glyphs {
                    let byte = tile.line_bytes[run.line_i] + glyph.start;
                    let source = tile.start + tile.preview.source_at(byte);
                    let color = if selection.contains(&source) {
                        Some(self.palette.selection)
                    } else if {
                        let at = self.highlights.partition_point(|(start, _)| *start <= source);
                        at > 0 && source < self.highlights[at - 1].1
                    } {
                        Some(blend(self.palette.background, self.palette.accent, 0.38))
                    } else {
                        let at = self.marks.partition_point(|m| m.start <= source);
                        self.marks[at.saturating_sub(8)..at]
                            .iter()
                            .rev()
                            .find(|m| source < m.end && !m.orphaned)
                            .map(|m| if m.active { comment_active } else { comment_tint })
                    };
                    if let Some(color) = color {
                        renderer.fill(
                            Rect {
                                x: rect.x + glyph.x as i32,
                                y,
                                width: glyph.w.ceil() as u32 + 1,
                                height: run.line_height.ceil() as u32,
                            }
                            .intersect(rect),
                            color,
                        );
                    }
                    if {
                        let at = self.misspelled.partition_point(|(start, _)| *start <= source);
                        // The word being typed is left alone until the caret moves on.
                        at > 0 && source < self.misspelled[at - 1].1 && !(doc.cursor >= self.misspelled[at - 1].0 && doc.cursor <= self.misspelled[at - 1].1)
                    } {
                        // A dotted red underline for a possible spelling mistake.
                        let base = y + (run.line_height * 0.9) as i32;
                        let mut dx = 0;
                        while (dx as f32) < glyph.w {
                            renderer.fill(
                                Rect { x: rect.x + glyph.x as i32 + dx, y: base + (dx / 2 % 2), width: 1, height: 1 }.intersect(rect),
                                0xd0605e,
                            );
                            dx += 2;
                        }
                    }
                }
            }
            let _ = orphan_tint;
            renderer.text(fonts, &mut tile.buffer, rect.x, rect.y + tile.y as i32, rect);
            if doc.cursor >= tile.start && doc.cursor <= tile.start + tile.preview.end {
                let cursor = tile.display_cursor(doc.cursor);
                if let Some((x, y)) = tile.buffer.cursor_position(&cursor) {
                    let y = rect.y + tile.y as i32 + y as i32;
                    if y < rect.y + rect.height as i32 {
                        renderer.fill(
                            Rect { x: rect.x + x as i32, y, width: 2, height: tile.size.ceil() as u32 + 5 }.intersect(rect),
                            self.palette.accent,
                        );
                    }
                }
            }
        }
    }
    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        let rect = self.rect?;
        let local_y = y - rect.y as f32;
        let tile = self
            .tiles
            .iter()
            .find(|tile| local_y < tile.y + tile.height)
            .or_else(|| self.tiles.last())?;
        tile.buffer
            .hit(x - rect.x as f32, local_y - tile.y)
            .map(|cursor| tile.source_cursor(cursor))
    }
    pub fn motion(
        &mut self,
        doc: &mut Document,
        blocks: &BlockIndex,
        settings: &Settings,
        fonts: &mut FontSystem,
        motion: Motion,
        extend: bool,
    ) {
        let Some(rect) = self.rect else {
            return;
        };
        let down = matches!(motion, Motion::Down | Motion::PageDown);
        for attempt in 0..2 {
            let position = self
                .tiles
                .iter()
                .find(|t| doc.cursor >= t.start && doc.cursor <= t.start + t.preview.end)
                .and_then(|t| t.caret(doc.cursor));
            if let Some((x, y)) = position {
                let step = settings.font_size * settings.line_height;
                let target_y = y + if down { step * 1.5 } else { -step * 0.5 };
                self.desired_x = Some(self.desired_x.unwrap_or(x as i32));
                if target_y >= 0. && target_y < rect.height as f32 {
                    if let Some(source) = self.hit(
                        rect.x as f32 + self.desired_x.unwrap() as f32,
                        rect.y as f32 + target_y,
                    ) {
                        doc.select(source, extend);
                    }
                    return;
                }
            }
            if attempt == 0 {
                self.scroll(doc, if down { 4 } else { -4 });
                self.layout(doc, blocks, settings, fonts, rect);
            }
        }
        doc.select(if down { doc.text.len_chars() } else { 0 }, extend);
    }
    pub fn scroll(&mut self, doc: &Document, rows: i32) {
        if rows > 0 {
            let mut starts = Vec::new();
            for tile in &self.tiles {
                for run in tile.buffer.layout_runs() {
                    let byte =
                        tile.line_bytes[run.line_i] + run.glyphs.first().map_or(0, |g| g.start);
                    starts.push(if run.line_top == 0. {
                        tile.start
                    } else {
                        tile.start + tile.preview.source_at(byte)
                    });
                }
            }
            starts.sort_unstable();
            starts.dedup();
            for start in starts.into_iter().skip(1).take(rows as usize) {
                self.history.push(self.top);
                self.top = start;
            }
            if self.history.len() > 4096 {
                self.history.drain(..2048);
            }
        } else {
            if !self.history.is_empty() {
                for _ in 0..-rows {
                    if let Some(top) = self.history.pop() {
                        self.top = top.min(doc.text.len_chars());
                    }
                }
                return;
            }
            let line = doc.text.char_to_line(self.top);
            let start = doc.text.line_to_char(line);
            if self.top > start {
                self.top = self.top.saturating_sub((-rows as usize) * 70).max(start);
            } else {
                self.top = doc.text.line_to_char(line.saturating_sub(-rows as usize));
            }
        }
    }
    /// The text of every tile on screen with where it starts and whether it opens inside a block
    /// note: what the spell checker needs.
    pub fn spell_chunks(&self) -> Vec<(usize, String, bool)> {
        self.tiles.iter().filter(|t| !t.preview.hidden).map(|t| (t.start, t.source.clone(), t.initial.in_block)).collect()
    }
    /// Where the caret is on screen, relative to the editor's rectangle.
    pub fn caret_at(&self, cursor: usize) -> Option<(f32, f32)> {
        // A caret on a line boundary belongs to two tiles; either may be the one that shows it.
        self.tiles
            .iter()
            .filter(|t| cursor >= t.start && cursor <= t.start + t.preview.end && !t.preview.hidden)
            .filter_map(|t| t.caret(cursor))
            .next()
    }
    /// Asks for the caret to be brought into view the next time the text is laid out.
    pub fn ensure_cursor(&mut self, _doc: &Document) {
        self.reveal = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;
    thread_local! {
        static FONTS: std::cell::RefCell<FontSystem> = std::cell::RefCell::new(FontSystem::new());
    }
    fn view(text: &str, cursor: usize, width: u32, edit: impl FnOnce(&mut Settings)) -> (EditorView, Document) {
        let mut doc = Document::new(Rope::from_str(text));
        doc.select(cursor.min(doc.text.len_chars()), false);
        let mut blocks = BlockIndex::default();
        let marker = Marker::default();
        blocks.update(&doc.text, 0, &marker);
        let mut settings = Settings::default();
        edit(&mut settings);
        let mut view = EditorView::default();
        view.configure(&settings, "* * *");
        let rect = Rect { x: 0, y: 0, width, height: 4000 };
        FONTS.with(|fonts| view.layout(&doc, &blocks, &settings, &mut fonts.borrow_mut(), rect));
        (view, doc)
    }
    fn shown(view: &EditorView) -> String {
        view.tiles.iter().map(|t| t.preview.text.clone()).collect()
    }

    #[test]
    fn markers_hide_off_the_cursor_line_and_notes_follow_the_settings() {
        let text = "A *bold* start\nSecond $$ aside $$ line\n#\n$$ whole note\nEnd\n";
        let (v, _) = view(text, 0, 800, |_| {});
        let s = shown(&v);
        assert!(s.contains("A *bold* start"), "cursor line shows its markers: {s}");
        assert!(s.contains("Second aside line") && s.contains("* * *") && s.contains("whole note"), "{s}");
        let (v, _) = view(text, 0, 800, |s| s.hide_notes = true);
        let s = shown(&v);
        assert!(s.contains("Second line") && !s.contains("whole note"), "{s}");
        assert_eq!(v.tiles.iter().filter(|t| t.height == 0.0).count(), 1, "the note-only line takes no room");
        let (v, _) = view(text, 0, 800, |s| s.show_markers = true);
        assert!(shown(&v).contains("$$ whole note") && shown(&v).contains("#"));
    }

    #[test]
    fn every_cursor_position_lays_out_and_hit_tests() {
        let text = "Ünï *cödé* 日本語 $$ x\n\n$$$\nblock\n$$$\n***\n";
        for cursor in 0..=text.chars().count() {
            let (v, _) = view(text, cursor, 300, |_| {});
            v.hit(10., 10.);
            assert!(!v.tiles.is_empty());
        }
    }

    #[test]
    fn long_paragraphs_are_cut_at_spaces_and_still_cover_the_text() {
        let words = "lorem ipsum dolor sit amet ".repeat(400);
        let (v, doc) = view(&words, 0, 600, |_| {});
        assert!(v.tiles.len() > 1);
        for pair in v.tiles.windows(2) {
            assert_eq!(pair[0].start + pair[0].preview.end, pair[1].start);
        }
        for t in &v.tiles[..v.tiles.len() - 1] {
            assert!(t.source.ends_with(' '), "{:?}", &t.source[t.source.len().saturating_sub(10)..]);
        }
        assert!(doc.text.len_chars() > 10_000);
    }
}
