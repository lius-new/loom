//! Read-only line decoration shared by Git and document views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineChange {
    Added,
    Modified,
    Deleted,
}
