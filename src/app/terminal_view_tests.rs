#[cfg(test)]
mod tests {
    use crate::app::terminal_binding::TerminalTabs;
    use crate::app::terminal_runtime::TerminalDisplay;
    use crate::app::terminal_view::{render, render_terminal_tabs, session_rects, shell_menu};
    use crate::features::terminal::{ShellKind, TerminalRun};
    use crate::state::AppState;
    use crate::ui::terminal::keys::key_bytes;
    use crate::ui::terminal::view::*;
    use crate::ui::{fonts, theme};
    use lgui::ApplicationHandle;
    use lgui::core::{EventPolicy, Point, PointerButton, UiElement, UiId};
    use lgui::core::{KeyState, KeyboardEvent, LogicalKey, NamedKey};
    use lgui::prelude::{Element, State, UiRect, VisualStyle, group};
    use std::sync::Arc;

    #[test]
    fn shell_menu_keeps_missing_profiles_disabled_and_creates_custom_sessions_in_layout() {
        use crate::domain::application_layout::ApplicationLayout;
        use crate::features::terminal::profiles::ShellProfile;
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{InputEvent, PointerData, UiScale, dispatch_runtime_output};
        use lgui::session::UiSession;
        use std::sync::Mutex;
        let viewport = UiRect::new(0.0, 0.0, 400.0, 400.0);
        let exposed = Arc::new(Mutex::new(
            None::<(State<AppState>, crate::app::terminal_binding::TerminalStore)>,
        ));
        let output = exposed.clone();
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(|| {
                let mut app = AppState::new();
                app.terminal_profiles = (1..=10)
                    .map(|id| ShellProfile {
                        id,
                        name: format!("Custom {id}"),
                        program: std::env::current_exe().unwrap(),
                        args: vec![format!("--profile={id}")],
                    })
                    .collect();
                app.shell_options = app
                    .terminal_profiles
                    .iter()
                    .map(crate::infrastructure::terminal::profiles::custom)
                    .collect();
                app.shell_options[0].program = None;
                let mut layout = ApplicationLayout::initial(viewport.into(), 40.0, 40.0, 200.0);
                layout.attach(1);
                app.layout.application_layout = Some(layout);
                app.terminal.ui.terminal_shell_menu = true;
                app
            });
            let tabs = crate::app::terminal_binding::test_store(cx, Some(state.clone()), || {
                let mut tabs = TerminalTabs::new();
                tabs.set_profiles(&state.get().terminal_profiles);
                tabs
            });
            *output.lock().unwrap() = Some((state.clone(), tabs.clone()));
            let id = cx.use_stable_id();
            let focus = cx.focus_handle(id);
            group(viewport).child(shell_menu(
                UiRect::new(300.0, 0.0, 390.0, 30.0),
                viewport,
                state,
                focus,
                tabs,
            ))
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let (state, tabs) = exposed.lock().unwrap().clone().unwrap();
        let context = ApplicationContext::empty(Default::default());
        fn click_label(
            session: &mut UiSession,
            context: &ApplicationContext,
            view: &AppView,
            viewport: UiRect,
            label: &str,
        ) {
            let node = session
                .tree()
                .nodes()
                .iter()
                .find(|n| {
                    n.text
                        .as_deref()
                        .is_some_and(|text| text.starts_with(label))
                })
                .unwrap_or_else(|| panic!("missing menu label {label}"));
            let point = Point::new(
                (node.layout_rect.left + node.layout_rect.right) * 0.5,
                (node.layout_rect.top + node.layout_rect.bottom) * 0.5,
            );
            for event in [
                InputEvent::PointerDown {
                    pointer: PointerData::mouse(point),
                    button: PointerButton::Left,
                },
                InputEvent::PointerUp {
                    pointer: PointerData::mouse(point),
                    button: PointerButton::Left,
                },
            ] {
                let events = session.handle_input(event);
                dispatch_runtime_output(
                    events,
                    context,
                    &lgui::window::WindowId::new("shell-menu-test"),
                    |action| session.handle_default_action(action),
                    |_| {},
                );
                session.render_view(view, viewport, UiScale::ONE);
            }
        }
        click_label(&mut session, &context, &view, viewport, "Custom 1");
        assert_eq!(tabs.get().tabs().len(), 1);
        state.update(|app| app.terminal.ui.terminal_shell_scroll = 9);
        session.render_view(&view, viewport, UiScale::ONE);
        click_label(&mut session, &context, &view, viewport, "Custom 10");
        let tab = tabs.get().active().unwrap().clone();
        assert_eq!(tab.controller.shell(), ShellKind::Custom(10));
        assert_eq!(tab.controller.short_label(), "Custom 10");
        assert_eq!(
            state
                .get()
                .layout
                .application_layout
                .as_ref()
                .unwrap()
                .default_terminal_active(),
            Some(tab.id)
        );
        assert!(!state.get().terminal.ui.terminal_shell_menu);
    }

