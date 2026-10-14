//! Transient availability of the saved default; configuration remains separate.
use super::ShellKind;
#[derive(Clone, Default)]
pub struct TerminalAvailability {
    fallback: Option<ShellKind>,
}
impl TerminalAvailability {
    pub fn fallback(&self) -> Option<ShellKind> {
        self.fallback
    }
    pub fn update(&mut self, fallback: Option<ShellKind>) -> bool {
        let changed = self.fallback != fallback;
        self.fallback = fallback;
        changed
    }
}
