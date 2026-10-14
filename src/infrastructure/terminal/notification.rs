//! Synchronous frame notification supplied by the application.
use std::sync::Arc;
#[derive(Clone)]
pub(crate) struct FrameNotifier(Arc<dyn Fn() + Send + Sync>);
impl FrameNotifier {
    pub fn new(callback: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(callback))
    }
    pub fn request_frame(&self) {
        (self.0)();
    }
}
impl From<&FrameNotifier> for FrameNotifier {
    fn from(value: &FrameNotifier) -> Self {
        value.clone()
    }
}
