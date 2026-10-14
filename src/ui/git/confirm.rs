//! Modal confirmation for Source Control actions that need a second look:
//! discarding changes, committing when nothing is staged, aborting an
//! operation in progress and dropping a stash.

use std::path::PathBuf;

use lgui::core::{CursorIcon, EventPolicy, SemanticRole, Semantics, UiElement, UiId};
use lgui::prelude::{Element, TextAlign, TextStyle, UiRect, VisualStyle, panel, text};

use crate::features::git::actions::Checkout;
use crate::features::git::diff_document::ChangeBlock;
use crate::features::git::{DiffTarget, RepositoryState};
use crate::ui::git::commands::{Binding, Command, Snapshot};
use crate::ui::theme;

pub(crate) const CARD_W: f32 = 480.0;
pub(crate) const CARD_H: f32 = 156.0;
pub(crate) const WORKTREE_CARD_H: f32 = 258.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitConfirmAction {
    CreateWorktree {
        repository: PathBuf,
        path: PathBuf,
        checkout: Checkout,
    },
    RemoveWorktree {
        repository: PathBuf,
        path: PathBuf,
        force: bool,
    },
    Discard {
        repository: PathBuf,
        tracked: Vec<PathBuf>,
        untracked: Vec<PathBuf>,
    },
    StageAllAndCommit,
    AbortOperation {
        repository: PathBuf,
        operation: RepositoryState,
    },
    DropStash {
        repository: PathBuf,
        oid: String,
    },
    DiscardBlock {
        repository: PathBuf,
        path: PathBuf,
        block: ChangeBlock,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitConfirm {
    pub title: String,
    pub message: String,
    pub confirm_label: &'static str,
    /// Draws the confirm button in the error color.
    pub destructive: bool,
    pub action: GitConfirmAction,
    /// Whether the terminal had focus when the dialog opened.
    pub restore_terminal: bool,
}

/// Close the dialog, performing its action when `confirmed`.
pub(crate) fn resolve(state: &Binding, confirmed: bool) {
    let Some(confirm) = state.get().ui.confirm else {
        return;
    };
    state.update(|app| app.ui.confirm = None);
    if !confirmed {
        return;
    }
    match confirm.action {
        GitConfirmAction::CreateWorktree {
            repository,
            path,
            checkout,
        } => {
            state.dispatch(Command::CreateWorktree {
                root: repository.to_path_buf(),
                path,
                checkout,
            });
        }
        GitConfirmAction::RemoveWorktree {
            repository,
            path,
            force,
        } => {
            state.dispatch(Command::RemoveWorktree {
                root: repository.to_path_buf(),
                path,
                force,
            });
        }
        GitConfirmAction::Discard {
            repository,
            tracked,
            untracked,
        } => {
            state.dispatch(Command::DiscardPaths {
                root: repository.to_path_buf(),
                tracked,
                untracked,
            });
        }
        GitConfirmAction::StageAllAndCommit => {
            state.dispatch(Command::CommitAll);
        }
        GitConfirmAction::AbortOperation {
            repository,
            operation,
        } => {
            state.dispatch(Command::AbortOperation {
                root: repository.to_path_buf(),
                operation,
            });
        }
        GitConfirmAction::DropStash { repository, oid } => {
            state.dispatch(Command::DropStash {
                root: repository.to_path_buf(),
                oid,
            });
        }
        GitConfirmAction::DiscardBlock {
            repository,
            path,
            block,
        } => {
            state.dispatch(Command::ApplyBlock {
                root: repository.to_path_buf(),
                path,
                target: DiffTarget::IndexToWorktree,
                block,
                action: crate::features::git::actions::BlockAction::Discard,
            });
        }
    }
}

pub(crate) fn centered(mut style: TextStyle) -> TextStyle {
    style.align = TextAlign::Center;
    style
}

pub fn render(
    viewport: UiRect,
    confirm: &GitConfirm,
    state: Binding,
    store: Snapshot,
    dialog_id: UiId,
) -> Element {
    let worktree = match &confirm.action {
        GitConfirmAction::CreateWorktree { path, checkout, .. } => Some((path, checkout)),
        _ => None,
    };
    let card_h = if worktree.is_some() {
        WORKTREE_CARD_H
    } else {
        CARD_H
    };
    let card_w = CARD_W.min((viewport.width() - 32.0).max(0.0));
    let center_x = viewport.left + viewport.width() / 2.0;
    let top = viewport.top + (viewport.height() - card_h) * 0.36;
    let card = UiRect::new(
        center_x - card_w / 2.0,
        top,
        center_x + card_w / 2.0,
        top + card_h,
    );

    // The overlay owns keyboard focus while open (see `app.rs`); Enter and
    // Esc arrive as `dialog::` actions and every other key stops here.
    let overlay_style = VisualStyle::filled(theme::c().bg).alpha(214);
    let mut root = Element::new(move |cx| {
        UiElement::panel(dialog_id.clone(), viewport, overlay_style).children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(Semantics::new(SemanticRole::Dialog).name(confirm.title.clone()))
    .on_key_down(move |cx, _event| {
        cx.prevent_default();
        cx.stop_propagation();
    })
    .child(theme::bordered(
        card,
        theme::c().sidebar,
        theme::c().border,
        8.0,
        1.0,
    ));

    let line =
        |top: f32, bottom: f32| UiRect::new(card.left + 22.0, top, card.right - 22.0, bottom);
    root = root.child(text(
        line(card.top + 20.0, card.top + 42.0),
        confirm.title.clone(),
        theme::sans_semibold(theme::c().text_bright, 15.0),
    ));
    if let Some((path, checkout)) = worktree {
        root = root.child(worktree_form(
            card,
            path,
            checkout,
            state.clone(),
            store.clone(),
        ));
    } else {
        root = root.child(text(
            line(card.top + 52.0, card.top + 92.0),
            confirm.message.clone(),
            theme::sans(theme::c().text_muted, theme::UI_SIZE),
        ));
    }

    let button_top = card.bottom - 46.0;
    let confirm_w = crate::ui::tabs::measure(confirm.confirm_label, theme::UI_SIZE, 600) + 36.0;
    let confirm_rect = UiRect::new(
        card.right - 22.0 - confirm_w,
        button_top,
        card.right - 22.0,
        button_top + 30.0,
    );
    let cancel_rect = UiRect::new(
        confirm_rect.left - 86.0,
        button_top,
        confirm_rect.left - 8.0,
        button_top + 30.0,
    );

    let cancel_state = state.clone();
    root = root.child(
        theme::bordered(cancel_rect, theme::c().sidebar, theme::c().border, 4.0, 1.0)
            .event_policy(EventPolicy::INTERACTIVE)
            .semantics(Semantics::new(SemanticRole::Button).name("Cancel"))
            .cursor(CursorIcon::Pointer)
            .on_click(move || resolve(&cancel_state, false))
            .child(text(
                cancel_rect,
                "Cancel",
                centered(theme::sans(theme::c().text_soft, theme::UI_SIZE)),
            )),
    );
    let (fill, label) = if confirm.destructive {
        (theme::c().error, theme::c().on_accent)
    } else {
        (theme::c().accent, theme::c().on_accent)
    };
    root.child(
        panel(confirm_rect, VisualStyle::filled(fill).radius(4.0))
            .event_policy(EventPolicy::INTERACTIVE)
            .semantics(Semantics::new(SemanticRole::Button).name(confirm.confirm_label))
            .cursor(CursorIcon::Pointer)
            .on_click(move || resolve(&state, true))
            .child(text(
                confirm_rect,
                confirm.confirm_label,
                centered(theme::sans_semibold(label, theme::UI_SIZE)),
            )),
    )
}

pub(crate) fn worktree_form(
    card: UiRect,
    path: &std::path::Path,
    checkout: &Checkout,
    state: Binding,
    _store: Snapshot,
) -> Element {
    let left = card.left + 22.0;
    let right = card.right - 22.0;
    let branch =
        crate::ui::tabs::ellipsize(checkout.target(), right - left - 74.0, theme::UI_SIZE, 400);
    let browse = UiRect::new(right - 82.0, card.top + 138.0, right, card.top + 170.0);
    let location = UiRect::new(left, browse.top, browse.left - 8.0, browse.bottom);
    let full_path = crate::ui::display_path(path);
    let path_label = location_label(&full_path, (location.width() - 20.0).max(0.0));
    let close = UiRect::new(
        card.right - 38.0,
        card.top + 12.0,
        card.right - 12.0,
        card.top + 38.0,
    );
    let close_state = state.clone();
    let location_state = state.clone();
    let mut form = panel(card, VisualStyle::default())
        .child(text(
            UiRect::new(left, card.top + 50.0, right, card.top + 70.0),
            "Check out a branch in a separate working directory.",
            theme::sans(theme::c().text_dim, theme::SMALL),
        ))
        .child(text(
            UiRect::new(left, card.top + 84.0, left + 64.0, card.top + 108.0),
            "Branch",
            theme::sans(theme::c().text_muted, theme::UI_SIZE),
        ))
        .child(text(
            UiRect::new(left + 74.0, card.top + 84.0, right, card.top + 108.0),
            branch,
            theme::mono(theme::c().text, theme::UI_SIZE),
        ))
        .child(text(
            UiRect::new(left, card.top + 116.0, right, card.top + 132.0),
            "Location",
            theme::sans(theme::c().text_muted, theme::SMALL),
        ))
        .child(
            theme::bordered(location, theme::c().bg, theme::c().border, 4.0, 1.0)
                .event_policy(EventPolicy::INTERACTIVE)
                .semantics(
                    Semantics::new(SemanticRole::Button)
                        .name("Worktree location")
                        .value(full_path),
                )
                .cursor(CursorIcon::Pointer)
                .on_click(move || {
                    location_state.dispatch(Command::ChooseWorktreeLocation);
                })
                .child(text(
                    UiRect::new(
                        location.left + 10.0,
                        location.top,
                        location.right - 10.0,
                        location.bottom,
                    ),
                    path_label,
                    theme::mono(theme::c().text, theme::UI_SIZE),
                )),
        )
        .child(
            theme::bordered(browse, theme::c().sidebar, theme::c().border, 4.0, 1.0)
                .event_policy(EventPolicy::INTERACTIVE)
                .semantics(Semantics::new(SemanticRole::Button).name("Choose worktree location"))
                .cursor(CursorIcon::Pointer)
                .on_click(move || {
                    state.dispatch(Command::ChooseWorktreeLocation);
                })
                .child(text(
                    browse,
                    "Browse…",
                    centered(theme::sans(theme::c().text_soft, theme::UI_SIZE)),
                )),
        )
        .child(text(
            UiRect::new(left, card.top + 178.0, right, card.top + 198.0),
            "Added to the current workspace after creation.",
            theme::sans(theme::c().text_dim, theme::SMALL),
        ));
    form = form.child(
        panel(close, VisualStyle::default().radius(4.0))
            .event_policy(EventPolicy::INTERACTIVE)
            .semantics(Semantics::new(SemanticRole::Button).name("Close"))
            .cursor(CursorIcon::Pointer)
            .on_click(move || resolve(&close_state, false))
            .child(crate::ui::drawer::drawer_icon(
                "worktree-dialog.close",
                "close",
                theme::icon_rect(close),
                theme::c().text_dim,
            )),
    );
    form
}

/// Keep the destination directory visible when its parent path is long.
pub(crate) fn location_label(path: &str, width: f32) -> String {
    if crate::ui::tabs::measure(path, theme::UI_SIZE, 400) <= width {
        return path.to_owned();
    }
    for (offset, _) in path.char_indices().skip(1) {
        let label = format!("…{}", &path[offset..]);
        if crate::ui::tabs::measure(&label, theme::UI_SIZE, 400) <= width {
            return label;
        }
    }
    String::new()
}
