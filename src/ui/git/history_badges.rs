//! Compact reference badges beside commit subjects.

use lgui::core::{SemanticRole, Semantics, UiId};
use lgui::prelude::{Color, Element, UiRect, VisualStyle, panel, text};

use crate::ui::{tabs, theme};

const HEIGHT: f32 = 14.0;
const FONT_SIZE: f32 = 9.0;
const ICON: f32 = 10.0;
const PAD: f32 = 4.0;
const ICON_GAP: f32 = 3.0;
const GAP: f32 = 4.0;
const SUBJECT_GAP: f32 = 6.0;
const CONTENT_PAD: f32 = PAD * 2.0 + ICON + ICON_GAP + 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Current,
    Branch,
    Remote,
    Tag,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Reference<'a> {
    label: &'a str,
    kind: Kind,
}

impl<'a> Reference<'a> {
    fn parse(value: &'a str) -> Option<Self> {
        let (label, kind) = if let Some(name) = value.strip_prefix("HEAD -> ") {
            (
                name.strip_prefix("refs/heads/").unwrap_or(name),
                Kind::Current,
            )
        } else if value == "HEAD" {
            (value, Kind::Current)
        } else if let Some(name) = value.strip_prefix("tag: ") {
            (name.strip_prefix("refs/tags/").unwrap_or(name), Kind::Tag)
        } else if let Some(name) = value.strip_prefix("refs/heads/") {
            (name, Kind::Branch)
        } else if let Some(name) = value.strip_prefix("refs/remotes/") {
            // The remote's default-branch alias repeats its branch badge.
            if name.ends_with("/HEAD") {
                return None;
            }
            (name, Kind::Remote)
        } else if let Some(name) = value.strip_prefix("refs/tags/") {
            (name, Kind::Tag)
        } else if value.starts_with("refs/") || value.is_empty() {
            return None;
        } else {
            // Also accept short decorations already cached by an older frame.
            (value, Kind::Branch)
        };
        Some(Self { label, kind })
    }

    fn color(self) -> Color {
        match self.kind {
            Kind::Current | Kind::Branch => theme::c().syntax.function,
            Kind::Remote => theme::c().syntax.property,
            Kind::Tag => theme::c().syntax.type_name,
        }
    }

    fn icon(self) -> &'static str {
        match self.kind {
            Kind::Current => "git-ref-current",
            Kind::Branch => "git-branch",
            Kind::Remote => "cloud",
            Kind::Tag => "git-ref-tag",
        }
    }

    fn description(self) -> String {
        let kind = match self.kind {
            Kind::Current => "Current branch",
            Kind::Branch => "Branch",
            Kind::Remote => "Remote branch",
            Kind::Tag => "Tag",
        };
        format!("{kind}: {}", self.label)
    }
}

/// Keep the hover card's reference names readable when Git supplies full refs.
pub(crate) fn display_name(value: &str) -> String {
    match Reference::parse(value) {
        Some(reference) if reference.kind == Kind::Current && reference.label != "HEAD" => {
            format!("HEAD → {}", reference.label)
        }
        Some(reference) if reference.kind == Kind::Tag => format!("tag: {}", reference.label),
        Some(reference) => reference.label.to_owned(),
        None => value
            .strip_prefix("refs/remotes/")
            .unwrap_or(value)
            .to_owned(),
    }
}

struct Badge<'a> {
    reference: Reference<'a>,
    label: String,
    rect: UiRect,
}

fn layout<'a>(row: UiRect, decorations: &'a [String]) -> (Vec<Badge<'a>>, f32) {
    let right = row.right - 12.0;
    let available = (right - row.left - 30.0).max(0.0);
    // Preserve room for the subject even with several refs or very long names.
    let budget = (available * 0.55).min((available - 46.0).max(0.0));
    let mut references = decorations
        .iter()
        .filter_map(|value| Reference::parse(value))
        .collect::<Vec<_>>();
    references.sort_by_key(|reference| reference.kind);
    let mut used = 0.0;
    let mut badges = Vec::new();
    for reference in references {
        let gap = if badges.is_empty() { 0.0 } else { GAP };
        let remaining = budget - used - gap;
        if remaining < 32.0 {
            break;
        }
        let width = (tabs::measure(reference.label, FONT_SIZE, 400) + CONTENT_PAD)
            .min(128.0)
            .min(remaining);
        let left = used + gap;
        let top = row.top + (row.height() - HEIGHT) / 2.0;
        badges.push(Badge {
            reference,
            label: tabs::ellipsize(reference.label, width - CONTENT_PAD, FONT_SIZE, 400),
            rect: UiRect::new(left, top, left + width, top + HEIGHT),
        });
        used = left + width;
    }
    let subject_right = if badges.is_empty() {
        right
    } else {
        right - used - SUBJECT_GAP
    };
    for badge in &mut badges {
        badge.rect = badge.rect.translate(right - used, 0.0);
    }
    (badges, subject_right)
}

