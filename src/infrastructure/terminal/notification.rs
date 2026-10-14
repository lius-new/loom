//! Synchronous frame notification supplied by the application.
use std::sync::Arc;
#[derive(Clone)]
pub(crate) struct FrameNotifier(Arc<dyn Fn(Option<u64>) + Send + Sync>);
impl FrameNotifier {
    pub fn new(callback: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(move |_| callback()))
    }
    pub fn for_generation(callback: impl Fn(u64) + Send + Sync + 'static) -> Self {
        Self(Arc::new(move |generation| {
            if let Some(generation) = generation {
                callback(generation);
            }
        }))
    }
    pub fn request_frame_for(&self, generation: u64) {
        (self.0)(Some(generation));
    }
    pub fn request_frame(&self) {
        (self.0)(None);
    }
}
impl From<&FrameNotifier> for FrameNotifier {
    fn from(value: &FrameNotifier) -> Self {
        value.clone()
    }
}
