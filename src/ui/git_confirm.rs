//! Modal confirmation for Source Control actions that need a second look:
//! discarding changes, committing when nothing is staged, aborting an
//! operation in progress and dropping a stash.

use std::path::PathBuf;

use lgui::core::{CursorIcon, EventPolicy, SemanticRole, Semantics, UiElement, UiId};
use lgui::prelude::{Element, State, TextAlign, TextStyle, UiRect, VisualStyle, panel, text};

use crate::git::{DiffTarget, GitStoreSnapshot, RepositoryState};
use crate::git_actions;
use crate::input::action::Action;
use crate::model::diff_document::ChangeBlock;
use crate::state::AppState;
use crate::theme;
use crate::ui::branch_picker::Checkout;

const CARD_W: f32 = 480.0;
const CARD_H: f32 = 156.0;
const WORKTREE_CARD_H: f32 = 258.0;

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
fn resolve(state: &State<AppState>, store: &State<GitStoreSnapshot>, confirmed: bool) {
    let Some(confirm) = state.get().git_confirm else {
        return;
    };
    state.update(|app| app.git_confirm = None);
    if !confirmed {
        return;
    }
    match confirm.action {
        GitConfirmAction::CreateWorktree {
            repository,
            path,
            checkout,
        } => {
            git_actions::create_worktree(state, store, &repository, path, checkout);
        }
        GitConfirmAction::RemoveWorktree {
            repository,
            path,
            force,
        } => {
            git_actions::remove_worktree(state, store, &repository, path, force);
        }
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
        GitConfirmAction::DiscardBlock {
            repository,
            path,
            block,
        } => {
            git_actions::apply_block(
                state,
                store,
                &repository,
                path,
                DiffTarget::IndexToWorktree,
                block,
                git_actions::BlockAction::Discard,
            );
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
            .semantics(Semantics::new(SemanticRole::Button).name("Cancel"))
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
            .semantics(Semantics::new(SemanticRole::Button).name(confirm.confirm_label))
            .cursor(CursorIcon::Pointer)
            .on_click(move || resolve(&state, &store, true))
            .child(text(
                confirm_rect,
                confirm.confirm_label,
                centered(theme::sans_semibold(label, theme::UI_SIZE)),
            )),
    )
}

fn worktree_form(
    card: UiRect,
    path: &std::path::Path,
    checkout: &Checkout,
    state: State<AppState>,
    store: State<GitStoreSnapshot>,
) -> Element {
    let left = card.left + 22.0;
    let right = card.right - 22.0;
    let branch =
        super::tabs::ellipsize(checkout.target(), right - left - 74.0, theme::UI_SIZE, 400);
    let browse = UiRect::new(right - 82.0, card.top + 138.0, right, card.top + 170.0);
    let location = UiRect::new(left, browse.top, browse.left - 8.0, browse.bottom);
    let full_path = super::display_path(path);
    let path_label = location_label(&full_path, (location.width() - 20.0).max(0.0));
    let close = UiRect::new(
        card.right - 38.0,
        card.top + 12.0,
        card.right - 12.0,
        card.top + 38.0,
    );
    let close_state = state.clone();
    let close_store = store;
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
                .on_click(move || git_actions::choose_worktree_location(&location_state))
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
                .on_click(move || git_actions::choose_worktree_location(&state))
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
            .on_click(move || resolve(&close_state, &close_store, false))
            .child(super::drawer::drawer_icon(
                "worktree-dialog.close",
                "close",
                theme::icon_rect(close),
                theme::c().text_dim,
            )),
    );
    form
}

