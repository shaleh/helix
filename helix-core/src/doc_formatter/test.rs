use crate::doc_formatter::{DocumentFormatter, TextFormat, BLOCK_SIZE};
use crate::text_annotations::{InlineAnnotation, Overlay, TextAnnotations};

impl TextFormat {
    fn new_test(softwrap: bool) -> Self {
        TextFormat {
            soft_wrap: softwrap,
            tab_width: 2,
            max_wrap: 3,
            max_indent_retain: 4,
            wrap_indicator: ".".into(),
            wrap_indicator_highlight: None,
            // use a prime number to allow lining up too often with repeat
            viewport_width: 17,
            soft_wrap_at_text_width: false,
        }
    }
}

impl<'t> DocumentFormatter<'t> {
    fn collect_to_str(&mut self) -> String {
        use std::fmt::Write;
        let mut res = String::new();
        let viewport_width = self.text_fmt.viewport_width;
        let soft_wrap_at_text_width = self.text_fmt.soft_wrap_at_text_width;
        let mut line = 0;

        for grapheme in self {
            if grapheme.visual_pos.row != line {
                line += 1;
                assert_eq!(grapheme.visual_pos.row, line);
                write!(res, "\n{}", ".".repeat(grapheme.visual_pos.col)).unwrap();
            }
            if !soft_wrap_at_text_width {
                assert!(
                    grapheme.visual_pos.col <= viewport_width as usize,
                    "softwrapped failed {}<={viewport_width}",
                    grapheme.visual_pos.col
                );
            }
            write!(res, "{}", grapheme.raw).unwrap();
        }

        res
    }
}

fn softwrap_text(text: &str) -> String {
    DocumentFormatter::new_at_prev_checkpoint(
        text.into(),
        &TextFormat::new_test(true),
        &TextAnnotations::default(),
        0,
    )
    .collect_to_str()
}

#[test]
fn basic_softwrap() {
    assert_eq!(
        softwrap_text(&"foo ".repeat(10)),
        "foo foo foo foo \n.foo foo foo foo \n.foo foo  "
    );
    assert_eq!(
        softwrap_text(&"fooo ".repeat(10)),
        "fooo fooo fooo \n.fooo fooo fooo \n.fooo fooo fooo \n.fooo  "
    );

    // check that we don't wrap unnecessarily
    assert_eq!(softwrap_text("\t\txxxx1xxxx2xx\n"), "    xxxx1xxxx2xx \n ");
}

#[test]
fn softwrap_indentation() {
    assert_eq!(
        softwrap_text("\t\tfoo1 foo2 foo3 foo4 foo5 foo6\n"),
        "    foo1 foo2 \n.....foo3 foo4 \n.....foo5 foo6 \n "
    );
    assert_eq!(
        softwrap_text("\t\t\tfoo1 foo2 foo3 foo4 foo5 foo6\n"),
        "      foo1 foo2 \n.foo3 foo4 foo5 \n.foo6 \n "
    );
}

#[test]
fn long_word_softwrap() {
    assert_eq!(
        softwrap_text("\t\txxxx1xxxx2xxxx3xxxx4xxxx5xxxx6xxxx7xxxx8xxxx9xxx\n"),
        "    xxxx1xxxx2xxx\n.....x3xxxx4xxxx5\n.....xxxx6xxxx7xx\n.....xx8xxxx9xxx \n "
    );
    assert_eq!(
        softwrap_text("xxxxxxxx1xxxx2xxx\n"),
        "xxxxxxxx1xxxx2xxx\n. \n "
    );
    assert_eq!(
        softwrap_text("\t\txxxx1xxxx 2xxxx3xxxx4xxxx5xxxx6xxxx7xxxx8xxxx9xxx\n"),
        "    xxxx1xxxx \n.....2xxxx3xxxx4x\n.....xxx5xxxx6xxx\n.....x7xxxx8xxxx9\n.....xxx \n "
    );
    assert_eq!(
        softwrap_text("\t\txxxx1xxx 2xxxx3xxxx4xxxx5xxxx6xxxx7xxxx8xxxx9xxx\n"),
        "    xxxx1xxx 2xxx\n.....x3xxxx4xxxx5\n.....xxxx6xxxx7xx\n.....xx8xxxx9xxx \n "
    );
}

