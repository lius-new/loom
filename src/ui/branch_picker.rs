//! Branch picker opened from the status bar's Git badge.
//!
//! Lists the repository's local branches, then its remote-tracking branches,
//! filtered by the query. A query that names no branch offers to create it.
//! Remote branches are refreshed only by the picker's explicit fetch, so
//! opening it is fast and never prompts for credentials.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use lgui::core::{
    CursorIcon, EventPolicy, IconStyle, SemanticRole, Semantics, UiElement, UiEventKind,
    UiEventPayload, UiFocusHandle, UiId, WheelUnit, clip, precompiled,
};
use lgui::prelude::{Color, Element, ShadowStyle, State, UiRect, VisualStyle, panel, text};

use crate::git::{BranchInfo, GitResult, GitStoreSnapshot, OperationKind};
use crate::state::AppState;
use crate::ui::components::input::{self, InputBinding, InputOptions, InputState, InputStyle};
use crate::{git_actions, theme};

const CARD_W: f32 = 420.0;
const PAD: f32 = 6.0;
const TITLE_H: f32 = 26.0;
const INPUT_H: f32 = 26.0;
const ROW_H: f32 = 22.0;
const HEADER_H: f32 = 22.0;
const MAX_LIST_H: f32 = ROW_H * 12.0;
const TEXT_MARGIN: f32 = 6.0;

static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq)]
pub struct BranchPicker {
    pub repository: PathBuf,
    pub repository_name: String,
    /// Left edge of the badge the picker opened from.
    pub anchor_x: f32,
    pub listing: BranchListing,
    /// Index into the selectable rows, valid for `query`.
    pub selected: usize,
    pub scroll: f32,
    /// The filter `selected` and `scroll` were chosen under; any other
    /// filter starts again from the top.
    pub query: String,
    /// Identifies this opening, so a late load cannot fill a newer picker.
    pub sequence: u64,
    /// Whether the terminal had focus when the picker opened.
    pub restore_terminal: bool,
}

