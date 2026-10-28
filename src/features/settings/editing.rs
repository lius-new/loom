//! Code editing preferences, shared by user, project and language overrides.
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Ending {
    #[default]
    Lf,
    Crlf,
}
impl Ending {
    pub fn core(self) -> loom_editor_core::text::LineEnding {
        match self {
            Self::Lf => loom_editor_core::text::LineEnding::Lf,
            Self::Crlf => loom_editor_core::text::LineEnding::Crlf,
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct EditorSettings {
    pub indent_size: usize,
    pub tab_width: usize,
    pub use_tabs: bool,
    pub auto_pairs: bool,
    pub new_file_line_ending: Ending,
}
impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            indent_size: 2,
            tab_width: 4,
            use_tabs: false,
            auto_pairs: true,
            new_file_line_ending: Ending::Lf,
        }
    }
}
impl EditorSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=16).contains(&self.indent_size) || !(1..=16).contains(&self.tab_width) {
            Err("editor: indent_size and tab_width must be between 1 and 16".into())
        } else {
            Ok(())
        }
    }
    pub fn core(self) -> loom_editor_core::text::EditingOptions {
        loom_editor_core::text::EditingOptions {
            language: "text",
            indent_size: self.indent_size,
            tab_width: self.tab_width,
            use_tabs: self.use_tabs,
            auto_pairs: self.auto_pairs,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct EditorOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indent_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_width: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_tabs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_pairs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_file_line_ending: Option<Ending>,
}
impl EditorOverrides {
    pub fn apply(self, s: &mut EditorSettings) {
        if let Some(v) = self.indent_size {
            s.indent_size = v;
        }
        if let Some(v) = self.tab_width {
            s.tab_width = v;
        }
        if let Some(v) = self.use_tabs {
            s.use_tabs = v;
        }
        if let Some(v) = self.auto_pairs {
            s.auto_pairs = v;
        }
        if let Some(v) = self.new_file_line_ending {
            s.new_file_line_ending = v;
        }
    }
    pub fn validate(self) -> Result<(), String> {
        let mut s = EditorSettings::default();
        self.apply(&mut s);
        s.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overrides_inherit_and_invalid_widths_and_endings_are_reported() {
        let mut settings = EditorSettings {
            tab_width: 8,
            ..Default::default()
        };
        serde_json::from_str::<EditorOverrides>(r#"{"indent_size":4}"#)
            .unwrap()
            .apply(&mut settings);
        assert_eq!(settings.tab_width, 8);
        assert_eq!(settings.indent_size, 4);
        for source in [
            r#"{"editor":{"tab_width":0}}"#,
            r#"{"editor_languages":{"rust":{"indent_size":17}}}"#,
            r#"{"editor":{"new_file_line_ending":"cr"}}"#,
        ] {
            let parsed = crate::features::settings::rules::parse(source).unwrap();
            assert!(!parsed.errors.is_empty(), "{source}");
        }
    }
}
