//! Widths and scrolling of the session tabs. A tab's width comes from its name,
//! kept between a minimum and a maximum; when they do not all fit they shrink
//! together toward the minimum, and past that the strip scrolls.
//!
//! The width is taken from the name alone, never from the selection, the pointer
//! or the program running in the session, so none of those moves a tab.

/// Narrowest a tab is made while the others have room to give.
pub(super) const MIN_W: f32 = 100.0;
/// Widest a tab gets.
pub(super) const MAX_W: f32 = 170.0;
/// Narrowest a tab squeezes to when there is not even room for `MIN_W` each.
pub(super) const FLOOR_W: f32 = 72.0;
/// Room kept at the end of a scrolling strip for the sessions list button.
pub(super) const LIST_W: f32 = 28.0;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Layout {
    /// Width of each tab, in order.
    pub widths: Vec<f32>,
    /// Width the strip shows tabs in; less than the strip when it scrolls.
    pub viewport_w: f32,
    /// How far the tabs can scroll; zero when they all fit.
    pub max_scroll: f32,
}

impl Layout {
    pub fn scrolls(&self) -> bool {
        self.max_scroll > 0.0
    }
}

/// Lays out tabs whose names ask for `naturals` pixels each in a strip `width`
/// wide, with `pad` before the first and `gap` between neighbours.
pub(super) fn layout(width: f32, naturals: &[f32], pad: f32, gap: f32) -> Layout {
    let mut widths = naturals
        .iter()
        .map(|natural| natural.clamp(MIN_W, MAX_W))
        .collect::<Vec<_>>();
    let gaps = gap * widths.len().saturating_sub(1) as f32;
    let total = pad + widths.iter().sum::<f32>() + gaps;
    if total <= width {
        return Layout {
            widths,
            viewport_w: width,
            max_scroll: 0.0,
        };
    }
    // Take the excess from the tabs that are wider than the minimum, each in
    // proportion to how much it can give.
    let excess = total - width;
    let slack = widths.iter().map(|w| w - MIN_W).sum::<f32>();
    if slack >= excess {
        for w in &mut widths {
            *w -= (*w - MIN_W) * excess / slack;
        }
        return Layout {
            widths,
            viewport_w: width,
            max_scroll: 0.0,
        };
    }
    // Everything is at the minimum and still too much: squeeze them all equally
    // toward the floor while that is enough.
    widths.iter_mut().for_each(|w| *w = MIN_W);
    let at_minimum = pad + widths.len() as f32 * MIN_W + gaps;
    let squeeze = widths.len() as f32 * (MIN_W - FLOOR_W);
    if squeeze > 0.0 && squeeze >= at_minimum - width {
        let narrower = MIN_W - (MIN_W - FLOOR_W) * (at_minimum - width) / squeeze;
        widths.iter_mut().for_each(|w| *w = narrower);
        return Layout {
            widths,
            viewport_w: width,
            max_scroll: 0.0,
        };
    }
    // Even at the floor they do not fit: scroll at the minimum, and keep a
    // button for the full list.
    let viewport_w = (width - LIST_W).max(0.0);
    let content = pad + widths.iter().sum::<f32>() + gaps;
    Layout {
        widths,
        viewport_w,
        max_scroll: (content - viewport_w).max(0.0),
    }
}

/// Left edge of tab `index` within the unscrolled content.
pub(super) fn tab_left(layout: &Layout, index: usize, pad: f32, gap: f32) -> f32 {
    pad + layout.widths[..index.min(layout.widths.len())]
        .iter()
        .sum::<f32>()
        + gap * index as f32
}

