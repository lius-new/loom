//! Pure key sequence/version state, separate from GUI search input.
use super::keystroke::Keystroke;
#[derive(Clone, Default)]
pub struct Model {
    pending: Vec<Keystroke>,
    pending_seq: u64,
    version: u64,
}
impl Model {
    pub fn pending(&self) -> &[Keystroke] {
        &self.pending
    }
    pub fn pending_seq(&self) -> u64 {
        self.pending_seq
    }
    pub fn version(&self) -> u64 {
        self.version
    }
    pub(crate) fn take_pending(&mut self) -> Vec<Keystroke> {
        std::mem::take(&mut self.pending)
    }
    pub(crate) fn set_pending(&mut self, pending: Vec<Keystroke>) {
        self.pending = pending;
    }
    pub(crate) fn clear_pending(&mut self) {
        self.pending.clear();
    }
    pub(crate) fn bump_pending(&mut self) {
        self.pending_seq += 1;
    }
    pub(crate) fn reloaded(&mut self) {
        self.version += 1;
    }
}
