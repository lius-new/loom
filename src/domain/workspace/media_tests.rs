//! Image tabs never become editable staging buffers, including session restore.
use crate::domain::document::DiskState;
use crate::domain::media::{Background, Frame, Image};
use crate::domain::pane_layout::Direction;
use crate::domain::view::Presentation;
use crate::domain::workspace::{OpenMode, PreparedTextFile, Workspace};
use crate::domain::workspace_session::{SavedView, capture};
use std::sync::Arc;
type Model = Workspace<(), ()>;
fn prepared() -> PreparedTextFile {
    PreparedTextFile::image(
        "icon.png".into(),
        Image {
            width: 32,
            height: 24,
            format: "PNG",
            byte_len: 123,
            identity: 1,
            frames: vec![Frame {
                bytes: Arc::new(vec![1, 2, 3]),
                duration_ms: 100,
                sequence: 0,
            }],
            animation: None,
        },
        DiskState::from_contents(""),
    )
}
#[test]
fn image_tabs_are_read_only_and_independent_views_share_pixels() {
    let mut workspace = Model::new();
    let id = workspace.open_prepared_text(prepared(), OpenMode::Preview);
    let view = workspace.active().unwrap();
    let left = workspace.active_pane();
    assert_eq!(
        workspace.view(view).unwrap().presentation,
        Presentation::Image
    );
    assert!(workspace.active_editor().is_none());
    assert!(workspace.document_editor_mut(id).is_none());
    assert!(workspace.save_snapshot(id).is_none());
    assert!(workspace.text_snapshots().is_empty());
    assert!(!workspace.is_dirty(id));
    assert!(!workspace.set_presentation(view, Presentation::Source));
    let right = workspace.split(left, Direction::Right).unwrap();
    let other = workspace.pane(right).unwrap().active().unwrap();
    workspace
        .views
        .get_mut(&other)
        .unwrap()
        .image
        .zoom(Some(2.0));
    assert_eq!(workspace.view(view).unwrap().image.scale, None);
    assert_eq!(workspace.view_document(other), Some(id));
    assert_eq!(workspace.documents.len(), 1);
}
#[test]
fn restored_image_placeholders_keep_identity_and_image_view_state() {
    let mut workspace = Model::new();
    let id = workspace.open_deferred("icon.png".into());
    let view = workspace.active().unwrap();
    workspace.views.get_mut(&view).unwrap().image.background = Background::Dark;
    assert!(workspace.install_restored(id, "icon.png".as_ref(), prepared()));
    assert_eq!(workspace.active(), Some(view));
    assert_eq!(
        workspace.view(view).unwrap().presentation,
        Presentation::Image
    );
    assert_eq!(
        workspace.view(view).unwrap().image.background,
        Background::Dark
    );
    assert!(workspace.save_snapshot(id).is_none());
}
#[test]
fn image_positions_and_svg_source_associations_round_trip_as_session_metadata() {
    let mut workspace = Model::new();
    let document = workspace.open_prepared_text(
        PreparedTextFile::with_disk_state(
            "logo.svg".into(),
            "<svg/>".into(),
            DiskState::from_contents("<svg/>"),
        ),
        OpenMode::Permanent,
    );
    let source = workspace.active().unwrap();
    workspace.set_presentation(source, Presentation::Source);
    let left = workspace.active_pane();
    let right = workspace.split(left, Direction::Right).unwrap();
    let preview = workspace.pane(right).unwrap().active().unwrap();
    workspace.set_presentation(preview, Presentation::Image);
    let view = workspace.views.get_mut(&preview).unwrap();
    view.associated_source = Some(source);
    view.image.zoom(Some(2.0));
    view.image.x = 50.0;
    view.image.background = Background::Light;
    let layout = capture(&workspace, None).unwrap();
    assert!(layout.validate().is_ok());
    let mut value = SavedView::from("logo.svg");
    let crate::domain::workspace_session::SavedPaneNode::Split { children, .. } = &layout.root
    else {
        panic!("split missing");
    };
    let crate::domain::workspace_session::SavedPaneNode::Pane { items, .. } = &children[1].node
    else {
        panic!("pane missing");
    };
    value.clone_from(&items[0]);
    let mut fresh = workspace.view(source).unwrap().clone();
    value.apply(&mut fresh);
    assert_eq!(fresh.image.scale, Some(2.0));
    assert_eq!(fresh.image.x, 50.0);
    assert_eq!(fresh.image.background, Background::Light);
    assert_eq!(workspace.view_document(preview), Some(document));
    assert!(value.association().is_some());
}
