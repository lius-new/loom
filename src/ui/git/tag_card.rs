//! Tag metadata and target commit details, using the history hover-card chrome.
use crate::features::highlighting::HighlightId;
use lgui::core::{SemanticRole, Semantics};
use lgui::prelude::{Element, UiRect};

use crate::features::git::RepositorySnapshot;
use crate::ui::git::panel::{self, CardLine, GitHover};
use crate::ui::git::state::GitPanelState;
use crate::ui::theme;

pub(crate) fn render(
    viewport: UiRect,
    app: &GitPanelState,
    repository: &RepositorySnapshot,
) -> Option<Element> {
    let card = app.ui.tag_card.as_ref()?;
    if app.ui.hover.as_ref() != Some(&GitHover::Tag(card.tag.name.clone()))
        || card.repository != repository.worktree_root
        || !repository.tags.contains(&card.tag)
        || app.ui.collapsed_sections.contains(&panel::TAGS)
        || app.ui.menu.is_some()
        || app.ui.confirm.is_some()
    {
        return None;
    }
    let width = panel::CARD_W.min(viewport.width() - 16.0).max(160.0);
    let inner = width - panel::CARD_PAD * 2.0;
    let mut lines = panel::wrap_text(&card.tag.name, inner)
        .into_iter()
        .map(|line| CardLine::Text(vec![(line, theme::syntax_color(HighlightId::TYPE))], true))
        .collect::<Vec<_>>();
    match &card.details {
        Ok(details) => {
            lines.push(CardLine::Text(
                vec![(
                    if details.annotation.is_some() {
                        "Annotated tag"
                    } else {
                        "Lightweight tag"
                    }
                    .into(),
                    theme::c().text_dim,
                )],
                false,
            ));
            if let Some(annotation) = &details.annotation {
                lines.push(CardLine::Separator);
                lines.push(CardLine::Text(
                    vec![(annotation.tagger.clone(), theme::c().text_bright)],
                    true,
                ));
                lines.push(CardLine::Text(
                    vec![(
                        format!("{}  ·  {}", annotation.email, annotation.date),
                        theme::c().text_dim,
                    )],
                    false,
                ));
                let message = panel::wrap_text(&annotation.message.replace('\t', "    "), inner);
                for line in message.iter().take(6) {
                    lines.push(CardLine::Text(vec![(line.clone(), theme::c().text)], false));
                }
                if message.len() > 6 {
                    lines.push(CardLine::Text(
                        vec![("…".into(), theme::c().text_dim)],
                        false,
                    ));
                }
            }
            lines.push(CardLine::Separator);
            lines.push(CardLine::Text(
                vec![(
                    format!(
                        "{}  {}",
                        details.target_kind,
                        crate::features::git::diff_document::short_oid(&details.target)
                    ),
                    theme::c().text_soft,
                )],
                false,
            ));
            if let Some(commit) = &details.commit {
                lines.push(CardLine::Separator);
                panel::append_commit_card_lines(&mut lines, &Ok(commit.clone()), inner);
            }
        }
        Err(error) => {
            lines.push(CardLine::Separator);
            lines.extend(
                panel::wrap_text(error, inner)
                    .into_iter()
                    .map(|line| CardLine::Text(vec![(line, theme::c().error_text)], false)),
            );
        }
    }
    Some(
        panel::render_hover_card(viewport, card.anchor, lines)
            .key("git-tag-hover-card")
            .semantics(Semantics::new(SemanticRole::Group).name("Tag details")),
    )
}