#[test]
fn softwrap_multichar_grapheme() {
    assert_eq!(
        softwrap_text("xxxx xxxx xxx a\u{0301}bc\n"),
        "xxxx xxxx xxx \n.ábc \n "
    )
}

fn softwrap_text_at_text_width(text: &str) -> String {
    let mut text_fmt = TextFormat::new_test(true);
    text_fmt.soft_wrap_at_text_width = true;
    let annotations = TextAnnotations::default();
    let mut formatter =
        DocumentFormatter::new_at_prev_checkpoint(text.into(), &text_fmt, &annotations, 0);
    formatter.collect_to_str()
}
#[test]
fn long_word_softwrap_text_width() {
    assert_eq!(
        softwrap_text_at_text_width("xxxxxxxx1xxxx2xxx\nxxxxxxxx1xxxx2xxx"),
        "xxxxxxxx1xxxx2xxx \nxxxxxxxx1xxxx2xxx "
    );
}

fn overlay_text(text: &str, char_pos: usize, softwrap: bool, overlays: &[Overlay]) -> String {
    DocumentFormatter::new_at_prev_checkpoint(
        text.into(),
        &TextFormat::new_test(softwrap),
        TextAnnotations::default().add_overlay(overlays, None),
        char_pos,
    )
    .collect_to_str()
}

#[test]
fn overlay() {
    assert_eq!(
        overlay_text(
            "foobar",
            0,
            false,
            &[Overlay::new(0, "X"), Overlay::new(2, "\t")],
        ),
        "Xo  bar "
    );
    assert_eq!(
        overlay_text(
            &"foo ".repeat(10),
            0,
            true,
            &[
                Overlay::new(2, "\t"),
                Overlay::new(5, "\t"),
                Overlay::new(16, "X"),
            ]
        ),
        "fo   f  o foo \n.foo Xoo foo foo \n.foo foo foo  "
    );
}

fn annotate_text(text: &str, softwrap: bool, annotations: &[InlineAnnotation]) -> String {
    DocumentFormatter::new_at_prev_checkpoint(
        text.into(),
        &TextFormat::new_test(softwrap),
        TextAnnotations::default().add_inline_annotations(annotations, None),
        0,
    )
    .collect_to_str()
}

#[test]
fn annotation() {
    assert_eq!(
        annotate_text("bar", false, &[InlineAnnotation::new(0, "foo")]),
        "foobar "
    );
    assert_eq!(
        annotate_text(
            &"foo ".repeat(10),
            true,
            &[InlineAnnotation::new(0, "foo ")]
        ),
        "foo foo foo foo \n.foo foo foo foo \n.foo foo foo  "
    );
}

#[test]
fn annotation_and_overlay() {
    let annotations = [InlineAnnotation {
        char_idx: 0,
        text: "fooo".into(),
    }];
    let overlay = [Overlay {
        char_idx: 0,
        grapheme: "\t".into(),
    }];
    assert_eq!(
        DocumentFormatter::new_at_prev_checkpoint(
            "bbar".into(),
            &TextFormat::new_test(false),
            TextAnnotations::default()
                .add_inline_annotations(annotations.as_slice(), None)
                .add_overlay(overlay.as_slice(), None),
            0,
        )
        .collect_to_str(),
        "fooo  bar "
    );
}

#[test]
fn block_checkpoint_skips_long_line_prefix() {
    let text = "x".repeat(BLOCK_SIZE * 3) + "\n";
    let rope: crate::Rope = text.as_str().into();
    let text_fmt = TextFormat::new_test(false);
    let annotations = TextAnnotations::default();

    let formatter = DocumentFormatter::new_at_prev_checkpoint(
        rope.slice(..),
        &text_fmt,
        &annotations,
        BLOCK_SIZE * 2 + 100,
    );
    assert_eq!(
        formatter.next_char_pos(),
        BLOCK_SIZE * 2,
        "checkpoint should snap to the block boundary, not rewind to line start"
    );

    let formatter = DocumentFormatter::new_at_prev_checkpoint(
        rope.slice(..),
        &text_fmt,
        &annotations,
        BLOCK_SIZE - 1,
    );
    assert_eq!(
        formatter.next_char_pos(),
        0,
        "positions within the first block should start at the line start"
    );
}

