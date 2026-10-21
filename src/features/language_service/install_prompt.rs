//! Session-scoped installation invitations, independent of UI and installation I/O.
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub server: String,
    pub name: String,
    pub languages: Vec<String>,
    /// Missing archives, including runtimes; IDs allow shared downloads to count once.
    pub archives: BTreeMap<String, u64>,
}

#[derive(Default)]
pub struct Prompts {
    seen: BTreeSet<String>,
    active: BTreeMap<String, Offer>,
}

impl Prompts {
    pub fn reconcile(&mut self, candidates: Vec<Offer>) -> Vec<Offer> {
        let candidates = candidates
            .into_iter()
            .map(|offer| (offer.server.clone(), offer))
            .collect::<BTreeMap<_, _>>();
        self.active
            .retain(|server, _| candidates.contains_key(server));
        for (server, offer) in candidates {
            if self.active.contains_key(&server) || self.seen.insert(server.clone()) {
                self.active.insert(server, offer);
            }
        }
        self.active.values().cloned().collect()
    }

    pub fn dismiss(&mut self, servers: &[String]) {
        for server in servers {
            self.active.remove(server);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn offer(server: &str) -> Offer {
        Offer {
            server: server.into(),
            name: server.into(),
            languages: vec!["Example".into()],
            archives: BTreeMap::new(),
        }
    }

    #[test]
    fn invitations_merge_and_each_server_is_presented_once_per_session() {
        let mut prompts = Prompts::default();
        assert_eq!(prompts.reconcile(vec![offer("a"), offer("b")]).len(), 2);
        assert_eq!(prompts.reconcile(vec![offer("a"), offer("b")]).len(), 2);
        prompts.dismiss(&["a".into()]);
        assert_eq!(
            prompts.reconcile(vec![offer("a"), offer("b")]),
            [offer("b")]
        );
        assert!(prompts.reconcile(Vec::new()).is_empty());
        assert!(prompts.reconcile(vec![offer("a"), offer("b")]).is_empty());
        assert_eq!(prompts.reconcile(vec![offer("c")]), [offer("c")]);
        assert_eq!(Prompts::default().reconcile(vec![offer("a")]), [offer("a")]);
    }

    #[test]
    fn an_action_for_an_older_batch_leaves_newly_arrived_servers_visible() {
        let mut prompts = Prompts::default();
        prompts.reconcile(vec![offer("a")]);
        prompts.reconcile(vec![offer("a"), offer("b")]);
        prompts.dismiss(&["a".into()]);
        assert_eq!(
            prompts.reconcile(vec![offer("a"), offer("b")]),
            [offer("b")]
        );
    }
}
