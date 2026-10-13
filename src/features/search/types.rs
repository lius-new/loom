//! Request/result values; no UI, runtime, or filesystem ownership.
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchSpec {
    pub query: String,
    pub include: String,
    pub exclude: String,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub include_hidden: bool,
    pub include_ignored: bool,
}

pub use crate::domain::document_snapshot::DocumentSnapshot as BufferSnapshot;

#[derive(Clone, Debug)]
pub struct Match {
    pub range: Range<usize>,
    pub line: usize,
    pub column: usize,
    pub preview: String,
    pub highlight: Range<usize>,
    pub line_hash: u64,
}

#[derive(Clone, Debug)]
pub struct FileMatches {
    pub path: PathBuf,
    pub root: PathBuf,
    pub matches: Vec<Match>,
    pub content_hash: u64,
}

#[derive(Clone, Debug, Default)]
pub struct SearchResults {
    pub files: Vec<Arc<FileMatches>>,
    pub matches: usize,
    pub searched: usize,
    pub skipped: usize,
    pub skipped_binary: usize,
    pub skipped_encoding: usize,
    pub skipped_large: usize,
    pub errors: usize,
    pub first_error: Option<String>,
    pub limit_hit: bool,
    pub cancelled: bool,
    pub complete: bool,
}

pub struct SearchRequest {
    pub spec: SearchSpec,
    pub roots: Vec<PathBuf>,
    pub buffers: Vec<BufferSnapshot>,
}

pub use self::SearchRequest as Request;
pub use self::SearchResults as Results;
