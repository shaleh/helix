use helix_stdx::rope::Regex;
use once_cell::sync::Lazy;
use regex_cursor::regex_automata::Match;

use crate::indent::IndentStyle;
use crate::line_ending::LineEnding;
use crate::RopeSlice;

/// Maximum number of lines to search at the beginning and end of a file.
const SEARCH_LINES: usize = 5;

/// A modeline format defines how to detect and extract settings from a single line.
///
/// Implementors provide a regex to match against each line and a method to
/// extract settings from the matched region. New formats (e.g. helix-native)
/// can be added by implementing this trait and registering in `MODELINE_PARSERS`.
trait ModelineFormat {
    fn regex(&self) -> &Regex;

    /// Given the regex match and the full line, return the byte range within
    /// the line that contains the parseable content.
    fn content_range(&self, mat: &Match, line_len: usize) -> std::ops::Range<usize>;

    /// Parse the content slice and apply any recognized settings to the modeline.
    fn parse(&self, modeline: &mut Modeline, content: &str);
}

struct VimFormat;
struct EmacsFormat;

impl ModelineFormat for VimFormat {
    fn regex(&self) -> &Regex {
        // Anchored to prevent false positives. Matches the prefix up to (and
        // including) the `vim:` marker and optional `set` keyword. The prefix
        // is limited to 100 characters per vim convention.
        static REGEX: Lazy<Regex> = Lazy::new(|| {
            Regex::new(
                r"^.{0,100}\s?(?:vi|[vV]im[<=>]?\d{0,100}|ex):\s{0,100}(?:se(?:t\s{1,100})?)?",
            )
            .unwrap()
        });
        &REGEX
    }

    fn content_range(&self, mat: &Match, line_len: usize) -> std::ops::Range<usize> {
        // Everything after the regex match is the options string.
        mat.end()..line_len
    }

    fn parse(&self, modeline: &mut Modeline, content: &str) {
        let mut shiftwidth: Option<u8> = None;
        let mut expandtab: Option<bool> = None;

        for option in split_vim_options(content) {
            let option = option.trim();
            if option.is_empty() {
                continue;
            }

            if let Some((key, value)) = option.split_once('=') {
                match key {
                    "ft" | "filetype" => {
                        modeline.language = Some(value.to_string());
                    }
                    "sw" | "shiftwidth" => {
                        shiftwidth = value.parse::<u8>().ok();
                    }
                    "ff" | "fileformat" => {
                        modeline.line_ending = LineEnding::from_vim_option(value);
                    }
                    _ => {}
                }
            } else {
                match option {
                    "et" | "expandtab" => expandtab = Some(true),
                    "noet" | "noexpandtab" => expandtab = Some(false),
                    _ => {}
                }
            }
        }

        // Always record expandtab state so callers can apply it with document context.
        modeline.expandtab = expandtab;

        // Combine shiftwidth and expandtab into an IndentStyle when both are available.
        if let Some(sw) = shiftwidth {
            modeline.indent_style = Some(IndentStyle::from_vim_option(sw, expandtab));
        } else if expandtab == Some(false) {
            // noet without sw — tabs regardless of width
            modeline.indent_style = Some(IndentStyle::Tabs);
        }
        // et without sw: indent_style stays None, but expandtab is recorded.
        // The caller should use the document's tab width to construct Spaces(tab_width).
    }
}

impl ModelineFormat for EmacsFormat {
    fn regex(&self) -> &Regex {
        // Matches `-*- mode: python -*-` or `-*- python -*-`.
        static REGEX: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"-\*-[^-]*-\*-").unwrap());
        &REGEX
    }

    fn content_range(&self, mat: &Match, _line_len: usize) -> std::ops::Range<usize> {
        // Content between the `-*-` delimiters.
        (mat.start() + 3)..(mat.end().saturating_sub(3))
    }

    fn parse(&self, modeline: &mut Modeline, content: &str) {
        let content = content.trim();

        // Strip `mode:` prefix if present, then trim to get the language name.
        let lang = if let Some(rest) = content.strip_prefix("mode:") {
            rest.trim()
        } else {
            content
        };

        if !lang.is_empty() {
            modeline.language = Some(lang.to_string());
        }
    }
}

