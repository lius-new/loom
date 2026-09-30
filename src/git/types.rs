use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RepositoryId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitRuntimeSource {
    Managed,
    System,
    Custom,
    RemoteEnvironment,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GitCapabilities {
    pub porcelain_v2: bool,
    pub switch: bool,
    pub restore: bool,
    pub pathspec_from_file: bool,
    pub force_with_lease: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChangeKind {
    #[default]
    Unmodified,
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Ignored,
    Unmerged,
}

impl ChangeKind {
    pub fn from_porcelain(value: u8) -> Self {
        match value {
            b'A' => Self::Added,
            b'M' => Self::Modified,
            b'D' => Self::Deleted,
            b'R' => Self::Renamed,
            b'C' => Self::Copied,
            b'T' => Self::TypeChanged,
            b'U' => Self::Unmerged,
            _ => Self::Unmodified,
        }
    }

    pub fn indicator(self) -> &'static str {
        match self {
            Self::Unmodified => "",
            Self::Added => "A",
            Self::Modified => "M",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Copied => "C",
            Self::TypeChanged => "T",
            Self::Untracked => "?",
            Self::Ignored => "!",
            Self::Unmerged => "U",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    BothAdded,
    BothDeleted,
    BothModified,
    AddedByUs,
    AddedByThem,
    DeletedByUs,
    DeletedByThem,
    Other,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffStat {
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileState {
    pub index: ChangeKind,
    pub worktree: ChangeKind,
    pub conflict: Option<ConflictKind>,
    pub original_path: Option<PathBuf>,
    pub index_stat: Option<DiffStat>,
    pub worktree_stat: Option<DiffStat>,
}

impl FileState {
    pub fn is_changed(&self) -> bool {
        self.index != ChangeKind::Unmodified
            || self.worktree != ChangeKind::Unmodified
            || self.conflict.is_some()
    }

    pub fn display_kind(&self) -> ChangeKind {
        self.conflict
            .map(|_| ChangeKind::Unmerged)
            .unwrap_or_else(|| {
                if self.worktree != ChangeKind::Unmodified {
                    self.worktree
                } else {
                    self.index
                }
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeadState {
    Branch(String),
    Detached(String),
    Unborn(String),
}

impl Default for HeadState {
    fn default() -> Self {
        Self::Unborn("HEAD".into())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpstreamState {
    pub name: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RepositoryState {
    #[default]
    Normal,
    MergeInProgress,
    RebaseInProgress,
    CherryPickInProgress,
    RevertInProgress,
    Bisecting,
    DetachedHead,
    UnbornBranch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    Scan,
    Clone,
    Stage,
    Unstage,
    Restore,
    Commit,
    Fetch,
    Pull,
    Push,
    Checkout,
    Merge,
    Rebase,
    Stash,
    Worktree,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OperationState {
    pub kind: OperationKind,
    pub started_at: SystemTime,
    pub progress: Option<f32>,
    pub cancellable: bool,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositorySnapshot {
    pub id: RepositoryId,
    pub worktree_root: PathBuf,
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub head: HeadState,
    pub upstream: Option<UpstreamState>,
    pub ahead: u32,
    pub behind: u32,
    pub files: BTreeMap<PathBuf, FileState>,
    pub repository_state: RepositoryState,
    pub features: RepositoryFeatures,
    pub generation: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepositoryFeatures {
    pub submodules: bool,
    pub lfs: bool,
    pub sparse_checkout: bool,
    pub shallow: bool,
}

impl RepositorySnapshot {
    pub fn branch_label(&self) -> String {
        match &self.head {
            HeadState::Branch(name) | HeadState::Unborn(name) => name.clone(),
            HeadState::Detached(oid) => oid.chars().take(8).collect(),
        }
    }

    pub fn dirty_count(&self) -> usize {
        self.files
            .values()
            .filter(|state| state.is_changed())
            .count()
    }

    pub fn state_for_absolute_path(&self, path: &std::path::Path) -> Option<&FileState> {
        path.strip_prefix(&self.worktree_root)
            .ok()
            .and_then(|path| self.files.get(path))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffTarget {
    HeadToIndex,
    IndexToWorktree,
    HeadToWorktree,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitSummary {
    pub oid: String,
    pub parents: Vec<String>,
    pub author: String,
    pub author_email: String,
    pub timestamp: i64,
    pub subject: String,
    pub decorations: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchInfo {
    pub name: String,
    pub remote: bool,
    pub current: bool,
    pub oid: String,
    pub upstream: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashInfo {
    pub index: usize,
    pub oid: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub oid: String,
    pub branch: Option<String>,
    pub bare: bool,
    pub detached: bool,
    pub locked: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteInfo {
    pub name: String,
    pub fetch_url: Option<String>,
    pub push_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameLine {
    pub oid: String,
    pub original_line: usize,
    pub final_line: usize,
    pub author: String,
    pub author_time: i64,
    pub summary: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConflictStages {
    pub path: PathBuf,
    pub base: Option<String>,
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineChange {
    Added,
    Modified,
    Deleted,
}
