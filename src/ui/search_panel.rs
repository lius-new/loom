//! Workspace search in the existing drawer, following the reviewed HTML design.
use std::sync::Arc;

use lgui::ApplicationHandle;
use lgui::core::{
    CursorIcon, EventPolicy, PointerButton, SemanticRole, Semantics, TextStyle, UiElement,
    UiFocusHandle, UiId, WheelUnit, clip,
};
use lgui::prelude::{Element, State, UiRect, VisualStyle, panel, text};
use unicode_segmentation::UnicodeSegmentation;

use super::components::input::{self, InputBinding, InputOptions, InputState, InputStyle};
use super::components::scrollbar;
use super::components::text::SingleLineText;
use super::sidebar::{drawer_header, drawer_icon};
use crate::search::{FileMatches, Results};
use crate::search_actions::{self, FILE_ROW_H, MATCH_ROW_H};
use crate::state::AppState;
use crate::theme;

// Search uses the compact UI scale for dense result lists.
const TEXT_SIZE: f32 = theme::SMALL;
const META_SIZE: f32 = 9.0;
const INPUT_H: f32 = 24.0;

fn file_labels(file: &FileMatches, multiple_roots: bool) -> (String, String) {
    // Buffer paths may use Windows' verbatim prefix while search roots do not.
    // Normalize both lexically; drawing rows must not access the filesystem.
    let path = std::path::PathBuf::from(super::display_path(&file.path));
    let root = std::path::PathBuf::from(super::display_path(&file.root));
    let in_root = path.strip_prefix(&root).ok();
    let relative = in_root.unwrap_or(&path);
    let parent = relative.parent().unwrap_or(std::path::Path::new(""));
    let context = if multiple_roots && in_root.is_some() {
        let mut context = root
            .file_name()
            .map(std::path::PathBuf::from)
            .unwrap_or_default();
        if !parent.as_os_str().is_empty() {
            context.push(parent);
        }
        context
    } else {
        parent.to_path_buf()
    };
    let labels = (
        relative.to_string_lossy().into_owned(),
        context.to_string_lossy().into_owned(),
    );
    #[cfg(windows)]
    let labels = (labels.0.replace('\\', "/"), labels.1.replace('\\', "/"));
    labels
}

#[derive(Clone)]
pub struct Inputs {
    pub ids: [UiId; 3],
    pub focus: [UiFocusHandle; 3],
    pub results_id: UiId,
    pub results_focus: UiFocusHandle,
}