#[test]
fn block_checkpoint_visual_offset_consistency() {
    use crate::visual_offset_from_block;

    let text = "ab".repeat(BLOCK_SIZE * 2) + "\n";
    let rope: crate::Rope = text.as_str().into();
    let text_fmt = TextFormat::new_test(false);
    let annotations = TextAnnotations::default();

    let target = BLOCK_SIZE + 50;

    let (full_pos, _) =
        visual_offset_from_block(rope.slice(..), 0, target, &text_fmt, &annotations);

    let (block_pos, block_start) =
        visual_offset_from_block(rope.slice(..), target, target, &text_fmt, &annotations);

    assert_eq!(block_pos.row, 0, "within a block there is only one row without soft-wrap");
    assert_eq!(
        full_pos.col,
        block_start + block_pos.col,
        "block-relative column plus block start should equal the absolute column"
    );
}

#[test]
fn skip_to_next_line_basic() {
    let rope: crate::Rope = "aaabbb\ncccddd\neeefff\n".into();
    let text_fmt = TextFormat::new_test(false);
    let annotations = TextAnnotations::default();

    let mut formatter = DocumentFormatter::new_at_prev_checkpoint(
        rope.slice(..),
        &text_fmt,
        &annotations,
        0,
    );

    // Consume the first grapheme ('a') then skip the rest of line 1.
    let first = formatter.next().unwrap();
    assert_eq!(first.raw.to_string(), "a");
    assert_eq!(first.line_idx, 0);

    formatter.skip_to_next_line(rope.slice(..));

    // Next grapheme should be the start of line 2.
    let after_skip = formatter.next().unwrap();
    assert_eq!(after_skip.raw.to_string(), "c");
    assert_eq!(after_skip.line_idx, 1);
    assert_eq!(after_skip.visual_pos.col, 0);
    assert_eq!(after_skip.visual_pos.row, 1);
}

#[test]
fn skip_to_next_line_at_last_line() {
    let rope: crate::Rope = "only_line\n".into();
    let text_fmt = TextFormat::new_test(false);
    let annotations = TextAnnotations::default();

    let mut formatter = DocumentFormatter::new_at_prev_checkpoint(
        rope.slice(..),
        &text_fmt,
        &annotations,
        0,
    );

    let first = formatter.next().unwrap();
    assert_eq!(first.raw.to_string(), "o");

    // Rope "only_line\n" has two lines: line 0 = "only_line\n", line 1 = "".
    // Skipping from line 0 moves to line 1 (the empty trailing line).
    formatter.skip_to_next_line(rope.slice(..));
    // The empty trailing line should produce the EOF grapheme then exhaust.
    let eof = formatter.next().unwrap();
    assert!(matches!(eof.source, crate::doc_formatter::GraphemeSource::Document { codepoints: 0 }));
    assert!(formatter.next().is_none());
}

#[test]
fn skip_to_next_line_consecutive() {
    let rope: crate::Rope = "line1\nline2\nline3\nline4\n".into();
    let text_fmt = TextFormat::new_test(false);
    let annotations = TextAnnotations::default();

    let mut formatter = DocumentFormatter::new_at_prev_checkpoint(
        rope.slice(..),
        &text_fmt,
        &annotations,
        0,
    );

    // Skip lines 1 and 2 entirely without consuming any graphemes from them.
    formatter.skip_to_next_line(rope.slice(..));
    formatter.skip_to_next_line(rope.slice(..));

    let g = formatter.next().unwrap();
    assert_eq!(g.raw.to_string(), "l");
    assert_eq!(g.line_idx, 2);
    assert_eq!(g.visual_pos.row, 2);
    assert_eq!(g.visual_pos.col, 0);
}