/// Keep the destination directory visible when its parent path is long.
fn location_label(path: &str, width: f32) -> String {
    if super::tabs::measure(path, theme::UI_SIZE, 400) <= width {
        return path.to_owned();
    }
    for (offset, _) in path.char_indices().skip(1) {
        let label = format!("…{}", &path[offset..]);
        if super::tabs::measure(&label, theme::UI_SIZE, 400) <= width {
            return label;
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgui::application::{AppView, ApplicationContext};
    use lgui::core::{
        InputEvent, Point, PointerButton, PointerData, UiScale, dispatch_runtime_output,
    };
    use lgui::session::UiSession;
    use std::sync::{Arc, Mutex};

    #[test]
    fn worktree_form_shows_destination_and_cancels_at_normal_and_narrow_widths() {
        for width in [640.0, 360.0] {
            let exposed = Arc::new(Mutex::new(None::<State<AppState>>));
            let output = exposed.clone();
            let viewport = UiRect::new(0.0, 0.0, width, 420.0);
            let destination = PathBuf::from(r"D:\Documents\cprojects\loom-feature-login");
            let expected_path = super::super::display_path(&destination);
            let view: AppView = Arc::new(move |cx| {
                let state = cx.state_with(|| {
                    let mut app = AppState::new();
                    app.git_confirm = Some(GitConfirm {
                        title: "New Worktree".into(),
                        message: String::new(),
                        confirm_label: "Create",
                        destructive: false,
                        action: GitConfirmAction::CreateWorktree {
                            repository: PathBuf::from(r"D:\Documents\cprojects\loom"),
                            path: destination.clone(),
                            checkout: Checkout::Switch("feature/login".into()),
                        },
                        restore_terminal: false,
                    });
                    app
                });
                let store = cx.state_with(GitStoreSnapshot::default);
                let dialog_id = cx.use_stable_id();
                *output.lock().unwrap() = Some(state.clone());
                let root = panel(viewport, VisualStyle::filled(theme::c().bg));
                if let Some(confirm) = state.get().git_confirm {
                    root.child(render(viewport, &confirm, state, store, dialog_id))
                } else {
                    root
                }
            });
            let context = ApplicationContext::empty(Default::default());
            let _text = lgui::backend::install_text_environment(
                &context,
                lgui::render_skia::skia_text_system_handle(),
            );
            let mut session = UiSession::new();
            session.render_view(&view, viewport, UiScale::ONE);
            let find = |name: &str| {
                session
                    .tree()
                    .nodes()
                    .iter()
                    .find(|node| {
                        node.semantics.as_ref().and_then(|s| s.name.as_deref()) == Some(name)
                    })
                    .unwrap_or_else(|| panic!("Missing dialog control: {name}"))
            };
            let location = find("Worktree location");
            assert_eq!(
                location.semantics.as_ref().unwrap().value.as_deref(),
                Some(expected_path.as_str())
            );
            assert!(location.layout_rect.left >= viewport.left);
            assert!(
                location.layout_rect.right <= find("Choose worktree location").layout_rect.left
            );
            let cancel = find("Cancel").layout_rect;
            assert!(find("Create").layout_rect.right <= viewport.right);

            if width == 640.0
                && let Some(path) = std::env::var_os("LOOM_WORKTREE_DIALOG_PREVIEW")
            {
                use lgui::core::PhysicalRect;
                use lgui::icons::IconRegistration;
                use lgui::renderer::{FrameInfo, FrameReason};
                context
                    .resources()
                    .provide(IconRegistration(Arc::new(crate::svg_icons())));
                let scale = UiScale::new(2.0);
                let mut surface = lgui::render_skia::SkiaSoftwareSurface::new(8 * 1024 * 1024);
                let frame = FrameInfo::new(
                    PhysicalRect::new(0, 0, 1280, 840),
                    &[],
                    scale,
                    FrameReason::SceneChange,
                    true,
                );
                lgui::assets::backend::with_render_resources(
                    &context,
                    lgui::RenderResources::new(),
                    || {
                        surface
                            .draw(&session.tree().scene().project_to_physical(scale), &frame)
                            .unwrap();
                    },
                );
                let mut pixels = surface.pixels().to_vec();
                for pixel in pixels.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
                image::save_buffer(path, &pixels, 1280, 840, image::ColorType::Rgba8).unwrap();
            }

            let point = Point::new(
                (cancel.left + cancel.right) / 2.0,
                (cancel.top + cancel.bottom) / 2.0,
            );
            for input in [
                InputEvent::PointerDown {
                    pointer: PointerData::mouse(point),
                    button: PointerButton::Left,
                },
                InputEvent::PointerUp {
                    pointer: PointerData::mouse(point),
                    button: PointerButton::Left,
                },
            ] {
                let events = session.handle_input(input);
                dispatch_runtime_output(
                    events,
                    &context,
                    &lgui::window::WindowId::new("worktree-dialog-test"),
                    |action| session.handle_default_action(action),
                    |_| {},
                );
                session.render_view(&view, viewport, UiScale::ONE);
            }
            assert!(
                exposed
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .get()
                    .git_confirm
                    .is_none()
            );
        }
    }
}
