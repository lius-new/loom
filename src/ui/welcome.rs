//! Welcome canvas shown when the workspace has no active document.

use lgui::core::{CursorIcon, EventPolicy, UiEventKind, UiFocusHandle, clip};
use lgui::prelude::{Element, State, UiRect, VisualStyle, group, panel, text};

use crate::state::AppState;
use crate::theme;
use crate::ui::tabs::{TEXT_MARGIN, ellipsize, measure};
use crate::workspace_actions;

const MAX_PAGE_W: f32 = 760.0;
const PAGE_PAD_X: f32 = 32.0;
const WIDE_BREAKPOINT: f32 = 600.0;
const COLUMN_GAP: f32 = 48.0;
const SECTION_TITLE_H: f32 = 22.0;
const ACTION_H: f32 = 34.0;
const ACTION_GAP: f32 = 4.0;
const WORKSPACE_CARD_H: f32 = 58.0;
const FOOTER_H: f32 = 52.0;

#[derive(Clone, Copy)]
enum StartAction {
    OpenFile,
    OpenFolder,
}

const ACTIONS: [(StartAction, &str); 2] = [
    (StartAction::OpenFile, "Open File..."),
    (StartAction::OpenFolder, "Open Folder..."),
];

pub fn render(rect: UiRect, state: State<AppState>, editor_focus: UiFocusHandle) -> Element {
    let snapshot = state.get();
    let page_w = (rect.width() - PAGE_PAD_X * 2.0).clamp(0.0, MAX_PAGE_W);
    let page_left = rect.left + (rect.width() - page_w) / 2.0;
    let page_right = page_left + page_w;
    let page_top = rect.top + if rect.height() >= 420.0 { 42.0 } else { 24.0 };

    let clear_hover = state.clone();
    let mut welcome = panel(rect, VisualStyle::default())
        .event_policy(EventPolicy::INTERACTIVE)
        .on_event_capture(UiEventKind::PointerMove, move |_cx, _payload| {
            clear_hover.try_update(|app| {
                let changed = app.welcome_hover.is_some();
                app.welcome_hover = None;
                changed
            });
        });

    let title_w = measure("Leditor", 18.0, 700);
    welcome = welcome.child(text(
        UiRect::new(
            page_left,
            page_top,
            page_left + title_w + TEXT_MARGIN,
            page_top + 26.0,
        ),
        "Leditor",
        theme::mono_bold(theme::ZINC_100, 18.0),
    ));
    welcome = welcome.child(text(
        UiRect::new(
            page_left + title_w + 14.0,
            page_top + 2.0,
            page_right,
            page_top + 24.0,
        ),
        format!("v{}", env!("CARGO_PKG_VERSION")),
        theme::mono(theme::ZINC_500, theme::SMALL),
    ));
    welcome = welcome.child(text(
        UiRect::new(page_left, page_top + 28.0, page_right, page_top + 48.0),
        "Open a file or folder to start editing.",
        theme::sans(theme::ZINC_500, theme::UI_SIZE),
    ));

    let sections_top = page_top + 76.0;
    let wide = page_w >= WIDE_BREAKPOINT;
    let (actions_rect, workspace_rect) = if wide {
        let actions_w = (page_w - COLUMN_GAP) * 5.0 / 12.0;
        (
            UiRect::new(page_left, sections_top, page_left + actions_w, rect.bottom),
            UiRect::new(
                page_left + actions_w + COLUMN_GAP,
                sections_top,
                page_right,
                rect.bottom,
            ),
        )
    } else {
        let workspace_top =
            sections_top + SECTION_TITLE_H + ACTIONS.len() as f32 * (ACTION_H + ACTION_GAP) + 24.0;
        (
            UiRect::new(page_left, sections_top, page_right, workspace_top),
            UiRect::new(page_left, workspace_top, page_right, rect.bottom),
        )
    };

    welcome = welcome.child(section_title(actions_rect, "START"));
    let mut action_top = actions_rect.top + SECTION_TITLE_H;
    for (index, (action, label)) in ACTIONS.into_iter().enumerate() {
        let row = UiRect::new(
            actions_rect.left,
            action_top,
            actions_rect.right,
            action_top + ACTION_H,
        );
        welcome = welcome.child(action_row(
            row,
            index,
            action,
            label,
            snapshot.welcome_hover == Some(index),
            state.clone(),
            editor_focus.clone(),
        ));
        action_top += ACTION_H + ACTION_GAP;
    }

    welcome = welcome.child(section_title(workspace_rect, "WORKSPACE"));
    let workspace_card = UiRect::new(
        workspace_rect.left,
        workspace_rect.top + SECTION_TITLE_H,
        workspace_rect.right,
        workspace_rect.top + SECTION_TITLE_H + WORKSPACE_CARD_H,
    );
    welcome = welcome.child(workspace_card_element(
        workspace_card,
        snapshot.open_dir.as_deref(),
        snapshot.welcome_hover == Some(ACTIONS.len()),
        state.clone(),
    ));

    let footer_min_height = if wide { 320.0 } else { 440.0 };
    if rect.height() >= footer_min_height {
        welcome = welcome.child(footer(UiRect::new(
            page_left,
            rect.bottom - FOOTER_H,
            page_right,
            rect.bottom,
        )));
    }

    clip(rect, 0.0, 0.0).child(welcome)
}

