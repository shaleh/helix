use std::fmt;
use std::hint::black_box;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use helix_core::doc_formatter::{DocumentFormatter, TextFormat};
use helix_core::text_annotations::TextAnnotations;
use helix_core::visual_offset_from_block;
use helix_core::Rope;

fn make_long_json_line(num_pairs: usize) -> Rope {
    let mut buf = String::with_capacity(num_pairs * 14 + 20);
    buf.push('{');
    for i in 0..num_pairs {
        if i > 0 {
            buf.push(',');
        }
        buf.push_str("\"foo\":\"bar\"");
    }
    buf.push_str("}\n");
    Rope::from_str(&buf)
}

#[derive(Clone, Copy)]
struct LineSize {
    label: &'static str,
    pairs: usize,
}

impl fmt::Display for LineSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label)
    }
}

const SIZES: &[LineSize] = &[
    LineSize { label: "10k_chars", pairs: 750 },
    LineSize { label: "100k_chars", pairs: 7_500 },
    LineSize { label: "1m_chars", pairs: 75_000 },
];

fn text_format() -> TextFormat {
    TextFormat {
        viewport_width: 80,
        ..TextFormat::default()
    }
}

fn formatter_iteration(c: &mut Criterion) {
    let mut group = c.benchmark_group("formatter_iteration");
    let text_fmt = text_format();
    let annotations = TextAnnotations::default();

    for size in SIZES {
        let rope = make_long_json_line(size.pairs);
        let char_len = rope.len_chars();
        group.throughput(Throughput::Elements(char_len as u64));

        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &rope,
            |b, rope| {
                b.iter(|| {
                    let mut formatter = DocumentFormatter::new_at_prev_checkpoint(
                        black_box(rope.slice(..)),
                        &text_fmt,
                        &annotations,
                        0,
                    );
                    let mut count = 0usize;
                    while formatter.next().is_some() {
                        count += 1;
                    }
                    count
                });
            },
        );
    }
    group.finish();
}

fn visual_offset(c: &mut Criterion) {
    let mut group = c.benchmark_group("visual_offset_from_block");
    let text_fmt = text_format();
    let annotations = TextAnnotations::default();

    for size in SIZES {
        let rope = make_long_json_line(size.pairs);
        let char_len = rope.len_chars();
        let midpoint = char_len / 2;
        let endpoint = char_len.saturating_sub(2);

        group.throughput(Throughput::Elements(char_len as u64));

        group.bench_with_input(
            BenchmarkId::new("midpoint", size),
            &rope,
            |b, rope| {
                b.iter(|| {
                    visual_offset_from_block(
                        black_box(rope.slice(..)),
                        0,
                        midpoint,
                        &text_fmt,
                        &annotations,
                    )
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("endpoint", size),
            &rope,
            |b, rope| {
                b.iter(|| {
                    visual_offset_from_block(
                        black_box(rope.slice(..)),
                        0,
                        endpoint,
                        &text_fmt,
                        &annotations,
                    )
                });
            },
        );
    }
    group.finish();
}

fn visual_offset_deep(c: &mut Criterion) {
    let mut group = c.benchmark_group("visual_offset_deep_anchor");
    let text_fmt = text_format();
    let annotations = TextAnnotations::default();

    for size in SIZES {
        let rope = make_long_json_line(size.pairs);
        let char_len = rope.len_chars();
        let midpoint = char_len / 2;
        let near_end = char_len.saturating_sub(100);

        group.throughput(Throughput::Elements(char_len as u64));

        group.bench_with_input(
            BenchmarkId::new("anchor_at_midpoint", size),
            &rope,
            |b, rope| {
                b.iter(|| {
                    visual_offset_from_block(
                        black_box(rope.slice(..)),
                        midpoint,
                        midpoint,
                        &text_fmt,
                        &annotations,
                    )
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("anchor_near_end", size),
            &rope,
            |b, rope| {
                b.iter(|| {
                    visual_offset_from_block(
                        black_box(rope.slice(..)),
                        near_end,
                        near_end,
                        &text_fmt,
                        &annotations,
                    )
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, formatter_iteration, visual_offset, visual_offset_deep);
criterion_main!(benches);