fn fit_text(value: &str, rect: UiRect, style: TextStyle) -> Element {
    // Measure and paint on a wide single line, then clip to the control bounds.
    // A narrow Skia paragraph can otherwise wrap and drop text after its first line.
    let wide = UiRect::new(rect.left, rect.top, rect.left + 100_000.0, rect.bottom);
    let measured = SingleLineText::new(value, wide, style);
    if measured.width() <= rect.width() {
        return clip(rect, 0.0, 0.0).child(measured.element());
    }
    let boundaries = value
        .grapheme_indices(true)
        .map(|(at, _)| at)
        .collect::<Vec<_>>();
    let mut low = 0;
    let mut high = boundaries.len();
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        let end = boundaries.get(mid).copied().unwrap_or(value.len());
        if SingleLineText::new(format!("{}…", &value[..end]), wide, style).width() <= rect.width()
        {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let end = boundaries.get(low).copied().unwrap_or(value.len());
    clip(rect, 0.0, 0.0).child(text(wide, format!("{}…", &value[..end]), style))
}

fn control_panel(rect: UiRect, active: bool) -> Element {
    let style = if active {
        VisualStyle::filled(theme::c().accent).alpha(28).radius(3.0)
    } else {
        VisualStyle::default()
    };
    panel(rect, style)
        .event_policy(EventPolicy {
            hover: true,
            press: true,
            focus: false,
        })
        .cursor(CursorIcon::Pointer)
}

fn icon_button(
    rect: UiRect,
    icon: &'static str,
    name: &'static str,
    active: bool,
    click: impl Fn() + Send + Sync + 'static,
) -> Element {
    let mut semantics = Semantics::new(SemanticRole::Button).name(name);
    semantics.state.selected = active;
    control_panel(rect, active)
        .key(format!("search-button-{name}"))
        .semantics(semantics)
        .on_click(click)
        .child(drawer_icon(
            UiId::owned(format!("search-icon-{name}")),
            icon,
            theme::icon_rect(rect),
            if active {
                theme::c().accent
            } else {
                theme::c().text_soft
            },
        ))
}

fn filter_count(app: &AppState) -> usize {
    usize::from(!app.search.include.text().trim().is_empty())
        + usize::from(!app.search.exclude.text().trim().is_empty())
        + usize::from(app.search.include_hidden)
        + usize::from(app.search.include_ignored)
}

/// Each option has a 24px target, a 2px gap and a 6px right inset.
fn query_option_rects(query: UiRect) -> Option<[UiRect; 3]> {
    if query.width() < 132.0 {
        return None;
    }
    let first = query.right - 6.0 - 76.0;
    Some(std::array::from_fn(|index| {
        let left = first + index as f32 * 26.0;
        UiRect::new(left, query.top + 2.0, left + 24.0, query.bottom - 2.0)
    }))
}

pub fn render(
    rect: UiRect,
    state: State<AppState>,
    application: ApplicationHandle,
    editor_focus: UiFocusHandle,
    editor_rect: UiRect,
    inputs: Inputs,
) -> Element {
    let app = state.get();
    let search = &app.search;
    let header = UiRect::new(rect.left, rect.top, rect.right, rect.top + theme::TABS_H);
    let mut root = panel(rect, VisualStyle::filled(theme::c().sidebar))
        .child(drawer_header(header, state.clone()));

    // Mode icons and collapse stay in the shared drawer header.
    if rect.width() >= 200.0 && !search.query.text().is_empty() {
        if search.running || (search.error.is_none() && !app.workspace_folders.is_empty()) {
            let action = state.clone();
            root = root.child(icon_button(
                UiRect::new(
                    rect.right - 96.0,
                    rect.top,
                    rect.right - 68.0,
                    header.bottom,
                ),
                if search.running {
                    "search-stop"
                } else {
                    "refresh"
                },
                if search.running {
                    "Stop search"
                } else {
                    "Refresh search"
                },
                false,
                move || {
                    if action.get().search.running {
                        search_actions::cancel(&action);
                    } else {
                        search_actions::refresh(&action);
                    }
                },
            ));
        }
        let clear = state.clone();
        let query_focus = inputs.focus[0].clone();
        root = root.child(icon_button(
            UiRect::new(
                rect.right - 64.0,
                rect.top,
                rect.right - 36.0,
                header.bottom,
            ),
            "search-clear",
            "Clear search",
            false,
            move || {
                search_actions::clear(&clear);
                query_focus.focus();
            },
        ));
    }

    let left = rect.left + 12.0;
    let right = (rect.right - 12.0).max(left);
    let mut top = header.bottom + 8.0;
    let query = UiRect::new(left, top, right, top + INPUT_H);
    let options = query_option_rects(query);
    let input_right = options
        .as_ref()
        .map_or(right - 4.0, |areas| areas[0].left - 2.0);
    root = root.child(
        theme::bordered(
            query,
            theme::c().surface,
            if search.error.is_some() {
                theme::c().error
            } else if search.query.focused {
                theme::c().accent
            } else {
                theme::c().border
            },
            4.0,
            1.0,
        )
        .child(search_input(
            UiRect::new(
                left + 2.0,
                top + 1.0,
                input_right.max(left + 2.0),
                top + INPUT_H - 1.0,
            ),
            state.clone(),
            inputs.ids[0].clone(),
            inputs.focus[0].clone(),
            0,
        )),
    );
    if let Some(areas) = options {
        for (index, icon, name, active) in [
            (
                0,
                "search-case",
                "Match case (Alt+C)",
                search.case_sensitive,
            ),
            (
                1,
                "search-word",
                "Match whole word (Alt+W)",
                search.whole_word,
            ),
            (
                2,
                "search-regex",
                "Use regular expression (Alt+R)",
                search.regex,
            ),
        ] {
            let toggle = state.clone();
            root = root.child(icon_button(areas[index], icon, name, active, move || {
                toggle.update(|app| match index {
                    0 => app.search.case_sensitive = !app.search.case_sensitive,
                    1 => app.search.whole_word = !app.search.whole_word,
                    _ => app.search.regex = !app.search.regex,
                });
            }));
        }
    }
    top += INPUT_H;
    let disclosure = UiRect::new(left, top + 2.0, (left + 84.0).min(right), top + 26.0);
    let filters = state.clone();
    let focus = inputs.focus[0].clone();
    let filter_button = control_panel(disclosure, false)
        .key("search-filter-disclosure")
        .semantics({
            let mut semantics = Semantics::new(SemanticRole::Button).name("Filters");
            semantics.state.expanded = Some(search.filters_open);
            semantics
        })
        .on_click(move || {
            let closing_input = filters.get().search.filters_open
                && (filters.get().search.include.focused || filters.get().search.exclude.focused);
            filters.update(|app| search_actions::toggle_filters(&mut app.search));
            if closing_input {
                focus.focus();
            }
        })
        .child(drawer_icon(
            "search-filter-chevron",
            if search.filters_open {
                "chevron-down"
            } else {
                "chevron-right"
            },
            UiRect::new(left, top + 8.0, left + 12.0, top + 20.0),
            theme::c().text_muted,
        ))
        .child(text(
            UiRect::new(left + 18.0, top, left + 54.0, top + 28.0),
            "Filters",
            theme::sans(theme::c().text_soft, TEXT_SIZE),
        ));
    root = root.child(filter_button);
    let active_filters = filter_count(&app);
    if active_filters > 0 {
        let badge = UiRect::new(left + 50.0, top + 7.0, left + 64.0, top + 21.0);
        root = root
            .child(panel(
                badge,
                VisualStyle::filled(theme::c().accent).alpha(24).radius(8.0),
            ))
            .child(text(
                badge,
                active_filters.to_string(),
                theme::sans(theme::c().accent, META_SIZE).centered(),
            ));
        let reset = state.clone();
        root = root.child(icon_button(
            UiRect::new(right - 24.0, top + 2.0, right, top + 26.0),
            "search-clear",
            "Reset filters",
            false,
            move || search_actions::reset_filters(&reset),
        ));
    }
    top += 28.0;
    if search.filters_open {
        for (index, label) in [(1, "Files to include"), (2, "Files to exclude")] {
            root = root.child(text(
                UiRect::new(left, top, right, top + 14.0),
                label,
                theme::sans(theme::c().text_soft, TEXT_SIZE),
            ));
            top += 14.0;
            let focused = if index == 1 {
                search.include.focused
            } else {
                search.exclude.focused
            };
            root = root.child(
                theme::bordered(
                    UiRect::new(left, top, right, top + INPUT_H),
                    theme::c().surface,
                    if focused {
                        theme::c().accent
                    } else {
                        theme::c().border
                    },
                    3.0,
                    1.0,
                )
                .child(search_input(
                    UiRect::new(
                        left + 2.0,
                        top + 1.0,
                        (right - 2.0).max(left + 2.0),
                        top + INPUT_H - 1.0,
                    ),
                    state.clone(),
                    inputs.ids[index].clone(),
                    inputs.focus[index].clone(),
                    index,
                )),
            );
            top += INPUT_H + 8.0;
        }
        for (index, label, checked) in [
            (0, "Search hidden files", search.include_hidden),
            (1, "Search ignored files", search.include_ignored),
        ] {
            let toggle = state.clone();
            root = root.child(checkbox(
                UiRect::new(left, top, right, top + 22.0),
                label,
                checked,
                move || {
                    toggle.update(|app| {
                        if index == 0 {
                            app.search.include_hidden = !app.search.include_hidden
                        } else {
                            app.search.include_ignored = !app.search.include_ignored
                        }
                    });
                },
            ));
            top += 22.0;
        }
        top += 6.0;
        root = root.child(panel(
            UiRect::new(left, top, right, top + 1.0),
            VisualStyle::filled(theme::c().border),
        ));
        top += 4.0;
    }

    if let Some(error) = &search.error {
        let area = UiRect::new(left, top, right, top + 26.0);
        root = root.child(control_panel(area, false).child(fit_text(
            error.lines().last().unwrap_or(error),
            area,
            theme::sans(theme::c().error_text, TEXT_SIZE),
        )));
        top += 26.0;
    } else if !search.query.text().is_empty() && !app.workspace_folders.is_empty() {
        let prefix = if search.running {
            "Searching… "
        } else if search.cancelled {
            "Stopped · "
        } else {
            ""
        };
        let result_label = if search.results.matches == 1 {
            "result"
        } else {
            "results"
        };
        let file_label = if search.results.files.len() == 1 {
            "file"
        } else {
            "files"
        };
        let status = if rect.width() > 260.0 {
            format!(
                "{prefix}{} {result_label} in {} {file_label}",
                number(search.results.matches),
                number(search.results.files.len())
            )
        } else {
            format!("{prefix}{} {result_label}", number(search.results.matches))
        };
        let summary = UiRect::new(left, top, (right - 28.0).max(left), top + 28.0);
        root = root.child(fit_text(
            &status,
            summary,
            theme::sans(theme::c().text_soft, TEXT_SIZE),
        ));
        if !search.results.files.is_empty() {
            let collapse = state.clone();
            root = root.child(icon_button(
                UiRect::new(right - 24.0, top + 2.0, right, top + 26.0),
                "search-collapse",
                "Collapse or expand all results",
                false,
                move || {
                    collapse.update(|app| {
                        if app
                            .search
                            .results
                            .files
                            .iter()
                            .all(|f| app.search.collapsed.contains(&f.path))
                        {
                            app.search.collapsed.clear();
                        } else {
                            for file in &app.search.results.files {
                                app.search.collapsed.insert(file.path.clone());
                            }
                        }
                    })
                },
            ));
        }
        top += 28.0;
        if search.results.limit_hit {
            root = root.child(fit_text(
                "Result limit reached. Narrow your search.",
                UiRect::new(left, top, right, top + 26.0),
                theme::sans(theme::c().warning, META_SIZE),
            ));
            top += 26.0;
        }
    }

    let footer_height =
        if search.error.is_none() && (search.results.skipped > 0 || search.results.errors > 0) {
            26.0 + if search.details_open {
                detail_lines(&search.results).len() as f32 * 18.0 + 6.0
            } else {
                0.0
            }
        } else {
            0.0
        };
    // A short window keeps the controls clipped and the results viewport ordered.
    let bottom = (rect.bottom - footer_height).max(top.min(rect.bottom));
    let viewport = UiRect::new(rect.left, (top + 2.0).min(bottom), rect.right, bottom);
    root = root.child(results(
        viewport,
        state.clone(),
        application,
        editor_focus,
        editor_rect,
        inputs,
        search.results.clone(),
    ));
    if footer_height > 0.0 && bottom < rect.bottom {
        root = root.child(footer(
            UiRect::new(rect.left, bottom, rect.right, rect.bottom),
            &search.results,
            search.details_open,
            state.clone(),
        ));
    }
    clip(rect, 0.0, 0.0).child(root)
}

fn number(value: usize) -> String {
    let raw = value.to_string();
    let mut out = String::with_capacity(raw.len() + raw.len() / 3);
    for (index, ch) in raw.chars().enumerate() {
        if index > 0 && (raw.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn checkbox(
    rect: UiRect,
    label: &'static str,
    checked: bool,
    click: impl Fn() + Send + Sync + 'static,
) -> Element {
    let mut semantics = Semantics::new(SemanticRole::CheckBox).name(label);
    semantics.state.checked = Some(checked);
    let square = UiRect::new(rect.left, rect.top + 5.0, rect.left + 12.0, rect.top + 17.0);
    let mut row = control_panel(rect, false)
        .key(label)
        .semantics(semantics)
        .on_click(click)
        .child(theme::bordered(
            square,
            if checked {
                theme::c().accent
            } else {
                theme::c().surface
            },
            if checked {
                theme::c().accent
            } else {
                theme::c().border
            },
            2.0,
            1.0,
        ))
        .child(text(
            UiRect::new(rect.left + 20.0, rect.top, rect.right, rect.bottom),
            label,
            theme::sans(theme::c().text_soft, TEXT_SIZE),
        ));
    if checked {
        row = row.child(drawer_icon(
            UiId::owned(format!("search-check-{label}")),
            "check",
            square,
            theme::c().on_accent,
        ));
    }
    row
}

fn search_input(
    rect: UiRect,
    state: State<AppState>,
    id: UiId,
    focus: UiFocusHandle,
    index: usize,
) -> Element {
    type Reader = fn(&AppState) -> &InputState;
    type Writer = fn(&mut AppState) -> &mut InputState;
    let (read, write): (Reader, Writer) = match index {
        1 => (|app| &app.search.include, |app| &mut app.search.include),
        2 => (|app| &app.search.exclude, |app| &mut app.search.exclude),
        _ => (|app| &app.search.query, |app| &mut app.search.query),
    };
    input::render(
        rect,
        id,
        focus,
        InputBinding::new(state, read, write),
        InputOptions {
            label: match index {
                1 => "Files to include",
                2 => "Files to exclude",
                _ => "Search workspace",
            },
            placeholder: match index {
                1 => "src/**, *.rs",
                2 => "target/**, *.lock",
                _ => "Search",
            },
            style: InputStyle {
                text: theme::mono(theme::c().text, TEXT_SIZE),
                placeholder: theme::mono(theme::c().text_muted, TEXT_SIZE),
                caret: theme::c().accent,
                selection: theme::c().accent,
            },
            on_submit: Some(search_actions::refresh),
            on_cancel: Some(search_actions::cancel),
            on_blur: None,
        },
    )
}

fn detail_lines(results: &Results) -> Vec<(&'static str, usize)> {
    [
        ("Binary files", results.skipped_binary),
        ("Files larger than 4 MiB", results.skipped_large),
        ("Unsupported encoding", results.skipped_encoding),
        ("Unreadable files", results.errors),
    ]
    .into_iter()
    .filter(|(_, n)| *n > 0)
    .collect()
}

fn footer(rect: UiRect, results: &Results, open: bool, state: State<AppState>) -> Element {
    let mut root = panel(rect, VisualStyle::filled(theme::c().sidebar)).child(panel(
        UiRect::new(rect.left, rect.top, rect.right, rect.top + 1.0),
        VisualStyle::filled(theme::c().border),
    ));
    let label = if results.errors > 0 && results.skipped > 0 {
        format!(
            "{} skipped · {} unreadable",
            number(results.skipped),
            number(results.errors)
        )
    } else if results.errors > 0 {
        format!("{} unreadable files", number(results.errors))
    } else {
        format!("{} files skipped", number(results.skipped))
    };
    let toggle = state.clone();
    let hit = UiRect::new(
        rect.left + 12.0,
        rect.top + 2.0,
        rect.right - 12.0,
        rect.top + 24.0,
    );
    let button = control_panel(hit, false)
        .key("search-skip-details")
        .semantics(Semantics::new(SemanticRole::Button).name("Search file details"))
        .on_click(move || toggle.update(|app| app.search.details_open = !app.search.details_open))
        .child(drawer_icon(
            "search-details-chevron",
            if open {
                "chevron-down"
            } else {
                "chevron-right"
            },
            UiRect::new(hit.left, hit.top + 5.0, hit.left + 12.0, hit.top + 17.0),
            theme::c().text_muted,
        ))
        .child(fit_text(
            &label,
            UiRect::new(hit.left + 18.0, hit.top, hit.right, hit.bottom),
            theme::sans(
                if results.errors > 0 {
                    theme::c().warning
                } else {
                    theme::c().text_muted
                },
                META_SIZE,
            ),
        ));
    root = root.child(button);
    if open {
        for (index, (label, value)) in detail_lines(results).into_iter().enumerate() {
            let top = rect.top + 26.0 + index as f32 * 18.0;
            root = root
                .child(fit_text(
                    label,
                    UiRect::new(rect.left + 30.0, top, rect.right - 48.0, top + 18.0),
                    theme::sans(theme::c().text_soft, META_SIZE),
                ))
                .child(text(
                    UiRect::new(rect.right - 44.0, top, rect.right - 12.0, top + 18.0),
                    number(value),
                    {
                        let mut style = theme::sans(theme::c().text_soft, META_SIZE);
                        style.align = lgui::core::TextAlign::Right;
                        style
                    },
                ));
        }
    }
    root
}

/// Index file groups in O(files); allocate nodes only for rows in the viewport.
fn results(
    rect: UiRect,
    state: State<AppState>,
    application: ApplicationHandle,
    editor_focus: UiFocusHandle,
    editor_rect: UiRect,
    inputs: Inputs,
    results: Arc<Results>,
) -> Element {
    let app = state.get();
    let mut content_h = 0.0;
    let groups = results
        .files
        .iter()
        .map(|file| {
            let top = content_h;
            let expanded = !app.search.collapsed.contains(&file.path);
            content_h += FILE_ROW_H
                + if expanded {
                    file.matches.len() as f32 * MATCH_ROW_H
                } else {
                    0.0
                };
            (top, expanded, file.clone())
        })
        .collect::<Vec<_>>();
    let max_scroll = (content_h - rect.height()).max(0.0);
    let scroll = app.search.scroll_y.clamp(0.0, max_scroll);
    let wheel_state = state.clone();
    let focus_state = state.clone();
    let blur_state = state.clone();
    let viewport_state = state.clone();
    let mut root = Element::new(move |cx| {
        UiElement::panel(inputs.results_id, rect, VisualStyle::default()).children(cx.children)
    })
    .event_policy(EventPolicy::INTERACTIVE)
    .semantics(Semantics::new(SemanticRole::Group).name("Search results"))
    .on_focus(move |_| {
        focus_state.update(|app| {
            app.search.results_focused = true;
            app.search.viewport_height = rect.height();
            search_actions::reveal_selection(app);
        })
    })
    .on_blur(move |_| blur_state.update(|app| app.search.results_focused = false))
    .on_event_capture(lgui::core::UiEventKind::PointerDown, move |_, _| {
        viewport_state.try_update(|app| {
            let changed = app.search.viewport_height != rect.height();
            app.search.viewport_height = rect.height();
            changed
        });
    })
    .on_wheel(move |cx, delta| {
        let step = match delta.unit {
            WheelUnit::Lines => delta.y * MATCH_ROW_H * 3.0,
            WheelUnit::Pixels => delta.y,
        };
        wheel_state.update(|app| {
            app.search.scroll_y = (scroll - step).clamp(0.0, max_scroll);
            app.search.viewport_height = rect.height();
        });
        cx.stop_propagation();
    });
    let mut clipped = clip(rect, 0.0, 0.0);
    for (offset, expanded, file) in groups {
        let end = offset
            + FILE_ROW_H
            + if expanded {
                file.matches.len() as f32 * MATCH_ROW_H
            } else {
                0.0
            };
        if offset >= scroll + rect.height() || end <= scroll {
            continue;
        }
        if offset + FILE_ROW_H > scroll {
            let area = UiRect::new(
                rect.left,
                rect.top + offset - scroll,
                rect.right,
                rect.top + offset - scroll + FILE_ROW_H,
            );
            let toggle = state.clone();
            let group_focus = inputs.results_focus.clone();
            let path = file.path.clone();
            let (relative, context) = file_labels(&file, app.workspace_folders.len() > 1);
            let name = file.path.file_name().unwrap_or_default().to_string_lossy();
            let count = number(file.matches.len());
            let badge_w =
                (SingleLineText::new(&count, area, theme::mono(theme::c().text_soft, META_SIZE))
                    .width()
                    + 10.0)
                    .max(16.0);
            let badge = UiRect::new(
                area.right - 12.0 - badge_w,
                area.top + 4.0,
                area.right - 12.0,
                area.top + 20.0,
            );
            let name_left = area.left + 45.0;
            let available = (badge.left - 8.0 - name_left).max(0.0);
            let name_w = SingleLineText::new(
                name.as_ref(),
                area,
                theme::mono(theme::c().text_soft, TEXT_SIZE),
            )
            .width()
            .min(available);
            let name_rect = UiRect::new(name_left, area.top, name_left + name_w, area.bottom);
            let mut heading = control_panel(area, false)
                .key(format!("search-file-{}", file.path.display()))
                .semantics({
                    let mut semantics = Semantics::new(SemanticRole::Button).name(relative);
                    semantics.state.expanded = Some(expanded);
                    semantics
                })
                .on_click(move || {
                    group_focus.focus();
                    toggle.update(|app| {
                        if !app.search.collapsed.remove(&path) {
                            app.search.collapsed.insert(path.clone());
                        }
                    });
                })
                .child(drawer_icon(
                    UiId::owned(format!("search-chevron-{}", file.path.display())),
                    if expanded {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    },
                    UiRect::new(
                        area.left + 9.0,
                        area.top + 6.0,
                        area.left + 21.0,
                        area.top + 18.0,
                    ),
                    theme::c().text_muted,
                ))
                .child(drawer_icon(
                    UiId::owned(format!("search-file-icon-{}", file.path.display())),
                    crate::file_icons::icon_for_file(&name),
                    UiRect::new(
                        area.left + 25.0,
                        area.top + 5.0,
                        area.left + 39.0,
                        area.top + 19.0,
                    ),
                    theme::c().text_soft,
                ))
                .child(fit_text(
                    &name,
                    name_rect,
                    theme::mono(theme::c().text_soft, TEXT_SIZE),
                ))
                .child(panel(
                    badge,
                    VisualStyle::filled(theme::c().active_line).radius(8.0),
                ))
                .child(text(
                    badge,
                    count,
                    theme::mono(theme::c().text_soft, META_SIZE).centered(),
                ));
            if rect.width() > 260.0 && available - name_w > 20.0 {
                heading = heading.child(fit_text(
                    &context,
                    UiRect::new(
                        name_rect.right + 8.0,
                        area.top,
                        badge.left - 8.0,
                        area.bottom,
                    ),
                    theme::mono(theme::c().text_muted, META_SIZE),
                ));
            }
            clipped = clipped.child(heading);
        }
        if expanded {
            let first = ((scroll - offset - FILE_ROW_H) / MATCH_ROW_H)
                .floor()
                .max(0.0) as usize;
            let last = ((scroll + rect.height() - offset - FILE_ROW_H) / MATCH_ROW_H)
                .ceil()
                .max(0.0) as usize;
            for index in first..file.matches.len().min(last) {
                let top = rect.top + offset + FILE_ROW_H + index as f32 * MATCH_ROW_H - scroll;
                clipped = clipped.child(match_row(
                    UiRect::new(rect.left, top, rect.right - 12.0, top + MATCH_ROW_H),
                    state.clone(),
                    application.clone(),
                    editor_focus.clone(),
                    editor_rect,
                    inputs.results_focus.clone(),
                    file.clone(),
                    index,
                    app.search.selected.as_ref() == Some(&(file.path.clone(), index)),
                ));
            }
        }
    }
    if results.matches == 0 && app.search.error.is_none() {
        let message = if app.workspace_folders.is_empty() {
            "Open a folder to search."
        } else if app.search.query.text().is_empty() {
            "Search across your workspace"
        } else if app.search.running {
            "Searching workspace…"
        } else if app.search.cancelled {
            "Search stopped."
        } else if results.complete {
            "No results found."
        } else {
            ""
        };
        clipped = clipped.child(fit_text(
            message,
            UiRect::new(
                rect.left + 16.0,
                rect.top + 7.0,
                (rect.right - 16.0).max(rect.left + 16.0),
                rect.top + 31.0,
            ),
            theme::sans(theme::c().text_soft, TEXT_SIZE),
        ));
    }
    root = root.child(clipped);
    if max_scroll > 0.0 && rect.height() > 0.0 {
        let thumb_h = (rect.height() * rect.height() / content_h)
            .max(scrollbar::MIN_THUMB_LENGTH)
            .min(rect.height());
        let travel = (rect.height() - thumb_h).max(0.0);
        let top = rect.top + travel * scroll / max_scroll;
        let down = state.clone();
        let drag = state.clone();
        let up = state;
        root = root.child(
            scrollbar::thumb(
                UiRect::new(rect.right - 9.0, top, rect.right - 2.0, top + thumb_h),
                100,
            )
            .on_pointer_down(move |cx, p| {
                down.update(|app| app.search.scrollbar_drag = Some((p.point.y, scroll)));
                cx.stop_propagation();
            })
            .on_pointer_drag(move |cx, p| {
                drag.update(|app| {
                    if let Some((origin, start)) = app.search.scrollbar_drag {
                        app.search.scroll_y = (start
                            + (p.point.y - origin) * max_scroll / travel.max(1.0))
                        .clamp(0.0, max_scroll);
                    }
                });
                cx.stop_propagation();
            })
            .on_pointer_up(move |cx, _| {
                up.update(|app| app.search.scrollbar_drag = None);
                cx.stop_propagation();
            }),
        );
    }
    root
}

/// Crop at grapheme boundaries and keep the first matched character visible.
fn match_preview(
    found: &crate::search::Match,
    bounds: UiRect,
) -> (SingleLineText, std::ops::Range<usize>) {
    let raw = found.preview.replace('\t', " ");
    let leading = raw.len() - raw.trim_start().len();
    // Remove indentation unless it contains the match itself.
    let leading = if leading <= found.highlight.start {
        leading
    } else {
        0
    };
    let value = &raw[leading..];
    let source_highlight = found.highlight.start - leading..found.highlight.end - leading;
    let style = theme::mono(theme::c().text_soft, TEXT_SIZE);
    let wide = UiRect::new(
        bounds.left,
        bounds.top,
        bounds.left + 100_000.0,
        bounds.bottom,
    );
    let measured = SingleLineText::new(value, wide, style);
    if measured.width() <= bounds.width() {
        return (measured, source_highlight);
    }
    let offsets = value
        .grapheme_indices(true)
        .map(|(offset, _)| offset)
        .chain(std::iter::once(value.len()))
        .collect::<Vec<_>>();
    let target = (measured.caret_offset(source_highlight.start) - bounds.width() * 0.25).max(0.0);
    let start = offsets
        .iter()
        .copied()
        .take_while(|offset| {
            *offset <= source_highlight.start && measured.caret_offset(*offset) <= target
        })
        .last()
        .unwrap_or(0);
    let prefix = if start > 0 { "…" } else { "" };
    let ellipsis_w = SingleLineText::new("…", bounds, style).width();
    let available = (bounds.width() - ellipsis_w * if start > 0 { 2.0 } else { 1.0 }).max(0.0);
    let origin = measured.caret_offset(start);
    let end = offsets
        .into_iter()
        .filter(|offset| *offset >= start)
        .take_while(|offset| measured.caret_offset(*offset) - origin <= available)
        .last()
        .unwrap_or(start);
    let suffix = if end < value.len() { "…" } else { "" };
    let visible = format!("{prefix}{}{suffix}", &value[start..end]);
    let highlight = prefix.len()
        + source_highlight
            .start
            .saturating_sub(start)
            .min(end - start)
        ..prefix.len() + source_highlight.end.saturating_sub(start).min(end - start);
    (SingleLineText::new(visible, wide, style), highlight)
}

fn match_row(
    rect: UiRect,
    state: State<AppState>,
    application: ApplicationHandle,
    editor_focus: UiFocusHandle,
    editor_rect: UiRect,
    results_focus: UiFocusHandle,
    file: Arc<FileMatches>,
    index: usize,
    selected: bool,
) -> Element {
    let found = &file.matches[index];
    let line_rect = UiRect::new(rect.left + 27.0, rect.top, rect.left + 52.0, rect.bottom);
    let bounds = UiRect::new(line_rect.right + 10.0, rect.top, rect.right, rect.bottom);
    let (measured, highlight) = match_preview(found, bounds);
    let highlight_left = measured.caret_x(highlight.start);
    let highlight_right = measured.caret_x(highlight.end).max(highlight_left + 2.0);
    let click_file = file.clone();
    let mut row = control_panel(rect, false)
        .key(format!("search-match-{}-{index}", file.path.display()))
        .semantics({
            let mut semantics = Semantics::new(SemanticRole::Button).name(format!(
                "Line {}: {}",
                found.line + 1,
                found.preview
            ));
            semantics.state.selected = selected;
            semantics
        })
        .on_pointer_down_with_button(move |cx, pointer, button| {
            if button != PointerButton::Left {
                return;
            }
            let mut clicks = 1;
            state.update(|app| {
                clicks =
                    app.editor
                        .tree_click_count(&click_file.path, pointer.point.x, pointer.point.y);
            });
            results_focus.focus();
            search_actions::navigate(
                &state,
                application.clone(),
                editor_rect,
                editor_focus.clone(),
                click_file.clone(),
                index,
                clicks > 1,
                clicks > 1,
            );
            cx.stop_propagation();
        });
    if selected {
        row = row
            .child(panel(rect, VisualStyle::filled(theme::c().active_line)))
            .child(panel(
                UiRect::new(rect.left, rect.top, rect.left + 2.0, rect.bottom),
                VisualStyle::filled(theme::c().accent),
            ));
    }
    row = row.child(text(
        line_rect,
        (found.line + 1).to_string(),
        theme::mono_right(theme::c().text_muted, META_SIZE),
    ));
    row.child(
        clip(bounds, 0.0, 0.0)
            .child(panel(
                UiRect::new(
                    highlight_left,
                    rect.top + 4.0,
                    highlight_right,
                    rect.bottom - 4.0,
                ),
                VisualStyle::filled(theme::c().accent).alpha(30).radius(2.0),
            ))
            .child(measured.element()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgui::application::{AppView, ApplicationContext};
    use lgui::core::{
        InputEvent, PhysicalRect, Point, PointerButton, PointerData, UiScale,
        dispatch_runtime_output,
    };
    use lgui::render_skia::SkiaSoftwareSurface;
    use lgui::renderer::{FrameInfo, FrameReason};
    use lgui::session::UiSession;
    use std::sync::Mutex;

    #[cfg(windows)]
    #[test]
    fn search_file_labels_normalize_verbatim_paths_before_relativizing() {
        let file = |path: &str, root: &str| FileMatches {
            path: path.into(),
            root: root.into(),
            matches: Vec::new(),
            content_hash: 0,
        };
        for (path, root) in [
            (
                r"\\?\C:\Users\yys\msites\cmd\matomo-db\main.go",
                r"C:\Users\yys\msites",
            ),
            (
                r"C:\Users\yys\msites\cmd\matomo-db\main.go",
                r"\\?\C:\Users\yys\msites",
            ),
            (
                r"\\?\UNC\server\share\msites\cmd\matomo-db\main.go",
                r"\\server\share\msites",
            ),
        ] {
            let result = file(path, root);
            assert_eq!(
                file_labels(&result, false),
                ("cmd/matomo-db/main.go".into(), "cmd/matomo-db".into())
            );
            assert_eq!(
                file_labels(&result, true),
                (
                    "cmd/matomo-db/main.go".into(),
                    "msites/cmd/matomo-db".into()
                )
            );
        }
        let top_level = file(r"\\?\C:\msites\main.go", r"C:\msites");
        assert_eq!(
            file_labels(&top_level, false),
            ("main.go".into(), String::new())
        );
        assert_eq!(
            file_labels(&top_level, true),
            ("main.go".into(), "msites".into())
        );
        let outside = file(r"\\?\C:\other\main.go", r"C:\msites");
        assert_eq!(
            file_labels(&outside, true),
            ("C:/other/main.go".into(), "C:/other".into())
        );
    }

    fn click(session: &mut UiSession, context: &ApplicationContext, x: f32, y: f32) {
        for input in [
            InputEvent::PointerDown {
                pointer: PointerData::mouse(Point::new(x, y)),
                button: PointerButton::Left,
            },
            InputEvent::PointerUp {
                pointer: PointerData::mouse(Point::new(x, y)),
                button: PointerButton::Left,
            },
        ] {
            dispatch_runtime_output(
                session.handle_input(input),
                context,
                &lgui::window::WindowId::new("search-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
        }
    }

    fn click_named(session: &mut UiSession, context: &ApplicationContext, name: &str) {
        let rect = session
            .tree()
            .nodes()
            .iter()
            .find(|node| {
                node.semantics
                    .as_ref()
                    .and_then(|semantics| semantics.name.as_deref())
                    == Some(name)
            })
            .unwrap_or_else(|| panic!("Missing control: {name}"))
            .hit_rect;
        click(
            session,
            context,
            (rect.left + rect.right) / 2.0,
            (rect.top + rect.bottom) / 2.0,
        );
    }

    fn render_pixels(
        session: &UiSession,
        context: &ApplicationContext,
        width: f32,
        height: f32,
    ) -> Vec<u8> {
        let physical = PhysicalRect::new(0, 0, width as i32, height as i32);
        let dirty = [physical];
        let frame = FrameInfo::new(
            physical,
            &dirty,
            UiScale::ONE,
            FrameReason::SceneChange,
            true,
        );
        let mut surface = SkiaSoftwareSurface::new(16 * 1024 * 1024);
        lgui::assets::backend::with_render_resources(
            context,
            lgui::assets::backend::render_resources(),
            || {
                surface.draw(
                    &session.tree().scene().project_to_physical(UiScale::ONE),
                    &frame,
                )
            },
        )
        .unwrap();
        let mut rgba = surface.pixels().to_vec();
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        rgba
    }

    fn save_preview(
        session: &UiSession,
        context: &ApplicationContext,
        width: f32,
        height: f32,
        name: &str,
    ) {
        let Some(directory) = std::env::var_os("LOOM_SEARCH_PREVIEW_DIR") else {
            return;
        };
        let rgba = render_pixels(session, context, width, height);
        image::save_buffer(
            std::path::Path::new(&directory).join(format!("{name}-{}.png", width as u32)),
            &rgba,
            width as u32,
            height as u32,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }

    #[test]
    fn search_panel_renders_bounded_rows_and_switches_drawer_at_normal_and_small_sizes() {
        let context = ApplicationContext::empty(Default::default());
        context
            .resources()
            .provide(lgui::icons::IconRegistration(Arc::new(crate::svg_icons())));
        let _fonts = lgui::backend::install_text_environment(
            &context,
            lgui::render_skia::skia_text_system_handle(),
        );
        for (width, height) in [(220.0, 600.0), (340.0, 600.0), (80.0, 80.0)] {
            let viewport = UiRect::new(0.0, 0.0, width, height);
            let exposed = Arc::new(Mutex::new(None::<State<AppState>>));
            let output = exposed.clone();
            let application = ApplicationHandle::new(|task| task(), || {});
            let view: AppView = Arc::new(move |cx| {
                let state = cx.state_with(|| {
                    let mut app = AppState::new();
                    app.apply(crate::input::action::Action::OpenSearch);
                    app.workspace_folders.push("demo".into());
                    app.search.query.set_text("needle");
                    app.search.filters_open = true;
                    app.search.include.set_text("*.rs, *.ts");
                    let spec = app.search.spec();
                    let matcher = crate::search::Matcher::new(&spec).unwrap();
                    let value = "let 中文 = needle; // search preview\n".repeat(10_000);
                    let matches =
                        matcher.matches(&value, &std::sync::atomic::AtomicBool::new(false), 10_000);
                    let file = Arc::new(FileMatches {
                        path: "demo/src/main.rs".into(),
                        root: "demo".into(),
                        content_hash: crate::search::hash(&value),
                        matches,
                    });
                    app.search.results = Arc::new(Results {
                        files: vec![file],
                        matches: 10_000,
                        searched: 100,
                        complete: true,
                        ..Results::default()
                    });
                    app
                });
                *output.lock().unwrap() = Some(state.clone());
                let ids = [
                    UiId::new("search-test-query"),
                    UiId::new("search-test-include"),
                    UiId::new("search-test-exclude"),
                ];
                let focus = [
                    cx.focus_handle(ids[0].clone()),
                    cx.focus_handle(ids[1].clone()),
                    cx.focus_handle(ids[2].clone()),
                ];
                let results_id = UiId::new("search-test-results");
                let editor_focus = cx.focus_handle(UiId::new("search-test-editor"));
                render(
                    viewport,
                    state,
                    application.clone(),
                    editor_focus,
                    viewport,
                    Inputs {
                        ids,
                        focus,
                        results_focus: cx.focus_handle(results_id.clone()),
                        results_id,
                    },
                )
            });
            let mut session = UiSession::new();
            session.render_view(&view, viewport, UiScale::ONE);
            session.render_view(&view, viewport, UiScale::ONE);
            if width >= 220.0 {
                let before = render_pixels(&session, &context, width, height);
                for name in [
                    "Filters",
                    "Search hidden files",
                    "Search ignored files",
                    "Match case (Alt+C)",
                ] {
                    let hit = session
                        .tree()
                        .nodes()
                        .iter()
                        .find(|node| {
                            node.semantics
                                .as_ref()
                                .and_then(|semantics| semantics.name.as_deref())
                                == Some(name)
                        })
                        .unwrap()
                        .hit_rect;
                    let point =
                        Point::new((hit.left + hit.right) / 2.0, (hit.top + hit.bottom) / 2.0);
                    dispatch_runtime_output(
                        session.handle_input(InputEvent::PointerMove(PointerData::mouse(point))),
                        &context,
                        &lgui::window::WindowId::new("search-test"),
                        |action| session.handle_default_action(action),
                        |_| {},
                    );
                    session.render_view(&view, viewport, UiScale::ONE);
                    assert!(
                        render_pixels(&session, &context, width, height) == before,
                        "Hovering {name} must not paint a background or tooltip"
                    );
                }
                save_preview(&session, &context, width, height, "search-hover-verified");
                let context = ApplicationContext::empty(Default::default());
                for input in [
                    InputEvent::PointerDown {
                        pointer: PointerData::mouse(Point::new(50.0, 48.0)),
                        button: PointerButton::Left,
                    },
                    InputEvent::PointerUp {
                        pointer: PointerData::mouse(Point::new(50.0, 48.0)),
                        button: PointerButton::Left,
                    },
                ] {
                    dispatch_runtime_output(
                        session.handle_input(input),
                        &context,
                        &lgui::window::WindowId::new("search-test"),
                        |action| session.handle_default_action(action),
                        |_| {},
                    );
                }
                session.render_view(&view, viewport, UiScale::ONE);
            }
            assert!(
                session.tree().nodes().len() < 300,
                "10,000 matches must not create 10,000 UI nodes"
            );
            let physical = PhysicalRect::new(0, 0, width as i32, height as i32);
            let dirty = [physical];
            let frame = FrameInfo::new(
                physical,
                &dirty,
                UiScale::ONE,
                FrameReason::SceneChange,
                true,
            );
            let mut surface = SkiaSoftwareSurface::new(16 * 1024 * 1024);
            lgui::assets::backend::with_render_resources(
                &context,
                lgui::assets::backend::render_resources(),
                || {
                    surface.draw(
                        &session.tree().scene().project_to_physical(UiScale::ONE),
                        &frame,
                    )
                },
            )
            .unwrap();
            if let Some(path) = std::env::var_os("LOOM_SEARCH_PREVIEW_DIR") {
                let mut rgba = surface.pixels().to_vec();
                for pixel in rgba.as_chunks_mut::<4>().0 {
                    pixel.swap(0, 2);
                }
                image::save_buffer(
                    std::path::Path::new(&path).join(format!("search-panel-{}.png", width as u32)),
                    &rgba,
                    width as u32,
                    height as u32,
                    image::ColorType::Rgba8,
                )
                .unwrap();
            }
            if width >= 220.0 {
                let context = ApplicationContext::empty(Default::default());
                context
                    .resources()
                    .provide(lgui::icons::IconRegistration(Arc::new(crate::svg_icons())));
                assert!(
                    exposed
                        .lock()
                        .unwrap()
                        .as_ref()
                        .unwrap()
                        .get()
                        .search
                        .query
                        .focused
                );
                // Option buttons preserve query focus so typing can continue.
                for input in [
                    InputEvent::PointerDown {
                        pointer: PointerData::mouse(Point::new(width - 72.0, 48.0)),
                        button: PointerButton::Left,
                    },
                    InputEvent::PointerUp {
                        pointer: PointerData::mouse(Point::new(width - 72.0, 48.0)),
                        button: PointerButton::Left,
                    },
                ] {
                    dispatch_runtime_output(
                        session.handle_input(input),
                        &context,
                        &lgui::window::WindowId::new("search-test"),
                        |action| session.handle_default_action(action),
                        |_| {},
                    );
                }
                let app = exposed.lock().unwrap().as_ref().unwrap().get();
                assert!(app.search.case_sensitive);
                assert!(app.search.query.focused);

                let state = exposed.lock().unwrap().as_ref().unwrap().clone();
                // Filter controls have real hit targets and preserve query focus.
                click_named(&mut session, &context, "Search hidden files");
                assert!(state.get().search.include_hidden);
                assert!(state.get().search.query.focused);
                session.render_view(&view, viewport, UiScale::ONE);
                click_named(&mut session, &context, "Reset filters");
                assert_eq!(filter_count(&state.get()), 0);
                assert!(state.get().search.filters_open);
                session.render_view(&view, viewport, UiScale::ONE);
                click_named(&mut session, &context, "Collapse or expand all results");
                assert!(
                    state
                        .get()
                        .search
                        .collapsed
                        .contains(std::path::Path::new("demo/src/main.rs"))
                );
                session.render_view(&view, viewport, UiScale::ONE);
                click_named(&mut session, &context, "Collapse or expand all results");
                assert!(state.get().search.collapsed.is_empty());
                session.render_view(&view, viewport, UiScale::ONE);

                if std::env::var_os("LOOM_SEARCH_PREVIEW_DIR").is_some() {
                    state.update(|app| {
                        app.search.query.set_text("search");
                        app.search.case_sensitive = false;
                        app.search.include.set_text("src/**, *.rs");
                        app.search.exclude.set_text("target/**, *.lock");
                        app.search.include_hidden = true;
                        let matcher = crate::search::Matcher::new(&app.search.spec()).unwrap();
                        let files = [
                            ("src/ui/search_panel.rs", "fn build_panel() {\n    let query = app.search.query.text();\n    let results = app.search.results.clone();\n}\n"),
                            ("src/search_actions.rs", "pub fn start(app: &mut AppState) {\n    app.search.running = true;\n}\n"),
                            ("src/input/key_actions.rs", "    let state = app.search.clone();\n    state.search_next();\n"),
                        ].into_iter().map(|(path, content)| Arc::new(FileMatches {
                            path: std::path::Path::new("demo").join(path), root: "demo".into(),
                            content_hash: crate::search::hash(content),
                            matches: matcher.matches(content, &std::sync::atomic::AtomicBool::new(false), 100),
                        })).collect::<Vec<_>>();
                        app.search.selected = Some((files[0].path.clone(), 0));
                        app.search.results = Arc::new(Results {
                            matches: files.iter().map(|file| file.matches.len()).sum(), files,
                            skipped: 290, skipped_binary: 246, skipped_large: 38, skipped_encoding: 6,
                            complete: true, ..Results::default()
                        });
                    });
                    session.render_view(&view, viewport, UiScale::ONE);
                    save_preview(&session, &context, width, height, "search-expanded");
                    state.update(|app| {
                        app.search.filters_open = false;
                        app.search.details_open = true;
                    });
                    session.render_view(&view, viewport, UiScale::ONE);
                    save_preview(&session, &context, width, height, "search-compact");
                    state.update(|app| {
                        app.search.running = true;
                        app.search.details_open = false;
                    });
                    session.render_view(&view, viewport, UiScale::ONE);
                    save_preview(&session, &context, width, height, "search-running");
                }

                // Clearing invalidates queued worker results immediately.
                let token = Arc::new(std::sync::atomic::AtomicBool::new(false));
                state.update(|app| {
                    app.search.cancel = Some(token.clone());
                    app.search.running = true;
                });
                let generation = state.get().search.generation;
                session.render_view(&view, viewport, UiScale::ONE);
                click(&mut session, &context, width - 50.0, 12.0);
                assert!(token.load(std::sync::atomic::Ordering::Relaxed));
                assert!(state.get().search.generation > generation);
                assert!(state.get().search.query.text().is_empty());
                assert!(state.get().search.query.focused);
                assert_eq!(state.get().search.results.matches, 0);
                session.render_view(&view, viewport, UiScale::ONE);
                for input in [
                    InputEvent::PointerDown {
                        pointer: PointerData::mouse(Point::new(22.0, 12.0)),
                        button: PointerButton::Left,
                    },
                    InputEvent::PointerUp {
                        pointer: PointerData::mouse(Point::new(22.0, 12.0)),
                        button: PointerButton::Left,
                    },
                ] {
                    dispatch_runtime_output(
                        session.handle_input(input),
                        &context,
                        &lgui::window::WindowId::new("search-test"),
                        |action| session.handle_default_action(action),
                        |_| {},
                    );
                }
                assert_eq!(
                    exposed.lock().unwrap().as_ref().unwrap().get().drawer_mode,
                    crate::state::DrawerMode::Files
                );
            }
        }
    }
}
