//! Targeting rules for terminal tab menus.
use super::model::TerminalModel;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    New,
    Split,
    Close,
    Others,
    Right,
    All,
}
pub fn close_targets(tabs: &TerminalModel, target: u64, action: MenuAction) -> Vec<u64> {
    let Some(index) = tabs.tabs().iter().position(|tab| tab.id() == target) else {
        return Vec::new();
    };
    tabs.tabs()
        .iter()
        .enumerate()
        .filter_map(|(i, tab)| {
            match action {
                MenuAction::New | MenuAction::Split => false,
                MenuAction::Close => tab.id() == target,
                MenuAction::Others => tab.group_id() != tabs.tabs()[index].group_id(),
                MenuAction::Right => i > index && tab.group_id() != tabs.tabs()[index].group_id(),
                MenuAction::All => true,
            }
            .then_some(tab.id())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::MenuAction as Action;
    use super::*;
    use crate::features::terminal::{ShellKind, model::TerminalModel};

    #[test]
    fn actions_target_the_clicked_terminal_and_preserve_unclosed_sessions() {
        let mut tabs = TerminalModel::new();
        let first = tabs.active_id().unwrap();
        let middle = tabs.add(ShellKind::Cmd);
        let last = tabs.add(ShellKind::Bash);
        assert_eq!(tabs.active_id(), Some(last));
        assert_eq!(close_targets(&tabs, middle, Action::Close), vec![middle]);
        assert_eq!(
            close_targets(&tabs, middle, Action::Others),
            vec![first, last]
        );
        assert_eq!(close_targets(&tabs, middle, Action::Right), vec![last]);
        assert!(close_targets(&tabs, last, Action::Right).is_empty());
        assert!(close_targets(&tabs, 999, Action::All).is_empty());
        for id in close_targets(&tabs, middle, Action::Others) {
            tabs.close(id);
        }
        assert_eq!(tabs.active_id(), Some(middle));
        assert!(close_targets(&tabs, middle, Action::Others).is_empty());
        let sibling = tabs.split(middle).unwrap();
        let other = tabs.add(ShellKind::Bash);
        assert_eq!(close_targets(&tabs, middle, Action::Others), vec![other]);
        assert_eq!(close_targets(&tabs, middle, Action::Right), vec![other]);
        assert_eq!(close_targets(&tabs, sibling, Action::Close), vec![sibling]);
    }
}
