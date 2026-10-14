//! Presentation of the default terminal region.
#[derive(Clone)]
pub struct TerminalChrome {
    pub show_terminal: bool,
    pub terminal_h: f32,
    pub resizing_terminal: bool,
}
impl Default for TerminalChrome {
    fn default() -> Self {
        Self {
            show_terminal: false,
            terminal_h: crate::theme::TERMINAL_H,
            resizing_terminal: false,
        }
    }
}
