use smartstring::{LazyCompact, SmartString};
use textwrap::{self, Options, WordSplitter::NoHyphenation};

use crate::LineEnding;

/// Given a slice of text, return the text re-wrapped to fit it
/// within the given width. Handles proper line endings and comment syntax.
pub fn reflow_region(
    fragment: &str,
    comment_tokens: &[&str],
    line_ending: LineEnding,
    width: usize,
) -> SmartString<LazyCompact> {
    let (segments, prefix) = unfill(fragment, comment_tokens, line_ending.as_str());

    let pieces: Vec<_> = segments
        .iter()
        .map(|segment| match segment {
            Segment::Paragraph(paragraph) => reflow_hard_wrap(
                paragraph,
                prefix,
                make_textwrap_lineending(line_ending),
                width,
            ),
            Segment::Separator(separator) => SmartString::from(*separator),
        })
        .collect();

    let line_ending_str = line_ending.as_str();
    // The paragraphs have had all of their line endings stripped before being wrapped. Now it is
    // time to add them back. Uses LineEnding so Windows is properly handled.
    // This mouthful of code is `intersperse`.
    let result: SmartString<LazyCompact> = pieces
        .into_iter()
        .flat_map(|item| std::iter::once(line_ending_str.into()).chain(std::iter::once(item)))
        .skip(1)
        .collect();
    result
}

// Use fill, not refill which means the caller owns breaking the paragraph down into lines as well
// as passing in proper line endings.
// prefix is the indentation before the original paragraph. Comment syntax is part of that indentation.
fn reflow_hard_wrap(
    text: &str,
    prefix: Option<&str>,
    line_ending: textwrap::LineEnding,
    text_width: usize,
) -> SmartString<LazyCompact> {
    let options = Options::new(text_width)
        .initial_indent(prefix.unwrap_or(""))
        .subsequent_indent(prefix.unwrap_or(""))
        .word_splitter(NoHyphenation)
        .word_separator(textwrap::WordSeparator::AsciiSpace)
        .line_ending(line_ending);
    textwrap::fill(text, options).into()
}

// Turn Helix LineEnding into TextWrap's LineEnding.
fn make_textwrap_lineending(line_ending: LineEnding) -> textwrap::LineEnding {
    match line_ending {
        LineEnding::Crlf => textwrap::LineEnding::CRLF,
        _ => textwrap::LineEnding::LF,
    }
}

fn is_all_whitespace(text: &str) -> bool {
    text.chars().all(|c| c.is_whitespace())
}

enum Segment<'a> {
    Paragraph(String),
    Separator(&'a str),
}

// A prefix is leading whitespace, an optional prefix which is usually a comment marker, and an optional
// trailing space or tab.
fn compute_prefix<'a>(prefixes: &'a [&str], text: &'a str) -> Option<&'a str> {
    let indent_length = text.len() - text.trim_start().len();
    let trimmed = &text[indent_length..];

    for p in prefixes {
        if let Some(rest) = trimmed.strip_prefix(p) {
            let space = rest.starts_with([' ', '\t']) as usize;
            let end = indent_length + p.len() + space;
            return Some(&text[..end]);
        }
    }

    if indent_length > 0 {
        Some(&text[..indent_length])
    } else {
        None
    }
}

// Text after the prefix has been stripped off.
fn without_prefix<'a>(text: &'a str, prefix: Option<&str>) -> &'a str {
    if let Some(prefix) = prefix {
        if let Some(stripped) = text.strip_prefix(prefix) {
            return stripped;
        }
    }

    text
}

fn is_separator(prefix: Option<&str>, line: &str) -> bool {
    // without_prefix enforces the trailing space which breaks the case of an otherwise empty line
    // that starts with a comment marker.
    //
    // This handles cases where the line:
    //  - starts with the prefix exactly
    //  - starts with the prefix but does not have the trailing space
    //  - does not start with the prefix at all
    let trimmed = prefix
        .and_then(|prefix| line.strip_prefix(prefix.trim_end()))
        .unwrap_or(line);

    trimmed.is_empty() || is_all_whitespace(trimmed)
}

