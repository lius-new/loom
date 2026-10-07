//! The keymap: bindings from keystroke sequences to actions, and the rules
//! that decide which binding wins.
//!
//! Resolution follows Zed's `gpui::Keymap` (Apache-2.0):
//! * bindings matching deeper in the context stack win over shallower ones;
//!   bindings without a context count as matching at the deepest level;
//! * at the same depth, later bindings win, so user bindings loaded after the
//!   defaults override them;
//! * `null` disables a keystroke for bindings it out-ranks that come from the
//!   same or a weaker source.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, RwLock};

use super::action::Action;
use super::context::{ContextPredicate, KeyContext};
use super::keystroke::Keystroke;

/// Where a binding came from. Lower values are stronger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BindingSource {
    User = 0,
    Default = 2,
}

impl BindingSource {
    pub fn label(self) -> &'static str {
        match self {
            BindingSource::User => "User",
            BindingSource::Default => "Default",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub keystrokes: Vec<Keystroke>,
    pub action: Action,
    pub predicate: Option<Arc<ContextPredicate>>,
    pub source: BindingSource,
}

impl Binding {
    /// Build a binding from keymap syntax. Panics on invalid input; meant for
    /// tests and built-in bindings.
    pub fn new(keystrokes: &str, action: Action, context: Option<&str>) -> Self {
        Self {
            keystrokes: Keystroke::parse_sequence(keystrokes).expect("valid keystrokes"),
            action,
            predicate: context
                .map(|context| Arc::new(ContextPredicate::parse(context).expect("valid context"))),
            source: BindingSource::Default,
        }
    }

    /// `Some(true)` when `typed` is a strict prefix of this binding,
    /// `Some(false)` when it matches exactly, `None` otherwise.
    pub fn match_keystrokes(&self, typed: &[Keystroke]) -> Option<bool> {
        if self.keystrokes.len() < typed.len() {
            return None;
        }
        self.keystrokes
            .iter()
            .zip(typed)
            .all(|(target, typed)| typed.should_match(target))
            .then_some(self.keystrokes.len() > typed.len())
    }

    pub fn label(&self) -> String {
        Keystroke::sequence_label(&self.keystrokes)
    }

