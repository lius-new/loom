//! Right-hand drawer chrome shared by Explorer and Search: the page switcher
//! header and the icon helper used across drawer views.

use std::sync::Arc;

use lgui::core::{
    CursorIcon, EventPolicy, IconStyle, SemanticRole, Semantics, UiElement, UiId, precompiled,
};
use lgui::prelude::{Element, UiRect, VisualStyle, panel};

use crate::theme;

/// Page shown in the right-hand drawer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawerMode {
    #[default]
    Files,
    Search,
}

/// `id` must be unique in the frame: elements sharing one replace each
/// other, so repeated rows need ids such as `UiId::owned(format!(..))`.
pub fn drawer_icon(
    id: impl Into<UiId>,
    key: &'static str,
    rect: UiRect,
    color: lgui::prelude::Color,
) -> Element {
    precompiled(UiElement::icon(id.into(), rect, key).icon_style(IconStyle::new(color)))
}

/// Header with the Explorer/Search page buttons and the collapse button.
pub fn header(
    rect: UiRect,
    mode: DrawerMode,
    select: Arc<dyn Fn(DrawerMode) + Send + Sync>,
    collapse: Arc<dyn Fn() + Send + Sync>,
) -> Element {
    let mut header = panel(rect, VisualStyle::filled(theme::c().sidebar));
    for (index, label, icon, target) in [
        (0, "Explorer", "explorer", DrawerMode::Files),
        (1, "Search", "search", DrawerMode::Search),
    ] {
        let button_width = ((rect.width() - 48.0).max(0.0) / 2.0).min(28.0);
        let left = rect.left + 8.0 + index as f32 * (button_width + 4.0);
        let hit = UiRect::new(
            left,
            rect.top + 2.0,
            left + button_width,
            (rect.bottom - 2.0).max(rect.top + 2.0),
        );
        let active = mode == target;
        let select = select.clone();
        let button = Element::new(move |cx| {
            let flags = cx.context.interaction_flags(&cx.id);
            let style = if mode != DrawerMode::Search && (flags.hovered || flags.pressed) {
                VisualStyle::filled(theme::c().surface).radius(3.0)
            } else {
                VisualStyle::default()
            };
            UiElement::panel(cx.id, hit, style).children(cx.children)
        })
        .key(format!("drawer-mode-{index}"))
        .event_policy(EventPolicy {
            hover: true,
            press: true,
            focus: false,
        })
        .semantics(Semantics::new(SemanticRole::Button).name(label))
        .cursor(CursorIcon::Pointer)
        .on_click(move || select(target))
        .child(drawer_icon(
            UiId::owned(format!("drawer-mode-icon-{index}")),
            icon,
            theme::icon_rect(hit),
            if active {
                theme::c().accent
            } else {
                theme::c().text_dim
            },
        ));
        header = header.child(button);
    }

    let collapse_hit = UiRect::new(
        rect.right - 32.0,
        rect.top + 2.0,
        rect.right - 4.0,
        rect.bottom,
    );
    let collapse_icon = theme::icon_rect(UiRect::new(
        collapse_hit.left + 7.0,
        rect.top,
        collapse_hit.left + 21.0,
        rect.bottom,
    ));
    header = header.child(
        panel(collapse_hit, VisualStyle::default().radius(3.0))
            .event_policy(EventPolicy::INTERACTIVE)
            .cursor(CursorIcon::Pointer)
            .on_click(move || collapse())
            .child(drawer_icon(
                "sidebar.collapse.icon",
                "panel-left",
                collapse_icon,
                theme::c().text_dim,
            )),
    );

    header.child(panel(
        UiRect::new(rect.left, rect.bottom - 1.0, rect.right, rect.bottom),
        VisualStyle::filled(theme::c().border),
    ))
}
