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
/// Farther than this into a line, its rows are no longer counted from the line's start.
const FAR: usize = 16 * CHUNK;
/// A number that depends only on the six characters ending at `at`.
fn fingerprint(text: &ropey::Rope, at: usize) -> u32 {
    text.slice(at.saturating_sub(5)..=at).chars().fold(0u32, |h, c| h.wrapping_mul(31).wrapping_add(c as u32))
}
pub struct Tile {
    pub start: usize,
    pub y: f32,
    pub height: f32,
    /// How many chars of the text this tile shows; the next tile starts after them.
    pub end: usize,
    pub preview: Presentation,
    pub buffer: Buffer,
    source: String,
    raw: bool,
    caret: Option<usize>,
    initial: Initial,
    hide: bool,
    width: f32,
    size: f32,
    indent: u8,
    justify: bool,
    line_bytes: Vec<usize>,
    /// Distance from `y` to the row the next tile starts on.
    tail: f32,
    /// The chunk of text this tile was made for. `source`, the text it shows, is shorter when
    /// the tile is `cut`.
    chunk: String,
    /// The chunk stops inside a paragraph.
    inside: bool,
    /// The chunk's last, partly filled row was left to the next tile, which starts there: a
    /// paragraph longer than one chunk then breaks its rows as if it had been shaped in one piece.
    cut: bool,
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
    /// After a closing newline the caret sits on the next row. Another tile owns that row, except
    /// at the end of the text, where this is the only place to find it.
    fn tail_caret(&self, source: usize) -> Option<(f32, f32)> {
        (self.source.ends_with('\n') && source == self.start + self.preview.end).then_some((0., self.y + self.tail))
    }
    /// Whether the caret at `cursor` is drawn by this tile.
    fn owns(&self, cursor: usize) -> bool {
        cursor >= self.start && if self.cut { cursor < self.start + self.end } else { cursor <= self.start + self.end }
    }
    /// Where the row of a layout run starts, in bytes of the shown text.
    fn run_byte(&self, run: &cosmic_text::LayoutRun) -> usize {
        self.line_bytes[run.line_i] + run.glyphs.iter().map(|g| g.start).min().unwrap_or(0)
    }
    /// The rows this tile shows: where each starts in the text, and its top inside the tile.
    fn rows(&self) -> impl Iterator<Item = (usize, f32)> + '_ {
        self.buffer
            .layout_runs()
            .filter(|_| !self.preview.hidden)
            .map(|run| (if run.line_top == 0. { self.start } else { self.start + self.preview.source_at(self.run_byte(&run)) }, run.line_top))
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
        let base = settings.note_colour().unwrap_or(palette.accent_text());
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
/// The alignment tag the line starting at `line_start` begins with.
fn line_align(doc: &Document, line_start: usize) -> Option<prose::Align> {
    let head: String = doc.text.slice(line_start..).chars().take(4).collect();
    prose::align_tag(&head).map(|t| t.0)
}
fn shape(preview: &Presentation, wrap: Wrap, width: f32, justify: bool, settings: &Settings, fonts: &mut FontSystem) -> Buffer {
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
    let align = match preview.align {
        Some(prose::Align::Left) => Some(Align::Left),
        Some(prose::Align::Center) => Some(Align::Center),
        Some(prose::Align::Right) => Some(Align::Right),
        Some(prose::Align::Justify) => Some(Align::Justified),
        None if preview.center => Some(Align::Center),
        None if justify => Some(Align::Justified),
        None => None,
    };
    if align.is_some() {
        for line in &mut buffer.lines {
            line.set_align(align);
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
                None => doc.cursor == doc.text.len_chars() && view.tiles.last().is_some_and(|t| t.start + t.end == doc.cursor && t.y + t.height + line_height <= rect.height as f32 + 1.0),
            }
        };
        if visible(self) {
            return;
        }
        // The caret has just left the window, as when typing past the bottom edge: move by the
        // row or two it takes, not by half a screen.
        let step = if doc.cursor >= self.top { 1 } else { -1 };
        let (top, history) = (self.top, self.history.clone());
        for _ in 0..3 {
            self.scroll_by(doc, blocks, settings, fonts, step);
            self.layout_pass(doc, blocks, settings, fonts, rect);
            if visible(self) {
                return;
            }
        }
        (self.top, self.history) = (top, history);
        // A jump: show the caret's row with some of what comes before it.
        let line = doc.text.char_to_line(doc.cursor.min(doc.text.len_chars()));
        let line_start = doc.text.line_to_char(line);
        self.top = if doc.cursor - line_start > FAR {
            doc.cursor.saturating_sub(160)
        } else {
            self.rows_between(doc, blocks, settings, fonts, line_start, doc.cursor + 1).last().copied().unwrap_or(line_start)
        };
        self.history.clear();
        let above = ((rect.height as f32 * 0.45) / (line_height + gap * 0.3)).floor() as i32;
        self.scroll_by(doc, blocks, settings, fonts, -above);
        self.layout_pass(doc, blocks, settings, fonts, rect);
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
        let mut start = self.top.min(doc.text.len_chars());
        let mut y = 0.0;
        let line_height = settings.font_size * settings.line_height;
        let gap = settings.font_size * settings.paragraph_gap;
        while y < rect.height as f32 + settings.font_size * 2.0 {
            let mut tile = self.tile_at(doc, blocks, settings, fonts, start, rect.width as f32);
            tile.y = y;
            if tile.cut {
                y += tile.height;
            } else if !tile.preview.hidden {
                // A terminating newline belongs to the next tile, not an extra blank layout row.
                let height = if tile.source.ends_with('\n') && tile.buffer.lines.len() > 1 {
                    tile.height - line_height
                } else {
                    tile.height
                };
                y += height.max(line_height);
                if tile.source.ends_with('\n') {
                    y += gap;
                }
            }
            tile.tail = y - tile.y;
            start = tile.start + tile.end;
            self.tiles.push(tile);
            if start == doc.text.len_chars() {
                break;
            }
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
    /// The tile for the chunk of text at `start`, taken from the cache or shaped. Its `y` and
    /// `tail` are for the caller to set.
    fn tile_at(&mut self, doc: &Document, blocks: &BlockIndex, settings: &Settings, fonts: &mut FontSystem, start: usize, width: f32) -> Tile {
        let cursor_line = doc.text.char_to_line(doc.cursor);
        let line_height = settings.font_size * settings.line_height;
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
        // chunks with the same words are found in the cache again.
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
        let chunk = doc.text.slice(start..end).to_string();
        // The same words can end a paragraph in one place and not in another.
        let inside = end < line_end;
        // The preview shows the finished text: no markers, even on the line being edited.
        let raw = !settings.preview && (settings.show_markers || line == cursor_line);
        // Off the caret's own emphasis the `*` markers stay hidden, so the caret moves the cache key.
        let caret = (raw && !settings.show_markers).then(|| {
            let at = doc.cursor.wrapping_sub(start);
            if doc.cursor >= start && doc.cursor <= end { at } else { usize::MAX }
        });
        let (indent, justify) = if settings.preview {
            (self.indent_for(doc, line, start == line_start, settings), settings.preview_justify)
        } else {
            (0, false)
        };
        let initial = Initial {
            in_block: blocks.in_block(line),
            in_line_note: start > line_start && self.line_note_before(doc, line_start, start),
            align: if start > line_start { line_align(doc, line_start) } else { None },
        };
        // The preview shows the text as exported, which has no notes, unless the setting keeps them.
        let hide = settings.hide_notes || (settings.preview && !settings.preview_notes);
        let cached = self.cache.iter().position(|tile| {
            tile.chunk == chunk
                && tile.inside == inside
                && tile.raw == raw
                && tile.caret == caret
                && tile.initial == initial
                && tile.hide == hide
                && tile.width == width
                && tile.size == settings.font_size
                && tile.indent == indent
                && tile.justify == justify
        });
        let mut tile = if let Some(index) = cached {
            self.cache.remove(index).unwrap()
        } else {
            let options = prose::Options {
                marker: &self.marker,
                show_markers: settings.show_markers && !settings.preview,
                hide_notes: hide,
                scene_symbol: &self.scene_symbol,
                indent,
                caret,
            };
            let build = |source: String, cut: bool, fonts: &mut FontSystem| {
                let preview = prose::present(&source, 0, if raw { Some(0) } else { None }, initial, options);
                let buffer = shape(&preview, Wrap::WordOrGlyph, width, justify, settings, fonts);
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
                // A hidden note takes no room.
                let height = if preview.hidden { 0. } else { height };
                Tile { start, y: 0., height, end: preview.end, preview, buffer, source, raw, caret, initial, hide, width, size: settings.font_size, indent, justify, line_bytes, tail: 0., chunk: String::new(), inside, cut }
            };
            let mut tile = build(chunk.clone(), false, fonts);
            // Inside a paragraph the last row is only partly filled: shape the chunk again without
            // it, and let the next chunk start with those words.
            let last_row = tile
                .buffer
                .layout_runs()
                .last()
                .filter(|run| inside && !tile.preview.hidden && run.line_top > 0.)
                .map(|run| tile.preview.source_at(tile.run_byte(&run)))
                .filter(|at| *at > 0);
            if let Some(at) = last_row {
                tile = build(chunk.chars().take(at).collect(), true, fonts);
            }
            tile.chunk = chunk;
            tile
        };
        tile.start = start;
        tile
    }
    /// Where the rows of the text in `from..to` start, in order. `from` begins a paragraph (or a
    /// row) and `to` is at most a few chunks after it.
    fn rows_between(&mut self, doc: &Document, blocks: &BlockIndex, settings: &Settings, fonts: &mut FontSystem, from: usize, to: usize) -> Vec<usize> {
        let width = self.rect.map_or(600., |r| r.width as f32);
        let to = to.min(doc.text.len_chars());
        let mut rows = Vec::new();
        let mut start = from;
        while start < to {
            let tile = self.tile_at(doc, blocks, settings, fonts, start, width);
            rows.extend(tile.rows().map(|(at, _)| at).filter(|at| *at < to));
            start = tile.start + tile.end;
            self.cache.push_back(tile);
        }
        rows.dedup();
        rows
    }
    /// The first-line indent for the chunk of `line` starting at `at_line_start`, as the export
    /// would set it: none inside a paragraph's later chunks, and none at a chapter start or after a
    /// scene break when the preset says so.
    fn indent_for(&self, doc: &Document, line: usize, at_line_start: bool, settings: &Settings) -> u8 {
        if !at_line_start || settings.preview_indent == 0 {
            return 0;
        }
        // A scene break is a short line; only its first characters are read.
        let after_break = || {
            let before: String = doc.text.line(line - 1).chars().take(64).collect();
            prose::is_scene_break(before.trim_end())
        };
        let flush = settings.preview_flush && (line == 0 || after_break());
        if flush { 0 } else { settings.preview_indent }
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
            let span = tile.start..tile.start + tile.end + 1;
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
        }
        if let Some((x, y)) = self.caret_at(doc.cursor) {
            let y = rect.y + y as i32;
            let size = self.tiles.first().map_or(18., |t| t.size);
            if y < rect.y + rect.height as i32 {
                renderer.fill(Rect { x: rect.x + x as i32, y, width: 2, height: size.ceil() as u32 + 5 }.intersect(rect), self.palette.accent);
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
                .find(|t| t.owns(doc.cursor))
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
                self.scroll_by(doc, blocks, settings, fonts, if down { 4 } else { -4 });
                self.layout(doc, blocks, settings, fonts, rect);
            }
        }
        doc.select(if down { doc.text.len_chars() } else { 0 }, extend);
    }
    /// Where each row on screen starts in the text, and its top.
    fn screen_rows(&self) -> Vec<(usize, f32)> {
        let mut rows: Vec<(usize, f32)> = self.tiles.iter().flat_map(|tile| tile.rows().map(move |(at, top)| (at, tile.y + top))).collect();
        rows.dedup_by_key(|row| row.0);
        rows
    }
    /// How many rows a Page Up or Page Down moves: a screenful, less two rows kept for context.
    pub fn page(&self, settings: &Settings) -> i32 {
        let (Some(rect), line_height) = (self.rect, settings.font_size * settings.line_height) else { return 1 };
        let whole = self.screen_rows().iter().filter(|(_, top)| top + line_height <= rect.height as f32).count();
        (whole as i32 - 2).max(1)
    }
    /// Scrolls down by rows of text, or back up through the positions scrolled down from.
    pub fn scroll(&mut self, doc: &Document, rows: i32) {
        if rows > 0 {
            for (start, _) in self.screen_rows().into_iter().skip(1).take(rows as usize) {
                self.history.push(self.top);
                self.top = start;
            }
            if self.history.len() > 4096 {
                self.history.drain(..2048);
            }
        } else {
            for _ in 0..-rows {
                if let Some(top) = self.history.pop() {
                    self.top = top.min(doc.text.len_chars());
                }
            }
        }
    }
    /// Scrolls by rows of text in either direction. Going up where no earlier position is
    /// remembered (after a jump), the paragraphs above are laid out to find where their rows start.
    pub fn scroll_by(&mut self, doc: &Document, blocks: &BlockIndex, settings: &Settings, fonts: &mut FontSystem, rows: i32) {
        if rows >= 0 || !self.history.is_empty() {
            return self.scroll(doc, rows);
        }
        let mut left = (-rows) as usize;
        while left > 0 && self.top > 0 {
            let top = self.top.min(doc.text.len_chars());
            let line = doc.text.char_to_line(top);
            let line_start = doc.text.line_to_char(line);
            let from = if top > line_start { line_start } else { doc.text.line_to_char(line - 1) };
            if top - from > FAR {
                // An enormous line: its start is too far away to lay out from, so estimate.
                self.top = top.saturating_sub(left * 70).max(from);
                return;
            }
            let above = self.rows_between(doc, blocks, settings, fonts, from, top);
            let take = left.min(above.len());
            // A paragraph with no rows is a hidden note: step over it.
            self.top = if take == 0 { from } else { above[above.len() - take] };
            left -= take;
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
        let mut owners = self.tiles.iter().filter(|t| t.owns(cursor) && !t.preview.hidden);
        owners.clone().find_map(|t| t.caret(cursor)).or_else(|| owners.find_map(|t| t.tail_caret(cursor)))
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
        let (v, _) = view(text, 5, 800, |_| {});
        let s = shown(&v);
        assert!(s.contains("A *bold* start"), "emphasis holding the caret shows its markers: {s}");
        assert!(shown(&view(text, 0, 800, |_| {}).0).contains("A bold start"), "other emphasis stays hidden");
        assert!(s.contains("Second aside line") && s.contains("* * *") && s.contains("whole note"), "{s}");
        let (v, _) = view(text, 0, 800, |s| s.hide_notes = true);
        let s = shown(&v);
        assert!(s.contains("Second line") && !s.contains("whole note"), "{s}");
        assert_eq!(v.tiles.iter().filter(|t| t.height == 0.0).count(), 1, "the note-only line takes no room");
        // The preview is the text as exported: no notes unless the setting keeps them.
        let (v, _) = view(text, 0, 800, |s| s.preview = true);
        assert!(!shown(&v).contains("whole note") && !shown(&v).contains("aside"), "{}", shown(&v));
        let (v, _) = view(text, 0, 800, |s| {
            s.preview = true;
            s.preview_notes = true;
        });
        assert!(shown(&v).contains("aside") && shown(&v).contains("whole note"), "{}", shown(&v));
        let (v, _) = view(text, 0, 800, |s| s.show_markers = true);
        assert!(shown(&v).contains("$$ whole note") && shown(&v).contains("#"));
    }

    #[test]
    fn preview_shows_the_finished_text_and_indents_like_the_export() {
        let text = "First *one*\nSecond\n***\nAfter break\nThird\n";
        let preview = |s: &mut Settings| {
            s.preview = true;
            s.preview_indent = 4;
        };
        let (v, doc) = view(text, 0, 800, preview);
        let shown: Vec<&str> = v.tiles.iter().map(|t| t.preview.text.as_str()).collect();
        let indent = "\u{2002}".repeat(4);
        // The cursor line shows no markers, and a chapter start is flush.
        assert_eq!(shown[0], "First one\n");
        assert_eq!(shown[1], format!("{indent}Second\n"));
        assert!(shown[2].contains("* * *") && !shown[2].starts_with('\u{2002}'));
        assert_eq!(shown[3], "After break\n");
        assert_eq!(shown[4], format!("{indent}Third\n"));
        // Without the "flush" rule every paragraph is indented.
        let (v, _) = view(text, 0, 800, |s| {
            preview(s);
            s.preview_flush = false;
        });
        assert_eq!(v.tiles[0].preview.text, format!("{indent}First one\n"));
        // The caret at the start of an indented paragraph sits after the indent, and it maps back.
        let second = text.find("Second").unwrap();
        let (v, _) = view(text, second, 800, preview);
        let (x, _) = v.caret_at(second).unwrap();
        let (flush, _) = v.caret_at(0).unwrap();
        let (inside, _) = v.caret_at(second + 3).unwrap();
        assert!(x > flush + 10., "caret after the indent: {x} vs {flush}; three letters in: {inside}; {:?}", v.tiles[1].buffer.layout_runs().next().map(|r| r.glyphs.iter().map(|g| (g.start, g.end, g.x, g.w)).collect::<Vec<_>>()));
        assert_eq!(v.hit(x + 1., v.caret_at(second).unwrap().1 + 3.), Some(second));
        // Raw view is untouched by the preview settings.
        let (raw, _) = view(text, 8, 800, |s| s.preview_indent = 4);
        assert_eq!(raw.tiles[1].preview.text, "Second\n");
        assert_eq!(raw.tiles[0].preview.text, "First *one*\n");
        let _ = doc;
    }

    #[test]
    fn caret_shows_on_the_empty_line_after_enter() {
        for text in ["Hello\n", "Hello\n\n", "Hello\nWorld\n", "Hello\r\n"] {
            let end = text.chars().count();
            let (v, doc) = view(text, end, 600, |_| {});
            assert_eq!(doc.cursor, end);
            let (_, y) = v.caret_at(end).unwrap_or_else(|| panic!("no caret at the end of {text:?}"));
            let (_, above) = v.caret_at(0).unwrap();
            assert!(y > above, "caret drops below the first line for {text:?}");
        }
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

    /// Words of uneven length, so that rows break in different places wherever a chunk starts.
    fn uneven(words: usize) -> String {
        (0..words).map(|i| &"abcdefghijkl"[..1 + (i * 7 + i / 5) % 11]).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn a_long_paragraph_wraps_as_if_it_were_shaped_in_one_piece() {
        let text = uneven(700);
        let (v, _) = view(&text, 0, 600, |_| {});
        assert!(v.tiles.len() > 3, "the paragraph spans several chunks");
        let settings = Settings::default();
        let marker = Marker::default();
        let options = prose::Options { marker: &marker, show_markers: false, hide_notes: false, scene_symbol: "* * *", indent: 0, caret: None };
        let whole = prose::present(&text, 0, Some(0), Initial::default(), options);
        let expected: Vec<usize> = FONTS.with(|fonts| {
            shape(&whole, Wrap::WordOrGlyph, 600., false, &settings, &mut fonts.borrow_mut())
                .layout_runs()
                .map(|run| whole.source_at(run.glyphs.iter().map(|g| g.start).min().unwrap_or(0)))
                .collect()
        });
        let rows = v.screen_rows();
        assert_eq!(rows.iter().map(|r| r.0).collect::<Vec<_>>(), expected);
        let line_height = settings.font_size * settings.line_height;
        for pair in rows.windows(2) {
            assert!((pair[1].1 - pair[0].1 - line_height).abs() < 0.01, "rows are evenly spaced: {pair:?}");
        }
        // Every position has its caret on the row it belongs to, and a click there finds it again.
        for (n, (at, top)) in rows.iter().enumerate() {
            let (x, y) = v.caret_at(*at).unwrap_or_else(|| panic!("no caret at row {n}"));
            assert!((y - top).abs() < 0.01 && x < 1., "row {n} starts at its left edge: {x}, {y} vs {top}");
            assert_eq!(v.hit(1., top + 3.), Some(*at));
        }
    }

    #[test]
    fn scrolling_up_after_a_jump_moves_by_rows() {
        let text = format!("{}

{}
{}
", uneven(300), uneven(40), uneven(500));
        let (mut v, doc) = view(&text, 0, 600, |_| {});
        let rows: Vec<usize> = v.screen_rows().into_iter().map(|r| r.0).collect();
        let settings = Settings::default();
        let mut blocks = BlockIndex::default();
        blocks.update(&doc.text, 0, &Marker::default());
        let rect = v.rect.unwrap();
        for from in [rows.len() - 2, rows.len() / 2, 30, 4] {
            for step in [1, 3, 20] {
                // A jump leaves no scroll history behind.
                v.top = rows[from];
                v.history.clear();
                FONTS.with(|fonts| {
                    let fonts = &mut fonts.borrow_mut();
                    v.layout(&doc, &blocks, &settings, fonts, rect);
                    v.scroll_by(&doc, &blocks, &settings, fonts, -(step as i32));
                });
                assert_eq!(v.top, rows[from.saturating_sub(step)], "{step} rows up from row {from}");
            }
        }
    }

    #[test]
    fn long_paragraphs_are_cut_at_spaces_and_still_cover_the_text() {
        let words = "lorem ipsum dolor sit amet ".repeat(400);
        let (v, doc) = view(&words, 0, 600, |_| {});
        assert!(v.tiles.len() > 1);
        for pair in v.tiles.windows(2) {
            assert_eq!(pair[0].start + pair[0].end, pair[1].start);
        }
        for t in &v.tiles[..v.tiles.len() - 1] {
            assert!(t.source.ends_with(' '), "{:?}", &t.source[t.source.len().saturating_sub(10)..]);
        }
        assert!(doc.text.len_chars() > 10_000);
    }
}
