//! Spelling. The dictionaries are the Hunspell ones for English and Spanish that ship with the
//! program; words the author adds are kept in a plain list next to the settings.
//!
//! Checking works on one paragraph at a time and only on prose: notes are left alone.

use super::{
    notes::{self, Kind, Marker},
    typography::Language,
};
use std::{collections::HashSet, ops::Range, sync::RwLock};

const EN_AFF: &str = include_str!("../../assets/dictionaries/en.index.aff");
const EN_DIC: &str = include_str!("../../assets/dictionaries/en.index.dic");
const ES_AFF: &str = include_str!("../../assets/dictionaries/es.index.aff");
const ES_DIC: &str = include_str!("../../assets/dictionaries/es.index.dic");

pub struct Speller {
    dictionary: zspell::Dictionary,
    language: Language,
    /// Words the author added, lower-cased.
    personal: RwLock<HashSet<String>>,
}

impl Speller {
    /// Builds the dictionary; takes a moment (tens of milliseconds), so do it off the UI thread.
    pub fn load(language: Language, personal: &[String]) -> Result<Self, String> {
        let (aff, dic) = match language {
            Language::English => (EN_AFF, EN_DIC),
            Language::Spanish => (ES_AFF, ES_DIC),
        };
        let dictionary = zspell::builder().config_str(aff).dict_str(dic).build().map_err(|e| format!("The dictionary could not be loaded: {e}"))?;
        Ok(Self { dictionary, language, personal: RwLock::new(personal.iter().map(|w| w.to_lowercase()).collect()) })
    }
    pub fn language(&self) -> Language {
        self.language
    }
    pub fn add(&self, word: &str) {
        if let Ok(mut set) = self.personal.write() {
            set.insert(word.to_lowercase());
        }
    }
    /// Whether `word` (letters, apostrophes and hyphens) is acceptable.
    pub fn check(&self, word: &str) -> bool {
        let word: String = word.chars().map(|c| if c == '’' || c == 'ʼ' { '\'' } else { c }).collect();
        if self.personal.read().is_ok_and(|set| set.contains(&word.to_lowercase())) {
            return true;
        }
        if self.dictionary.check_word(&word) {
            return true;
        }
        // Hyphenated compounds are fine when every part is.
        if word.contains('-') {
            return word.split('-').filter(|p| !p.is_empty()).all(|p| self.check(p));
        }
        // A sentence-initial capital, or an all-capitals shout, of a known word.
        let lower = word.to_lowercase();
        if lower != word && (word.chars().skip(1).all(|c| !c.is_lowercase()) || word.chars().next().is_some_and(char::is_uppercase)) && self.dictionary.check_word(&lower) {
            return true;
        }
        // Possessives of anything that is itself fine.
        if let Some(base) = word.strip_suffix("'s") {
            return !base.is_empty() && self.check(base);
        }
        false
    }
}

/// Words made of letters, with apostrophes and hyphens between letters; `(char range, text)`.
pub fn words(text: &str) -> Vec<(Range<usize>, String)> {
    let chars: Vec<char> = text.chars().collect();
    let letter = |c: char| c.is_alphabetic();
    let joiner = |c: char| matches!(c, '\'' | '’' | '-');
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if !letter(chars[i]) && !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        let start = i;
        let mut digits = false;
        while i < chars.len() && (letter(chars[i]) || chars[i].is_ascii_digit() || (joiner(chars[i]) && i + 1 < chars.len() && letter(chars[i + 1]) && i > start)) {
            digits |= chars[i].is_ascii_digit();
            i += 1;
        }
        if !digits {
            out.push((start..i, chars[start..i].iter().collect()));
        }
    }
    out
}

/// Misspelled words in `text` as char ranges. Notes are skipped. `text` is display text, so
/// the author's marker is needed to tell notes from prose.
pub fn misspelled(speller: &Speller, text: &str, marker: &Marker, in_block: bool) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut block = in_block;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let scan = notes::scan_line(line, marker, block);
        block = scan.block_after;
        for seg in scan.segs.iter().filter(|s| s.kind == Kind::Prose) {
            let piece = &line[seg.range.clone()];
            let base = offset + line[..seg.range.start].chars().count();
            for (range, word) in words(piece) {
                // Initials and short capitalised abbreviations are not mistakes.
                if word.chars().count() > 1 && !speller.check(&word) && !is_abbreviation(&word) {
                    found.push(base + range.start..base + range.end);
                }
            }
        }
        offset += line.chars().count();
    }
    found
}

fn is_abbreviation(word: &str) -> bool {
    word.chars().count() <= 3 && word.chars().all(|c| c.is_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_and_spanish_dictionaries_know_ordinary_words() {
        let start = std::time::Instant::now();
        let en = Speller::load(Language::English, &[]).unwrap();
        let loaded = start.elapsed();
        assert!(en.check("walked") && en.check("Walked") && en.check("don't") && en.check("don’t") && en.check("mother-in-law"));
        assert!(!en.check("walkd") && !en.check("tiem"));
        assert!(en.check("lantern's"));
        let es = Speller::load(Language::Spanish, &[]).unwrap();
        assert!(es.check("camino") && es.check("niño") && es.check("dijo") && es.check("Adiós"));
        assert!(!es.check("camimo"));
        eprintln!("english dictionary: {loaded:?}, both: {:?}", start.elapsed());
    }

    #[test]
    fn personal_words_and_case() {
        let en = Speller::load(Language::English, &["Zorblax".into()]).unwrap();
        assert!(en.check("zorblax") && en.check("Zorblax"));
        assert!(!en.check("Quillfeather"));
        en.add("Quillfeather");
        assert!(en.check("quillfeather"));
        assert!(en.check("THE"));
    }

    #[test]
    fn only_prose_is_checked() {
        let en = Speller::load(Language::English, &[]).unwrap();
        let marker = Marker::default();
        let text = "The wlak was long. $$ somthing odd here\nShe sadd $$ inlinee note $$ nothing.\n";
        let bad: Vec<String> = misspelled(&en, text, &marker, false).into_iter().map(|r| text.chars().skip(r.start).take(r.len()).collect()).collect();
        assert_eq!(bad, ["wlak", "sadd"]);
        let block = "$$$\nhidden mispelled\n$$$\nvisible mispelled\n";
        let bad: Vec<String> = misspelled(&en, block, &marker, false).into_iter().map(|r| block.chars().skip(r.start).take(r.len()).collect()).collect();
        assert_eq!(bad, ["mispelled"]);
        assert!(words("it’s 42nd o'clock").iter().map(|w| w.1.as_str()).eq(["it’s", "o'clock"]));
    }
}