    pub fn context_label(&self) -> String {
        self.predicate
            .as_ref()
            .map(|predicate| predicate.to_string())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Keymap {
    bindings: Vec<Binding>,
    by_action: HashMap<Action, Vec<usize>>,
    disabled: Vec<usize>,
}

impl Keymap {
    pub fn new(bindings: impl IntoIterator<Item = Binding>) -> Self {
        let mut keymap = Self::default();
        keymap.add_bindings(bindings);
        keymap
    }

    pub fn add_bindings(&mut self, bindings: impl IntoIterator<Item = Binding>) {
        for binding in bindings {
            let index = self.bindings.len();
            if binding.action == Action::NoAction {
                self.disabled.push(index);
            } else {
                self.by_action
                    .entry(binding.action)
                    .or_default()
                    .push(index);
            }
            self.bindings.push(binding);
        }
    }

    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// Bindings matching `input` in precedence order, and whether a longer
    /// binding could still match if more keys were typed.
    pub fn bindings_for_input(
        &self,
        input: &[Keystroke],
        context_stack: &[KeyContext],
    ) -> (Vec<&Binding>, bool) {
        let mut matched = Vec::new();
        let mut pending_bindings = Vec::new();
        for (index, binding) in self.bindings.iter().enumerate().rev() {
            let Some(depth) = binding_depth(binding, context_stack) else {
                continue;
            };
            match binding.match_keystrokes(input) {
                Some(false) => matched.push((depth, index, binding)),
                Some(true) => pending_bindings.push((index, binding)),
                None => {}
            }
        }
        matched.sort_by(|(depth_a, index_a, _), (depth_b, index_b, _)| {
            depth_b.cmp(depth_a).then(index_b.cmp(index_a))
        });

        let mut bindings = Vec::new();
        let mut first_binding_index = None;
        // A `null` suppresses out-ranked bindings from sources at least as
        // weak as its own; stronger sources still apply.
        let mut no_action_source: Option<BindingSource> = None;
        for (_, index, binding) in matched {
            if binding.action == Action::NoAction {
                no_action_source = Some(
                    no_action_source
                        .map_or(binding.source, |existing| existing.min(binding.source)),
                );
                continue;
            }
            if no_action_source.is_some_and(|source| binding.source >= source) {
                continue;
            }
            bindings.push(binding);
            first_binding_index.get_or_insert(index);
        }

        let mut pending = HashSet::new();
        for (index, binding) in pending_bindings.into_iter().rev() {
            if first_binding_index.is_some_and(|first| first > index) {
                continue;
            }
            if binding.action == Action::NoAction {
                pending.remove(&binding.keystrokes);
                continue;
            }
            pending.insert(&binding.keystrokes);
        }

        (bindings, !pending.is_empty())
    }

    /// Bindings for `action` in the order they were added, minus those a
    /// later `null` disables. The last one is the one to display.
    pub fn bindings_for_action(&self, action: Action) -> impl DoubleEndedIterator<Item = &Binding> {
        self.by_action
            .get(&action)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter(move |&&index| {
                let binding = &self.bindings[index];
                !self.disabled.iter().any(|&disabled_index| {
                    let disabled = &self.bindings[disabled_index];
                    disabled_index > index
                        && disabled.keystrokes == binding.keystrokes
                        && disabled_matches_context(disabled, binding)
                })
            })
            .map(|&index| &self.bindings[index])
    }

    /// Every binding that runs an action, in keymap order, minus those a later
    /// `null` disables. Used to list shortcuts.
    pub fn active_bindings(&self) -> impl Iterator<Item = &Binding> {
        self.bindings
            .iter()
            .enumerate()
            .filter_map(|(index, binding)| {
                let disabled = binding.action == Action::NoAction
                    || self.disabled.iter().any(|&disabled_index| {
                        let disabled = &self.bindings[disabled_index];
                        disabled_index > index
                            && disabled.keystrokes == binding.keystrokes
                            && disabled_matches_context(disabled, binding)
                    });
                (!disabled).then_some(binding)
            })
    }

    /// The binding that would actually trigger `action` in `context_stack`,
    /// for menu and tooltip labels.
    pub fn binding_for_action(
        &self,
        action: Action,
        context_stack: &[KeyContext],
    ) -> Option<&Binding> {
        self.bindings_for_action(action).rev().find(|binding| {
            binding_depth(binding, context_stack).is_some()
                && self
                    .bindings_for_input(&binding.keystrokes, context_stack)
                    .0
                    .first()
                    .is_some_and(|winner| winner.action == action)
        })
    }
}

fn binding_depth(binding: &Binding, context_stack: &[KeyContext]) -> Option<usize> {
    match &binding.predicate {
        Some(predicate) => predicate.depth_of(context_stack),
        None => Some(context_stack.len()),
    }
}

fn disabled_matches_context(disabled: &Binding, binding: &Binding) -> bool {
    match (&disabled.predicate, &binding.predicate) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(disabled), Some(predicate)) => disabled.is_superset(predicate),
    }
}

// ---- Active keymap ---------------------------------------------------------

fn active() -> &'static RwLock<Arc<Keymap>> {
    static ACTIVE: OnceLock<RwLock<Arc<Keymap>>> = OnceLock::new();
    ACTIVE.get_or_init(|| RwLock::new(Arc::new(super::keymap_file::default_keymap())))
}

/// The keymap currently used for dispatch and labels.
pub fn current() -> Arc<Keymap> {
    active().read().expect("keymap lock poisoned").clone()
}

