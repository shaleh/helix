use helix_stdx::rope::Regex;
use once_cell::sync::Lazy;

use crate::indent::IndentStyle;
use crate::line_ending::LineEnding;
use crate::RopeSlice;

/// Maximum number of lines to search at the beginning and end of a file.
const SEARCH_LINES: usize = 5;

// Vim modeline regex, anchored to prevent false positives.
// Matches: `vim:`, `vi:`, `Vim:`, `ex:` with optional version and `set` keyword.
// The prefix is limited to 100 characters per vim convention.
static VIM_MODELINE_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^.{0,100}\s?(vi|[vV]im[<=>]?\d{0,100}|ex):\s{0,100}(se(t\s{1,100})?)?(.+)")
        .unwrap()
});

// Emacs modeline regex. Matches `-*- mode: python -*-` or `-*- python -*-`.
// Only extracts language (no indent/line-ending support for emacs modelines).
static EMACS_MODELINE_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"-\*-\s*(?:mode:\s*)?(\w[\w+-]*)\s*-\*-").unwrap()
});

/// Parsed modeline directives from a file's content.
///
/// Modelines are special comments (vim-style or emacs-style) that declare
/// language, indent style, and line ending preferences within the file itself.
/// Only the first 5 and last 5 lines are searched.
#[derive(Debug, Default)]
pub struct Modeline {
    language: Option<String>,
    indent_style: Option<IndentStyle>,
    line_ending: Option<LineEnding>,
}

impl Modeline {
    /// Parse modelines from the first 5 and last 5 lines of the given text.
    pub fn parse(text: RopeSlice) -> Self {
        let total_lines = text.len_lines();
        let mut modeline = Modeline::default();

        // Collect line indices to search: first 5 and last 5 (deduplicated).
        let head_end = SEARCH_LINES.min(total_lines);
        let tail_start = total_lines.saturating_sub(SEARCH_LINES).max(head_end);

        for line_idx in (0..head_end).chain(tail_start..total_lines) {
            let line = text.line(line_idx);
            // Vim takes priority over emacs.
            if modeline.parse_vim_modeline(line) {
                return modeline;
            }
            if modeline.parse_emacs_modeline(line) {
                return modeline;
            }
        }

        modeline
    }

    /// Try to parse a vim-style modeline from a single line.
    /// Returns `true` if a vim modeline was found.
    fn parse_vim_modeline(&mut self, line: RopeSlice) -> bool {
        let line_str: std::borrow::Cow<str> = line.into();

        let Some(captures) = VIM_MODELINE_REGEX
            .captures_iter(regex_cursor::Input::new(line))
            .next()
        else {
            return false;
        };

        // Group 4 contains the options string after the vim: prefix.
        let Some(options_match) = captures.get_group(4) else {
            return false;
        };
        let options_range = options_match.range();
        let options_str = &line_str[options_range.start..options_range.end];

        self.parse_vim_options(options_str);
        true
    }

    /// Parse vim option key=value pairs from the options portion of a modeline.
    fn parse_vim_options(&mut self, options: &str) {
        // Vim options are separated by spaces or colons.
        // The trailing colon terminates the `set` form.
        let mut shiftwidth: Option<u8> = None;
        let mut expandtab: Option<bool> = None;

        for option in options.split([' ', ':']) {
            let option = option.trim();
            if option.is_empty() {
                continue;
            }

            if let Some((key, value)) = option.split_once('=') {
                match key {
                    "ft" | "filetype" => {
                        self.language = Some(value.to_string());
                    }
                    "sw" | "shiftwidth" => {
                        shiftwidth = value.parse::<u8>().ok();
                    }
                    "ff" | "fileformat" => {
                        self.line_ending = LineEnding::from_vim_option(value);
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

        // Combine shiftwidth and expandtab into an IndentStyle.
        if let Some(sw) = shiftwidth {
            self.indent_style = Some(IndentStyle::from_vim_option(sw, expandtab));
        } else if expandtab == Some(false) {
            self.indent_style = Some(IndentStyle::Tabs);
        }
    }

    /// Try to parse an Emacs-style modeline from a single line.
    /// Only extracts language; emacs modelines don't set indent/line-ending in our implementation.
    /// Returns `true` if an emacs modeline was found.
    fn parse_emacs_modeline(&mut self, line: RopeSlice) -> bool {
        let line_str: std::borrow::Cow<str> = line.into();

        let Some(captures) = EMACS_MODELINE_REGEX
            .captures_iter(regex_cursor::Input::new(line))
            .next()
        else {
            return false;
        };

        let Some(lang_match) = captures.get_group(1) else {
            return false;
        };
        let range = lang_match.range();
        self.language = Some(line_str[range.start..range.end].to_string());
        true
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
        // 11 lines, modeline on line 6 (0-indexed: 5) — outside first 5 and last 5
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
        // Without explicit et/noet, defaults to spaces (vim-like default)
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
    fn vim_modeline_expandtab_without_shiftwidth() {
        let doc = Rope::from("# vim: noet\n");
        let modeline = Modeline::parse(doc.slice(..));
        assert_eq!(modeline.indent_style(), Some(IndentStyle::Tabs));
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
}
