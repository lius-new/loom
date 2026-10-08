//! Every keyboard-reachable command, named the way keymap files refer to it
//! (`namespace::Name`).
//!
//! Actions are a closed enum rather than a dynamic registry: Loom has no
//! extensions, and the compiler then checks that every action is handled.

use serde_json::Value;

use crate::editor::commands::Command;
use crate::editor::normal::Movement;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionMeta {
    pub name: &'static str,
    pub description: &'static str,
    /// Whether the action is written `["name", argument]` in keymap files.
    pub takes_argument: bool,
}

macro_rules! actions {
    ($($variant:ident => $name:literal, $description:literal;)*) => {
        // `NoAction` keeps Zed's name for what `null` binds to.
        #[allow(clippy::enum_variant_names)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Action {
            $($variant,)*
            /// `["pane::ActivateItem", index]`: select a tab by position.
            ActivateItem(usize),
            /// `null` in a keymap file: disables the keystroke in that context.
            NoAction,
        }

        /// Actions that keymap files may name, in display order.
        pub const ACTIONS: &[ActionMeta] = &[
            $(ActionMeta { name: $name, description: $description, takes_argument: false },)*
            ActionMeta {
                name: "pane::ActivateItem",
                description: "Activate the tab at a zero-based position",
                takes_argument: true,
            },
        ];

        fn unit_action(name: &str) -> Option<Action> {
            match name {
                $($name => Some(Action::$variant),)*
                _ => None,
            }
        }

        impl Action {
            pub fn name(self) -> &'static str {
                match self {
                    $(Action::$variant => $name,)*
                    Action::ActivateItem(_) => "pane::ActivateItem",
                    Action::NoAction => "null",
                }
            }
        }
    };
}

