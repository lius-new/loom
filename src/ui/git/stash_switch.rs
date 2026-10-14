//! Modal offered when local changes stop a branch switch: stash them and
//! switch, or carry them over to the new branch.

use std::path::PathBuf;

use lgui::core::{CursorIcon, EventPolicy, SemanticRole, Semantics, UiElement, UiId};
use lgui::prelude::{Element, TextAlign, TextStyle, UiRect, VisualStyle, panel, text};

use crate::features::git::actions::Checkout;
use crate::features::git::actions::StashMode;
use crate::ui::git::commands::{Binding, Command, Snapshot};
use crate::ui::theme;

pub(crate) const CARD_W: f32 = 520.0;
pub(crate) const CARD_H: f32 = 186.0;
/// Files named in the message before the rest are counted.
pub(crate) const LISTED_FILES: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashSwitchRequest {
    pub repository: PathBuf,
    pub checkout: Checkout,
    /// Files whose changes the switch would overwrite, as Git listed them.
    pub files: Vec<String>,
    /// Whether the terminal had focus when the dialog opened.
    pub restore_terminal: bool,
}

/// `a`, `a and b`, `a, b and c`, or `a, b, c and 2 more files`.
pub(crate) fn describe_files(files: &[String]) -> String {
    match files {
        [] => "your files".to_owned(),
        [only] => only.clone(),
        _ if files.len() <= LISTED_FILES => format!(
            "{} and {}",
            files[..files.len() - 1].join(", "),
            files[files.len() - 1]
        ),
        _ => format!(
            "{} and {} more files",
            files[..LISTED_FILES].join(", "),
            files.len() - LISTED_FILES
        ),
    }
}

/// Resolve the open request: `Some(mode)` retries the switch that way,
/// `None` cancels it.
pub(crate) fn resolve(state: &Binding, mode: Option<StashMode>) {
    let Some(request) = state.get().ui.stash_switch else {
        return;
    };
    state.update(|app| app.ui.stash_switch = None);
    if let Some(mode) = mode {
        state.dispatch(Command::Checkout {
            root: request.repository.to_path_buf(),
            checkout: request.checkout,
            mode,
        });
    }
}

pub(crate) fn centered(mut style: TextStyle) -> TextStyle {
    style.align = TextAlign::Center;
    style
}

pub fn render(
    viewport: UiRect,
    request: &StashSwitchRequest,
    state: Binding,
    _store: Snapshot,
    dialog_id: UiId,
) -> Element {
    let target = request.checkout.target().to_owned();
    let center_x = viewport.left + viewport.width() / 2.0;
    let top = viewport.top + (viewport.height() - CARD_H) * 0.36;
    let card = UiRect::new(
        center_x - CARD_W / 2.0,
        top,
        center_x + CARD_W / 2.0,
        top + CARD_H,
    );

    // The overlay owns keyboard focus while open (see `app.rs`); its
    // choices arrive as `dialog::` actions and every other key stops here.
    let overlay_style = VisualStyle::filled(theme::c().bg).alpha(214);
    let mut root = Element::new(move |cx| {
        UiElement::panel(dialog_id.clone(), viewport, overlay_style).children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(Semantics::new(SemanticRole::Dialog).name("Switch with local changes?"))
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
            format!("Switch to {target} with local changes?"),
            theme::sans_semibold(theme::c().text_bright, 15.0),
        ))
        .child(text(
            line(card.top + 52.0, card.top + 75.0),
            format!(
                "Switching would overwrite your changes to {}.",
                describe_files(&request.files)
            ),
            theme::sans(theme::c().text_muted, theme::UI_SIZE),
        ))
        .child(text(
            line(card.top + 78.0, card.top + 112.0),
            format!(
                "Stash and Switch keeps the changes in the stash. Bring Changes reapplies \
                 them on {target}."
            ),
            theme::sans(theme::c().text_dim, theme::SMALL),
        ));

    let button_top = card.bottom - 46.0;
    let button = |left: f32, right: f32| UiRect::new(left, button_top, right, button_top + 30.0);
    let cancel_rect = button(card.left + 22.0, card.left + 100.0);
    let carry_rect = button(card.right - 274.0, card.right - 148.0);
    let keep_rect = button(card.right - 140.0, card.right - 22.0);

    let cancel_state = state.clone();
    root = root.child(
        theme::bordered(cancel_rect, theme::c().sidebar, theme::c().border, 4.0, 1.0)
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .on_click(move || resolve(&cancel_state, None))
            .child(text(
                cancel_rect,
                "Cancel",
                centered(theme::sans(theme::c().text_soft, theme::UI_SIZE)),
            )),
    );
    let carry_state = state.clone();
    root = root.child(
        theme::bordered(carry_rect, theme::c().sidebar, theme::c().border, 4.0, 1.0)
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .on_click(move || resolve(&carry_state, Some(StashMode::Carry)))
            .child(text(
                carry_rect,
                "Bring Changes",
                centered(theme::sans(theme::c().text_soft, theme::UI_SIZE)),
            )),
    );
    root.child(
        panel(
            keep_rect,
            VisualStyle::filled(theme::c().accent).radius(4.0),
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_click(move || resolve(&state, Some(StashMode::Keep)))
        .child(text(
            keep_rect,
            "Stash and Switch",
            centered(theme::sans_semibold(theme::c().on_accent, theme::UI_SIZE)),
        )),
    )
}