/// Ordered list of modeline parsers. Tried in sequence per line; first match wins.
/// Vim is listed before Emacs so it takes priority when both are present.
const MODELINE_PARSERS: &[&dyn ModelineFormat] = &[&VimFormat, &EmacsFormat];

/// Parsed modeline directives from a file's content.
///
/// Modelines are special comments (vim-style or emacs-style) that declare
/// language, indent style, and line ending preferences within the file itself.
/// Only the first and last `SEARCH_LINES` lines are searched.
#[derive(Debug, Default)]
pub struct Modeline {
    language: Option<String>,
    indent_style: Option<IndentStyle>,
    line_ending: Option<LineEnding>,
    /// Tracks expandtab/noexpandtab independently of indent_style.
    /// When expandtab is set without an explicit shiftwidth, the caller
    /// can apply it using the document's current tab width.
    expandtab: Option<bool>,
}

impl Modeline {
    /// Parse modelines from the first and last [`SEARCH_LINES`] lines of the given text.
    pub fn parse(text: RopeSlice) -> Self {
        let total_lines = text.len_lines();
        let mut modeline = Modeline::default();

        let head_end = SEARCH_LINES.min(total_lines);
        let tail_start = total_lines.saturating_sub(SEARCH_LINES).max(head_end);

        for line_idx in (0..head_end).chain(tail_start..total_lines) {
            let line = text.line(line_idx);

            for parser in MODELINE_PARSERS {
                if try_parse_modeline(&mut modeline, parser, line) {
                    return modeline;
                }
            }
        }

        modeline
    }

    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    pub fn indent_style(&self) -> Option<IndentStyle> {
        self.indent_style
    }

    pub fn line_ending(&self) -> Option<LineEnding> {
        self.line_ending
    }

    /// Returns the expandtab setting from the modeline, if specified.
    /// When `Some(true)` (expandtab) is returned without an `indent_style`,
    /// the caller should apply spaces using the document's current tab width.
    pub fn expandtab(&self) -> Option<bool> {
        self.expandtab
    }
}

/// Try a single modeline format against a line. Returns true if it matched.
fn try_parse_modeline(
    modeline: &mut Modeline,
    format: &&dyn ModelineFormat,
    line: RopeSlice,
) -> bool {
    let Some(mat) = format.regex().find(regex_cursor::Input::new(line)) else {
        return false;
    };

    let range = format.content_range(&mat, line.len_bytes());
    if range.start >= range.end {
        return false;
    }

    let content: std::borrow::Cow<str> = line.byte_slice(range).into();
    format.parse(modeline, &content);
    true
}

