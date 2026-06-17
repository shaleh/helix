//! Two unrelated ways to change the case of text live in this module. They
//! read alike and behave differently, so keep them apart.
//!
//! IdentifierCase and recase normalize a name from one convention to another.
//! They break a name into words on separators and on camelCase humps, then
//! rebuild it in the requested style. They walk grapheme clusters, so an
//! accented letter built from a base plus a combining mark is never split.
//! Hand them HTTPServer in snake style and the result is http_server. The
//! to-case command is the caller.
//!
//! to_pascal_case and the functions next to it back the snippet variable
//! syntax, the pascalcase and camelcase modifiers and the like. They preserve
//! the inner shape of a name and recase only the first letter of each word.
//! Hand them HTTPServer and it stays HTTPServer. They run over a capture group
//! as a snippet expands.
//!
//! The split is deliberate. The snippet transforms follow a contract shared
//! with other editors, so they must not normalize. Recasing answers to a user
//! running a command, so it can.

use std::str::FromStr;

use unicode_segmentation::UnicodeSegmentation;

use crate::Tendril;

/// A naming convention an identifier can be rewritten into. This is identifier
/// normalization, not the snippet case transforms. The source style does not
/// matter and inner capitals are not preserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentifierCase {
    /// foo_bar_baz
    Snake,
    /// FOO_BAR_BAZ
    ScreamingSnake,
    /// foo-bar-baz
    Kebab,
    /// fooBarBaz
    Camel,
    /// FooBarBaz
    Pascal,
    /// Foo Bar Baz
    Title,
}

/// Every accepted spelling of every case, for argument completion and parsing.
/// The first spelling of each group is the canonical one shown to users.
pub const IDENTIFIER_CASE_NAMES: &[&str] = &[
    "snake",
    "screaming",
    "constant",
    "kebab",
    "dash",
    "camel",
    "pascal",
    "title",
];

impl FromStr for IdentifierCase {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "snake" => Ok(IdentifierCase::Snake),
            "screaming" | "constant" => Ok(IdentifierCase::ScreamingSnake),
            "kebab" | "dash" => Ok(IdentifierCase::Kebab),
            "camel" => Ok(IdentifierCase::Camel),
            "pascal" => Ok(IdentifierCase::Pascal),
            "title" => Ok(IdentifierCase::Title),
            _ => Err(format!("unknown case format: {s}")),
        }
    }
}

/// Rewrite an identifier in the requested case. The source style does not
/// matter.
pub fn recase(text: &str, case: IdentifierCase) -> Tendril {
    let mut buf = Tendril::new();
    recase_with(text, case, &mut buf);
    buf
}

/// Rewrite an identifier in the requested case, appending to buf.
///
/// The input is walked one grapheme cluster at a time so an accented letter
/// written as a base plus a combining mark stays intact and never reads as a
/// word separator. Words are detected regardless of the source style. The walk
/// breaks on separators like underscores, dashes, dots, and spaces, and on
/// camelCase humps. A run of capitals stays together up to the start of the
/// next word, so HTTPServer splits into HTTP and Server and parseURLNow splits
/// into parse, URL, and Now. Digits stay attached to the letters they follow,
/// so utf8 stays whole.
///
/// When the input holds no word characters it is appended unchanged so a
/// selection of pure punctuation survives.
fn recase_with(text: &str, case: IdentifierCase, buf: &mut Tendril) {
    let mut graphemes = text.graphemes(true).peekable();
    // Base char of the previous word grapheme. None at the start and after a
    // separator, which is exactly when the next word grapheme opens a new word.
    let mut prev_base: Option<char> = None;
    let mut words_emitted: usize = 0;

    while let Some(grapheme) = graphemes.next() {
        let base = grapheme.chars().next().unwrap();

        if !is_word(base) {
            prev_base = None;
            continue;
        }

        let next_base = graphemes.peek().map(|g| g.chars().next().unwrap());
        let starts_word = match prev_base {
            None => true,
            Some(prev) => is_word_boundary(prev, base, next_base),
        };

        if starts_word {
            if words_emitted > 0 {
                push_separator(buf, case);
            }
            words_emitted += 1;
        }

        let uppercase = match case {
            IdentifierCase::ScreamingSnake => true,
            IdentifierCase::Snake | IdentifierCase::Kebab => false,
            IdentifierCase::Title | IdentifierCase::Pascal => starts_word,
            // The first word stays lowercase, later words capitalize their head.
            IdentifierCase::Camel => starts_word && words_emitted > 1,
        };

        for c in grapheme.chars() {
            if uppercase {
                buf.extend(c.to_uppercase());
            } else {
                buf.extend(c.to_lowercase());
            }
        }

        prev_base = Some(base);
    }

    if words_emitted == 0 {
        buf.push_str(text);
    }
}

fn is_word(base: char) -> bool {
    base.is_alphanumeric()
}

/// Decide whether the current char opens a new word given the previous and
/// next base chars within the same separator-free run.
fn is_word_boundary(prev: char, cur: char, next: Option<char>) -> bool {
    let starts_hump = cur.is_uppercase() && (prev.is_lowercase() || prev.is_ascii_digit());
    let ends_acronym = cur.is_uppercase()
        && prev.is_uppercase()
        && next.is_some_and(|n| n.is_lowercase());
    starts_hump || ends_acronym
}

