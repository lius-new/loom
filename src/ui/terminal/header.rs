//! Right-hand tools of the terminal header, laid out right to left in three
//! groups: actions on the sessions, tools of the terminal itself, and actions
//! of the whole panel. A thin rule separates the groups.

use lgui::core::{CursorIcon, EventPolicy, SemanticRole, Semantics, UiElement, UiId};
use lgui::prelude::{Color, Element, UiRect, VisualStyle, panel};

use crate::ui::theme;

/// Distances of the right-hand tools. A wide header spreads them out; a narrow
/// one keeps them close so the session tabs still have room.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Metrics {
    /// Edge of a square tool button.
    button: f32,
    /// Space between neighbouring buttons.
    gap: f32,
    /// Space between the last button and the header's right edge.
    inset: f32,
    /// Space on each side of a group rule.
    rule_gap: f32,
}
const COMPACT: Metrics = Metrics {
    button: 22.0,
    gap: 4.0,
    inset: 8.0,
    rule_gap: 9.0,
};
const ROOMY: Metrics = Metrics {
    button: 24.0,
    gap: 8.0,
    inset: 10.0,
    rule_gap: 14.0,
};
/// Narrowest header that uses the roomy distances.
const ROOMY_MIN_W: f32 = 560.0;
/// Space before the category capsule on the header's left.
const INSET: f32 = 8.0;
const RULE_H: f32 = 14.0;

