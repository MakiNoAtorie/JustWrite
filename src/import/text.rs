//! Plain text and Markdown readers.

use super::{Kind, Raw, RawPara};
use crate::{core::notes, core::prose::is_scene_break};

const CP1252: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
];

/// Decodes a text file: UTF-8 (with or without a byte-order mark), UTF-16 with one, or else
/// Windows-1252, which is what most old word processors wrote.
pub fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    for (bom, little) in [([0xFF, 0xFE], true), ([0xFE, 0xFF], false)] {
        if let Some(rest) = bytes.strip_prefix(&bom) {
            let units: Vec<u16> = rest.chunks_exact(2).map(|c| if little { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
            return String::from_utf16_lossy(&units);
        }
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes
            .iter()
            .map(|&b| match b {
                0x80..=0x9F => CP1252[(b - 0x80) as usize],
                b => b as char,
            })
            .collect(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paragraphs {
    /// Decide from how many blank lines the file has.
    Auto,
    /// Every line is a paragraph.
    Lines,
    /// Blank lines separate paragraphs; lines inside one are joined.
    BlankLines,
}

fn lines_of(text: &str) -> Vec<&str> {
    text.lines().collect()
}

/// `_italic_` becomes `*italic*`; underscores inside words are left alone.
fn underscores(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    let boundary = |c: Option<&char>| c.is_none_or(|c| !c.is_alphanumeric());
    while i < chars.len() {
        if chars[i] == '_' {
            let double = chars.get(i + 1) == Some(&'_');
            let n = if double { 2 } else { 1 };
            let before = if i == 0 { None } else { chars.get(i - 1) };
            if boundary(before) && chars.get(i + n).is_some_and(|c| !c.is_whitespace() && *c != '_') {
                // Find the closing delimiter.
                let mut j = i + n;
                while j < chars.len() {
                    let closes = chars[j] == '_' && (n == 1 || chars.get(j + 1) == Some(&'_')) && !chars[j - 1].is_whitespace();
                    if closes && boundary(chars.get(j + n)) {
                        out.push_str(if double { "**" } else { "*" });
                        out.extend(&chars[i + n..j]);
                        out.push_str(if double { "**" } else { "*" });
                        i = j + n;
                        break;
                    }
                    j += 1;
                }
                if i == j + n {
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

pub fn read_plain(text: &str, mode: Paragraphs) -> Raw {
    let lines = lines_of(text);
    let blank = lines.iter().filter(|l| l.trim().is_empty()).count();
    let by_blank = match mode {
        Paragraphs::BlankLines => true,
        Paragraphs::Lines => false,
        // Most lines blank-separated means paragraphs are hard-wrapped.
        Paragraphs::Auto => !lines.is_empty() && blank * 100 / lines.len() >= 25 && {
            // ...unless the lines are long, which means they already are whole paragraphs.
            let long = lines.iter().filter(|l| l.chars().count() > 100).count();
            long * 4 < lines.len().saturating_sub(blank)
        },
    };
    let mut paras = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, paras: &mut Vec<RawPara>| {
        let t = std::mem::take(current);
        let t = t.trim();
        if !t.is_empty() {
            paras.push(finish_line(t));
        }
    };
    for line in lines {
        if line.trim().is_empty() {
            flush(&mut current, &mut paras);
        } else if by_blank {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(line.trim());
        } else {
            flush(&mut current, &mut paras);
            current.push_str(line.trim());
            flush(&mut current, &mut paras);
        }
    }
    flush(&mut current, &mut paras);
    Raw { title: None, paras, comments: Vec::new() }
}
fn finish_line(text: &str) -> RawPara {
    if is_scene_break(text) {
        return RawPara { text: String::new(), kind: Kind::SceneBreak, page_break_before: false };
    }
    RawPara { text: underscores(text), kind: Kind::Body, page_break_before: false }
}

// ---- Markdown ---------------------------------------------------------------------------

/// Inline Markdown to the prose subset: emphasis stays, links keep their text, the rest goes.
fn inline(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() => {
                // `\*` and `\\` keep their escape; other escapes are just the character.
                if matches!(chars[i + 1], '*' | '\\') {
                    out.push('\\');
                }
                out.push(chars[i + 1]);
                i += 2;
            }
            '!' if chars.get(i + 1) == Some(&'[') => {
                // An image: dropped.
                if let Some(close) = chars[i..].iter().position(|c| *c == ')') {
                    i += close + 1;
                } else {
                    i += 1;
                }
            }
            '[' => {
                // [text](url) becomes text.
                let rest: String = chars[i..].iter().collect();
                if let Some(mid) = rest.find("](") {
                    if let Some(end) = rest[mid..].find(')') {
                        out.push_str(&inline(&rest[1..mid]));
                        i += rest[..mid + end + 1].chars().count();
                        continue;
                    }
                }
                out.push('[');
                i += 1;
            }
            '`' => i += 1,
            '<' => {
                let rest: String = chars[i..].iter().collect();
                if let Some(body) = rest.strip_prefix("<!--") {
                    if let Some(end) = body.find("-->") {
                        let note = body[..end].trim();
                        if !note.is_empty() {
                            out.push(notes::OPEN);
                            out.push_str(&format!(" {note} "));
                            out.push(notes::CLOSE);
                        }
                        i += 4 + body[..end].chars().count() + 3;
                        continue;
                    }
                }
                // Other tags are dropped; `<br>` is a space.
                if let Some(end) = rest.find('>') {
                    let tag = rest[1..end].trim().to_ascii_lowercase();
                    if tag.starts_with("br") {
                        out.push(' ');
                    }
                    if tag.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '/') {
                        i += rest[..=end].chars().count();
                        continue;
                    }
                }
                out.push('<');
                i += 1;
            }
            '_' => {
                // Handled below as a whole line.
                out.push('_');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    underscores(&out)
}

pub fn read_markdown(text: &str) -> Raw {
    let lines = lines_of(text);
    let mut raw = Raw::default();
    let mut current = String::new();
    let mut in_fence = false;
    let mut in_front_matter = lines.first().is_some_and(|l| l.trim() == "---") && lines.iter().skip(1).any(|l| l.trim() == "---" || l.trim() == "...");
    let flush = |current: &mut String, paras: &mut Vec<RawPara>| {
        let t = std::mem::take(current);
        let t = t.trim();
        if !t.is_empty() {
            paras.push(RawPara { text: inline(t), kind: Kind::Body, page_break_before: false });
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        i += 1;
        if in_front_matter {
            // YAML front matter: pick up the title, skip the rest.
            if i > 1 && (trimmed == "---" || trimmed == "...") {
                in_front_matter = false;
            } else if let Some(title) = trimmed.strip_prefix("title:") {
                raw.title = Some(title.trim().trim_matches(['"', '\'']).to_owned());
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            flush(&mut current, &mut raw.paras);
            continue;
        }
        if in_fence {
            if !trimmed.is_empty() {
                raw.paras.push(RawPara { text: trimmed.to_owned(), kind: Kind::Body, page_break_before: false });
            }
            continue;
        }
        if trimmed.is_empty() {
            flush(&mut current, &mut raw.paras);
            continue;
        }
        // `## Name ##` right after a scene break is that scene's name, not a heading.
        if current.is_empty() && crate::core::prose::scene_name(trimmed).is_some() && raw.paras.last().is_some_and(|p| p.kind == Kind::SceneBreak) {
            raw.paras.push(RawPara { text: trimmed.to_owned(), kind: Kind::Body, page_break_before: false });
            continue;
        }
        // ATX heading.
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && trimmed[hashes..].starts_with([' ', '\t']) {
            flush(&mut current, &mut raw.paras);
            let title = trimmed[hashes..].trim().trim_end_matches('#').trim();
            raw.paras.push(RawPara { text: inline(title).replace('*', ""), kind: Kind::Heading(hashes as u8), page_break_before: false });
            continue;
        }
        // Setext heading: the next line is === or ---.
        if let Some(next) = lines.get(i).map(|l| l.trim()) {
            let underline = |c: char| next.len() >= 2 && next.chars().all(|x| x == c);
            if !current.is_empty() {
                // part of a paragraph
            } else if underline('=') || (underline('-') && !is_scene_break(trimmed)) {
                raw.paras.push(RawPara { text: inline(trimmed).replace('*', ""), kind: Kind::Heading(if underline('=') { 1 } else { 2 }), page_break_before: false });
                i += 1;
                continue;
            }
        }
        if is_scene_break(trimmed) && trimmed != "#" {
            flush(&mut current, &mut raw.paras);
            raw.paras.push(RawPara { text: String::new(), kind: Kind::SceneBreak, page_break_before: false });
            continue;
        }
        // Quotes and list markers are dropped; the text stays.
        let mut body = trimmed;
        while let Some(rest) = body.strip_prefix('>') {
            body = rest.trim_start();
        }
        let after_digits = body.trim_start_matches(|c: char| c.is_ascii_digit());
        let mut item = false;
        if body.starts_with(['-', '+']) && body[1..].starts_with(' ') {
            flush(&mut current, &mut raw.paras);
            body = body[1..].trim_start();
            item = true;
        } else if body.starts_with("* ") && !body.trim_end().ends_with('*') {
            flush(&mut current, &mut raw.paras);
            body = body[2..].trim_start();
            item = true;
        } else if after_digits.len() < body.len() && after_digits.starts_with(". ") {
            flush(&mut current, &mut raw.paras);
            body = after_digits[2..].trim_start();
            item = true;
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(body);
        // A list item is a paragraph of its own; so is a line ended by a hard break (two
        // trailing spaces or a backslash).
        if item || line.ends_with("  ") || line.ends_with('\\') {
            flush(&mut current, &mut raw.paras);
        }
    }
    flush(&mut current, &mut raw.paras);
    raw
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(raw: &Raw) -> Vec<String> {
        raw.paras
            .iter()
            .map(|p| match &p.kind {
                Kind::Body => p.text.clone(),
                Kind::Heading(n) => format!("H{n}:{}", p.text),
                Kind::SceneBreak => "#".into(),
            })
            .collect()
    }

    #[test]
    fn decoding_handles_boms_utf16_and_windows_1252() {
        assert_eq!(decode(b"\xEF\xBB\xBFhello"), "hello");
        assert_eq!(decode(b"\xFF\xFEh\x00i\x00"), "hi");
        assert_eq!(decode(b"\xFE\xFF\x00h\x00i"), "hi");
        assert_eq!(decode("café — “x”".as_bytes()), "café — “x”");
        assert_eq!(decode(b"caf\xe9 \x97 \x93x\x94"), "café — “x”");
    }

    #[test]
    fn plain_text_paragraphs_follow_the_layout_of_the_file() {
        let wrapped = "First line of a\nwrapped paragraph.\n\nSecond one\nalso wrapped.\n\n***\n\nThird.\n";
        assert_eq!(texts(&read_plain(wrapped, Paragraphs::Auto)), ["First line of a wrapped paragraph.", "Second one also wrapped.", "#", "Third."]);
        let lines = "A whole paragraph on one line.\nAnother whole paragraph.\nAnd a third.\n";
        assert_eq!(texts(&read_plain(lines, Paragraphs::Auto)).len(), 3);
        assert_eq!(texts(&read_plain("a\nb\n\nc\n", Paragraphs::Lines)), ["a", "b", "c"]);
        assert_eq!(texts(&read_plain("a\nb\n\nc\n", Paragraphs::BlankLines)), ["a b", "c"]);
        // Long lines separated by blank lines are already whole paragraphs.
        let long = format!("{}\n\n{}\n\n{}\n", "word ".repeat(40), "word ".repeat(40), "word ".repeat(40));
        assert_eq!(read_plain(&long, Paragraphs::Auto).paras.len(), 3);
        assert_eq!(texts(&read_plain("He said _hello_ to snake_case_name.\n", Paragraphs::Lines)), ["He said *hello* to snake_case_name."]);
        assert!(read_plain("", Paragraphs::Auto).paras.is_empty());
    }

    #[test]
    fn markdown_headings_emphasis_links_and_scene_breaks() {
        let md = "---\ntitle: \"My Novel\"\n---\n# Part One\n\n## Chapter 1\n\nIt was *dark*, **very** dark. See [the map](http://x.y) now.\n\n* * *\n\nNext _scene_ starts.\n\nChapter Two\n===========\n\n- not a list\n> quoted\n\n<!-- remember this -->\nEnd \\* star.\n";
        let raw = read_markdown(md);
        assert_eq!(raw.title.as_deref(), Some("My Novel"));
        let t = texts(&raw);
        assert_eq!(t[0], "H1:Part One");
        assert_eq!(t[1], "H2:Chapter 1");
        assert_eq!(t[2], "It was *dark*, **very** dark. See the map now.");
        assert_eq!(t[3], "#");
        assert_eq!(t[4], "Next *scene* starts.");
        assert_eq!(t[5], "H1:Chapter Two");
        assert_eq!(t[6], "not a list");
        assert_eq!(t[7], "quoted");
        assert!(t[8].contains(notes::OPEN) && t[8].contains("remember this") && t[8].ends_with("End \\* star."), "{:?}", t[8]);
        // A closed heading is a scene name only right after a break.
        assert_eq!(texts(&read_markdown("a\n\n***\n\n## The storm ##\n\nb\n\n## Two ##\n")), ["a", "#", "## The storm ##", "b", "H2:Two"]);
    }

    #[test]
    fn markdown_never_panics_on_hostile_input() {
        let alphabet = ["#", " ", "\n", "*", "_", "[", "](", ")", "<!--", "-->", "`", "```", "\\", "-", "=", "!", ">", "é", "1. "];
        let mut seed = 11u64;
        for _ in 0..4000 {
            let mut text = String::new();
            for _ in 0..(seed % 25) {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                text.push_str(alphabet[(seed >> 33) as usize % alphabet.len()]);
            }
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let _ = read_markdown(&text);
            let _ = read_plain(&text, Paragraphs::Auto);
        }
    }
}
