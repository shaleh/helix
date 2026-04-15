/// Per-line blame information for a file.
#[derive(Clone, Debug, Default)]
pub struct BlameResult {
    /// One entry per line (0-indexed). `lines[n]` is the blame for document line `n`.
    pub lines: Vec<LineBlame>,
}

/// Blame information for a single line.
#[derive(Clone, Debug)]
pub struct LineBlame {
    /// The abbreviated commit hash (hex string, 7 chars).
    pub short_hash: String,
    /// Unix timestamp (seconds since epoch) of the commit.
    pub timestamp: i64,
}
