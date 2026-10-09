use std::path::{Component, Path};

/// Storage and future sync providers share this workspace-relative path boundary.
pub fn valid_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|p| matches!(p, Component::Normal(_) | Component::CurDir))
}

pub fn markdown_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| {
        ["md", "markdown", "mdown", "mkd"]
            .iter()
            .any(|extension| e.eq_ignore_ascii_case(extension))
    })
}
pub fn fuzzy(query: &str, candidate: &str) -> Option<i32> {
    let query = query.to_lowercase();
    let candidate = candidate.to_lowercase();
    let mut score = 0;
    let mut offset = 0;
    for character in query.chars() {
        let position = candidate[offset..].find(character)?;
        score += if position == 0 { 12 } else { 1 };
        score -= position.min(64) as i32;
        offset += position + character.len_utf8();
    }
    Some(score - candidate.len().min(100) as i32 / 8)
}
pub fn index(storage: &dyn super::storage::Storage) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![std::path::PathBuf::from(".")];
    while let Some(folder) = pending.pop() {
        if let Ok(entries) = storage.list(&folder) {
            for entry in entries {
                if entry.directory {
                    let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
                    if !name.starts_with('.')
                        && !["node_modules", "target"].contains(&name.as_ref())
                    {
                        pending.push(entry.path);
                    }
                } else if markdown_file(&entry.path) {
                    files.push(entry.path);
                }
            }
        }
    }
    files.sort();
    files
}
#[derive(Clone)]
pub struct Match {
    pub path: std::path::PathBuf,
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub preview: String,
}
/// ASCII lowercasing keeps byte offsets identical, so match positions stay valid in the original text.
fn fold(text: &str, case: bool) -> std::borrow::Cow<'_, str> {
    if case {
        text.into()
    } else {
        text.to_ascii_lowercase().into()
    }
}
pub fn replace_all(text: &str, query: &str, with: &str, case: bool) -> String {
    if query.is_empty() {
        return text.to_owned();
    }
    let (haystack, needle) = (fold(text, case), fold(query, case));
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (start, _) in haystack.match_indices(needle.as_ref()) {
        out.push_str(&text[last..start]);
        out.push_str(with);
        last = start + needle.len();
    }
    out.push_str(&text[last..]);
    out
}
/// The words around a match for a results list: it starts and ends on a word boundary, with an
/// ellipsis where the text goes on, and the match itself is never cut.
pub fn preview_around(text: &ropey::Rope, start: usize, end: usize) -> String {
    let len = text.len_chars();
    let mut from = start.saturating_sub(24);
    let mut to = (end + 64).min(len);
    let breaks = |c: char| c.is_whitespace();
    // Move the left edge forward to the start of a word, but never past the match.
    while from > 0 && from < start && !breaks(text.char(from - 1)) {
        from += 1;
    }
    // Move the right edge back to the end of a word, but never into the match.
    while to < len && to > end && !breaks(text.char(to)) {
        to -= 1;
    }
    let body: String = text
        .slice(from..to)
        .chars()
        .map(|c| if matches!(c, '\n' | '\r') { ' ' } else { c })
        .collect();
    format!("{}{}{}", if from > 0 { "…" } else { "" }, body.trim(), if to < len { "…" } else { "" })
}
pub fn find(
    text: &ropey::Rope,
    query: &str,
    case: bool,
    limit: usize,
) -> Vec<(usize, usize, usize, String)> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle = fold(query, case).into_owned();
    // Keep a small overlap so matches can cross rope chunk boundaries.
    let mut matches = Vec::new();
    let mut carry = String::new();
    let mut offset = 0;
    for chunk in text.chunks() {
        carry.push_str(chunk);
        let safe = carry.len().saturating_sub(query.len());
        let mut until = safe;
        while !carry.is_char_boundary(until) {
            until = until.saturating_sub(1);
        }
        let haystack = fold(&carry, case).into_owned();
        for (byte, _) in haystack.match_indices(needle.as_str()) {
            if byte >= until && offset + carry.chars().count() < text.len_chars() {
                break;
            }
            let start = offset + carry[..byte].chars().count();
            let end = start + query.chars().count();
            let line = text.char_to_line(start);
            let preview = preview_around(text, start, end);
            matches.push((start, end, line, preview));
            if matches.len() >= limit {
                return matches;
            }
        }
        let consumed = carry[..until].chars().count();
        carry.drain(..until);
        offset += consumed;
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn previews_cut_on_whole_words_and_mark_where_text_goes_on() {
        let source = "Marta stopped at the gate and listened. The house behind her was silent, and so it stayed for a very long while after.";
        let text = ropey::Rope::from_str(source);
        let preview = &find(&text, "Marta", false, 10)[0].3;
        assert!(preview.starts_with("Marta") && preview.ends_with('…'), "{preview}");
        let body = preview.trim_end_matches('…');
        let at = source.find(body).expect("the preview is text from the source");
        assert!(source[at + body.len()..].starts_with(' '), "cut inside a word: {preview}");
        let middle = &find(&text, "silent", false, 10)[0].3;
        assert!(middle.starts_with('…') && middle.contains("silent"), "{middle}");
        let short = ropey::Rope::from_str("A quiet night.");
        assert_eq!(find(&short, "quiet", false, 10)[0].3, "A quiet night.");
    }
    #[test]
    fn search_matches_cross_rope_chunks_and_map_to_character_offsets() {
        let text =
            ropey::Rope::from_str(&("é".repeat(1023) + "日本語\n" + &"é".repeat(1300) + "日本語"));
        let matches = find(&text, "日本語", true, 100);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].0, 1023);
        for (start, end, _, _) in matches {
            assert_eq!(text.slice(start..end).to_string(), "日本語");
        }
        assert!(fuzzy("jwr", "JustWrite").is_some());
        assert!(fuzzy("zzz", "JustWrite").is_none());
    }
    #[test]
    fn case_insensitive_find_and_replace_keep_offsets() {
        let text = ropey::Rope::from_str("Hello é hello HELLO");
        assert_eq!(find(&text, "hello", false, 10).len(), 3);
        assert_eq!(find(&text, "hello", true, 10).len(), 1);
        assert_eq!(replace_all("Hello é hello", "hello", "x", false), "x é x");
        assert_eq!(replace_all("Hello é hello", "hello", "x", true), "Hello é x");
    }
}