/// Returns badge elements and the right edge left available to the subject.
pub(crate) fn render(row: UiRect, oid: &str, decorations: &[String]) -> (Vec<Element>, f32) {
    let (badges, subject_right) = layout(row, decorations);
    let elements = badges
        .into_iter()
        .enumerate()
        .map(|(index, badge)| {
            let rect = badge.rect;
            let color = badge.reference.color();
            panel(rect, VisualStyle::filled(color).alpha(48).radius(4.0))
                .key(format!("git-commit-ref-{oid}-{index}"))
                .semantics(Semantics::new(SemanticRole::Label).name(badge.reference.description()))
                .child(super::super::drawer::drawer_icon(
                    UiId::owned(format!("git-commit-ref-icon-{oid}-{index}")),
                    badge.reference.icon(),
                    UiRect::new(
                        rect.left + PAD,
                        rect.top + 2.0,
                        rect.left + PAD + ICON,
                        rect.top + 2.0 + ICON,
                    ),
                    color,
                ))
                .child(text(
                    UiRect::new(
                        rect.left + PAD + ICON + ICON_GAP,
                        rect.top,
                        rect.right - PAD,
                        rect.bottom,
                    ),
                    badge.label,
                    theme::mono(color, FONT_SIZE),
                ))
        })
        .collect();
    (elements, subject_right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_local_remote_tag_and_detached_head_references() {
        for (value, label, kind) in [
            ("HEAD -> refs/heads/main", "main", Kind::Current),
            ("HEAD", "HEAD", Kind::Current),
            ("refs/heads/upstream/topic", "upstream/topic", Kind::Branch),
            ("refs/remotes/upstream/main", "upstream/main", Kind::Remote),
            ("tag: refs/tags/v1.0", "v1.0", Kind::Tag),
            ("HEAD -> main", "main", Kind::Current),
        ] {
            assert_eq!(Reference::parse(value), Some(Reference { label, kind }));
        }
        assert_eq!(Reference::parse("refs/remotes/upstream/HEAD"), None);
        assert_eq!(Reference::parse("refs/stash"), None);
        assert_eq!(display_name("tag: refs/tags/v1.0"), "tag: v1.0");
    }

    #[test]
    fn long_and_multiple_refs_preserve_the_subject_and_stay_within_the_row() {
        let decorations = [
            "refs/remotes/company/a-very-long-remote-branch-name",
            "tag: refs/tags/release-2026.10.15",
            "HEAD -> refs/heads/feature/很长的分支名称-for-the-editor",
        ]
        .map(str::to_owned);
        for width in [80.0, 160.0, 288.0, 400.0] {
            let row = UiRect::new(50.0, 20.0, 50.0 + width, 40.0);
            let (badges, subject_right) = layout(row, &decorations);
            assert!(subject_right >= row.left + 30.0);
            if width == 80.0 {
                assert!(badges.is_empty());
                continue;
            }
            assert_eq!(badges[0].reference.kind, Kind::Current);
            assert!(subject_right - row.left - 30.0 >= 40.0);
            assert!(badges[0].label.ends_with('…'));
            assert!(subject_right + SUBJECT_GAP <= badges[0].rect.left + 0.001);
            for badge in &badges {
                assert!(badge.rect.right <= row.right - 12.0 + 0.001);
                assert!(badge.rect.top >= row.top && badge.rect.bottom <= row.bottom);
                assert!(
                    tabs::measure(&badge.label, FONT_SIZE, 400)
                        <= badge.rect.width() - CONTENT_PAD + 0.001
                );
            }
            for pair in badges.windows(2) {
                assert!(pair[0].rect.right + GAP <= pair[1].rect.left + 0.001);
            }
        }
        let row = UiRect::new(0.0, 0.0, 288.0, 20.0);
        let (badges, subject_right) = layout(row, &[]);
        assert!(badges.is_empty());
        assert_eq!(subject_right, row.right - 12.0);
    }
}