/// Replace the active keymap (after loading or reloading `keymap.json`).
pub fn install(keymap: Keymap) {
    *active().write().expect("keymap lock poisoned") = Arc::new(keymap);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(source: &str) -> Vec<Keystroke> {
        Keystroke::parse_sequence(source).unwrap()
    }

    fn ctx(source: &str) -> KeyContext {
        KeyContext::parse(source).unwrap()
    }

    fn user(mut binding: Binding) -> Binding {
        binding.source = BindingSource::User;
        binding
    }

    fn actions(bindings: &[&Binding]) -> Vec<Action> {
        bindings.iter().map(|binding| binding.action).collect()
    }

    #[test]
    fn deeper_contexts_take_precedence() {
        let keymap = Keymap::new([
            Binding::new("ctrl-a", Action::Undo, Some("pane")),
            Binding::new("ctrl-a", Action::Redo, Some("editor")),
        ]);
        let (bindings, pending) =
            keymap.bindings_for_input(&ks("ctrl-a"), &[ctx("pane"), ctx("editor")]);
        assert!(!pending);
        assert_eq!(actions(&bindings), [Action::Redo, Action::Undo]);
    }

    #[test]
    fn global_bindings_match_everywhere_at_the_deepest_level() {
        let keymap = Keymap::new([
            Binding::new("ctrl-a", Action::Undo, Some("editor")),
            Binding::new("ctrl-a", Action::Redo, None),
        ]);
        let (bindings, _) = keymap.bindings_for_input(&ks("ctrl-a"), &[ctx("editor")]);
        assert_eq!(actions(&bindings), [Action::Redo, Action::Undo]);
        let (bindings, _) = keymap.bindings_for_input(&ks("ctrl-a"), &[ctx("terminal")]);
        assert_eq!(actions(&bindings), [Action::Redo]);
    }

    #[test]
    fn later_bindings_win_at_the_same_depth() {
        let keymap = Keymap::new([
            Binding::new("ctrl-a", Action::Undo, Some("editor")),
            user(Binding::new("ctrl-a", Action::Redo, Some("editor"))),
        ]);
        let (bindings, _) = keymap.bindings_for_input(&ks("ctrl-a"), &[ctx("editor")]);
        assert_eq!(bindings[0].action, Action::Redo);
    }

    #[test]
    fn null_disables_bindings_in_its_context() {
        let keymap = Keymap::new([
            Binding::new("ctrl-a", Action::Undo, Some("editor")),
            Binding::new("ctrl-b", Action::Undo, Some("editor")),
            Binding::new("ctrl-a", Action::NoAction, Some("editor && mode==full")),
            Binding::new("ctrl-b", Action::NoAction, None),
        ]);
        let resolve = |keys: &str, context: &str| {
            keymap
                .bindings_for_input(&ks(keys), &[ctx(context)])
                .0
                .is_empty()
        };
        assert!(resolve("ctrl-a", "barf"));
        assert!(!resolve("ctrl-a", "editor"));
        assert!(resolve("ctrl-a", "editor mode=full"));
        assert!(resolve("ctrl-b", "editor"));
    }

    #[test]
    fn null_only_disables_weaker_or_equal_sources() {
        let keymap = Keymap::new([
            user(Binding::new("ctrl-a", Action::Undo, Some("Workspace"))),
            Binding::new("ctrl-a", Action::NoAction, Some("Terminal")),
        ]);
        let stack = [ctx("Workspace"), ctx("Terminal")];
        let (bindings, _) = keymap.bindings_for_input(&ks("ctrl-a"), &stack);
        assert_eq!(actions(&bindings), [Action::Undo]);

        let keymap = Keymap::new([
            Binding::new("ctrl-a", Action::Undo, Some("Workspace")),
            Binding::new("ctrl-a", Action::NoAction, Some("Terminal")),
        ]);
        assert!(
            keymap
                .bindings_for_input(&ks("ctrl-a"), &stack)
                .0
                .is_empty()
        );
    }

    #[test]
    fn a_shallower_null_does_not_disable_a_deeper_binding() {
        let keymap = Keymap::new([
            Binding::new("ctrl-a", Action::Undo, Some("editor")),
            Binding::new("ctrl-a", Action::NoAction, Some("workspace")),
        ]);
        let (bindings, _) =
            keymap.bindings_for_input(&ks("ctrl-a"), &[ctx("workspace"), ctx("editor")]);
        assert_eq!(actions(&bindings), [Action::Undo]);
    }

    #[test]
    fn prefixes_report_pending() {
        let keymap = Keymap::new([
            Binding::new("ctrl-k", Action::Undo, Some("Workspace")),
            Binding::new("ctrl-k ctrl-s", Action::OpenKeymapFile, Some("Workspace")),
        ]);
        let stack = [ctx("Workspace")];
        let (bindings, pending) = keymap.bindings_for_input(&ks("ctrl-k"), &stack);
        assert!(pending);
        assert_eq!(actions(&bindings), [Action::Undo]);
        let (bindings, pending) = keymap.bindings_for_input(&ks("ctrl-k ctrl-s"), &stack);
        assert!(!pending);
        assert_eq!(actions(&bindings), [Action::OpenKeymapFile]);
        let (bindings, pending) = keymap.bindings_for_input(&ks("ctrl-k x"), &stack);
        assert!(!pending && bindings.is_empty());
    }

    #[test]
    fn a_later_exact_binding_overrides_an_earlier_prefix() {
        // As in Zed, a prefix only stays pending when it is defined after the
        // exact binding it would otherwise shadow.
        let keymap = Keymap::new([
            Binding::new("ctrl-k ctrl-s", Action::OpenKeymapFile, Some("Workspace")),
            user(Binding::new("ctrl-k", Action::Undo, Some("Workspace"))),
        ]);
        let (bindings, pending) = keymap.bindings_for_input(&ks("ctrl-k"), &[ctx("Workspace")]);
        assert!(!pending);
        assert_eq!(actions(&bindings), [Action::Undo]);
    }

    #[test]
    fn null_on_a_sequence_cancels_pending() {
        let keymap = Keymap::new([
            Binding::new("space w w", Action::Undo, Some("workspace")),
            Binding::new("space w w", Action::NoAction, Some("editor")),
        ]);
        let workspace = [ctx("workspace")];
        let editor = [ctx("workspace"), ctx("editor")];
        assert!(keymap.bindings_for_input(&ks("space"), &workspace).1);
        assert!(!keymap.bindings_for_input(&ks("space"), &editor).1);
        assert!(
            !keymap
                .bindings_for_input(&ks("space w w"), &workspace)
                .0
                .is_empty()
        );
        assert!(
            keymap
                .bindings_for_input(&ks("space w w"), &editor)
                .0
                .is_empty()
        );
    }

    #[test]
    fn display_binding_respects_context_and_overrides() {
        let keymap = Keymap::new([
            Binding::new("ctrl-z", Action::Undo, Some("Editor")),
            user(Binding::new("ctrl-u", Action::Undo, Some("Editor"))),
            Binding::new("ctrl-y", Action::Redo, Some("Editor")),
            user(Binding::new("ctrl-y", Action::NoAction, Some("Editor"))),
        ]);
        let stack = [ctx("Workspace"), ctx("Editor")];
        assert_eq!(
            keymap
                .binding_for_action(Action::Undo, &stack)
                .map(Binding::label),
            Some(Keystroke::sequence_label(&ks("ctrl-u")))
        );
        assert!(keymap.binding_for_action(Action::Redo, &stack).is_none());
        assert!(
            keymap
                .binding_for_action(Action::Undo, &[ctx("Workspace")])
                .is_none()
        );
        assert_eq!(keymap.bindings_for_action(Action::Undo).count(), 2);
        assert_eq!(keymap.bindings_for_action(Action::Redo).count(), 0);
    }
}
