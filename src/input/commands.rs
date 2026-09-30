use crate::input::keymap::Action;
use crate::state::AppState;

#[derive(Clone, Copy, Debug)]
pub struct CommandDescriptor {
    pub id: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub action: Action,
    pub enabled: fn(&AppState) -> bool,
}

const fn always(_: &AppState) -> bool {
    true
}

fn can_clone(state: &AppState) -> bool {
    !state.cloning_repository
}

pub const COMMANDS: &[CommandDescriptor] = &[
    CommandDescriptor {
        id: "workbench.view.explorer",
        title: "Open Explorer",
        category: "View",
        action: Action::OpenExplorer,
        enabled: always,
    },
    CommandDescriptor {
        id: "workbench.view.sourceControl",
        title: "Open Source Control",
        category: "Git",
        action: Action::OpenSourceControl,
        enabled: always,
    },
    CommandDescriptor {
        id: "git.clone",
        title: "Clone Repository",
        category: "Git",
        action: Action::CloneRepository,
        enabled: can_clone,
    },
    CommandDescriptor {
        id: "workbench.action.terminal.toggleTerminal",
        title: "Toggle Terminal",
        category: "View",
        action: Action::ToggleTerminal,
        enabled: always,
    },
];

pub fn all() -> &'static [CommandDescriptor] {
    COMMANDS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_ids_are_unique_and_stable() {
        let mut ids = std::collections::HashSet::new();
        for command in COMMANDS {
            assert!(ids.insert(command.id));
            assert!(!command.title.is_empty());
        }
    }
}
