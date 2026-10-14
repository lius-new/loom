//! Runtime resource ownership, independent of model and view lifetimes.
use crate::features::terminal::SessionId;
use std::collections::{HashMap, HashSet};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

pub(super) trait Resource {
    fn stop(&self);
}
pub(super) struct Registry<R: Resource> {
    entries: Mutex<(HashMap<SessionId, R>, HashSet<SessionId>)>,
    stopped: AtomicBool,
}
impl<R: Resource> Default for Registry<R> {
    fn default() -> Self {
        Self {
            entries: Mutex::new((HashMap::new(), HashSet::new())),
            stopped: AtomicBool::new(false),
        }
    }
}
impl<R: Resource> Registry<R> {
    pub fn install(&self, id: SessionId, create: impl FnOnce() -> R) -> bool {
        let mut entries = self.entries.lock().unwrap();
        if self.stopped.load(Ordering::Acquire)
            || entries.0.contains_key(&id)
            || entries.1.contains(&id)
        {
            return false;
        }
        entries.0.insert(id, create());
        true
    }
    pub fn get(&self, id: SessionId) -> Option<R>
    where
        R: Clone,
    {
        self.entries.lock().unwrap().0.get(&id).cloned()
    }
    /// Clone current resources in one lookup pass; callers release this lock
    /// before inspecting controllers or disposing any resource.
    pub fn get_many(&self, ids: &[SessionId]) -> Vec<(SessionId, R)>
    where
        R: Clone,
    {
        let entries = self.entries.lock().unwrap();
        ids.iter()
            .filter_map(|id| entries.0.get(id).cloned().map(|entry| (*id, entry)))
            .collect()
    }
    pub fn remove(&self, id: SessionId) -> bool {
        let entry = {
            let mut entries = self.entries.lock().unwrap();
            let entry = entries.0.remove(&id);
            if entry.is_some() {
                entries.1.insert(id);
            }
            entry
        };
        if let Some(entry) = entry {
            entry.stop();
            true
        } else {
            false
        }
    }
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        let entries = std::mem::take(&mut self.entries.lock().unwrap().0);
        for entry in entries.into_values() {
            entry.stop();
        }
    }
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().0.len()
    }
}
impl<R: Resource> Drop for Registry<R> {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Weak};
    struct Fake {
        id: SessionId,
        stops: Arc<Mutex<Vec<SessionId>>>,
        registry: Weak<Registry<Fake>>,
    }
    impl Resource for Fake {
        fn stop(&self) {
            if let Some(registry) = self.registry.upgrade() {
                assert!(
                    registry.entries.try_lock().is_ok(),
                    "resource stop ran under the registry lock"
                );
            }
            self.stops.lock().unwrap().push(self.id);
        }
    }
    #[test]
    fn create_move_split_close_and_shutdown_have_exact_resource_counts() {
        let registry = Arc::new(Registry::default());
        let creates = Mutex::new(Vec::new());
        let stops = Arc::new(Mutex::new(Vec::new()));
        let install = |id| {
            registry.install(id, || {
                creates.lock().unwrap().push(id);
                Fake {
                    id,
                    stops: stops.clone(),
                    registry: Arc::downgrade(&registry),
                }
            })
        };
        let mut model = crate::features::terminal::model::TerminalModel::new();
        let first = model.active_id().unwrap();
        assert!(install(first));
        assert!(!install(first));
        assert_eq!(*creates.lock().unwrap(), vec![first]);
        let moved = model.region_view(&[first], Some(first));
        model.detach_group(first);
        assert_eq!(moved.active_id(), Some(first));
        assert!(!install(first));
        assert!(stops.lock().unwrap().is_empty());
        let second = model.split(first).unwrap();
        assert!(install(second));
        assert_eq!(*creates.lock().unwrap(), vec![first, second]);
        let _snapshot = model.clone();
        assert!(model.close(first).is_some());
        assert!(registry.remove(first));
        assert!(!registry.remove(first));
        assert_eq!(*stops.lock().unwrap(), vec![first]);
        assert!(!install(first));
        registry.shutdown();
        registry.shutdown();
        assert!(!install(3));
        assert_eq!(*stops.lock().unwrap(), vec![first, second]);
        assert_eq!(*creates.lock().unwrap(), vec![first, second]);
        assert_eq!(registry.len(), 0);
    }
}