fn section_title(rect: UiRect, label: &'static str) -> Element {
    text(
        UiRect::new(rect.left, rect.top, rect.right, rect.top + SECTION_TITLE_H),
        label,
        theme::mono(theme::ZINC_500, theme::SMALL).tracking(0.8),
    )
    .into()
}

fn action_row(
    rect: UiRect,
    index: usize,
    action: StartAction,
    label: &'static str,
    hovered: bool,
    state: State<AppState>,
    editor_focus: UiFocusHandle,
) -> Element {
    let surface = if hovered {
        theme::bordered(rect, theme::SURFACE, theme::BORDER, 4.0, 1.0)
    } else {
        panel(rect, VisualStyle::default().radius(4.0))
    };
    let hover_state = state.clone();
    let click_state = state.clone();
    surface
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_pointer_move(move |_cx, _pointer| {
            hover_state.try_update(move |app| {
                let changed = app.welcome_hover != Some(index);
                app.welcome_hover = Some(index);
                changed
            });
        })
        .on_click(move || match action {
            StartAction::OpenFile => {
                if workspace_actions::choose_file(&click_state) {
                    editor_focus.focus();
                }
            }
            StartAction::OpenFolder => {
                workspace_actions::choose_folder(&click_state);
            }
        })
        .child(text(
            UiRect::new(rect.left + 12.0, rect.top, rect.right - 12.0, rect.bottom),
            label,
            theme::sans(
                if hovered {
                    theme::ZINC_100
                } else {
                    theme::ZINC_300
                },
                theme::UI_SIZE,
            ),
        ))
}

fn workspace_card_element(
    rect: UiRect,
    open_dir: Option<&std::path::Path>,
    hovered: bool,
    state: State<AppState>,
) -> Element {
    let Some(path) = open_dir else {
        return theme::bordered(rect, theme::BG, theme::BORDER, 5.0, 1.0).child(text(
            UiRect::new(rect.left + 12.0, rect.top, rect.right - 12.0, rect.bottom),
            "No folder opened",
            theme::mono(theme::ZINC_500, theme::UI_SIZE),
        ));
    };

    let name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let path_label = path.to_string_lossy();
    let display_path = ellipsize(
        &path_label,
        (rect.width() - 24.0).max(0.0),
        theme::SMALL,
        400,
    );
    let surface = if hovered {
        theme::bordered(rect, theme::SURFACE, theme::BORDER, 5.0, 1.0)
    } else {
        theme::bordered(rect, theme::BG, theme::BORDER, 5.0, 1.0)
    };
    let hover_state = state.clone();
    let click_state = state;
    surface
        .event_policy(EventPolicy::INTERACTIVE)
        .cursor(CursorIcon::Pointer)
        .on_pointer_move(move |_cx, _pointer| {
            hover_state.try_update(|app| {
                let index = ACTIONS.len();
                let changed = app.welcome_hover != Some(index);
                app.welcome_hover = Some(index);
                changed
            });
        })
        .on_click(move || click_state.update(|app| app.show_drawer = true))
        .child(text(
            UiRect::new(
                rect.left + 12.0,
                rect.top + 8.0,
                rect.right - 12.0,
                rect.top + 28.0,
            ),
            name,
            theme::sans_semibold(
                if hovered {
                    theme::ZINC_100
                } else {
                    theme::ZINC_200
                },
                theme::UI_SIZE,
            ),
        ))
        .child(text(
            UiRect::new(
                rect.left + 12.0,
                rect.top + 29.0,
                rect.right - 12.0,
                rect.bottom - 6.0,
            ),
            display_path,
            theme::mono(theme::ZINC_500, theme::SMALL),
        ))
}

fn footer(rect: UiRect) -> Element {
    group(rect)
        .child(panel(
            UiRect::new(rect.left, rect.top, rect.right, rect.top + 1.0),
            VisualStyle::filled(theme::BORDER),
        ))
        .child(text(
            UiRect::new(rect.left, rect.top + 10.0, rect.right, rect.top + 28.0),
            "Welcome to Leditor",
            theme::sans_semibold(theme::ZINC_300, theme::UI_SIZE),
        ))
        .child(text(
            UiRect::new(rect.left, rect.top + 28.0, rect.right, rect.bottom),
            "A focused workspace for exploring projects, editing code, and running local shells.",
            theme::sans(theme::ZINC_500, theme::SMALL),
        ))
}
