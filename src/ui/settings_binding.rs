//! Settings value frame, projected drafts and semantic command port.
use super::settings::{FontField, FontTarget, Setting};
use crate::features::settings::rules::Settings;
use crate::features::settings::theme_id::ThemeId;
use crate::ui::components::{input::InputState, state_access::StateAccess};
use std::sync::Arc;
#[derive(Clone, Default)]
pub struct SettingsUiState {
    pub editor_font_input: InputState,
    pub terminal_font_input: InputState,
}
#[derive(Clone)]
pub struct Frame {
    pub prefs: Settings,
    pub scroll: (f32, f32),
    pub profile: crate::ui::terminal::settings_binding::Frame,
    pub terminal_ui: crate::ui::terminal::state::TerminalUiState,
}
pub enum Command {
    Toggle(Setting),
    Theme(ThemeId),
    FontPreset(FontTarget, String),
    FontStep(FontTarget, FontField, bool),
    FontCommit(FontTarget),
    FontCancel(FontTarget),
    FontFocus,
    Scroll { delta: f32, max: f32 },
    OpenUserFile,
    OpenKeymap,
}
#[derive(Clone)]
pub struct Binding {
    pub ui: StateAccess<SettingsUiState>,
    pub profile: crate::ui::terminal::settings_binding::Binding,
    read: Arc<dyn Fn() -> Frame + Send + Sync>,
    run: Arc<dyn Fn(Command) + Send + Sync>,
}
impl Binding {
    pub fn new(
        ui: StateAccess<SettingsUiState>,
        profile: crate::ui::terminal::settings_binding::Binding,
        read: impl Fn() -> Frame + Send + Sync + 'static,
        run: impl Fn(Command) + Send + Sync + 'static,
    ) -> Self {
        Self {
            ui,
            profile,
            read: Arc::new(read),
            run: Arc::new(run),
        }
    }
    pub fn frame(&self) -> Frame {
        (self.read)()
    }
    pub fn run(&self, c: Command) {
        (self.run)(c);
    }
}
