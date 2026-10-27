use crate::domain::encoding::TextFormat;
use std::sync::Arc;
#[derive(Clone)]
pub struct Frame {
    pub selected: usize,
    pub hovered: Option<usize>,
    pub current: Option<TextFormat>,
    pub suggested: Option<TextFormat>,
    pub formats: Vec<TextFormat>,
}
pub enum Command {
    Close,
    Select(TextFormat),
    Scroll(bool),
    Hover(Option<usize>),
}
#[derive(Clone)]
pub struct Binding {
    read: Arc<dyn Fn() -> Option<Frame> + Send + Sync>,
    execute: Arc<dyn Fn(Command) + Send + Sync>,
}
impl Binding {
    pub fn new(
        read: impl Fn() -> Option<Frame> + Send + Sync + 'static,
        execute: impl Fn(Command) + Send + Sync + 'static,
    ) -> Self {
        Self {
            read: Arc::new(read),
            execute: Arc::new(execute),
        }
    }
    pub fn frame(&self) -> Option<Frame> {
        (self.read)()
    }
    pub fn run(&self, command: Command) {
        (self.execute)(command);
    }
}