fn push_separator(buf: &mut Tendril, case: IdentifierCase) {
    match case {
        IdentifierCase::Snake | IdentifierCase::ScreamingSnake => buf.push('_'),
        IdentifierCase::Kebab => buf.push('-'),
        IdentifierCase::Title => buf.push(' '),
        IdentifierCase::Camel | IdentifierCase::Pascal => {}
    }
}

// The functions below back the LSP snippet case modifiers. They keep a name's
// inner shape and recase only word-start letters. Normalizing a name across
// conventions is IdentifierCase further up.

// todo: should this be grapheme aware?

pub fn to_pascal_case(text: impl Iterator<Item = char>) -> Tendril {
    let mut res = Tendril::new();
    to_pascal_case_with(text, &mut res);
    res
}

pub fn to_pascal_case_with(text: impl Iterator<Item = char>, buf: &mut Tendril) {
    let mut at_word_start = true;
    for c in text {
        // we don't count _ as a word char here so case conversions work well
        if !c.is_alphanumeric() {
            at_word_start = true;
            continue;
        }
        if at_word_start {
            at_word_start = false;
            buf.extend(c.to_uppercase());
        } else {
            buf.push(c)
        }
    }
}

pub fn to_upper_case_with(text: impl Iterator<Item = char>, buf: &mut Tendril) {
    for c in text {
        for c in c.to_uppercase() {
            buf.push(c)
        }
    }
}

pub fn to_lower_case_with(text: impl Iterator<Item = char>, buf: &mut Tendril) {
    for c in text {
        for c in c.to_lowercase() {
            buf.push(c)
        }
    }
}

pub fn to_camel_case(text: impl Iterator<Item = char>) -> Tendril {
    let mut res = Tendril::new();
    to_camel_case_with(text, &mut res);
    res
}
pub fn to_camel_case_with(mut text: impl Iterator<Item = char>, buf: &mut Tendril) {
    for c in &mut text {
        if c.is_alphanumeric() {
            buf.extend(c.to_lowercase())
        }
    }
    let mut at_word_start = false;
    for c in text {
        // we don't count _ as a word char here so case conversions work well
        if !c.is_alphanumeric() {
            at_word_start = true;
            continue;
        }
        if at_word_start {
            at_word_start = false;
            buf.extend(c.to_uppercase());
        } else {
            buf.push(c)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn round_trips_between_styles() {
        for input in ["foo_bar_baz", "fooBarBaz", "FooBarBaz", "foo-bar-baz"] {
            assert_eq!(recase(input, IdentifierCase::Snake), "foo_bar_baz");
            assert_eq!(recase(input, IdentifierCase::ScreamingSnake), "FOO_BAR_BAZ");
            assert_eq!(recase(input, IdentifierCase::Kebab), "foo-bar-baz");
            assert_eq!(recase(input, IdentifierCase::Camel), "fooBarBaz");
            assert_eq!(recase(input, IdentifierCase::Pascal), "FooBarBaz");
            assert_eq!(recase(input, IdentifierCase::Title), "Foo Bar Baz");
        }
    }

    #[test]
    fn keeps_acronyms_and_digits_intact() {
        // Snake output reveals exactly where the word boundaries landed.
        assert_eq!(recase("HTTPServer", IdentifierCase::Snake), "http_server");
        assert_eq!(recase("parseURLNow", IdentifierCase::Snake), "parse_url_now");
        assert_eq!(recase("utf8", IdentifierCase::Snake), "utf8");
        assert_eq!(recase("parse2Json", IdentifierCase::Snake), "parse2_json");
    }

    #[test]
    fn normalizes_mixed_input() {
        assert_eq!(recase("FOO_bar", IdentifierCase::Camel), "fooBar");
        assert_eq!(recase("parseURLNow", IdentifierCase::Pascal), "ParseUrlNow");
    }

    #[test]
    fn ignores_surrounding_and_repeated_separators() {
        assert_eq!(recase("__foo__bar__", IdentifierCase::Snake), "foo_bar");
    }

    #[test]
    fn leaves_wordless_text_alone() {
        assert_eq!(recase("___", IdentifierCase::Pascal), "___");
        assert_eq!(recase("", IdentifierCase::Snake), "");
    }

    #[test]
    fn keeps_combining_marks_attached_to_their_letter() {
        // "café" with the accent as a base letter plus a combining acute.
        // The mark must ride along with its letter and never read as a
        // separator that would split the word or drop the accent.
        let decomposed = "cafe\u{301}Menu";
        assert_eq!(recase(decomposed, IdentifierCase::Snake), "cafe\u{301}_menu");
        assert_eq!(recase(decomposed, IdentifierCase::Pascal), "Cafe\u{301}Menu");

        // Uppercasing the head of a word preserves the trailing mark.
        let lead = "e\u{301}ditor_name";
        assert_eq!(recase(lead, IdentifierCase::Pascal), "E\u{301}ditorName");
    }

    #[test]
    fn handles_non_ascii_letters_and_multichar_case_mapping() {
        // German eszett uppercases to two characters.
        assert_eq!(
            recase("straßeName", IdentifierCase::ScreamingSnake),
            "STRASSE_NAME"
        );
        assert_eq!(recase("straßeName", IdentifierCase::Snake), "straße_name");

        // Greek casing and hump detection work the same as for ASCII.
        assert_eq!(recase("ΑΒΓ_δεζ", IdentifierCase::Snake), "αβγ_δεζ");
        assert_eq!(recase("φοοΒαρ", IdentifierCase::Snake), "φοο_βαρ");
    }
}