impl BranchPicker {
    fn view(&self, query: &str) -> (usize, f32) {
        if self.query == query {
            (self.selected, self.scroll)
        } else {
            (0, 0.0)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum BranchListing {
    Loading,
    Ready(Arc<[BranchInfo]>),
    Failed(String),
}

impl BranchListing {
    pub fn from_result(result: GitResult<Vec<BranchInfo>>) -> Self {
        match result {
            Ok(branches) => Self::Ready(branches.into()),
            Err(error) => Self::Failed(error.user_message()),
        }
    }

    fn branches(&self) -> &[BranchInfo] {
        match self {
            Self::Ready(branches) => branches,
            Self::Loading | Self::Failed(_) => &[],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Header(&'static str),
    Create(String),
    Branch(BranchInfo),
}

/// What choosing a row does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Checkout {
    Switch(String),
    Create(String),
    /// Create a local branch tracking this remote branch.
    Track(String),
}

impl Checkout {
    /// The branch being checked out, as the user chose it.
    pub fn target(&self) -> &str {
        match self {
            Self::Switch(name) | Self::Create(name) | Self::Track(name) => name,
        }
    }
}

/// Rows for `query`: a create entry, then local branches (current first),
/// then remote branches, each in Git's most-recent-first order.
pub fn rows(branches: &[BranchInfo], query: &str) -> Vec<Row> {
    let query = query.trim();
    let needle = query.to_lowercase();
    let matches =
        |branch: &&BranchInfo| needle.is_empty() || branch.name.to_lowercase().contains(&needle);
    let mut rows = Vec::new();
    if valid_branch_name(query) && !branches.iter().any(|branch| branch.name == query) {
        rows.push(Row::Create(query.to_owned()));
    }
    let mut local = branches
        .iter()
        .filter(|branch| !branch.remote)
        .filter(matches)
        .collect::<Vec<_>>();
    local.sort_by_key(|branch| !branch.current);
    if !local.is_empty() {
        rows.push(Row::Header("LOCAL"));
        rows.extend(local.into_iter().cloned().map(Row::Branch));
    }
    let remote = branches
        .iter()
        .filter(|branch| branch.remote)
        .filter(matches)
        .collect::<Vec<_>>();
    if !remote.is_empty() {
        rows.push(Row::Header("REMOTE"));
        rows.extend(remote.into_iter().cloned().map(Row::Branch));
    }
    rows
}

/// What choosing `row` does, or `None` when it names the current branch.
pub fn checkout_for(row: &Row, branches: &[BranchInfo]) -> Option<Checkout> {
    match row {
        Row::Header(_) => None,
        Row::Create(name) => Some(Checkout::Create(name.clone())),
        Row::Branch(branch) if !branch.remote => {
            (!branch.current).then(|| Checkout::Switch(branch.name.clone()))
        }
        Row::Branch(remote) => {
            // Prefer a local branch already tracking it, then one named
            // like it, before creating a new tracking branch.
            let short = remote.name.split_once('/').map(|(_, name)| name);
            let local = branches
                .iter()
                .filter(|branch| !branch.remote)
                .find(|branch| branch.upstream.as_deref() == Some(remote.name.as_str()))
                .or_else(|| {
                    branches
                        .iter()
                        .find(|branch| !branch.remote && Some(branch.name.as_str()) == short)
                });
            match local {
                Some(local) if local.current => None,
                Some(local) => Some(Checkout::Switch(local.name.clone())),
                None => Some(Checkout::Track(remote.name.clone())),
            }
        }
    }
}

/// A conservative subset of `git check-ref-format`; Git reports the rest.
fn valid_branch_name(name: &str) -> bool {
    !name.is_empty()
        && name != "@"
        && !name.starts_with(['-', '/', '.'])
        && !name.ends_with(['/', '.'])
        && !name.ends_with(".lock")
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && !name.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
}

fn row_height(row: &Row) -> f32 {
    match row {
        Row::Header(_) => HEADER_H,
        Row::Create(_) | Row::Branch(_) => ROW_H,
    }
}

/// Indices of the rows that can be selected.
fn selectable(rows: &[Row]) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| !matches!(row, Row::Header(_)))
        .map(|(index, _)| index)
        .collect()
}

struct ListLayout {
    tops: Vec<f32>,
    content_h: f32,
    view_h: f32,
}

fn layout(rows: &[Row]) -> ListLayout {
    let mut tops = Vec::with_capacity(rows.len());
    let mut y = 0.0;
    for row in rows {
        tops.push(y);
        y += row_height(row);
    }
    ListLayout {
        tops,
        content_h: y,
        view_h: y.min(MAX_LIST_H),
    }
}

impl ListLayout {
    fn max_scroll(&self) -> f32 {
        (self.content_h - self.view_h).max(0.0)
    }

    /// `scroll` adjusted so row `index` is visible, with its section header
    /// when the header directly precedes it.
    fn reveal(&self, rows: &[Row], index: usize, scroll: f32) -> f32 {
        let top = if index > 0 && matches!(rows[index - 1], Row::Header(_)) {
            self.tops[index - 1]
        } else {
            self.tops[index]
        };
        let bottom = self.tops[index] + row_height(&rows[index]);
        let scroll = if top < scroll {
            top
        } else if bottom > scroll + self.view_h {
            bottom - self.view_h
        } else {
            scroll
        };
        scroll.clamp(0.0, self.max_scroll())
    }
}

/// Move the selection by `delta` items, wrapping at either end.
pub fn move_selection(app: &mut AppState, delta: isize) {
    let query = app.branch_picker_input.text().to_owned();
    let Some(picker) = app.branch_picker.as_mut() else {
        return;
    };
    let rows = rows(picker.listing.branches(), &query);
    let items = selectable(&rows);
    if items.is_empty() {
        return;
    }
    let (selected, scroll) = picker.view(&query);
    let next = (selected as isize + delta).rem_euclid(items.len() as isize) as usize;
    picker.scroll = layout(&rows).reveal(&rows, items[next], scroll);
    picker.selected = next;
    picker.query = query;
}

/// Open the picker for the active repository; without one, show Source
/// Control, which explains how to get one.
pub fn open(state: &State<AppState>, store: &State<GitStoreSnapshot>, anchor_x: f32) {
    let Some(repository) = store
        .get()
        .active()
        .map(|repository| repository.worktree_root.clone())
    else {
        state.update(|app| app.show_source_control = true);
        return;
    };
    let sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
    let repository_name = repository.file_name().map_or_else(
        || super::display_path(&repository),
        |name| name.to_string_lossy().into_owned(),
    );
    let picker_repository = repository.clone();
    state.update(move |app| {
        app.branch_picker_input = InputState::default();
        app.branch_picker = Some(BranchPicker {
            repository: picker_repository,
            repository_name,
            anchor_x,
            listing: BranchListing::Loading,
            selected: 0,
            scroll: 0.0,
            query: String::new(),
            sequence,
            restore_terminal: app.terminal_focused,
        });
    });
    git_actions::load_branches(state, repository, sequence);
}

/// Choose the selected row. Returns whether the picker was open.
pub fn confirm(state: &State<AppState>, store: &State<GitStoreSnapshot>) -> bool {
    choose(state, store, None)
}

/// Choose item `index`, or the selected item without one.
fn choose(state: &State<AppState>, store: &State<GitStoreSnapshot>, index: Option<usize>) -> bool {
    let app = state.get();
    let Some(picker) = app.branch_picker else {
        return false;
    };
    let query = app.branch_picker_input.text();
    let branches = picker.listing.branches();
    let rows = rows(branches, query);
    let index = index.unwrap_or_else(|| picker.view(query).0);
    let Some(row) = selectable(&rows).get(index).map(|&row| &rows[row]) else {
        return true;
    };
    let checkout = checkout_for(row, branches);
    state.update(|app| app.branch_picker = None);
    if let Some(checkout) = checkout {
        git_actions::checkout(
            state,
            store,
            &picker.repository,
            checkout,
            git_actions::StashMode::Refuse,
        );
    }
    true
}

fn measure(s: &str, size: f32, weight: i32) -> f32 {
    super::tabs::measure(s, size, weight)
}

fn icon(id: &'static str, key: &'static str, rect: UiRect, color: Color) -> Element {
    precompiled(UiElement::icon(UiId::new(id), rect, key).icon_style(IconStyle::new(color)))
}

/// The picker card, anchored above the status bar whose top is `bottom`.
#[allow(clippy::too_many_arguments)]
pub fn render(
    viewport: UiRect,
    bottom: f32,
    picker: &BranchPicker,
    query: &str,
    state: State<AppState>,
    store: State<GitStoreSnapshot>,
    input_focus: UiFocusHandle,
    input_id: UiId,
) -> Element {
    let branches = picker.listing.branches();
    let rows = rows(branches, query);
    let items = selectable(&rows);
    let list = layout(&rows);
    let (selected, scroll) = picker.view(query);
    let selected_row = items.get(selected).copied();
    let scroll = scroll.clamp(0.0, list.max_scroll());
    let message = match &picker.listing {
        BranchListing::Loading => Some("Loading branches…".to_owned()),
        BranchListing::Failed(error) => Some(error.clone()),
        BranchListing::Ready(_) if rows.is_empty() => Some("No branches match.".to_owned()),
        BranchListing::Ready(_) => None,
    };
    let list_h = if message.is_some() {
        ROW_H
    } else {
        list.view_h
    };
    let card_h = PAD + TITLE_H + INPUT_H + PAD + list_h + PAD;
    let card_w = CARD_W.min(viewport.width() - 16.0).max(0.0);
    let left = picker.anchor_x.clamp(
        viewport.left + 8.0,
        (viewport.right - 8.0 - card_w).max(viewport.left + 8.0),
    );
    let card = UiRect::new(left, bottom - 4.0 - card_h, left + card_w, bottom - 4.0);

    // Invisible backdrop: a press outside the card closes the picker.
    let close_state = state.clone();
    let overlay = panel(viewport, VisualStyle::default())
        .event_policy(EventPolicy::INTERACTIVE)
        .on_pointer_down_with_button(move |_cx, pointer, _button| {
            if !card.contains(pointer.point) {
                close_state.update(|app| app.branch_picker = None);
            }
        });

    let wheel_state = state.clone();
    let max_scroll = list.max_scroll();
    let mut surface = theme::bordered(card, theme::c().surface, theme::c().border, 6.0, 1.0)
        .shadow(
            ShadowStyle::new(Color::BLACK)
                .alpha(theme::c().shadow_alpha)
                .offset(0.0, 2.0)
                .blur(3.0),
        )
        .event_policy(EventPolicy::INTERACTIVE)
        .on_event(UiEventKind::Wheel, move |_cx, payload| {
            if let UiEventPayload::Wheel { delta } = payload {
                let step = match delta.unit {
                    WheelUnit::Lines => delta.y * ROW_H * 3.0,
                    WheelUnit::Pixels => delta.y,
                };
                wheel_state.update(move |app| {
                    let query = app.branch_picker_input.text().to_owned();
                    if let Some(picker) = app.branch_picker.as_mut() {
                        let (selected, scroll) = picker.view(&query);
                        picker.selected = selected;
                        picker.scroll = (scroll - step).clamp(0.0, max_scroll);
                        picker.query = query;
                    }
                });
            }
        });

    // Title: what the picker does, the repository it acts on, and fetch.
    let title = UiRect::new(
        card.left + PAD + 4.0,
        card.top + PAD,
        card.right - PAD,
        card.top + PAD + TITLE_H,
    );
    let title_label = "Switch Branch";
    let title_w = measure(title_label, theme::UI_SIZE, 600) + TEXT_MARGIN;
    surface = surface.child(text(
        UiRect::new(title.left, title.top, title.left + title_w, title.bottom),
        title_label,
        theme::sans_semibold(theme::c().text_bright, theme::UI_SIZE),
    ));
    let operation = store.get().operation;
    let fetching = operation
        .as_ref()
        .is_some_and(|operation| operation.kind == OperationKind::Fetch);
    let fetch_rect = UiRect::new(
        title.right - 24.0,
        title.top + 1.0,
        title.right,
        title.bottom - 1.0,
    );
    let repository_right = if fetching {
        let label = "Fetching…";
        let width = measure(label, theme::SMALL, 400) + TEXT_MARGIN;
        surface = surface.child(text(
            UiRect::new(title.right - width, title.top, title.right, title.bottom),
            label,
            theme::mono_right(theme::c().text_dim, theme::SMALL),
        ));
        title.right - width - 8.0
    } else {
        let enabled = operation.is_none();
        let fetch_state = state.clone();
        let fetch_store = store.clone();
        surface = surface.child(
            panel(fetch_rect, VisualStyle::default().radius(4.0))
                .event_policy(EventPolicy::INTERACTIVE)
                .semantics(Semantics::new(SemanticRole::Button).name("Fetch from remotes"))
                .cursor(if enabled {
                    CursorIcon::Pointer
                } else {
                    CursorIcon::NotAllowed
                })
                .on_click(move || {
                    if enabled {
                        git_actions::fetch_branches(&fetch_state, &fetch_store);
                    }
                })
                .child(icon(
                    "branch-picker.fetch",
                    "refresh",
                    theme::icon_rect(fetch_rect),
                    if enabled {
                        theme::c().text_muted
                    } else {
                        theme::c().text_ghost
                    },
                )),
        );
        fetch_rect.left - 6.0
    };
    surface = surface.child(text(
        UiRect::new(
            title.left + title_w + 8.0,
            title.top,
            repository_right,
            title.bottom,
        ),
        picker.repository_name.clone(),
        theme::mono_right(theme::c().text_dim, theme::SMALL),
    ));

    // Filter.
    let field = UiRect::new(
        card.left + PAD,
        title.bottom,
        card.right - PAD,
        title.bottom + INPUT_H,
    );
    surface = surface.child(theme::bordered(
        field,
        theme::c().bg,
        theme::c().accent,
        4.0,
        1.0,
    ));
    let search_icon = UiRect::new(
        field.left + 7.0,
        field.top + (INPUT_H - 12.0) / 2.0,
        field.left + 19.0,
        field.top + (INPUT_H + 12.0) / 2.0,
    );
    surface = surface.child(icon(
        "branch-picker.search",
        "search",
        search_icon,
        theme::c().text_dim,
    ));
    surface = surface.child(input::render(
        UiRect::new(
            field.left + 22.0,
            field.top + 1.0,
            field.right - 4.0,
            field.bottom - 1.0,
        ),
        input_id,
        input_focus,
        InputBinding::new(state.clone(), filter_state, filter_state_mut),
        InputOptions {
            label: "Branch name",
            placeholder: "Filter branches, or type a name to create one",
            style: InputStyle {
                text: theme::mono(theme::c().text, theme::UI_SIZE),
                placeholder: theme::mono(theme::c().text_faint, theme::UI_SIZE),
                caret: theme::c().text,
                selection: theme::c().selection,
            },
            on_submit: None,
            on_cancel: None,
            on_blur: None,
        },
    ));

    let list_rect = UiRect::new(
        card.left + 1.0,
        field.bottom + PAD,
        card.right - 1.0,
        field.bottom + PAD + list_h,
    );
    if let Some(message) = message {
        surface = surface.child(text(
            UiRect::new(
                list_rect.left + 10.0,
                list_rect.top,
                list_rect.right - 10.0,
                list_rect.bottom,
            ),
            message,
            theme::sans(theme::c().text_dim, theme::UI_SIZE),
        ));
        return overlay.child(surface);
    }

    let head_label = branches
        .iter()
        .find(|branch| branch.current)
        .map_or("HEAD", |branch| branch.name.as_str())
        .to_owned();
    let mut viewport_list = clip(list_rect, 0.0, -scroll);
    let mut item_index = 0usize;
    for (index, row) in rows.iter().enumerate() {
        let top = list_rect.top + list.tops[index];
        let row_rect = UiRect::new(list_rect.left, top, list_rect.right, top + row_height(row));
        if row_rect.bottom < list_rect.top + scroll || row_rect.top > list_rect.bottom + scroll {
            if !matches!(row, Row::Header(_)) {
                item_index += 1;
            }
            continue;
        }
        let Row::Header(label) = row else {
            viewport_list = viewport_list.child(item_row(
                row_rect,
                row,
                item_index,
                selected_row == Some(index),
                &head_label,
                state.clone(),
                store.clone(),
            ));
            item_index += 1;
            continue;
        };
        viewport_list = viewport_list.child(text(
            UiRect::new(
                row_rect.left + 10.0,
                row_rect.top + 2.0,
                row_rect.right - 10.0,
                row_rect.bottom,
            ),
            *label,
            theme::mono_bold(theme::c().text_dim, theme::SMALL).tracking(0.8),
        ));
    }
    surface = surface.child(viewport_list);
    overlay.child(surface)
}

fn item_row(
    rect: UiRect,
    row: &Row,
    index: usize,
    selected: bool,
    head: &str,
    state: State<AppState>,
    store: State<GitStoreSnapshot>,
) -> Element {
    let (key, label, meta, current) = match row {
        Row::Create(name) => (
            "plus",
            format!("Create branch {name}"),
            format!("from {head}"),
            false,
        ),
        Row::Branch(branch) if branch.remote => {
            ("cloud", branch.name.clone(), String::new(), false)
        }
        Row::Branch(branch) => (
            if branch.current {
                "check"
            } else {
                "git-branch"
            },
            branch.name.clone(),
            branch.upstream.clone().unwrap_or_default(),
            branch.current,
        ),
        Row::Header(_) => unreachable!("headers are not items"),
    };
    let hover_state = state.clone();
    let click_state = state;
    let mut item = panel(
        rect,
        if selected {
            VisualStyle::filled(theme::c().selection)
        } else {
            VisualStyle::default()
        },
    )
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(Semantics::new(SemanticRole::Button).name(label.clone()))
    .cursor(CursorIcon::Pointer)
    .on_event(UiEventKind::PointerMove, move |_cx, _payload| {
        hover_state.try_update(move |app| {
            let query = app.branch_picker_input.text().to_owned();
            let Some(picker) = app.branch_picker.as_mut() else {
                return false;
            };
            let (selected, scroll) = picker.view(&query);
            if selected == index && picker.query == query {
                return false;
            }
            picker.selected = index;
            picker.scroll = scroll;
            picker.query = query;
            true
        });
    })
    .on_click(move || {
        choose(&click_state, &store, Some(index));
    });

    let icon_color = if current {
        theme::c().accent
    } else if selected {
        theme::c().text
    } else {
        theme::c().text_muted
    };
    item = item.child(icon(
        "branch-picker.row-icon",
        key,
        UiRect::new(
            rect.left + 10.0,
            rect.top + (rect.height() - 14.0) / 2.0,
            rect.left + 24.0,
            rect.top + (rect.height() + 14.0) / 2.0,
        ),
        icon_color,
    ));
    let meta_w = if meta.is_empty() {
        0.0
    } else {
        measure(&meta, theme::SMALL, 400) + TEXT_MARGIN
    };
    let label_right = (rect.right - 10.0 - meta_w - 8.0).max(rect.left + 32.0);
    item = item.child(text(
        UiRect::new(rect.left + 32.0, rect.top, label_right, rect.bottom),
        label,
        theme::mono(
            if selected {
                theme::c().text_bright
            } else {
                theme::c().text
            },
            theme::UI_SIZE,
        ),
    ));
    if !meta.is_empty() {
        item = item.child(text(
            UiRect::new(
                rect.right - 10.0 - meta_w,
                rect.top,
                rect.right - 10.0,
                rect.bottom,
            ),
            meta,
            theme::mono_right(theme::c().text_dim, theme::SMALL),
        ));
    }
    item
}

fn filter_state(app: &AppState) -> &InputState {
    &app.branch_picker_input
}

fn filter_state_mut(app: &mut AppState) -> &mut InputState {
    &mut app.branch_picker_input
}

#[cfg(test)]
mod tests {
    use super::*;

    fn branch(name: &str, remote: bool, current: bool, upstream: Option<&str>) -> BranchInfo {
        BranchInfo {
            name: name.into(),
            remote,
            current,
            oid: "0".into(),
            upstream: upstream.map(Into::into),
        }
    }

    fn sample() -> Vec<BranchInfo> {
        vec![
            branch("feature", false, false, None),
            branch("main", false, true, Some("origin/main")),
            branch("origin/main", true, false, None),
            branch("origin/release/1.0", true, false, None),
        ]
    }

    fn names(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Header(label) => format!("[{label}]"),
                Row::Create(name) => format!("+{name}"),
                Row::Branch(branch) => branch.name.clone(),
            })
            .collect()
    }

    #[test]
    fn lists_current_branch_first_then_remotes() {
        assert_eq!(
            names(&rows(&sample(), "")),
            [
                "[LOCAL]",
                "main",
                "feature",
                "[REMOTE]",
                "origin/main",
                "origin/release/1.0"
            ]
        );
    }

    #[test]
    fn filters_case_insensitively_and_offers_to_create() {
        assert_eq!(
            names(&rows(&sample(), "REL")),
            ["+REL", "[REMOTE]", "origin/release/1.0"]
        );
        // An existing name, local or remote, is not offered for creation.
        assert_eq!(names(&rows(&sample(), "feature")), ["[LOCAL]", "feature"]);
        assert_eq!(
            names(&rows(&sample(), "origin/main")),
            ["[REMOTE]", "origin/main"]
        );
        assert_eq!(names(&rows(&sample(), "no such")), Vec::<String>::new());
    }

    #[test]
    fn rejects_invalid_branch_names() {
        for name in [
            "", "-x", "a..b", "a b", "a~1", "x.lock", "a/", "/a", "@", "a@{1",
        ] {
            assert!(!valid_branch_name(name), "{name:?}");
        }
        for name in ["topic", "feature/login", "fix-1.2"] {
            assert!(valid_branch_name(name), "{name:?}");
        }
    }

    #[test]
    fn remote_rows_reuse_matching_local_branches() {
        let branches = sample();
        let remote = |name: &str| Row::Branch(branch(name, true, false, None));
        // Tracked by the current branch: nothing to do.
        assert_eq!(checkout_for(&remote("origin/main"), &branches), None);
        assert_eq!(
            checkout_for(&remote("origin/release/1.0"), &branches),
            Some(Checkout::Track("origin/release/1.0".into()))
        );
        assert_eq!(
            checkout_for(&remote("origin/feature"), &branches),
            Some(Checkout::Switch("feature".into()))
        );
        assert_eq!(
            checkout_for(&Row::Branch(branches[0].clone()), &branches),
            Some(Checkout::Switch("feature".into()))
        );
        assert_eq!(
            checkout_for(&Row::Branch(branches[1].clone()), &branches),
            None
        );
        assert_eq!(
            checkout_for(&Row::Create("topic".into()), &branches),
            Some(Checkout::Create("topic".into()))
        );
    }

    fn open_picker(app: &mut AppState, branches: Vec<BranchInfo>) {
        app.branch_picker = Some(BranchPicker {
            repository: PathBuf::from("repo"),
            repository_name: "repo".into(),
            anchor_x: 0.0,
            listing: BranchListing::Ready(branches.into()),
            selected: 0,
            scroll: 0.0,
            query: String::new(),
            sequence: 1,
            restore_terminal: false,
        });
    }

    #[test]
    fn selection_wraps_and_resets_when_the_filter_changes() {
        let mut app = AppState::new();
        open_picker(&mut app, sample());
        move_selection(&mut app, -1);
        assert_eq!(app.branch_picker.as_ref().unwrap().selected, 3);
        move_selection(&mut app, 1);
        assert_eq!(app.branch_picker.as_ref().unwrap().selected, 0);
        move_selection(&mut app, 1);
        app.branch_picker_input.set_text("rel");
        let picker = app.branch_picker.as_ref().unwrap();
        assert_eq!(picker.view("rel"), (0, 0.0));
    }

    #[test]
    fn keyboard_selection_scrolls_into_view() {
        let mut app = AppState::new();
        let branches = (0..30)
            .map(|index| branch(&format!("b{index:02}"), false, index == 0, None))
            .collect();
        open_picker(&mut app, branches);
        move_selection(&mut app, -1);
        let picker = app.branch_picker.as_ref().unwrap();
        assert_eq!(picker.selected, 29);
        // Header plus 30 rows, scrolled so the last row ends at the bottom.
        assert_eq!(picker.scroll, HEADER_H + 30.0 * ROW_H - MAX_LIST_H);
        move_selection(&mut app, 1);
        assert_eq!(app.branch_picker.as_ref().unwrap().scroll, 0.0);
    }
}