fn metrics(header: UiRect) -> Metrics {
    if header.width() >= ROOMY_MIN_W {
        ROOMY
    } else {
        COMPACT
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Spec {
    /// A foreground program is running, so it can be interrupted.
    pub running: bool,
    /// The default bottom panel owns maximize and shows the collapse icon; a
    /// terminal docked in another region has neither.
    pub panel: bool,
}

#[derive(Debug)]
pub(super) struct Layout {
    pub stop: Option<UiRect>,
    pub new: UiRect,
    pub shell: UiRect,
    pub split: UiRect,
    pub search: UiRect,
    pub more: UiRect,
    pub maximize: Option<UiRect>,
    pub collapse: Option<UiRect>,
    /// Rules between groups, as their horizontal centers.
    pub rules: Vec<f32>,
    /// Left edge of the tools; the session strip must end before it.
    pub left: f32,
}

/// Walks leftwards from the header's right edge.
struct Cursor {
    x: f32,
    top: f32,
    m: Metrics,
}

impl Cursor {
    fn button(&mut self) -> UiRect {
        let b = self.m.button;
        let rect = UiRect::new(self.x - b, self.top, self.x, self.top + b);
        self.x -= b + self.m.gap;
        rect
    }

    /// Drops the gap left after the previous button and returns the centre of
    /// a rule placed there.
    fn rule(&mut self) -> f32 {
        let center = self.x + self.m.gap - self.m.rule_gap;
        self.x = center - self.m.rule_gap;
        center
    }
}

pub(super) fn layout(header: UiRect, spec: Spec) -> Layout {
    let m = metrics(header);
    let mut at = Cursor {
        x: header.right - m.inset,
        top: header.top + (header.height() - m.button) / 2.0,
        m,
    };
    let mut rules = Vec::new();
    let collapse = spec.panel.then(|| at.button());
    let maximize = spec.panel.then(|| at.button());
    if collapse.is_some() {
        rules.push(at.rule());
    }
    let more = at.button();
    let search = at.button();
    let split = at.button();
    rules.push(at.rule());
    let shell = at.button();
    let new = at.button();
    let stop = spec.running.then(|| at.button());
    Layout {
        stop,
        new,
        shell,
        split,
        search,
        more,
        maximize,
        collapse,
        rules,
        left: at.x + m.gap,
    }
}

/// The category switcher's only capsule for now: the terminal icon, its name
/// and how many sessions there are, on a rounded track. Returns the element and
/// the track's right edge.
pub(super) fn category(header: UiRect, sessions: usize) -> (Element, f32) {
    use crate::ui::tabs::measure;
    let label_w = measure("Terminal", theme::UI_SIZE, 600);
    let count = sessions.to_string();
    let count_w = (measure(&count, theme::SMALL, 400) + 10.0).max(16.0);
    let track_h = 24.0;
    let top = header.top + (header.height() - track_h) / 2.0;
    let left = header.left + INSET;
    // Icon slot, gap, label, gap, count pill, with 8px inside each end.
    let capsule_w = 8.0 + 16.0 + 5.0 + label_w + 6.0 + count_w + 8.0;
    let track = UiRect::new(left, top, left + capsule_w + 4.0, top + track_h);
    let capsule = UiRect::new(
        track.left + 2.0,
        track.top + 2.0,
        track.right - 2.0,
        track.bottom - 2.0,
    );
    let icon_left = capsule.left + 8.0;
    let label_left = icon_left + 16.0 + 5.0;
    let pill_left = label_left + label_w + 6.0;
    let pill = UiRect::new(
        pill_left,
        capsule.top + 3.0,
        pill_left + count_w,
        capsule.bottom - 3.0,
    );
    let element = panel(
        track,
        VisualStyle::filled(theme::c().active_line).radius(track_h / 2.0),
    )
    .key("terminal-category")
    .child(
        panel(
            capsule,
            VisualStyle::filled(theme::c().accent)
                .alpha(40)
                .radius(capsule.height() / 2.0),
        )
        .child(super::view::icon(
            "terminal-category.icon",
            "terminal",
            theme::icon_rect(UiRect::new(
                icon_left,
                capsule.top,
                icon_left + 16.0,
                capsule.bottom,
            )),
            theme::c().accent,
        ))
        .child(lgui::prelude::text(
            UiRect::new(label_left, capsule.top, pill_left, capsule.bottom),
            "Terminal",
            theme::sans_semibold(theme::c().accent, theme::UI_SIZE),
        ))
        .child(
            panel(
                pill,
                VisualStyle::filled(theme::c().accent)
                    .alpha(48)
                    .radius(pill.height() / 2.0),
            )
            .child(lgui::prelude::text(pill, count, {
                let mut style = theme::mono(theme::c().accent, theme::SMALL);
                style.align = lgui::prelude::TextAlign::Center;
                style
            })),
        ),
    );
    (element, track.right)
}

/// Transparent until hovered or pressed, filled while `active`.
pub(super) fn button(
    id: &'static str,
    rect: UiRect,
    icon: &'static str,
    name: impl Into<String>,
    active: bool,
    color: Color,
    click: impl Fn() + Send + Sync + 'static,
) -> Element {
    button_in(id, rect, rect, icon, name, active, color, click)
}

/// Like [`button`], with the icon centred in `slot` instead of in `rect`, for a
/// button whose box is wider on one side than its glyph's visual centre.
#[allow(clippy::too_many_arguments)]
pub(super) fn button_in(
    id: &'static str,
    rect: UiRect,
    slot: UiRect,
    icon: &'static str,
    name: impl Into<String>,
    active: bool,
    color: Color,
    click: impl Fn() + Send + Sync + 'static,
) -> Element {
    button_styled(
        id,
        rect,
        slot,
        icon,
        name,
        active,
        color,
        (theme::c().surface, 255),
        click,
    )
}

/// Like [`button_in`], with the fill drawn while hovered or active given as a
/// colour and opacity, so a button on a tinted row can use a matching tint.
#[allow(clippy::too_many_arguments)]
pub(super) fn button_styled(
    id: &'static str,
    rect: UiRect,
    slot: UiRect,
    icon: &'static str,
    name: impl Into<String>,
    active: bool,
    color: Color,
    fill: (Color, u8),
    click: impl Fn() + Send + Sync + 'static,
) -> Element {
    let mut semantics = Semantics::new(SemanticRole::Button).name(name.into());
    semantics.state.selected = active;
    // An explicit node id, so a neighbour such as a highlighted menu row can ask
    // whether this button is hovered.
    let node_id = UiId::owned(id);
    Element::new(move |cx| {
        let flags = cx.context.interaction_flags(&node_id);
        let style = if active || flags.hovered || flags.pressed {
            VisualStyle::filled(fill.0).alpha(fill.1).radius(3.0)
        } else {
            VisualStyle::default()
        };
        UiElement::panel(node_id.clone(), rect, style).children(cx.children)
    })
    .key(id)
    .event_policy(EventPolicy::INTERACTIVE)
    .cursor(CursorIcon::Pointer)
    .semantics(semantics)
    .on_click(click)
    .child(super::view::icon(
        id,
        icon,
        theme::icon_rect(slot),
        if active { theme::c().accent } else { color },
    ))
}

pub(super) fn rule(header: UiRect, center: f32) -> Element {
    let top = header.top + (header.height() - RULE_H) / 2.0;
    panel(
        UiRect::new(center - 0.5, top, center + 0.5, top + RULE_H),
        VisualStyle::filled(theme::c().border),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> UiRect {
        UiRect::new(0.0, 0.0, 600.0, 28.0)
    }

    #[test]
    fn groups_run_right_to_left_without_overlap() {
        let layout = layout(
            header(),
            Spec {
                running: true,
                panel: true,
            },
        );
        let order = [
            layout.collapse.unwrap(),
            layout.maximize.unwrap(),
            layout.more,
            layout.search,
            layout.split,
            layout.shell,
            layout.new,
            layout.stop.unwrap(),
        ];
        for pair in order.windows(2) {
            assert!(pair[1].right <= pair[0].left, "{pair:?}");
        }
        assert_eq!(layout.rules.len(), 2);
        // Each rule sits strictly between two adjacent groups.
        assert!(layout.maximize.unwrap().left > layout.rules[0]);
        assert!(layout.rules[0] > layout.more.right);
        assert!(layout.split.left > layout.rules[1]);
        assert!(layout.rules[1] > layout.shell.right);
        assert!(layout.left <= layout.stop.unwrap().left);
    }

    #[test]
    fn a_docked_terminal_has_no_panel_group_and_stop_only_while_running() {
        let layout = layout(
            header(),
            Spec {
                running: false,
                panel: false,
            },
        );
        assert!(layout.collapse.is_none() && layout.maximize.is_none());
        assert!(layout.stop.is_none());
        assert_eq!(layout.rules.len(), 1);
        assert_eq!(layout.more.right, header().right - metrics(header()).inset);
        assert!(layout.left <= layout.new.left);
    }

    #[test]
    fn buttons_are_centred_vertically_in_the_header() {
        let layout = layout(
            header(),
            Spec {
                running: false,
                panel: true,
            },
        );
        for rect in [layout.more, layout.new, layout.collapse.unwrap()] {
            assert_eq!(rect.top - header().top, header().bottom - rect.bottom);
        }
    }

    #[test]
    fn a_wide_header_spaces_the_tools_out_and_a_narrow_one_keeps_them_close() {
        let spec = Spec {
            running: false,
            panel: true,
        };
        let roomy = layout(UiRect::new(0.0, 0.0, 900.0, 28.0), spec);
        let compact = layout(UiRect::new(0.0, 0.0, 400.0, 28.0), spec);
        let pitch = |l: &Layout| l.search.left - l.split.left;
        // Neighbouring tools sit further apart in the wide header...
        assert!(pitch(&roomy) > pitch(&compact));
        assert_eq!(roomy.split.width(), ROOMY.button);
        assert_eq!(compact.split.width(), COMPACT.button);
        // ...and so does the space around each group rule.
        let around = |l: &Layout| l.maximize.unwrap().left - l.rules[0];
        assert!(around(&roomy) > around(&compact));
        // The narrow tools still leave the strip room in the layout's 200px minimum.
        let tight = layout(
            UiRect::new(0.0, 0.0, 200.0, 28.0),
            Spec {
                running: false,
                panel: false,
            },
        );
        assert!(tight.left > 40.0, "left edge of the tools: {}", tight.left);
    }
}