#[test]
fn skip_to_next_line_with_inline_annotations() {
    let rope: crate::Rope = "ab\ncd\n".into();
    let text_fmt = TextFormat::new_test(false);
    let annots = [InlineAnnotation::new(0, "ZZ")];
    let mut annotations = TextAnnotations::default();
    let annotations = annotations.add_inline_annotations(&annots, None);

    let mut formatter = DocumentFormatter::new_at_prev_checkpoint(
        rope.slice(..),
        &text_fmt,
        annotations,
        0,
    );

    // First grapheme is the inline annotation "Z" (first char of "ZZ").
    let g = formatter.next().unwrap();
    assert_eq!(g.raw.to_string(), "Z");

    // Skip rest of line 1 (remaining annotation text + "ab\n").
    formatter.skip_to_next_line(rope.slice(..));

    // Should land cleanly on line 2 with no leftover annotation state.
    let g = formatter.next().unwrap();
    assert_eq!(g.raw.to_string(), "c");
    assert_eq!(g.line_idx, 1);
    assert_eq!(g.visual_pos.row, 1);
    assert_eq!(g.visual_pos.col, 0);
}

#[test]
fn skip_to_next_line_drops_virtual_lines() {
    use crate::text_annotations::LineAnnotation;
    use crate::Position;
    use std::cell::Cell;

    struct AddVirtualLines {
        target_line: usize,
        extra_rows: usize,
        pos: Cell<usize>,
    }

    impl LineAnnotation for AddVirtualLines {
        fn reset_pos(&mut self, char_idx: usize) -> usize {
            self.pos.set(0);
            let _ = char_idx;
            usize::MAX
        }

        fn insert_virtual_lines(
            &mut self,
            _char_idx: usize,
            _line_end_visual_pos: Position,
            doc_line: usize,
        ) -> Position {
            if doc_line == self.target_line {
                Position::new(self.extra_rows, 0)
            } else {
                Position::new(0, 0)
            }
        }
    }

    let rope: crate::Rope = "aaaa\nbbbb\ncccc\n".into();
    let text_fmt = TextFormat::new_test(false);

    // First: iterate normally (no skip) to get the "correct" row for line 2.
    let mut annotations_normal = TextAnnotations::default();
    annotations_normal.add_line_annotation(Box::new(AddVirtualLines {
        target_line: 0,
        extra_rows: 2,
        pos: Cell::new(0),
    }));

    let mut fmt_normal = DocumentFormatter::new_at_prev_checkpoint(
        rope.slice(..),
        &text_fmt,
        &annotations_normal,
        0,
    );
    let mut line2_row_normal = None;
    while let Some(g) = fmt_normal.next() {
        if g.line_idx == 2 && line2_row_normal.is_none() {
            line2_row_normal = Some(g.visual_pos.row);
        }
    }
    let expected_row = line2_row_normal.unwrap();
    // Line 0 takes 1 row + 2 virtual = 3, line 1 takes 1 row, so line 2
    // should start at row 4.
    assert_eq!(expected_row, 4, "sanity: line 2 should be at row 4 with 2 virtual lines after line 0");

    // Now: skip line 0 and check what row line 2 gets.
    let mut annotations_skip = TextAnnotations::default();
    annotations_skip.add_line_annotation(Box::new(AddVirtualLines {
        target_line: 0,
        extra_rows: 2,
        pos: Cell::new(0),
    }));

    let mut fmt_skip = DocumentFormatter::new_at_prev_checkpoint(
        rope.slice(..),
        &text_fmt,
        &annotations_skip,
        0,
    );
    fmt_skip.skip_to_next_line(rope.slice(..));

    // Consume through to line 2.
    let mut line2_row_skip = None;
    while let Some(g) = fmt_skip.next() {
        if g.line_idx == 2 && line2_row_skip.is_none() {
            line2_row_skip = Some(g.visual_pos.row);
        }
    }
    let actual_row = line2_row_skip.unwrap();

    assert_eq!(
        actual_row, expected_row,
        "skip_to_next_line must account for virtual lines on the skipped line",
    );
}
