//! Multi-keystroke dispatch: tracks pending sequences and decides what each
//! keystroke does.
//!
//! Follows Zed's `DispatchTree::dispatch_key` (Apache-2.0). The caller keeps
//! the pending keystrokes between calls; this module is pure so the state
//! machine can be tested without a window.

use std::time::Duration;

use super::action::Action;
use super::context::KeyContext;
use super::keymap::Keymap;
use super::keystroke::Keystroke;

/// How long a pending sequence waits for its next keystroke.
pub const PENDING_TIMEOUT: Duration = Duration::from_millis(1000);

/// A keystroke from an abandoned sequence, to run before the current one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replay {
    pub keystroke: Keystroke,
    /// Actions bound to the sequence ending at this keystroke, in precedence
    /// order. Empty means the keystroke should be replayed as typed input.
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DispatchResult {
    /// Keystrokes to keep waiting on. Non-empty means the current keystroke
    /// was consumed as part of a sequence.
    pub pending: Vec<Keystroke>,
    /// Whether the pending keystrokes alone already match a binding, which
    /// runs if the sequence times out.
    pub pending_has_binding: bool,
    /// Actions for the current keystroke, in precedence order.
    pub actions: Vec<Action>,
    /// Earlier keystrokes to run, in order, before the current one.
    pub replay: Vec<Replay>,
}

impl DispatchResult {
    /// No binding claimed the keystroke; it belongs to the focused element.
    pub fn is_unbound(&self) -> bool {
        self.pending.is_empty() && self.actions.is_empty()
    }
}

fn actions_for(keymap: &Keymap, input: &[Keystroke], stack: &[KeyContext]) -> (Vec<Action>, bool) {
    let (bindings, pending) = keymap.bindings_for_input(input, stack);
    (
        bindings.iter().map(|binding| binding.action).collect(),
        pending,
    )
}

/// Process `keystroke` given the keystrokes still `pending` from earlier calls.
pub fn dispatch_key(
    keymap: &Keymap,
    mut pending: Vec<Keystroke>,
    keystroke: Keystroke,
    stack: &[KeyContext],
) -> DispatchResult {
    pending.push(keystroke.clone());
    let (actions, is_pending) = actions_for(keymap, &pending, stack);
    if is_pending {
        return DispatchResult {
            pending,
            pending_has_binding: !actions.is_empty(),
            ..Default::default()
        };
    }
    if !actions.is_empty() {
        return DispatchResult {
            actions,
            ..Default::default()
        };
    }
    if pending.len() == 1 {
        return DispatchResult::default();
    }
    pending.pop();
    let (suffix, mut replay) = replay_prefix(keymap, pending, stack);
    let mut result = dispatch_key(keymap, suffix, keystroke, stack);
    replay.append(&mut result.replay);
    result.replay = replay;
    result
}

/// Resolve a sequence that timed out (or lost focus) into replays.
pub fn flush(keymap: &Keymap, pending: Vec<Keystroke>, stack: &[KeyContext]) -> Vec<Replay> {
    let mut replay = Vec::new();
    let mut input = pending;
    while !input.is_empty() {
        let (rest, mut prefix) = replay_prefix(keymap, input, stack);
        replay.append(&mut prefix);
        input = rest;
    }
    replay
}

/// Turn the longest bound prefix of `input` into one replay (or, without
/// one, the first keystroke as typed input) and return the remainder.
fn replay_prefix(
    keymap: &Keymap,
    mut input: Vec<Keystroke>,
    stack: &[KeyContext],
) -> (Vec<Keystroke>, Vec<Replay>) {
    for last in (0..input.len()).rev() {
        let (actions, _) = actions_for(keymap, &input[..=last], stack);
        if !actions.is_empty() {
            let keystroke = input.drain(..=last).next_back().expect("non-empty prefix");
            return (input, vec![Replay { keystroke, actions }]);
        }
    }
    let keystroke = input.remove(0);
    (
        input,
        vec![Replay {
            keystroke,
            actions: Vec::new(),
        }],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::keymap::Binding;

    fn ks(source: &str) -> Keystroke {
        Keystroke::parse(source).unwrap()
    }

    fn stack() -> Vec<KeyContext> {
        vec![KeyContext::new("Workspace"), KeyContext::new("Editor")]
    }

    fn keymap() -> Keymap {
        Keymap::new([
            Binding::new("ctrl-k", Action::Undo, Some("Workspace")),
            Binding::new("ctrl-k ctrl-s", Action::OpenKeymapFile, Some("Workspace")),
            Binding::new("ctrl-b", Action::ToggleDrawer, Some("Workspace")),
            Binding::new("j k", Action::Cancel, Some("Editor")),
        ])
    }

    #[test]
    fn single_bindings_dispatch_immediately() {
        let result = dispatch_key(&keymap(), Vec::new(), ks("ctrl-b"), &stack());
        assert_eq!(result.actions, [Action::ToggleDrawer]);
        assert!(result.pending.is_empty() && result.replay.is_empty());
    }

    #[test]
    fn unbound_keys_fall_through() {
        assert!(dispatch_key(&keymap(), Vec::new(), ks("x"), &stack()).is_unbound());
    }

    #[test]
    fn sequences_wait_then_complete() {
        let keymap = keymap();
        let first = dispatch_key(&keymap, Vec::new(), ks("ctrl-k"), &stack());
        assert_eq!(first.pending, [ks("ctrl-k")]);
        assert!(first.pending_has_binding);
        let second = dispatch_key(&keymap, first.pending, ks("ctrl-s"), &stack());
        assert_eq!(second.actions, [Action::OpenKeymapFile]);
        assert!(second.replay.is_empty());
    }

    #[test]
    fn abandoned_sequences_replay_their_bound_prefix() {
        let keymap = keymap();
        let first = dispatch_key(&keymap, Vec::new(), ks("ctrl-k"), &stack());
        let second = dispatch_key(&keymap, first.pending, ks("ctrl-b"), &stack());
        assert_eq!(
            second.replay,
            [Replay {
                keystroke: ks("ctrl-k"),
                actions: vec![Action::Undo],
            }]
        );
        assert_eq!(second.actions, [Action::ToggleDrawer]);
    }

    #[test]
    fn abandoned_text_replays_as_input_and_the_new_key_falls_through() {
        let keymap = keymap();
        let first = dispatch_key(&keymap, Vec::new(), ks("j"), &stack());
        assert_eq!(first.pending, [ks("j")]);
        assert!(!first.pending_has_binding);
        let second = dispatch_key(&keymap, first.pending, ks("x"), &stack());
        assert_eq!(
            second.replay,
            [Replay {
                keystroke: ks("j"),
                actions: Vec::new(),
            }]
        );
        assert!(second.is_unbound());
    }

    #[test]
    fn timeouts_flush_pending_keys() {
        let keymap = keymap();
        assert_eq!(
            flush(&keymap, vec![ks("ctrl-k")], &stack()),
            [Replay {
                keystroke: ks("ctrl-k"),
                actions: vec![Action::Undo],
            }]
        );
        assert_eq!(flush(&keymap, vec![ks("j")], &stack())[0].actions, []);
    }
}