    #[test]
    fn mouse_coordinates_use_cells_instead_of_selection_boundaries() {
        let content = UiRect::new(12.0, 6.0, 500.0, 400.0);
        let point = Point::new(
            content.left + terminal_cell_width() * 3.75,
            content.top + terminal_line_h() * 2.5,
        );
        assert_eq!(mouse_cell_at(content, point, 10, 20), (2, 3));
        assert_eq!(cell_at(content, point, 10, 20), (2, 4));
        assert_eq!(
            mouse_cell_at(content, Point::new(-10.0, -10.0), 10, 20),
            (0, 0)
        );
        assert_eq!(
            mouse_cell_at(content, Point::new(1000.0, 1000.0), 10, 20),
            (9, 19)
        );
    }

    #[test]
    fn resizing_adjacent_panes_preserves_other_widths_and_fits_narrow_windows() {
        let mut tabs = TerminalTabs::new();
        let first = tabs.active_id().unwrap();
        let second = tabs.split(first).unwrap();
        tabs.split(second);
        let bounds = UiRect::new(20.0, 0.0, 1020.0, 300.0);
        let before = session_rects(bounds, &tabs.active_group());
        tabs.resize_pair(first, second, 0.8);
        let after = session_rects(bounds, &tabs.active_group());
        assert!(after[0].width() > before[0].width());
        assert!(after[1].width() < before[1].width());
        assert!((after[2].left - before[2].left).abs() < 0.001);
        assert!((after[2].width() - before[2].width()).abs() < 0.001);
        assert!(after.iter().all(|pane| pane.width() >= 199.0));
        let narrow = session_rects(UiRect::new(0.0, 0.0, 90.0, 300.0), &tabs.active_group());
        assert_eq!(narrow.last().unwrap().right, 90.0);
        assert!(narrow.iter().all(|pane| pane.width() >= 29.0));
    }

