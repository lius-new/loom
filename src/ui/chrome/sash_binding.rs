use crate::domain::pane_layout::Sash;
use std::sync::Arc;
pub enum Command {
    Start(Sash),
    Drag(f32, f32),
    End,
}
#[derive(Clone)]
pub struct Binding {
    active: Arc<dyn Fn(&Sash) -> bool + Send + Sync>,
    run: Arc<dyn Fn(Command) + Send + Sync>,
}
impl Binding {
    pub fn new(
        active: impl Fn(&Sash) -> bool + Send + Sync + 'static,
        run: impl Fn(Command) + Send + Sync + 'static,
    ) -> Self {
        Self {
            active: Arc::new(active),
            run: Arc::new(run),
        }
    }
    pub fn active(&self, s: &Sash) -> bool {
        (self.active)(s)
    }
    pub fn run(&self, c: Command) {
        (self.run)(c)
    }
}