actions! {
    ToggleDrawer => "workspace::ToggleDrawer", "Show or hide the Explorer drawer";
    OpenExplorer => "workspace::OpenExplorer", "Show the Explorer drawer";
    OpenSourceControl => "workspace::OpenSourceControl", "Show the Source Control panel";
    ToggleTerminal => "workspace::ToggleTerminal", "Show or hide the terminal panel";
    OpenSettings => "workspace::OpenSettings", "Open the Settings page";
    OpenKeymap => "workspace::OpenKeymap", "Open the Keymap page";
    OpenKeymapFile => "workspace::OpenKeymapFile", "Open the user keymap.json";
    CloneRepository => "workspace::CloneRepository", "Clone a Git repository";
    CloseOverlay => "workspace::CloseOverlay", "Close menus and transient overlays";
    ActivatePaneLeft => "workspace::ActivatePaneLeft", "Focus the pane to the left";
    ActivatePaneRight => "workspace::ActivatePaneRight", "Focus the pane to the right";
    ActivatePaneUp => "workspace::ActivatePaneUp", "Focus the pane above";
    ActivatePaneDown => "workspace::ActivatePaneDown", "Focus the pane below";
    ActivateNextPane => "workspace::ActivateNextPane", "Focus the next pane";
    ActivatePrevPane => "workspace::ActivatePrevPane", "Focus the previous pane";

    ActivateNextItem => "pane::ActivateNextItem", "Activate the next tab";
    ActivatePrevItem => "pane::ActivatePrevItem", "Activate the previous tab";
    ActivateLastItem => "pane::ActivateLastItem", "Activate the last tab";
    CloseActiveItem => "pane::CloseActiveItem", "Close the active tab";
    SplitRight => "pane::SplitRight", "Split the pane, copying the active tab to the right";
    SplitLeft => "pane::SplitLeft", "Split the pane, copying the active tab to the left";
    SplitUp => "pane::SplitUp", "Split the pane, copying the active tab above";
    SplitDown => "pane::SplitDown", "Split the pane, copying the active tab below";
    MoveItemToPaneLeft => "pane::MoveItemToPaneLeft", "Move the active tab to the pane on the left";
    MoveItemToPaneRight => "pane::MoveItemToPaneRight", "Move the active tab to the pane on the right";
    MoveItemToPaneUp => "pane::MoveItemToPaneUp", "Move the active tab to the pane above";
    MoveItemToPaneDown => "pane::MoveItemToPaneDown", "Move the active tab to the pane below";
    ClosePane => "pane::ClosePane", "Close the focused pane and its tabs";

    Save => "editor::Save", "Save the active file";
    Undo => "editor::Undo", "Undo";
    Redo => "editor::Redo", "Redo";
    SelectAll => "editor::SelectAll", "Select all text";
    Copy => "editor::Copy", "Copy the selection";
    Cut => "editor::Cut", "Cut the selection";
    Paste => "editor::Paste", "Paste from the clipboard";
    Cancel => "editor::Cancel", "Clear the selection or close the editor menu";
    MoveLeft => "editor::MoveLeft", "Move the cursor left";
    MoveRight => "editor::MoveRight", "Move the cursor right";
    MoveUp => "editor::MoveUp", "Move the cursor up";
    MoveDown => "editor::MoveDown", "Move the cursor down";
    SelectLeft => "editor::SelectLeft", "Extend the selection left";
    SelectRight => "editor::SelectRight", "Extend the selection right";
    SelectUp => "editor::SelectUp", "Extend the selection up";
    SelectDown => "editor::SelectDown", "Extend the selection down";
    MoveToPreviousWordStart => "editor::MoveToPreviousWordStart", "Move to the previous word start";
    MoveToNextWordEnd => "editor::MoveToNextWordEnd", "Move to the next word end";
    SelectToPreviousWordStart => "editor::SelectToPreviousWordStart", "Select to the previous word start";
    SelectToNextWordEnd => "editor::SelectToNextWordEnd", "Select to the next word end";
    MoveToBeginningOfLine => "editor::MoveToBeginningOfLine", "Move to the start of the line";
    MoveToEndOfLine => "editor::MoveToEndOfLine", "Move to the end of the line";
    SelectToBeginningOfLine => "editor::SelectToBeginningOfLine", "Select to the start of the line";
    SelectToEndOfLine => "editor::SelectToEndOfLine", "Select to the end of the line";
    MoveToBeginning => "editor::MoveToBeginning", "Move to the start of the file";
    MoveToEnd => "editor::MoveToEnd", "Move to the end of the file";
    SelectToBeginning => "editor::SelectToBeginning", "Select to the start of the file";
    SelectToEnd => "editor::SelectToEnd", "Select to the end of the file";
    MovePageUp => "editor::MovePageUp", "Move up one page";
    MovePageDown => "editor::MovePageDown", "Move down one page";
    SelectPageUp => "editor::SelectPageUp", "Select up one page";
    SelectPageDown => "editor::SelectPageDown", "Select down one page";
    Backspace => "editor::Backspace", "Delete the previous character";
    Delete => "editor::Delete", "Delete the next character";
    DeleteToPreviousWordStart => "editor::DeleteToPreviousWordStart", "Delete to the previous word start";
    DeleteToNextWordEnd => "editor::DeleteToNextWordEnd", "Delete to the next word end";
    Newline => "editor::Newline", "Insert a line break";
    Tab => "editor::Tab", "Indent";
    Backtab => "editor::Backtab", "Outdent";

    TerminalCopy => "terminal::Copy", "Copy the terminal selection";
    TerminalCopyOrInterrupt => "terminal::CopyOrInterrupt", "Copy the selection, or send Ctrl+C without one";
    TerminalPaste => "terminal::Paste", "Paste into the terminal";

    MenuCancel => "menu::Cancel", "Close the open menu";

    DialogConfirm => "dialog::Confirm", "Confirm the open dialog";
    DialogCancel => "dialog::Cancel", "Dismiss the open dialog";
    DialogSecondary => "dialog::Secondary", "Choose the dialog's secondary option";
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionError(pub String);

impl Action {
    /// Parse a keymap value: `"name"`, `["name", argument]` or `null`.
    pub fn from_json(value: &Value) -> Result<Self, ActionError> {
        match value {
            Value::Null => Ok(Action::NoAction),
            Value::String(name) => {
                if ACTIONS
                    .iter()
                    .any(|meta| meta.name == name && meta.takes_argument)
                {
                    return Err(ActionError(format!(
                        "`{name}` needs an argument: [\"{name}\", …]"
                    )));
                }
                unit_action(name).ok_or_else(|| ActionError(format!("unknown action `{name}`")))
            }
            Value::Array(items) => {
                let [Value::String(name), argument] = items.as_slice() else {
                    return Err(ActionError(
                        "expected [\"action::Name\", argument]".to_owned(),
                    ));
                };
                match name.as_str() {
                    "pane::ActivateItem" => argument
                        .as_u64()
                        .map(|index| Action::ActivateItem(index as usize))
                        .ok_or_else(|| {
                            ActionError(
                                "`pane::ActivateItem` needs a non-negative index".to_owned(),
                            )
                        }),
                    _ if unit_action(name).is_some() => {
                        Err(ActionError(format!("`{name}` does not take an argument")))
                    }
                    _ => Err(ActionError(format!("unknown action `{name}`"))),
                }
            }
            _ => Err(ActionError(
                "expected an action name, [name, argument] or null".to_owned(),
            )),
        }
    }

    pub fn namespace(self) -> &'static str {
        self.name().split("::").next().unwrap_or_default()
    }

    /// The editor primitive an `editor::` action performs. `page` is the
    /// number of visible lines, used by page movements.
    pub fn editor_command(self, page: usize) -> Option<Command> {
        use Movement::*;
        Some(match self {
            Action::Undo => Command::Undo,
            Action::Redo => Command::Redo,
            Action::SelectAll => Command::SelectAll,
            Action::Copy => Command::Copy,
            Action::Cut => Command::Cut,
            Action::Paste => Command::Paste,
            Action::Cancel => Command::Escape,
            Action::MoveLeft => Command::Move(Left, false),
            Action::MoveRight => Command::Move(Right, false),
            Action::MoveUp => Command::Move(Up, false),
            Action::MoveDown => Command::Move(Down, false),
            Action::SelectLeft => Command::Move(Left, true),
            Action::SelectRight => Command::Move(Right, true),
            Action::SelectUp => Command::Move(Up, true),
            Action::SelectDown => Command::Move(Down, true),
            Action::MoveToPreviousWordStart => Command::Move(WordLeft, false),
            Action::MoveToNextWordEnd => Command::Move(WordRight, false),
            Action::SelectToPreviousWordStart => Command::Move(WordLeft, true),
            Action::SelectToNextWordEnd => Command::Move(WordRight, true),
            Action::MoveToBeginningOfLine => Command::Move(Home, false),
            Action::MoveToEndOfLine => Command::Move(End, false),
            Action::SelectToBeginningOfLine => Command::Move(Home, true),
            Action::SelectToEndOfLine => Command::Move(End, true),
            Action::MoveToBeginning => Command::Move(Start, false),
            Action::MoveToEnd => Command::Move(Finish, false),
            Action::SelectToBeginning => Command::Move(Start, true),
            Action::SelectToEnd => Command::Move(Finish, true),
            Action::MovePageUp => Command::Move(PageUp(page), false),
            Action::MovePageDown => Command::Move(PageDown(page), false),
            Action::SelectPageUp => Command::Move(PageUp(page), true),
            Action::SelectPageDown => Command::Move(PageDown(page), true),
            Action::Backspace => Command::Delete(false, false),
            Action::Delete => Command::Delete(true, false),
            Action::DeleteToPreviousWordStart => Command::Delete(false, true),
            Action::DeleteToNextWordEnd => Command::Delete(true, true),
            Action::Newline => Command::Enter,
            Action::Tab => Command::Indent(false),
            Action::Backtab => Command::Indent(true),
            _ => return None,
        })
    }

    /// The `editor::` action that performs a context-menu command, so menu
    /// labels can show its binding.
    pub fn for_editor_command(command: Command) -> Option<Self> {
        Some(match command {
            Command::Undo => Action::Undo,
            Command::Redo => Action::Redo,
            Command::SelectAll => Action::SelectAll,
            Command::Copy => Action::Copy,
            Command::Cut => Action::Cut,
            Command::Paste => Action::Paste,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_round_trip_for_every_listed_action() {
        for meta in ACTIONS {
            let value = if meta.takes_argument {
                json!([meta.name, 0])
            } else {
                json!(meta.name)
            };
            let action = Action::from_json(&value).unwrap();
            assert_eq!(action.name(), meta.name);
        }
    }

    #[test]
    fn parses_arguments_null_and_reports_mistakes() {
        assert_eq!(
            Action::from_json(&json!(["pane::ActivateItem", 3])),
            Ok(Action::ActivateItem(3))
        );
        assert_eq!(Action::from_json(&Value::Null), Ok(Action::NoAction));
        assert!(Action::from_json(&json!("pane::ActivateItem")).is_err());
        assert!(Action::from_json(&json!(["pane::ActivateItem", -1])).is_err());
        assert!(Action::from_json(&json!(["editor::Undo", 1])).is_err());
        assert!(Action::from_json(&json!("editor::Nope")).is_err());
        assert!(Action::from_json(&json!(42)).is_err());
    }

    #[test]
    fn editor_actions_map_to_commands() {
        assert_eq!(
            Action::SelectPageDown.editor_command(20),
            Some(Command::Move(Movement::PageDown(20), true))
        );
        assert_eq!(
            Action::Backtab.editor_command(1),
            Some(Command::Indent(true))
        );
        assert_eq!(Action::Save.editor_command(1), None);
        assert_eq!(Action::Save.namespace(), "editor");
    }
}
