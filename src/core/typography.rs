//! Per-project language and auto-typography: smart quotes, dashes, ellipsis and the Spanish
//! dialogue dash (raya). Everything here works on the text before the caret and returns how to
//! rewrite it as a character is typed.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "es")]
    Spanish,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quotes {
    pub open: char,
    pub close: char,
    pub inner_open: char,
    pub inner_close: char,
}
impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::Spanish];
    /// BCP 47 tag, used for spellcheck, hyphenation and the `lang` attribute of exports.
    pub fn tag(self) -> &'static str {
        match self {
            Self::English => "en-US",
            Self::Spanish => "es-ES",
        }
    }
    /// Two-letter ISO 639-1 code.
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Spanish => "es",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Spanish => "Español",
        }
    }
    pub fn from_code(code: &str) -> Option<Self> {
        match code.get(..2)?.to_ascii_lowercase().as_str() {
            "en" => Some(Self::English),
            "es" => Some(Self::Spanish),
            _ => None,
        }
    }
    pub fn quotes(self) -> Quotes {
        match self {
            Self::English => Quotes { open: '“', close: '”', inner_open: '‘', inner_close: '’' },
            Self::Spanish => Quotes { open: '«', close: '»', inner_open: '“', inner_close: '”' },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Typography {
    pub smart_quotes: bool,
    /// `--` becomes an em dash.
    pub em_dash: bool,
    /// `...` becomes an ellipsis character.
    pub ellipsis: bool,
    /// Spanish only: a dash typed at the start of a line, or after a space in a line that starts
    /// with one, becomes a raya.
    pub dialogue_dash: bool,
}
impl Default for Typography {
    fn default() -> Self {
        Self { smart_quotes: true, em_dash: true, ellipsis: true, dialogue_dash: true }
    }
}

/// Replace the `delete_before` characters before the caret with `insert`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replacement {
    pub delete_before: usize,
    pub insert: String,
}

fn opens(previous: Option<char>) -> bool {
    match previous {
        None => true,
        Some(c) => c.is_whitespace() || "([{<—–-«“‘/\u{a0}".contains(c),
    }
}

/// What typing `typed` after `before` (the text of the current line before the caret) should
/// produce. `None` means insert the character as typed.
pub fn on_type(before: &str, typed: char, language: Language, options: &Typography) -> Option<Replacement> {
    let previous = before.chars().next_back();
    let quotes = language.quotes();
    match typed {
        '"' if options.smart_quotes => {
            let opening = opens(previous);
            let count = |c: char| before.chars().filter(|x| *x == c).count() as isize;
            let ch = match language {
                Language::English => if opening { quotes.open } else { quotes.close },
                Language::Spanish => {
                    let outer = count('«') - count('»');
                    let inner = count('“') - count('”');
                    if opening {
                        if outer > 0 { '“' } else { '«' }
                    } else if inner > 0 {
                        '”'
                    } else {
                        '»'
                    }
                }
            };
            Some(Replacement { delete_before: 0, insert: ch.to_string() })
        }
        '\'' if options.smart_quotes => {
            let ch = if previous.is_some_and(char::is_alphanumeric) || !opens(previous) {
                '’'
            } else {
                '‘'
            };
            Some(Replacement { delete_before: 0, insert: ch.to_string() })
        }
        '-' => {
            if options.em_dash && previous == Some('-') {
                return Some(Replacement { delete_before: 1, insert: "—".into() });
            }
            if language == Language::Spanish && options.dialogue_dash {
                // `--` typed at the start of a line: the first dash already became a raya.
                if before.trim() == "—" && previous == Some('—') {
                    return Some(Replacement { delete_before: 1, insert: "—".into() });
                }
                let line_start = before.trim().is_empty();
                let in_dialogue = before.trim_start().starts_with('—') && previous.is_some_and(char::is_whitespace);
                if line_start || in_dialogue {
                    return Some(Replacement { delete_before: 0, insert: "—".into() });
                }
            }
            None
        }
        '.' if options.ellipsis && before.ends_with("..") => {
            Some(Replacement { delete_before: 2, insert: "…".into() })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Types `input` one character at a time, applying the replacements, like the editor does.
    fn type_all(input: &str, language: Language, options: &Typography) -> String {
        let mut line = String::new();
        for c in input.chars() {
            if c == '\n' {
                line.clear();
                continue;
            }
            // Keep the whole text for assertions but only feed the current line to the rules.
            match on_type(&line, c, language, options) {
                Some(r) => {
                    for _ in 0..r.delete_before {
                        line.pop();
                    }
                    line.push_str(&r.insert);
                }
                None => line.push(c),
            }
        }
        line
    }
    #[test]
    fn english_quotes_and_apostrophes() {
        let o = Typography::default();
        let t = |s: &str| type_all(s, Language::English, &o);
        assert_eq!(t("\"Hello,\" she said."), "“Hello,” she said.");
        assert_eq!(t("It's 'quoted' text"), "It’s ‘quoted’ text");
        assert_eq!(t("(\"in parens\")"), "(“in parens”)");
        assert_eq!(t("He said, \"'Tis.\""), "He said, “‘Tis.”");
        assert_eq!(t("rock 'n' roll"), "rock ‘n’ roll");
        assert_eq!(t("don't"), "don’t");
    }
    #[test]
    fn dashes_and_ellipsis() {
        let o = Typography::default();
        assert_eq!(type_all("wait--what...", Language::English, &o), "wait—what…");
        assert_eq!(type_all("a-b", Language::English, &o), "a-b");
        let off = Typography { em_dash: false, ellipsis: false, smart_quotes: false, dialogue_dash: false };
        assert_eq!(type_all("a--b... \"x\"", Language::English, &off), "a--b... \"x\"");
    }
    #[test]
    fn spanish_quotes_nest_and_dialogue_uses_the_raya() {
        let o = Typography::default();
        let t = |s: &str| type_all(s, Language::Spanish, &o);
        assert_eq!(t("Dijo \"hola\" y se fue"), "Dijo «hola» y se fue");
        assert_eq!(t("Dijo \"ella dijo \"no\" a todo\" y se fue"), "Dijo «ella dijo “no” a todo» y se fue");
        assert_eq!(t("-Hola -dijo Juan--. Adiós."), "—Hola —dijo Juan—. Adiós.");
        assert_eq!(t("--Hola"), "—Hola");
        assert_eq!(t("co-operar"), "co-operar");
        let plain = Typography { dialogue_dash: false, ..o };
        assert_eq!(type_all("-Hola", Language::Spanish, &plain), "-Hola");
        // The raya is Spanish only.
        assert_eq!(type_all("-Hello", Language::English, &o), "-Hello");
    }
    #[test]
    fn languages_round_trip_through_codes() {
        for l in Language::ALL {
            assert_eq!(Language::from_code(l.code()), Some(l));
            assert_eq!(Language::from_code(l.tag()), Some(l));
        }
        assert_eq!(Language::from_code("fr"), None);
        let json = serde_json::to_string(&Language::Spanish).unwrap();
        assert_eq!(json, "\"es\"");
    }
}
