//! Review comments.
//!
//! A comment is anchored to a range of a chapter or page but stored outside its text, in
//! `comments.json`. The range is mapped through every edit; the quoted text and the words around
//! it let the comment find its place again after the text changes under it (restoring a
//! snapshot, say). A comment whose text is gone becomes *orphaned*: it stays in the list, with its
//! quote, until it is resolved or deleted. Single-user, so there is no author.

use serde::{Deserialize, Serialize};
use std::ops::Range;

pub const TAGS: [&str; 4] = ["plot", "continuity", "prose", "research"];
const CONTEXT: usize = 40;
const MAX_QUOTE: usize = 2000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    #[default]
    Open,
    Resolved,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    pub id: String,
    pub body: String,
    pub created: i64,
}

/// Character offsets into the *stored* (neutral) text of the item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    /// The chapter or page the comment belongs to.
    pub item: String,
    pub anchor: Anchor,
    /// The commented text (cut at 2000 characters).
    pub quote: String,
    /// Up to 40 characters before and after the quote.
    pub before: String,
    pub after: String,
    pub body: String,
    pub created: i64,
    #[serde(default)]
    pub status: Status,
    /// `plot`, `continuity`, `prose`, `research`, a custom word, or empty.
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub replies: Vec<Reply>,
    /// The commented text no longer exists.
    #[serde(default)]
    pub orphaned: bool,
}

/// One change to a text: `removed` characters at `start` were replaced by `inserted` characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditRec {
    pub start: usize,
    pub removed: usize,
    pub inserted: usize,
}

/// Where a range ends up after an edit. `None` means the commented text was deleted or
/// overwritten.
///
/// Text typed at the very start moves the range along; text typed at its end is not part of it.
pub fn map_range(range: Range<usize>, edit: EditRec) -> Option<Range<usize>> {
    let (a, b) = (range.start, range.end);
    let (s, e) = (edit.start, edit.start + edit.removed);
    let delta = edit.inserted as isize - edit.removed as isize;
    let shift = |p: usize| (p as isize + delta) as usize;
    if a == b {
        // An empty range never counts as covered.
        return Some(if s < a || (s == a && edit.removed == 0) { shift(a)..shift(a) } else { a..a });
    }
    if s >= b {
        Some(a..b)
    } else if e <= a {
        // Entirely before the range (an insertion exactly at its start also lands here).
        Some(shift(a)..shift(b))
    } else if s <= a && e >= b {
        None
    } else if s > a && e < b || (s > a && e == b && edit.removed == 0) {
        Some(a..shift(b))
    } else if s <= a {
        // Cuts the beginning off.
        let start = s + edit.inserted;
        (start < shift(b)).then(|| start..shift(b))
    } else {
        // Cuts the end off.
        Some(a..s).filter(|r| r.start < r.end)
    }
}

fn context_of(text: &[char], range: Range<usize>) -> (String, String) {
    let before = text[range.start.saturating_sub(CONTEXT)..range.start].iter().collect();
    let after = text[range.end..(range.end + CONTEXT).min(text.len())].iter().collect();
    (before, after)
}