// Comment syntax aware version of textwrap::unfill().
//
// This tries to break up existing paragraphs and transform them into flat collections of sentences
// in preparation for being split up again by textwrap::fill(). The secret sauce here is comment
// markers are peeled off before filling and reapplied by the fill() logic because they are passed
// in as indent values.
fn unfill<'a>(
    text: &'a str,
    comment_tokens: &'a [&str],
    line_ending: &str,
) -> (Vec<Segment<'a>>, Option<&'a str>) {
    let prefix = if let Some(initial) = text.split(line_ending).next() {
        // Assume the leading prefix is the uniform prefix to apply.
        compute_prefix(comment_tokens, initial)
    } else {
        None
    };

    let mut segments = Vec::new();
    let mut paragraph = Vec::new();

    for line in text.split(line_ending) {
        if is_separator(prefix, line) {
            if !paragraph.is_empty() {
                segments.push(Segment::Paragraph(paragraph.join(" ")));
                paragraph.clear();
            }
            segments.push(Segment::Separator(line));
        } else {
            paragraph.push(without_prefix(line, prefix));
        }
    }
    // Wrap up the final paragraph.
    if !paragraph.is_empty() {
        segments.push(Segment::Paragraph(paragraph.join(" ")));
    }

    (segments, prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflow_is_idempotent() {
        let input = "hello,\nworld";
        let width = 80;
        assert_eq!(
            reflow_region(
                reflow_region(input, &[], LineEnding::LF, width).as_str(),
                &[],
                LineEnding::LF,
                width
            ),
            reflow_region(input, &[], LineEnding::LF, width)
        );
    }

    #[test]
    fn fitting_line_is_untouched() {
        assert_eq!(
            reflow_region("hello world\n", &[], LineEnding::LF, 80).as_str(),
            "hello world\n"
        );
    }

    #[test]
    fn lines_are_wrapped() {
        let region = reflow_region("one two three four five\n", &[], LineEnding::LF, 10);
        let lines = region.lines();
        for line in lines {
            assert!(line.len() <= 10);
        }
    }

    #[test]
    fn two_lines_are_one_paragraph() {
        assert_eq!(
            reflow_region("hello\nworld\n", &[], LineEnding::LF, 80).as_str(),
            "hello world\n"
        );
    }

    #[test]
    fn blank_separator_is_preserved() {
        assert_eq!(
            reflow_region("alpha beta\n\ngamma delta\n", &[], LineEnding::LF, 80).as_str(),
            "alpha beta\n\ngamma delta\n"
        );
    }

    #[test]
    fn whitespace_only_line_is_valid_separator_and_is_preserved() {
        assert_eq!(
            reflow_region("alpha\n    \nbeta\n", &[], LineEnding::LF, 80).as_str(),
            "alpha\n    \nbeta\n"
        );
    }

    #[test]
    fn no_final_newline() {
        assert_eq!(
            reflow_region("hello, world", &[], LineEnding::LF, 80).as_str(),
            "hello, world"
        );
    }

    #[test]
    fn comment_block_is_handled() {
        assert_eq!(
            reflow_region(
                "  // one two\n  // three four five six\n",
                &["///", "//", "/*"],
                LineEnding::LF,
                12
            )
            .as_str(),
            "  // one two\n  // three\n  // four\n  // five\n  // six\n"
        );
    }

    // Not fully formed yet.
    // #[test]
    // fn comment_block_with_paragraphs() {
    //     assert_eq!(
    //         reflow_region(
    //             "/*\n* Foo ......\n* Bar ......\n*\n* More ......\n* Still more ......\n*/",
    //             &["///", "//", "/*", "*", "*/"],
    //             LineEnding::LF,
    //             10
    //         )
    //         .as_str(),
    //         "/*\n* Foo ......\n* Bar ......\n*\n* More ......\n* Still more ......\n*/",
    //     );
    // }

    #[test]
    fn windows_lines_are_honored() {
        assert_eq!(
            reflow_region("a\r\nb\r\n", &[], LineEnding::Crlf, 80).as_str(),
            "a b\r\n"
        );
    }

    #[test]
    fn two_one_word_paragraphs() {
        assert_eq!(
            reflow_region("// a\n//\n// b", &["//"], LineEnding::LF, 80).as_str(),
            "// a\n//\n// b"
        );
    }

    #[test]
    fn indent_is_preserved_in_non_comment_paragraphs() {
        assert_eq!(
            reflow_region("    foo bar\n    baz", &[], LineEnding::LF, 80).as_str(),
            "    foo bar baz"
        );
    }

    #[test]
    fn prefix_needs_longest_first() {
        assert_eq!(
            reflow_region("/// doc here\n", &["///", "//"], LineEnding::LF, 80).as_str(),
            "/// doc here\n"
        );
    }
}
