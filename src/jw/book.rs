//! `book.json`: publishing metadata that feeds every export preset.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BookDetails {
    /// The book's title; the project title is used when this is empty.
    pub title: String,
    pub subtitle: String,
    /// Used in running headers.
    pub short_title: String,
    pub series: String,
    pub legal_name: String,
    pub pen_name: String,
    pub email: String,
    pub phone: String,
    /// Postal address, one line per line.
    pub address: String,
    pub website: String,
    pub publisher: String,
    pub isbn: String,
    pub copyright_line: String,
    /// The blurb.
    pub description: String,
    /// Comma-separated subjects or keywords.
    pub subjects: String,
    /// Path of the cover image inside the project, such as `assets/cover.jpg`.
    pub cover: String,
    pub dedication: String,
    pub epigraph: String,
    pub acknowledgments: String,
    pub about_author: String,
    /// Replaces the generated copyright page when not empty.
    pub copyright_page: String,
}
impl BookDetails {
    /// The name printed on the title page: pen name, else legal name.
    pub fn author(&self) -> &str {
        if self.pen_name.trim().is_empty() { self.legal_name.trim() } else { self.pen_name.trim() }
    }
    pub fn surname(&self) -> String {
        self.author().split_whitespace().last().unwrap_or("").to_owned()
    }
    pub fn display_title<'a>(&'a self, project_title: &'a str) -> &'a str {
        if self.title.trim().is_empty() { project_title } else { self.title.trim() }
    }
    /// The title used in running headers.
    pub fn running_title<'a>(&'a self, project_title: &'a str) -> String {
        if !self.short_title.trim().is_empty() {
            self.short_title.trim().to_uppercase()
        } else {
            self.display_title(project_title).to_uppercase()
        }
    }
    pub fn parse(bytes: &[u8]) -> Self {
        serde_json::from_slice(bytes).unwrap_or_default()
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("serialize book details")
    }
    /// The copyright page text: the author's own, or one built from the other fields.
    pub fn copyright_text(&self, project_title: &str, year: i32) -> String {
        if !self.copyright_page.trim().is_empty() {
            return self.copyright_page.trim().to_owned();
        }
        let mut lines = Vec::new();
        lines.push(if self.copyright_line.trim().is_empty() {
            let name = if self.legal_name.trim().is_empty() { self.author() } else { self.legal_name.trim() };
            format!("Copyright © {year} {name}").trim().to_owned()
        } else {
            self.copyright_line.trim().to_owned()
        });
        lines.push("All rights reserved.".into());
        if !self.publisher.trim().is_empty() {
            lines.push(format!("Published by {}", self.publisher.trim()));
        }
        if !self.isbn.trim().is_empty() {
            lines.push(format!("ISBN {}", self.isbn.trim()));
        }
        let _ = project_title;
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn author_surname_and_titles() {
        let mut b = BookDetails { legal_name: "Jane Q. Doe".into(), ..Default::default() };
        assert_eq!(b.author(), "Jane Q. Doe");
        assert_eq!(b.surname(), "Doe");
        b.pen_name = "J. Q. Smith".into();
        assert_eq!(b.surname(), "Smith");
        assert_eq!(b.display_title("Working"), "Working");
        b.title = "The Book".into();
        assert_eq!(b.running_title("Working"), "THE BOOK");
        b.short_title = "Book".into();
        assert_eq!(b.running_title("Working"), "BOOK");
    }
    #[test]
    fn copyright_page_is_generated_or_overridden() {
        let mut b = BookDetails { legal_name: "Jane Doe".into(), isbn: "978-1".into(), ..Default::default() };
        let t = b.copyright_text("T", 2026);
        assert!(t.starts_with("Copyright © 2026 Jane Doe") && t.contains("ISBN 978-1"));
        b.copyright_page = "Custom".into();
        assert_eq!(b.copyright_text("T", 2026), "Custom");
        assert_eq!(BookDetails::parse(b"nonsense"), BookDetails::default());
        assert_eq!(BookDetails::parse(b.to_json().as_bytes()), b);
    }
}