fn common_suffix(a: &[char], b: &[char]) -> usize {
    a.iter().rev().zip(b.iter().rev()).take_while(|(x, y)| x == y).count()
}
fn common_prefix(a: &[char], b: &[char]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

/// Finds where a comment's text is in `text` now. `hint` is where it used to be.
pub fn find_anchor(text: &str, comment: &Comment) -> Option<Range<usize>> {
    let chars: Vec<char> = text.chars().collect();
    let quote: Vec<char> = comment.quote.chars().collect();
    let before: Vec<char> = comment.before.chars().collect();
    let after: Vec<char> = comment.after.chars().collect();
    let hint = comment.anchor.start;
    let length = comment.anchor.end.saturating_sub(comment.anchor.start);
    if quote.is_empty() || quote.len() > chars.len() {
        return None;
    }
    // Exact quote, preferring the occurrence whose surroundings match, then the nearest.
    let mut best: Option<(usize, usize, usize)> = None; // (score, distance, start)
    let mut from = 0;
    while from + quote.len() <= chars.len() {
        let Some(offset) = chars[from..].windows(quote.len()).position(|w| w == quote.as_slice()) else { break };
        let start = from + offset;
        let end = start + quote.len();
        let score = common_suffix(&chars[..start], &before).min(CONTEXT) + common_prefix(&chars[end..], &after).min(CONTEXT);
        let distance = start.abs_diff(hint);
        if best.is_none_or(|(s, d, _)| score > s || (score == s && distance < d)) {
            best = Some((score, distance, start));
        }
        from = start + 1;
    }
    if let Some((_, _, start)) = best {
        let end = if quote.len() >= MAX_QUOTE { (start + length).min(chars.len()) } else { start + quote.len() };
        return Some(start..end);
    }
    // The text inside changed: look for the words around it, in order and close together.
    let tail = &before[before.len().saturating_sub(16)..];
    let head = &after[..after.len().min(16)];
    if tail.len() < 8 || head.len() < 8 {
        return None;
    }
    let find_all = |needle: &[char]| -> Vec<usize> {
        chars.windows(needle.len()).enumerate().filter(|(_, w)| *w == needle).map(|(i, _)| i).collect()
    };
    let starts: Vec<usize> = find_all(tail).into_iter().map(|i| i + tail.len()).collect();
    let ends = find_all(head);
    let reach = length * 2 + 40;
    starts
        .into_iter()
        .flat_map(|s| ends.iter().filter(move |&&e| e >= s && e - s <= reach).map(move |&e| s..e))
        .filter(|r| r.start < r.end)
        .min_by_key(|r| r.start.abs_diff(hint))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentSet {
    #[serde(default = "version")]
    pub version: u32,
    #[serde(default)]
    pub comments: Vec<Comment>,
}
fn version() -> u32 {
    1
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub status: Option<Status>,
    pub tag: Option<String>,
    pub item: Option<String>,
    pub orphaned: Option<bool>,
}

impl CommentSet {
    pub fn parse(bytes: &[u8]) -> Self {
        serde_json::from_slice(bytes).unwrap_or_default()
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("serialize comments")
    }
    pub fn get(&self, id: &str) -> Option<&Comment> {
        self.comments.iter().find(|c| c.id == id)
    }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Comment> {
        self.comments.iter_mut().find(|c| c.id == id)
    }
    /// The comment after (or before) `from` in reading order, wrapping round: chapters in the
    /// order of `items`, then position in the text. Comments of items not listed come last.
    pub fn step(&self, items: &[String], from: Option<&str>, forward: bool) -> Option<&str> {
        let mut order: Vec<&Comment> = self.comments.iter().collect();
        order.sort_by_key(|c| (items.iter().position(|i| *i == c.item).unwrap_or(items.len()), c.anchor.start));
        let at = from.and_then(|id| order.iter().position(|c| c.id == id));
        let n = order.len();
        let next = match (at, forward) {
            (_, _) if n == 0 => return None,
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        Some(order[next].id.as_str())
    }
    /// Adds a comment on `range` of `text` (the neutral text of `item`).
    pub fn add(&mut self, item: &str, text: &str, range: Range<usize>, body: &str, tag: &str, now: i64) -> Result<String, String> {
        let chars: Vec<char> = text.chars().collect();
        if range.start >= range.end || range.end > chars.len() {
            return Err("Select some text to comment on".into());
        }
        if body.trim().is_empty() {
            return Err("Write a comment first".into());
        }
        let (before, after) = context_of(&chars, range.clone());
        let id = crate::jw::new_id();
        self.comments.push(Comment {
            id: id.clone(),
            item: item.to_owned(),
            anchor: Anchor { start: range.start, end: range.end },
            quote: chars[range.start..range.end.min(range.start + MAX_QUOTE)].iter().collect(),
            before,
            after,
            body: body.trim().to_owned(),
            created: now,
            status: Status::Open,
            tag: tag.trim().to_lowercase(),
            replies: Vec::new(),
            orphaned: false,
        });
        Ok(id)
    }
    pub fn reply(&mut self, id: &str, body: &str, now: i64) -> Result<(), String> {
        if body.trim().is_empty() {
            return Err("Write a reply first".into());
        }
        let comment = self.get_mut(id).ok_or("No such comment")?;
        comment.replies.push(Reply { id: crate::jw::new_id(), body: body.trim().to_owned(), created: now });
        Ok(())
    }
    pub fn set_status(&mut self, id: &str, status: Status) {
        if let Some(c) = self.get_mut(id) {
            c.status = status;
        }
    }
    pub fn set_tag(&mut self, id: &str, tag: &str) {
        if let Some(c) = self.get_mut(id) {
            c.tag = tag.trim().to_lowercase();
        }
    }
    pub fn edit_body(&mut self, id: &str, body: &str) {
        if let Some(c) = self.get_mut(id).filter(|_| !body.trim().is_empty()) {
            c.body = body.trim().to_owned();
        }
    }
    pub fn delete(&mut self, id: &str) -> Option<Comment> {
        let at = self.comments.iter().position(|c| c.id == id)?;
        Some(self.comments.remove(at))
    }
    /// Deletes the comments of a removed chapter or page.
    pub fn delete_item(&mut self, item: &str) -> Vec<Comment> {
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.comments).into_iter().partition(|c| c.item == item);
        self.comments = kept;
        gone
    }
    pub fn query(&self, filter: &Filter) -> Vec<&Comment> {
        self.comments
            .iter()
            .filter(|c| filter.status.is_none_or(|s| c.status == s))
            .filter(|c| filter.tag.as_ref().is_none_or(|t| &c.tag == t))
            .filter(|c| filter.item.as_ref().is_none_or(|i| &c.item == i))
            .filter(|c| filter.orphaned.is_none_or(|o| c.orphaned == o))
            .collect()
    }
    /// Every tag in use, built-in ones first.
    pub fn tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = TAGS.iter().map(|t| t.to_string()).collect();
        for c in &self.comments {
            if !c.tag.is_empty() && !tags.contains(&c.tag) {
                tags.push(c.tag.clone());
            }
        }
        tags
    }
    pub fn count_open(&self, item: &str) -> usize {
        self.comments.iter().filter(|c| c.item == item && c.status == Status::Open).count()
    }

    /// Maps the anchors of `item` through an edit. Comments whose text was deleted become
    /// orphaned; they keep a point at the place the text used to be.
    pub fn map_edit(&mut self, item: &str, edit: EditRec) {
        for c in self.comments.iter_mut().filter(|c| c.item == item && !c.orphaned) {
            match map_range(c.anchor.start..c.anchor.end, edit) {
                Some(r) => c.anchor = Anchor { start: r.start, end: r.end },
                None => {
                    c.orphaned = true;
                    let at = edit.start.min(c.anchor.start);
                    c.anchor = Anchor { start: at, end: at };
                }
            }
        }
    }
    /// Checks every comment of `item` against its current text, re-anchoring those that moved and
    /// orphaning those that cannot be found; orphans that can be found again are restored.
    /// Returns how many comments changed.
    pub fn reanchor_item(&mut self, item: &str, text: &str) -> usize {
        let chars: Vec<char> = text.chars().collect();
        let mut changed = 0;
        for c in self.comments.iter_mut().filter(|c| c.item == item) {
            let in_place = !c.orphaned
                && c.anchor.end <= chars.len()
                && c.anchor.start < c.anchor.end
                && chars[c.anchor.start..c.anchor.end].iter().copied().take(MAX_QUOTE).eq(c.quote.chars());
            if in_place {
                continue;
            }
            match find_anchor(text, c) {
                Some(r) => {
                    let new = Anchor { start: r.start, end: r.end };
                    if new != c.anchor || c.orphaned {
                        changed += 1;
                    }
                    c.anchor = new;
                    c.orphaned = false;
                }
                None => {
                    if !c.orphaned {
                        changed += 1;
                    }
                    c.orphaned = true;
                }
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(start: usize, removed: usize, inserted: usize) -> EditRec {
        EditRec { start, removed, inserted }
    }

    #[test]
    fn ranges_move_with_edits() {
        let r = 10..20;
        // Before: shifts. After: stays.
        assert_eq!(map_range(r.clone(), e(0, 0, 5)), Some(15..25));
        assert_eq!(map_range(r.clone(), e(2, 3, 0)), Some(7..17));
        assert_eq!(map_range(r.clone(), e(30, 2, 9)), Some(10..20));
        // Typing at the start moves the range; typing at the end does not extend it.
        assert_eq!(map_range(r.clone(), e(10, 0, 3)), Some(13..23));
        assert_eq!(map_range(r.clone(), e(20, 0, 3)), Some(10..20));
        // Typing inside grows it; deleting inside shrinks it.
        assert_eq!(map_range(r.clone(), e(15, 0, 4)), Some(10..24));
        assert_eq!(map_range(r.clone(), e(12, 3, 0)), Some(10..17));
        // Cutting an end.
        assert_eq!(map_range(r.clone(), e(5, 10, 0)), Some(5..10));
        assert_eq!(map_range(r.clone(), e(15, 10, 0)), Some(10..15));
        assert_eq!(map_range(r.clone(), e(8, 6, 2)), Some(10..16));
        // Everything covered: gone.
        assert_eq!(map_range(r.clone(), e(10, 10, 0)), None);
        assert_eq!(map_range(r.clone(), e(5, 20, 3)), None);
        assert_eq!(map_range(r.clone(), e(10, 10, 4)), None);
        assert_eq!(map_range(r, e(0, 25, 0)), None);
    }

    #[test]
    fn mapping_matches_the_text_for_every_single_edit() {
        let text: Vec<char> = "the quick brown fox jumps over".chars().collect();
        let range = 4..15; // "quick brown"
        for start in 0..=text.len() {
            for removed in 0..=(text.len() - start).min(8) {
                for inserted in [0, 1, 3] {
                    let mut after = text.clone();
                    after.splice(start..start + removed, std::iter::repeat_n('#', inserted));
                    let mapped = map_range(range.clone(), e(start, removed, inserted));
                    if let Some(m) = mapped {
                        assert!(m.start <= m.end && m.end <= after.len(), "edit {start},{removed},{inserted}");
                        // Characters that survived the edit are still inside.
                        let kept: String = after[m.clone()].iter().filter(|c| **c != '#').collect();
                        let expected: String = range
                            .clone()
                            .filter(|i| !(start..start + removed).contains(i))
                            .map(|i| text[i])
                            .collect();
                        assert_eq!(kept, expected, "edit {start},{removed},{inserted}");
                    }
                }
            }
        }
    }

    fn comment_on(text: &str, range: Range<usize>) -> (CommentSet, String) {
        let mut set = CommentSet::default();
        let id = set.add("ch1", text, range, "Check this", "Continuity", 100).unwrap();
        (set, id)
    }

    #[test]
    fn stepping_walks_chapters_in_order_then_position_and_wraps() {
        let mut set = CommentSet::default();
        let late = set.add("ch2", "other text", 6..10, "b", "", 1).unwrap();
        let early = set.add("ch2", "other text", 0..5, "a", "", 2).unwrap();
        let first = set.add("ch1", "The quick brown fox", 4..9, "c", "", 3).unwrap();
        let items = ["ch1".to_owned(), "ch2".to_owned()];
        assert_eq!(set.step(&items, None, true), Some(first.as_str()));
        assert_eq!(set.step(&items, None, false), Some(late.as_str()));
        assert_eq!(set.step(&items, Some(&first), true), Some(early.as_str()));
        assert_eq!(set.step(&items, Some(&early), true), Some(late.as_str()));
        assert_eq!(set.step(&items, Some(&late), true), Some(first.as_str()));
        assert_eq!(set.step(&items, Some(&first), false), Some(late.as_str()));
        assert_eq!(CommentSet::default().step(&items, None, true), None);
    }

    #[test]
    fn comments_have_status_tags_and_replies() {
        let (mut set, id) = comment_on("The quick brown fox", 4..9);
        let c = set.get(&id).unwrap();
        assert_eq!((c.quote.as_str(), c.before.as_str(), c.after.as_str()), ("quick", "The ", " brown fox"));
        assert_eq!(c.tag, "continuity");
        set.reply(&id, "Fixed in chapter 3", 200).unwrap();
        assert!(set.reply(&id, "  ", 200).is_err());
        set.set_status(&id, Status::Resolved);
        set.add("ch2", "other text", 0..5, "Plot hole", "plot", 300).unwrap();
        set.add("ch2", "other text", 6..10, "Odd word", "pacing", 400).unwrap();
        let open = Filter { status: Some(Status::Open), ..Default::default() };
        assert_eq!(set.query(&open).len(), 2);
        assert_eq!(set.query(&Filter { tag: Some("plot".into()), ..Default::default() }).len(), 1);
        assert_eq!(set.query(&Filter { item: Some("ch1".into()), ..Default::default() }).len(), 1);
        assert_eq!(set.tags(), ["plot", "continuity", "prose", "research", "pacing"]);
        assert_eq!(set.count_open("ch2"), 2);
        assert!(set.add("ch1", "abc", 2..2, "x", "", 1).is_err());
        assert!(set.add("ch1", "abc", 0..9, "x", "", 1).is_err());
        assert!(set.add("ch1", "abc", 0..2, " ", "", 1).is_err());
        assert_eq!(CommentSet::parse(set.to_json().as_bytes()), set);
        assert_eq!(CommentSet::parse(b"garbage"), CommentSet::default());
        let gone = set.delete_item("ch2");
        assert_eq!(gone.len(), 2);
        assert!(set.delete(&id).is_some() && set.comments.is_empty());
    }

    #[test]
    fn deleted_text_orphans_the_comment_and_undo_restores_it() {
        let text = "Intro. The quick brown fox jumps. Outro.";
        let (mut set, id) = comment_on(text, 11..16);
        assert_eq!(set.get(&id).unwrap().quote, "quick");
        // Typing before moves it.
        set.map_edit("ch1", e(0, 0, 3));
        assert_eq!(set.get(&id).unwrap().anchor, Anchor { start: 14, end: 19 });
        // Deleting the commented word orphans it, but the comment is kept with its quote.
        set.map_edit("ch1", e(14, 5, 0));
        let c = set.get(&id).unwrap();
        assert!(c.orphaned && c.quote == "quick" && c.anchor.start == c.anchor.end);
        // Undo puts the text back; the comment finds it again.
        let restored = "xxxIntro. The quick brown fox jumps. Outro.";
        assert_eq!(set.reanchor_item("ch1", restored), 1);
        let c = set.get(&id).unwrap();
        assert!(!c.orphaned);
        assert_eq!(&restored.chars().skip(c.anchor.start).take(5).collect::<String>(), "quick");
        // Edits to other chapters leave it alone.
        set.map_edit("other", e(0, 100, 0));
        assert!(!set.get(&id).unwrap().orphaned);
    }

    #[test]
    fn comments_find_their_place_after_text_moves_or_changes() {
        let text = "Alpha beta gamma. The old lighthouse keeper smiled at the storm. Delta epsilon zeta.";
        let start = text.find("old lighthouse keeper").unwrap();
        let (mut set, id) = comment_on(text, start..start + 21);
        // The paragraph moved and another one now says the same words earlier.
        let moved = "The old lighthouse keeper smiled. Filler. Alpha beta gamma. The old lighthouse keeper smiled at the storm. Delta epsilon zeta.";
        set.reanchor_item("ch1", moved);
        let a = set.get(&id).unwrap().anchor;
        let found: String = moved.chars().skip(a.start).take(a.end - a.start).collect();
        assert_eq!(found, "old lighthouse keeper");
        assert_eq!(moved.find(&found).unwrap(), moved.rfind(&found).unwrap().min(moved.find(&found).unwrap()));
        assert!(a.start > 40, "chose the occurrence with matching context, not the first: {a:?}");
        // The commented words were edited but the sentence around them survived.
        let edited = "Alpha beta gamma. The ancient lighthouse keeper smiled at the storm. Delta epsilon zeta.";
        set.reanchor_item("ch1", edited);
        let c = set.get(&id).unwrap();
        assert!(!c.orphaned, "context should re-anchor it");
        let found: String = edited.chars().skip(c.anchor.start).take(c.anchor.end - c.anchor.start).collect();
        assert!(found.contains("lighthouse"), "{found:?}");
        // Gone entirely: orphaned, never silently dropped.
        set.reanchor_item("ch1", "Nothing like it remains.");
        assert!(set.get(&id).unwrap().orphaned);
        assert_eq!(set.comments.len(), 1);
    }

    #[test]
    fn reanchoring_never_panics_on_unicode_and_tiny_texts() {
        let (mut set, _) = comment_on("日本語のテキスト、そして é café", 3..8);
        for text in ["", "a", "日本語", "日本語のテキスト、そして é café", "のテキスト"] {
            set.reanchor_item("ch1", text);
        }
    }
}