    #[test]
    fn split_panes_select_independently_preserve_drag_selection_and_route_input() {
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{InputEvent, PointerData, UiScale, dispatch_runtime_output};
        use lgui::session::UiSession;
        use std::sync::Mutex;

        let exposed = Arc::new(Mutex::new(
            None::<(State<AppState>, crate::app::terminal_binding::TerminalStore)>,
        ));
        let output = exposed.clone();
        let application = Arc::new(ApplicationHandle::new(|task| task(), || {}));
        let viewport = UiRect::new(0.0, 0.0, 600.0, 300.0);
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(AppState::new);
            let tabs = crate::app::terminal_binding::test_store(cx, Some(state.clone()), || {
                let mut tabs = TerminalTabs::new();
                tabs.split(1);
                tabs
            });
            *output.lock().unwrap() = Some((state.clone(), tabs.clone()));
            let id = UiId::new("split-terminal-input");
            let focus = cx.focus_handle(id.clone());
            let editor_id = cx.use_stable_id();
            let editor_focus = cx.focus_handle(editor_id.clone());
            let controller = tabs.get().active().unwrap().controller.clone();
            group(viewport)
                .child(
                    Element::new(move |cx| {
                        UiElement::panel(
                            editor_id,
                            UiRect::new(0.0, 260.0, 600.0, 300.0),
                            VisualStyle::default(),
                        )
                        .children(cx.children)
                    })
                    .event_policy(EventPolicy::INTERACTIVE),
                )
                .child(render(
                    UiRect::new(0.0, 0.0, 600.0, 260.0),
                    state,
                    editor_focus,
                    focus,
                    id,
                    controller,
                    tabs,
                    application.clone(),
                    true,
                    260.0,
                ))
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let (state, tabs) = exposed.lock().unwrap().clone().unwrap();
        assert_eq!(
            session
                .tree()
                .node(&UiId::new("split-terminal-input"))
                .unwrap()
                .ime_cursor_rect
                .unwrap()
                .left,
            313.0
        );
        assert_eq!(
            session
                .tree()
                .nodes()
                .iter()
                .filter(|node| node.text.as_deref() == Some(ShellKind::default().short_label()))
                .count(),
            3 // Two tab labels and the shell selector in the terminal header.
        );
        let context = ApplicationContext::empty(Default::default());
        let mut send = |input| {
            let events = session.handle_input(input);
            dispatch_runtime_output(
                events,
                &context,
                &lgui::window::WindowId::new("terminal-split-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
            session
                .tree()
                .node(&UiId::new("split-terminal-input"))
                .unwrap()
                .ime_cursor_rect
                .unwrap()
        };
        let divider = PointerData::mouse(Point::new(300.5, 80.0));
        send(InputEvent::PointerDown {
            pointer: divider,
            button: PointerButton::Left,
        });
        assert_eq!(state.get().terminal.ui.terminal_split_drag, Some(1));
        let moved = PointerData::mouse(Point::new(350.0, 80.0));
        send(InputEvent::PointerMove(moved));
        assert_eq!(tabs.get().active_id(), Some(2));
        let layout = session_rects(
            UiRect::new(0.0, 0.0, 600.0, 260.0),
            &tabs.get().active_group(),
        );
        assert_eq!(layout[0].right, 350.0);
        assert!(pty_size_for_body(layout[0]).cols > pty_size_for_body(layout[1]).cols);
        send(InputEvent::PointerUp {
            pointer: moved,
            button: PointerButton::Left,
        });
        assert_eq!(state.get().terminal.ui.terminal_split_drag, None);
        let weights = tabs
            .get()
            .active_group()
            .iter()
            .map(|tab| tab.width_weight)
            .collect::<Vec<_>>();
        send(InputEvent::PointerMove(divider));
        assert_eq!(
            tabs.get()
                .active_group()
                .iter()
                .map(|tab| tab.width_weight)
                .collect::<Vec<_>>(),
            weights
        );

        let start = PointerData::mouse(Point::new(50.0, 80.0));
        let caret = send(InputEvent::PointerDown {
            pointer: start,
            button: PointerButton::Left,
        });
        assert_eq!(tabs.get().active_id(), Some(1));
        assert_eq!(caret.left, 12.0);
        let end = PointerData::mouse(Point::new(85.0, 80.0));
        send(InputEvent::PointerMove(end));
        send(InputEvent::PointerUp {
            pointer: end,
            button: PointerButton::Left,
        });
        assert!(!state.get().terminal.ui.selecting_terminal);
        let snapshot = tabs.get();
        let left = snapshot.tabs()[0].controller.clone();
        let right = snapshot.tabs()[1].controller.clone();
        assert!(!left.snapshot().selection.is_empty());
        assert!(right.snapshot().selection.is_empty());
        right.begin_selection(0, 0);
        right.extend_selection(0, 2);
        send(InputEvent::TextInput("left terminal input".into()));
        assert!(left.snapshot().selection.is_empty());
        assert!(!right.snapshot().selection.is_empty());

        let add = PointerData::mouse(Point::new(585.0, 17.0));
        send(InputEvent::PointerDown {
            pointer: add,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: add,
            button: PointerButton::Left,
        });
        assert_eq!(tabs.get().tabs().len(), 3);
        assert_eq!(tabs.get().active_group().len(), 1);
        assert_eq!(tabs.get().saved_groups(), vec![0, 0, 1]);
        let panes = pane_rects(UiRect::new(0.0, 0.0, 600.0, 260.0), 2);
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].right + 1.0, panes[1].left);
        assert!(
            pty_size_for_body(panes[0]).cols < pty_size(UiRect::new(0.0, 0.0, 600.0, 260.0)).cols
        );
    }