/// The scroll offset that shows tab `index` completely, moving as little as
/// possible from `scroll`.
pub(super) fn reveal(scroll: f32, layout: &Layout, index: usize, pad: f32, gap: f32) -> f32 {
    let left = tab_left(layout, index, pad, gap);
    let right = left + layout.widths.get(index).copied().unwrap_or(MIN_W);
    let next = if left < scroll {
        left - pad
    } else if right > scroll + layout.viewport_w {
        right - layout.viewport_w
    } else {
        scroll
    };
    next.clamp(0.0, layout.max_scroll)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_is_as_wide_as_its_name_between_the_minimum_and_the_maximum() {
        let l = layout(2000.0, &[40.0, 130.0, 150.0, 400.0], 0.0, 0.0);
        assert_eq!(l.widths, vec![MIN_W, 130.0, 150.0, MAX_W]);
        assert!(!l.scrolls());
        assert_eq!(l.viewport_w, 2000.0);
    }

    #[test]
    fn tabs_shrink_together_toward_the_minimum_when_they_do_not_fit() {
        // Three wide tabs (510px) in 450px: each gives up a share of its slack.
        let l = layout(450.0, &[400.0, 400.0, 400.0], 0.0, 0.0);
        assert!((l.widths.iter().sum::<f32>() - 450.0).abs() < 0.01);
        assert!(l.widths.iter().all(|w| (*w - 150.0).abs() < 0.01));
        // A tab already at the minimum does not shrink.
        let l = layout(430.0, &[40.0, 400.0, 400.0], 0.0, 0.0);
        assert_eq!(l.widths[0], MIN_W);
        assert!((l.widths.iter().sum::<f32>() - 430.0).abs() < 0.01);
        assert!(l.widths[1] < MAX_W && l.widths[1] >= MIN_W);
    }

    #[test]
    fn below_the_minimum_tabs_squeeze_together_toward_the_floor() {
        // Six tabs at 100px need 600; 500 is enough once each gives up a bit.
        let l = layout(500.0, &[10.0; 6], 0.0, 0.0);
        assert!(!l.scrolls());
        assert!((l.widths.iter().sum::<f32>() - 500.0).abs() < 0.01);
        assert!(l.widths.iter().all(|w| *w < MIN_W && *w > FLOOR_W));
        // Just enough room for the floor: every tab is at it.
        let l = layout(6.0 * FLOOR_W, &[10.0; 6], 0.0, 0.0);
        assert!(!l.scrolls());
        assert!(l.widths.iter().all(|w| (*w - FLOOR_W).abs() < 0.01));
    }

    #[test]
    fn past_the_floor_the_strip_scrolls_at_the_minimum_and_keeps_room_for_the_list() {
        let l = layout(400.0, &[10.0; 6], 0.0, 0.0);
        assert!(l.widths.iter().all(|w| *w == MIN_W));
        assert!(l.scrolls());
        assert_eq!(l.viewport_w, 400.0 - LIST_W);
        assert_eq!(l.max_scroll, 6.0 * MIN_W - l.viewport_w);
    }

    #[test]
    fn a_width_never_depends_on_anything_but_the_names_and_the_room() {
        let names = [90.0, 160.0, 200.0];
        assert_eq!(
            layout(800.0, &names, 0.0, 0.0),
            layout(800.0, &names, 0.0, 0.0)
        );
        // A longer name for one tab never narrows another.
        let before = layout(2000.0, &names, 0.0, 0.0);
        let after = layout(2000.0, &[90.0, 160.0, 500.0], 0.0, 0.0);
        assert_eq!(before.widths[..2], after.widths[..2]);
    }

    #[test]
    fn reveal_moves_the_least_to_show_a_tab_and_stays_in_range() {
        let l = layout(400.0, &[10.0; 8], 0.0, 0.0);
        // Already visible: unchanged.
        assert_eq!(reveal(0.0, &l, 1, 0.0, 0.0), 0.0);
        // Off to the right: its right edge meets the viewport's.
        assert_eq!(reveal(0.0, &l, 7, 0.0, 0.0), l.max_scroll);
        // Off to the left: its left edge meets the viewport's.
        assert_eq!(reveal(l.max_scroll, &l, 0, 0.0, 0.0), 0.0);
        let mid = reveal(0.0, &l, 5, 0.0, 0.0);
        assert_eq!(mid, 6.0 * MIN_W - l.viewport_w);
        assert!(mid > 0.0 && mid < l.max_scroll);
    }
}
