//! Cross-thread UI context verification; rendering modules keep no thread orchestration.

#[test]
fn parallel_render_contexts_keep_their_own_device_scale() {
    use crate::ui::device_scale::{observe, observed};
    use std::sync::{Arc, Barrier};
    observe(0.75);
    let barrier = Arc::new(Barrier::new(2));
    let other = barrier.clone();
    let worker = std::thread::spawn(move || {
        let initial = observed();
        observe(2.0);
        other.wait();
        (initial, observed())
    });
    barrier.wait();
    let local = observed();
    let remote = worker.join().unwrap();
    assert_eq!(local, 0.75);
    assert_eq!(remote, (1.0, 2.0));
    observe(f32::NAN);
    observe(0.0);
    assert_eq!(observed(), 0.75);
}

#[test]
fn parallel_render_contexts_keep_their_own_theme() {
    use crate::ui::theme::{ThemeId, current, set};
    use std::sync::{Arc, Barrier};
    let light = ThemeId::parse("aura-light").unwrap();
    let contrast = ThemeId::parse("high-contrast").unwrap();
    set(light);
    let barrier = Arc::new(Barrier::new(2));
    let other = barrier.clone();
    let worker = std::thread::spawn(move || {
        let initial = current();
        set(contrast);
        other.wait();
        (initial, current())
    });
    barrier.wait();
    let local = current();
    let remote = worker.join().unwrap();
    assert_eq!(local, light);
    assert_eq!(remote, (ThemeId::default(), contrast));
}

#[test]
fn parallel_render_contexts_keep_their_own_reader_feedback() {
    use crate::features::markdown::model::NodeId;
    use crate::ui::markdown::reader::{context_feedback, set_context_feedback};
    use std::sync::{Arc, Barrier};
    set_context_feedback(3.0, NodeId(1));
    let barrier = Arc::new(Barrier::new(2));
    let other = barrier.clone();
    let worker = std::thread::spawn(move || {
        let initial = context_feedback();
        set_context_feedback(9.0, NodeId(2));
        other.wait();
        (initial, context_feedback())
    });
    barrier.wait();
    let local = context_feedback();
    let remote = worker.join().unwrap();
    assert_eq!(local, (3.0, Some(NodeId(1))));
    assert_eq!(remote, ((0.0, None), (9.0, Some(NodeId(2)))));
}