    #[test]
    fn tab_context_menu_targets_inactive_tabs_and_blocks_shell_input_until_dismissed() {
        use lgui::application::{AppView, ApplicationContext};
        use lgui::core::{InputEvent, UiScale, dispatch_runtime_output};
        use lgui::session::UiSession;
        use std::sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        };

        let exposed = Arc::new(Mutex::new(
            None::<(State<AppState>, crate::app::terminal_binding::TerminalStore)>,
        ));
        let output = exposed.clone();
        let writes = Arc::new(AtomicUsize::new(0));
        let received = writes.clone();
        let application = Arc::new(ApplicationHandle::new(|task| task(), || {}));
        let viewport = UiRect::new(0.0, 0.0, 600.0, 300.0);
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(|| {
                let mut app = AppState::new();
                app.terminal_chrome.show_terminal = true;
                app
            });
            let tabs = crate::app::terminal_binding::test_store(cx, Some(state.clone()), || {
                let mut tabs = TerminalTabs::new();
                tabs.add(ShellKind::Cmd);
                tabs.add(ShellKind::Bash);
                tabs
            });
            *output.lock().unwrap() = Some((state.clone(), tabs.clone()));
            let terminal_id = cx.use_stable_id();
            let terminal_focus = cx.focus_handle(terminal_id.clone());
            let editor_id = cx.use_stable_id();
            let editor_focus = cx.focus_handle(editor_id.clone());
            crate::ui::terminal::menu::restore_focus_on_dismiss(
                cx,
                state.get().terminal.ui.terminal_tab_context_menu.is_some(),
                state.get().terminal_chrome.show_terminal,
                terminal_focus.clone(),
                editor_focus.clone(),
            );
            let received = received.clone();
            let terminal = Element::new(move |cx| {
                UiElement::panel(
                    terminal_id,
                    UiRect::new(0.0, 40.0, 600.0, 200.0),
                    VisualStyle::default(),
                )
                .auto_focus()
                .children(cx.children)
            })
            .event_policy(EventPolicy::INTERACTIVE)
            .on_input(move |_, _| {
                received.fetch_add(1, Ordering::AcqRel);
            });
            let editor = Element::new(move |cx| {
                UiElement::panel(
                    editor_id,
                    UiRect::new(0.0, 200.0, 600.0, 300.0),
                    VisualStyle::default(),
                )
                .children(cx.children)
            })
            .event_policy(EventPolicy::INTERACTIVE);
            let snapshot = tabs.get();
            let mut root = group(viewport).child(editor);
            if state.get().terminal_chrome.show_terminal {
                root = root.child(terminal);
            }
            if let Some(active_id) = snapshot.active_id() {
                root = root.child(render_terminal_tabs(
                    UiRect::new(0.0, 0.0, 600.0, 34.0),
                    &snapshot.tabs(),
                    active_id,
                    tabs.clone(),
                    state.clone(),
                    crate::app::layout_map::MapStore::default(),
                    terminal_focus.clone(),
                    editor_focus.clone(),
                    application.clone(),
                ));
            }
            if state.get().terminal.ui.terminal_tab_context_menu.is_some() {
                root = root.child(crate::app::terminal_view::menu(
                    viewport,
                    state,
                    tabs,
                    application.clone(),
                    terminal_focus,
                    editor_focus,
                ));
            }
            root
        });
        let mut session = UiSession::new();
        session.render_view(&view, viewport, UiScale::ONE);
        let (state, tabs) = exposed.lock().unwrap().clone().unwrap();
        let context = ApplicationContext::empty(Default::default());
        let mut send = |input| {
            let events = session.handle_input(input);
            dispatch_runtime_output(
                events,
                &context,
                &lgui::window::WindowId::new("terminal-menu-test"),
                |action| session.handle_default_action(action),
                |_| {},
            );
            session.render_view(&view, viewport, UiScale::ONE);
            session.runtime().run_effects();
            session.render_view(&view, viewport, UiScale::ONE);
        };
        let point = lgui::core::PointerData::mouse(Point::new(30.0, 10.0));
        send(InputEvent::PointerDown {
            pointer: point,
            button: PointerButton::Right,
        });
        send(InputEvent::PointerUp {
            pointer: point,
            button: PointerButton::Right,
        });
        assert_eq!(
            state
                .get()
                .terminal
                .ui
                .terminal_tab_context_menu
                .unwrap()
                .target,
            1
        );
        assert_eq!(tabs.get().active_id(), Some(3));
        send(InputEvent::TextInput("must not reach shell".into()));
        assert_eq!(writes.load(Ordering::Acquire), 0);
        send(InputEvent::Keyboard(key(
            LogicalKey::Named(NamedKey::Escape),
            Default::default(),
        )));
        assert!(state.get().terminal.ui.terminal_tab_context_menu.is_none());
        send(InputEvent::TextInput("shell input restored".into()));
        assert_eq!(writes.load(Ordering::Acquire), 1);

        send(InputEvent::PointerDown {
            pointer: point,
            button: PointerButton::Right,
        });
        send(InputEvent::PointerUp {
            pointer: point,
            button: PointerButton::Right,
        });
        let others = lgui::core::PointerData::mouse(Point::new(45.0, 97.0));
        send(InputEvent::PointerDown {
            pointer: others,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: others,
            button: PointerButton::Left,
        });
        assert!(state.get().terminal.ui.terminal_tab_context_menu.is_none());
        assert_eq!(tabs.get().tabs().len(), 1);
        assert_eq!(tabs.get().active_id(), Some(1));
        send(InputEvent::PointerDown {
            pointer: point,
            button: PointerButton::Right,
        });
        send(InputEvent::PointerUp {
            pointer: point,
            button: PointerButton::Right,
        });
        let outside = lgui::core::PointerData::mouse(Point::new(500.0, 250.0));
        send(InputEvent::PointerDown {
            pointer: outside,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: outside,
            button: PointerButton::Left,
        });
        assert!(state.get().terminal.ui.terminal_tab_context_menu.is_none());
        send(InputEvent::TextInput(
            "outside dismissal restores input".into(),
        ));
        assert_eq!(writes.load(Ordering::Acquire), 2);

        // Single click selects a terminal; double click opens its purpose menu.
        let label = lgui::core::PointerData::mouse(Point::new(30.0, 10.0));
        send(InputEvent::PointerDown {
            pointer: label,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: label,
            button: PointerButton::Left,
        });
        assert!(state.get().terminal.ui.terminal_tab_context_menu.is_none());
        send(InputEvent::PointerDown {
            pointer: label,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: label,
            button: PointerButton::Left,
        });
        let purpose_menu = state.get().terminal.ui.terminal_tab_context_menu.unwrap();
        assert_eq!(
            purpose_menu.kind,
            crate::ui::terminal::state::TerminalTabMenuKind::Purpose
        );
        assert_eq!(purpose_menu.target, 1);
        let origin = purpose_menu.position;
        let testing = lgui::core::PointerData::mouse(Point::new(origin.0 + 20.0, origin.1 + 59.0));
        send(InputEvent::PointerDown {
            pointer: testing,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: testing,
            button: PointerButton::Left,
        });
        assert_eq!(tabs.get().tabs()[0].label(), "pwsh · Testing");
        assert!(state.get().terminal.ui.terminal_tab_context_menu.is_none());
        send(InputEvent::TextInput(
            "typing resumes after purpose selection".into(),
        ));
        assert_eq!(writes.load(Ordering::Acquire), 3);

        // The general preset replaces free-form purpose entry.
        state.update(|app| {
            app.terminal.ui.terminal_tab_context_menu =
                Some(crate::ui::terminal::state::TerminalTabContextMenuState {
                    position: (20.0, 40.0),
                    target: 1,
                    kind: crate::ui::terminal::state::TerminalTabMenuKind::Purpose,
                })
        });
        send(InputEvent::PointerMove(lgui::core::PointerData::mouse(
            Point::new(40.0, 165.0),
        )));
        let other = lgui::core::PointerData::mouse(Point::new(40.0, 165.0));
        send(InputEvent::TextInput(
            "must not rename or reach shell".into(),
        ));
        assert_eq!(writes.load(Ordering::Acquire), 3);
        send(InputEvent::PointerDown {
            pointer: other,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: other,
            button: PointerButton::Left,
        });
        assert!(state.get().terminal.ui.terminal_tab_context_menu.is_none());
        assert_eq!(tabs.get().tabs()[0].label(), "pwsh · Other");

        tabs.update(|tabs| tabs.update_programs(&[(1, Some("node".into()))]));
        state.update(|app| {
            app.terminal.ui.terminal_tab_context_menu =
                Some(crate::ui::terminal::state::TerminalTabContextMenuState {
                    position: (20.0, 40.0),
                    target: 1,
                    kind: crate::ui::terminal::state::TerminalTabMenuKind::Purpose,
                })
        });
        let automatic = lgui::core::PointerData::mouse(Point::new(40.0, 191.0));
        send(InputEvent::PointerMove(automatic));
        send(InputEvent::PointerDown {
            pointer: automatic,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: automatic,
            button: PointerButton::Left,
        });
        assert_eq!(tabs.get().tabs()[0].label(), "pwsh · node");
        tabs.update(|tabs| tabs.update_programs(&[(1, None)]));

        send(InputEvent::PointerDown {
            pointer: point,
            button: PointerButton::Right,
        });
        send(InputEvent::PointerUp {
            pointer: point,
            button: PointerButton::Right,
        });
        let split = lgui::core::PointerData::mouse(Point::new(45.0, 47.0));
        send(InputEvent::PointerDown {
            pointer: split,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: split,
            button: PointerButton::Left,
        });
        assert_eq!(tabs.get().active_group().len(), 2);
        assert_eq!(tabs.get().saved_groups(), vec![0, 0]);

        send(InputEvent::PointerDown {
            pointer: point,
            button: PointerButton::Right,
        });
        send(InputEvent::PointerUp {
            pointer: point,
            button: PointerButton::Right,
        });
        let new = lgui::core::PointerData::mouse(Point::new(45.0, 25.0));
        send(InputEvent::PointerDown {
            pointer: new,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: new,
            button: PointerButton::Left,
        });
        assert_eq!(tabs.get().tabs().len(), 3);
        assert_eq!(tabs.get().active_id(), Some(5));
        assert_eq!(tabs.get().saved_groups(), vec![0, 0, 1]);
        assert_eq!(
            tabs.get().active().unwrap().controller.shell(),
            ShellKind::default()
        );

        send(InputEvent::PointerDown {
            pointer: point,
            button: PointerButton::Right,
        });
        send(InputEvent::PointerUp {
            pointer: point,
            button: PointerButton::Right,
        });
        let all = lgui::core::PointerData::mouse(Point::new(45.0, 140.0));
        send(InputEvent::PointerDown {
            pointer: all,
            button: PointerButton::Left,
        });
        send(InputEvent::PointerUp {
            pointer: all,
            button: PointerButton::Left,
        });
        assert!(tabs.get().is_empty());
        assert!(!state.get().terminal_chrome.show_terminal);
        assert!(state.get().terminal.ui.terminal_tab_context_menu.is_none());
    }

    #[test]
    fn measured_cell_width_drives_cursor_runs_hit_testing_and_pty_size() {
        // Simulate a renderer font advance that differs from the old 7.2px.
        let _restore = fonts::override_terminal(fonts::Metrics {
            char_w: 6.25,
            ..fonts::terminal()
        });
        let content = UiRect::new(10.0, 20.0, 2010.0, 100.0);
        let mut parser = vt100::Parser::new(4, 300, 0);
        for count in 1..=200 {
            parser.process(b"x");
            let cursor = terminal_cursor_rect(content, parser.screen().cursor_position(), 4, 300);
            assert_eq!(cursor.left, content.left + count as f32 * 6.25);
            let run = TerminalRun {
                start_col: 0,
                columns: count,
                text: "x".repeat(usize::from(count)),
                style: Default::default(),
            };
            assert_eq!(run_span(content, &run).1 - 2.0, cursor.left);
            assert_eq!(
                cell_at(content, Point::new(cursor.left, content.top), 4, 300),
                (0, count)
            );
        }
        assert_eq!(pty_size(UiRect::new(0.0, 0.0, 400.0, 160.0)).cols, 60);
    }

    #[test]
    fn cursor_remains_visible_while_a_full_line_waits_to_wrap() {
        let width = terminal_cell_width();
        let content = UiRect::new(0.0, 0.0, width * 8.0, terminal_line_h() * 2.0);
        let mut parser = vt100::Parser::new(2, 8, 0);
        parser.process(b"12345678");
        assert_eq!(parser.screen().cursor_position(), (0, 8));
        let cursor = terminal_cursor_rect(content, parser.screen().cursor_position(), 2, 8);
        assert!((cursor.right - content.right).abs() < 0.001);
        assert_eq!(cursor.top, 2.0);
        parser.process(b"9");
        let cursor = terminal_cursor_rect(content, parser.screen().cursor_position(), 2, 8);
        assert_eq!(cursor.left, width);
        assert_eq!(cursor.top, terminal_line_h() + 2.0);
    }

    #[test]
    fn focused_terminal_cursor_follows_blink_phase() {
        assert!(cursor_is_drawn(true, true, true));
        assert!(!cursor_is_drawn(true, true, false));
        assert!(cursor_is_drawn(true, false, false));
        assert!(!cursor_is_drawn(false, true, true));
    }

    fn key(key: LogicalKey, modifiers: lgui::core::KeyModifiers) -> KeyboardEvent {
        KeyboardEvent {
            state: KeyState::Down,
            key,
            modifiers,
            ..Default::default()
        }
    }

    #[test]
    fn modified_navigation_keys_use_xterm_sequences() {
        use lgui::core::KeyModifiers;
        let left = LogicalKey::Named(NamedKey::ArrowLeft);
        assert_eq!(
            key_bytes(&key(left.clone(), KeyModifiers::empty()), false).unwrap(),
            b"\x1b[D"
        );
        assert_eq!(
            key_bytes(&key(left.clone(), KeyModifiers::empty()), true).unwrap(),
            b"\x1bOD"
        );
        assert_eq!(
            key_bytes(&key(left, KeyModifiers::CONTROL), false).unwrap(),
            b"\x1b[1;5D"
        );
        assert_eq!(
            key_bytes(
                &key(LogicalKey::Named(NamedKey::Delete), KeyModifiers::CONTROL),
                false
            )
            .unwrap(),
            b"\x1b[3;5~"
        );
        assert_eq!(
            key_bytes(
                &key(
                    LogicalKey::Named(NamedKey::Backspace),
                    KeyModifiers::CONTROL
                ),
                false
            )
            .unwrap(),
            b"\x08"
        );
    }

    #[test]
    fn pointer_maps_to_the_nearest_cell_boundary() {
        let content = UiRect::new(10.0, 20.0, 400.0, 200.0);
        let point = |x, y| Point { x, y };
        assert_eq!(cell_at(content, point(10.0, 20.0), 5, 40), (0, 0));
        assert_eq!(
            cell_at(
                content,
                point(
                    10.0 + terminal_cell_width() * 2.6,
                    20.0 + terminal_line_h() * 1.5
                ),
                5,
                40
            ),
            (1, 3)
        );
        assert_eq!(cell_at(content, point(-50.0, 900.0), 5, 40), (4, 0));
        assert_eq!(cell_at(content, point(9_000.0, 0.0), 5, 40), (0, 40));
    }

    #[test]
    fn terminal_resize_height_is_clamped_to_its_allowed_range() {
        assert_eq!(resized_height(600.0, 450.0, 420.0), 150.0);
        assert_eq!(resized_height(600.0, 550.0, 420.0), theme::TERMINAL_MIN_H);
        assert_eq!(resized_height(600.0, 100.0, 420.0), 420.0);
    }

    #[test]
    fn pty_size_tracks_the_visible_terminal_body() {
        let small = pty_size(UiRect::new(0.0, 0.0, 400.0, 160.0));
        let large = pty_size(UiRect::new(0.0, 0.0, 800.0, 320.0));
        assert!(large.cols > small.cols);
        assert!(large.rows > small.rows);
        assert!(small.rows > 0 && small.cols > 1);
    }
}
