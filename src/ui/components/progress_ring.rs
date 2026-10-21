//! A circular progress indicator, drawn from icons prepared for each 5% step.
use crate::ui::tabs::icon;
use lgui::core::UiId;
use lgui::prelude::{Color, Element, UiRect};
use std::sync::LazyLock;

const STEPS: usize = 20;
const RADIUS: f32 = 6.0;

/// `(key, svg)` for 0%, 5%, … 100%; a faint track with an arc over it.
static RINGS: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(|| {
    let circumference = 2.0 * std::f32::consts::PI * RADIUS;
    (0..=STEPS)
        .map(|step| {
            let arc = circumference * step as f32 / STEPS as f32;
            let svg = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2"><circle cx="8" cy="8" r="{RADIUS}" opacity="0.2"/><circle cx="8" cy="8" r="{RADIUS}" stroke-dasharray="{arc:.2} {circumference:.2}" stroke-linecap="round" transform="rotate(-90 8 8)"/></svg>"#
            );
            let key = format!("progress-ring-{step}");
            // Prepared once for the process's life.
            (
                &*Box::leak(key.into_boxed_str()),
                &*Box::leak(svg.into_boxed_str()),
            )
        })
        .collect()
});

pub fn icons() -> impl Iterator<Item = (&'static str, &'static str)> {
    RINGS.iter().copied()
}

/// The ring for `percent` (0–100), rounded to the nearest step.
pub fn render(id: UiId, rect: UiRect, percent: u8, color: Color) -> Element {
    let step = (usize::from(percent.min(100)) * STEPS + 50) / 100;
    icon(id, RINGS[step].0, rect, color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_step_has_a_distinct_icon() {
        let keys: std::collections::BTreeSet<_> = icons().map(|(key, _)| key).collect();
        assert_eq!(keys.len(), STEPS + 1);
        assert_eq!(RINGS[(50 * STEPS + 50) / 100].0, "progress-ring-10");
        assert_eq!(RINGS[STEPS].0, "progress-ring-20");
    }
}
