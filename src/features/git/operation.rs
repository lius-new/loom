//! How the Git panel words an operation Git is in the middle of: what it is
//! called, what finishes it and why it cannot finish yet.

use super::types::RepositoryState;

/// The wording for one kind of unfinished operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperationText {
    /// Present participle, as in "Rebasing".
    pub title: &'static str,
    /// The primary button; `None` for an operation with no continue, which is
    /// ended instead.
    pub continue_label: Option<&'static str>,
    /// The entry that gives the operation up.
    pub abort_label: &'static str,
}

pub fn operation_text(state: RepositoryState) -> Option<OperationText> {
    let (title, continue_label, abort_label) = match state {
        RepositoryState::MergeInProgress => ("Merging", Some("Continue Merge"), "Abort Merge"),
        RepositoryState::RebaseInProgress => ("Rebasing", Some("Continue Rebase"), "Abort Rebase"),
        RepositoryState::CherryPickInProgress => (
            "Cherry-picking",
            Some("Continue Cherry-pick"),
            "Abort Cherry-pick",
        ),
        RepositoryState::RevertInProgress => ("Reverting", Some("Continue Revert"), "Abort Revert"),
        RepositoryState::Bisecting => ("Bisecting", None, "End Bisect"),
        RepositoryState::Normal | RepositoryState::DetachedHead | RepositoryState::UnbornBranch => {
            return None;
        }
    };
    Some(OperationText {
        title,
        continue_label,
        abort_label,
    })
}

/// The operation with its progress, like "Rebasing 1/2".
pub fn title(text: OperationText, step: Option<(u32, u32)>) -> String {
    match step {
        Some((step, total)) => format!("{} {step}/{total}", text.title),
        None => text.title.to_owned(),
    }
}

/// Why the primary button is disabled, or `None` when it can act.
pub fn primary_reason(text: OperationText, conflicted: usize, busy: bool) -> Option<&'static str> {
    if busy {
        Some("Git operation in progress")
    } else if text.continue_label.is_some() && conflicted > 0 {
        Some("Resolve the conflicts first")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_unfinished_operations_have_wording() {
        assert_eq!(operation_text(RepositoryState::Normal), None);
        assert_eq!(operation_text(RepositoryState::DetachedHead), None);
        let rebase = operation_text(RepositoryState::RebaseInProgress).unwrap();
        assert_eq!(rebase.continue_label, Some("Continue Rebase"));
        assert_eq!(rebase.abort_label, "Abort Rebase");
        assert_eq!(title(rebase, Some((1, 2))), "Rebasing 1/2");
        assert_eq!(title(rebase, None), "Rebasing");
    }

    #[test]
    fn bisect_is_ended_not_continued() {
        let bisect = operation_text(RepositoryState::Bisecting).unwrap();
        assert_eq!(bisect.continue_label, None);
        assert_eq!(bisect.abort_label, "End Bisect");
        // Conflicts do not block ending a bisect.
        assert_eq!(primary_reason(bisect, 3, false), None);
    }

    #[test]
    fn continuing_waits_for_conflicts_and_for_git() {
        let merge = operation_text(RepositoryState::MergeInProgress).unwrap();
        assert_eq!(
            primary_reason(merge, 1, false),
            Some("Resolve the conflicts first")
        );
        assert_eq!(primary_reason(merge, 0, false), None);
        assert_eq!(
            primary_reason(merge, 0, true),
            Some("Git operation in progress")
        );
    }
}
