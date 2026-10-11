//! Modal confirmation for Source Control actions that need a second look:
//! discarding changes, committing when nothing is staged, aborting an
//! operation in progress and dropping a stash.

use std::path::PathBuf;

use lgui::core::{CursorIcon, EventPolicy, SemanticRole, Semantics, UiElement, UiId};
use lgui::prelude::{Element, State, TextAlign, TextStyle, UiRect, VisualStyle, panel, text};

use crate::git::{GitStoreSnapshot, RepositoryState};
use crate::git_actions;
use crate::input::action::Action;
use crate::state::AppState;
use crate::theme;

const CARD_W: f32 = 480.0;
const CARD_H: f32 = 156.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitConfirmAction {
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
fn resolve(state: &State<AppState>, store: &State<GitStoreSnapshot>, confirmed: bool) {
    let Some(confirm) = state.get().git_confirm else {
        return;
    };
    state.update(|app| app.git_confirm = None);
    if !confirmed {
        return;
    }
    match confirm.action {
        GitConfirmAction::Discard {
            repository,
            tracked,
            untracked,
        } => {
            git_actions::discard_paths(state, store, &repository, tracked, untracked);
        }
        GitConfirmAction::StageAllAndCommit => {
            git_actions::commit_all(state, store);
        }
        GitConfirmAction::AbortOperation {
            repository,
            operation,
        } => {
            git_actions::abort_operation(state, store, &repository, operation);
        }
        GitConfirmAction::DropStash { repository, oid } => {
            git_actions::drop_stash(state, store, &repository, oid);
        }
    }
}

/// Perform a `dialog::` action from the keymap.
pub fn perform(state: &State<AppState>, store: &State<GitStoreSnapshot>, action: Action) -> bool {
    match action {
        Action::DialogConfirm => resolve(state, store, true),
        Action::DialogCancel => resolve(state, store, false),
        _ => return false,
    }
    true
}

fn centered(mut style: TextStyle) -> TextStyle {
    style.align = TextAlign::Center;
    style
}

pub fn render(
    viewport: UiRect,
    confirm: &GitConfirm,
    state: State<AppState>,
    store: State<GitStoreSnapshot>,
    dialog_id: UiId,
) -> Element {
    let center_x = viewport.left + viewport.width() / 2.0;
    let top = viewport.top + (viewport.height() - CARD_H) * 0.36;
    let card = UiRect::new(
        center_x - CARD_W / 2.0,
        top,
        center_x + CARD_W / 2.0,
        top + CARD_H,
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
    root = root
        .child(text(
            line(card.top + 20.0, card.top + 42.0),
            confirm.title.clone(),
            theme::sans_semibold(theme::c().text_bright, 15.0),
        ))
        .child(text(
            line(card.top + 52.0, card.top + 92.0),
            confirm.message.clone(),
            theme::sans(theme::c().text_muted, theme::UI_SIZE),
        ));

    let button_top = card.bottom - 46.0;
    let confirm_w = super::tabs::measure(confirm.confirm_label, theme::UI_SIZE, 600) + 36.0;
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
    let cancel_store = store.clone();
    root = root.child(
        theme::bordered(cancel_rect, theme::c().sidebar, theme::c().border, 4.0, 1.0)
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .on_click(move || resolve(&cancel_state, &cancel_store, false))
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
            .cursor(CursorIcon::Pointer)
            .on_click(move || resolve(&state, &store, true))
            .child(text(
                confirm_rect,
                confirm.confirm_label,
                centered(theme::sans_semibold(label, theme::UI_SIZE)),
            )),
    )
}
