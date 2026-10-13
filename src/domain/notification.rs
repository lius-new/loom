//! Notification data and duration policy; no framework or service handles.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

impl ToastKind {
    /// How long the toast stays up; errors linger so they can be read.
    pub fn duration(self) -> std::time::Duration {
        std::time::Duration::from_millis(match self {
            Self::Info | Self::Success => 3_000,
            Self::Error => 6_000,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub message: String,
}
