/// Blame information for a file, stored as sorted non-overlapping ranges.
#[derive(Clone, Debug, Default)]
pub struct BlameResult {
    /// Consecutive, non-overlapping entries sorted by `start_line`.
    entries: Vec<BlameEntry>,
}

/// A range of consecutive lines introduced by a single commit.
#[derive(Clone, Debug)]
struct BlameEntry {
    /// 0-based first line of this range.
    start_line: u32,
    /// Number of lines in this range.
    len: u32,
    /// The abbreviated commit hash (hex string, 7 chars).
    short_hash: String,
    /// Unix timestamp (seconds since epoch) of the commit.
    timestamp: i64,
}

/// Blame information for a single line (returned by lookup).
pub struct LineBlame<'a> {
    pub short_hash: &'a str,
    pub timestamp: i64,
}

impl BlameResult {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a range entry. Entries must be added in order.
    pub fn push(&mut self, start_line: u32, len: u32, short_hash: String, timestamp: i64) {
        self.entries.push(BlameEntry {
            start_line,
            len,
            short_hash,
            timestamp,
        });
    }

    /// Look up blame for a given 0-based line number.
    pub fn line(&self, line: u32) -> Option<LineBlame<'_>> {
        let idx = self
            .entries
            .partition_point(|e| e.start_line + e.len <= line);
        let entry = self.entries.get(idx)?;
        if line >= entry.start_line && line < entry.start_line + entry.len {
            Some(LineBlame {
                short_hash: &entry.short_hash,
                timestamp: entry.timestamp,
            })
        } else {
            None
        }
    }
}