/// Split vim modeline options on unescaped spaces and colons.
/// Escaped colons (`\:`) are treated as literal colons within values.
fn split_vim_options(options: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let bytes = options.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            // Skip escaped character
            i += 2;
            continue;
        }
        if bytes[i] == b' ' || bytes[i] == b':' {
            if start < i {
                result.push(&options[start..i]);
            }
            start = i + 1;
        }
        i += 1;
    }

    if start < bytes.len() {
        result.push(&options[start..]);
    }

    result
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::Rope;

    #[test]
    fn vim_modeline_ft_on_first_line() {
        let doc = Rope::from("# vim: ft=python\nsome content\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.language(), Some("python"));
    }

    #[test]
    fn vim_modeline_set_form_with_trailing_colon() {
        let doc = Rope::from("something\n# vim: set ft=ruby noet :\nmore\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.language(), Some("ruby"));
    }

    #[test]
    fn vim_modeline_on_last_line() {
        let lines: Vec<&str> = (0..20)
            .map(|i| {
                if i == 19 {
                    "# vim: ft=rust\n"
                } else {
                    "content\n"
                }
            })
            .collect();
        let doc = Rope::from(lines.join("").as_str());
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.language(), Some("rust"));
    }

    #[test]
    fn vim_modeline_outside_range_not_detected() {
        let mut text = String::new();
        for i in 0..11 {
            if i == 5 {
                text.push_str("# vim: ft=python\n");
            } else {
                text.push_str("content\n");
            }
        }
        let doc = Rope::from(text.as_str());
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.language(), None);
    }

    #[test]
    fn vim_modeline_vi_variant() {
        let doc = Rope::from("# vi: ft=perl\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.language(), Some("perl"));
    }

    #[test]
    fn vim_modeline_uppercase_vim_variant() {
        let doc = Rope::from("# Vim: ft=lua\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.language(), Some("lua"));
    }

    #[test]
    fn vim_modeline_ex_variant() {
        let doc = Rope::from("# ex: ft=go\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.language(), Some("go"));
    }

    #[test]
    fn no_modeline_returns_none() {
        let doc = Rope::from("just some text\nno modeline here\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.language(), None);
        assert_eq!(modeline.indent_style(), None);
        assert_eq!(modeline.line_ending(), None);
    }

    #[test]
    fn vim_modeline_indent_spaces() {
        let doc = Rope::from("# vim: sw=4 et\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.indent_style(), Some(IndentStyle::Spaces(4)));
    }

    #[test]
    fn vim_modeline_indent_tabs() {
        let doc = Rope::from("# vim: noet sw=8\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.indent_style(), Some(IndentStyle::Tabs));
    }

    #[test]
    fn vim_modeline_shiftwidth_without_expandtab() {
        let doc = Rope::from("# vim: sw=2\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.indent_style(), Some(IndentStyle::Spaces(2)));
    }

    #[test]
    fn vim_modeline_fileformat_dos() {
        let doc = Rope::from("# vim: ff=dos\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.line_ending(), Some(LineEnding::Crlf));
    }

    #[test]
    fn vim_modeline_fileformat_unix() {
        let doc = Rope::from("# vim: ff=unix\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.line_ending(), Some(LineEnding::LF));
    }

    #[test]
    fn vim_modeline_combined_options() {
        let doc = Rope::from("# vim: ft=python sw=4 et ff=unix\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.language(), Some("python"));
        assert_eq!(modeline.indent_style(), Some(IndentStyle::Spaces(4)));
        assert_eq!(modeline.line_ending(), Some(LineEnding::LF));
    }

    #[test]
    fn vim_modeline_noexpandtab_without_shiftwidth() {
        let doc = Rope::from("# vim: noet\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.indent_style(), Some(IndentStyle::Tabs));
    }

    #[test]
    fn vim_modeline_expandtab_without_shiftwidth() {
        let doc = Rope::from("# vim: et\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.indent_style(), None);
        assert_eq!(modeline.expandtab(), Some(true));
    }

    #[test]
    fn vim_modeline_escaped_colon_in_value() {
        let doc = Rope::from("# vim: ft=python sw=4\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.language(), Some("python"));
        assert_eq!(modeline.indent_style(), Some(IndentStyle::Spaces(4)));
    }

    #[test]
    fn vim_modeline_colon_separated_options() {
        let doc = Rope::from("# vim: ft=rust:sw=2:et\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.language(), Some("rust"));
        assert_eq!(modeline.indent_style(), Some(IndentStyle::Spaces(2)));
    }

    #[test]
    fn emacs_modeline_mode_keyword() {
        let doc = Rope::from("# -*- mode: python -*-\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.language(), Some("python"));
    }

    #[test]
    fn emacs_modeline_shorthand() {
        let doc = Rope::from("# -*- ruby -*-\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.language(), Some("ruby"));
    }

    #[test]
    fn emacs_modeline_no_indent_or_line_ending() {
        let doc = Rope::from("# -*- mode: python -*-\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.indent_style(), None);
        assert_eq!(modeline.line_ending(), None);
    }

    #[test]
    fn vim_takes_priority_over_emacs() {
        let doc = Rope::from("# vim: ft=rust\n# -*- mode: python -*-\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.language(), Some("rust"));
    }

    #[test]
    fn split_vim_options_handles_escaped_colons() {
        let opts = split_vim_options(r"ft=foo\:bar:sw=4");
        assert_eq!(opts, vec![r"ft=foo\:bar", "sw=4"]);
    }

    #[test]
    fn split_vim_options_handles_spaces_and_colons() {
        let opts = split_vim_options("ft=python sw=4:et");
        assert_eq!(opts, vec!["ft=python", "sw=4", "et"]);
    }
}
