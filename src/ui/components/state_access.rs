//! A synchronous projection; callers can read/update only the projected value.
use lgui::prelude::State;
use std::sync::Arc;

type Update<T> = dyn Fn(&mut dyn FnMut(&mut T) -> bool) -> bool + Send + Sync;

#[derive(Clone)]
pub struct StateAccess<T> {
    read: Arc<dyn Fn() -> T + Send + Sync>,
    write: Arc<Update<T>>,
}
impl<T: Clone + Send + 'static> StateAccess<T> {
    pub fn project<S: Clone + Send + 'static>(
        state: State<S>,
        read: fn(&S) -> &T,
        write: fn(&mut S) -> &mut T,
    ) -> Self {
        let reader = state.clone();
        Self {
            read: Arc::new(move || read(&reader.get()).clone()),
            write: Arc::new(move |update| state.try_update(|s| update(write(s)))),
        }
    }
    pub fn get(&self) -> T {
        (self.read)()
    }
    pub fn update(&self, update: impl FnOnce(&mut T)) {
        self.try_update(move |value| {
            update(value);
            true
        });
    }
    pub fn try_update(&self, update: impl FnOnce(&mut T) -> bool) -> bool {
        let mut update = Some(update);
        (self.write)(&mut |value| update.take().expect("projection update called twice")(value))
    }
}
impl<T: Clone + Send + 'static> From<State<T>> for StateAccess<T> {
    fn from(state: State<T>) -> Self {
        let reader = state.clone();
        Self {
            read: Arc::new(move || reader.get()),
            write: Arc::new(move |update| state.try_update(update)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lgui::application::AppView;
    use lgui::core::UiScale;
    use lgui::prelude::{UiRect, VisualStyle, panel};
    use lgui::session::UiSession;
    use std::sync::Mutex;

    #[test]
    fn projection_changes_only_its_value_and_preserves_no_change_signals() {
        #[derive(Clone)]
        struct Root {
            search: usize,
            workspace: String,
        }
        let exposed = Arc::new(Mutex::new(None));
        let output = exposed.clone();
        let view: AppView = Arc::new(move |cx| {
            let state = cx.state_with(|| Root {
                search: 1,
                workspace: "untouched".into(),
            });
            *output.lock().unwrap() = Some(state);
            panel(UiRect::new(0.0, 0.0, 100.0, 100.0), VisualStyle::default())
        });
        let mut session = UiSession::new();
        session.render_view(&view, UiRect::new(0.0, 0.0, 100.0, 100.0), UiScale::ONE);
        let root = exposed.lock().unwrap().clone().unwrap();
        let access = StateAccess::project(root.clone(), |r| &r.search, |r| &mut r.search);
        access.update(|value| *value = 2);
        assert_eq!(access.get(), 2);
        assert_eq!(root.get().workspace, "untouched");
        assert!(!access.try_update(|_| false));
        assert_eq!(root.get().search, 2);
    }
}
