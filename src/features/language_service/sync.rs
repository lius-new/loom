//! Keeps a server's idea of the open documents equal to the editor's.
//!
//! Rather than turning each edit into a message, the caller states which
//! documents should be open and in what state, and [`DocumentSync::reconcile`]
//! returns the notifications that bring the server there. Whatever path changed a
//! buffer (typing, undo, replace-all, reload from disk, a rename), the next
//! reconcile sees the new version and sends it, so no write entry can be missed.
//!
//! The text is always sent whole. A change event without a range is valid for
//! servers that asked for incremental updates too, and it makes the server's copy
//! correct by construction; sending ranges is an optimisation for later.
use super::capabilities::{SyncKind, TextSync};
use super::uri;
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

/// Which state of which document: a document that was closed and opened again
/// has a new `incarnation`, and `version` counts the changes to its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub incarnation: u64,
    pub version: u64,
}

/// A document as the editor holds it right now.
#[derive(Clone, Debug)]
pub struct DesiredDocument {
    pub uri: String,
    /// The server's name for the language, from the server binding.
    pub language_id: String,
    pub stamp: Stamp,
    pub text: Arc<String>,
}

#[derive(Debug, PartialEq)]
pub struct Notification {
    pub method: &'static str,
    pub params: Value,
}

/// How many protocol versions are remembered per document, to match a server's
/// answer (for example diagnostics) with the editor state it was computed from.
const HISTORY: usize = 32;

struct Open {
    uri: String,
    language_id: String,
    sent: Stamp,
    version: i32,
    history: VecDeque<(i32, Stamp)>,
}

impl Open {
    fn remember(&mut self) {
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back((self.version, self.sent));
    }
}

pub struct DocumentSync {
    sync: TextSync,
    open: HashMap<String, Open>,
}

fn did_open(document: &DesiredDocument, version: i32) -> Notification {
    Notification {
        method: "textDocument/didOpen",
        params: json!({ "textDocument": {
            "uri": document.uri,
            "languageId": document.language_id,
            "version": version,
            "text": &*document.text,
        } }),
    }
}

fn did_close(uri: &str) -> Notification {
    Notification {
        method: "textDocument/didClose",
        params: json!({ "textDocument": { "uri": uri } }),
    }
}

impl DocumentSync {
    pub fn new(sync: TextSync) -> Self {
        Self {
            sync,
            open: HashMap::new(),
        }
    }

    pub fn open_count(&self) -> usize {
        self.open.len()
    }

    pub fn is_open(&self, uri: &str) -> bool {
        self.open.contains_key(&uri::key(uri))
    }

    /// The protocol version last sent for `uri`.
    pub fn version_of(&self, uri: &str) -> Option<i32> {
        self.open.get(&uri::key(uri)).map(|open| open.version)
    }

    /// The editor state that was sent as protocol `version` of `uri`, if still
    /// remembered.
    pub fn stamp_of(&self, uri: &str, version: i32) -> Option<Stamp> {
        let open = self.open.get(&uri::key(uri))?;
        open.history
            .iter()
            .find(|(v, _)| *v == version)
            .map(|(_, stamp)| *stamp)
    }

    /// The notifications that make the server's documents match `desired`: closes
    /// first, then opens, then changes. Documents that already match produce none.
    pub fn reconcile(&mut self, desired: &[DesiredDocument]) -> Vec<Notification> {
        if !self.sync.open_close {
            return Vec::new();
        }
        let wanted: HashMap<String, &DesiredDocument> =
            desired.iter().map(|d| (uri::key(&d.uri), d)).collect();
        let mut closes = Vec::new();
        let mut opens = Vec::new();
        let mut changes = Vec::new();

        // A stable order keeps the traffic (and the tests) deterministic.
        let mut gone: Vec<String> = self
            .open
            .iter()
            .filter(|(key, open)| match wanted.get(*key) {
                None => true,
                // A new document under the same name, another language, or a
                // protocol version that cannot grow any further starts over.
                Some(d) => {
                    d.stamp.incarnation != open.sent.incarnation
                        || d.language_id != open.language_id
                        || open.version == i32::MAX
                }
            })
            .map(|(key, _)| key.clone())
            .collect();
        gone.sort();
        for key in gone {
            if let Some(open) = self.open.remove(&key) {
                closes.push(did_close(&open.uri));
            }
        }

        let mut keys: Vec<&String> = wanted.keys().collect();
        keys.sort();
        for key in keys {
            let document = wanted[key];
            match self.open.get_mut(key) {
                None => {
                    opens.push(did_open(document, 1));
                    let mut open = Open {
                        uri: document.uri.clone(),
                        language_id: document.language_id.clone(),
                        sent: document.stamp,
                        version: 1,
                        history: VecDeque::new(),
                    };
                    open.remember();
                    self.open.insert(key.clone(), open);
                }
                Some(open) if open.sent == document.stamp => {}
                Some(open) => {
                    open.version += 1;
                    open.sent = document.stamp;
                    open.remember();
                    if self.sync.change != SyncKind::None {
                        changes.push(Notification {
                            method: "textDocument/didChange",
                            params: json!({
                                "textDocument": { "uri": open.uri, "version": open.version },
                                "contentChanges": [{ "text": &*document.text }],
                            }),
                        });
                    }
                }
            }
        }
        closes.extend(opens);
        closes.extend(changes);
        closes
    }

    /// `textDocument/didSave` for an open document, when the server wants it.
    /// `text` is only sent if the server asked for it.
    pub fn saved(&self, uri: &str, text: &Arc<String>) -> Option<Notification> {
        let save = self.sync.save?;
        let open = self.open.get(&uri::key(uri))?;
        let mut params = json!({ "textDocument": { "uri": open.uri } });
        if save.include_text {
            params["text"] = Value::String(text.to_string());
        }
        Some(Notification {
            method: "textDocument/didSave",
            params,
        })
    }

    /// Forgets every document without sending anything, for a server that is gone.
    pub fn forget_all(&mut self) {
        self.open.clear();
    }

    /// Closes every document, for a server that is about to be stopped.
    pub fn close_all(&mut self) -> Vec<Notification> {
        let mut uris: Vec<String> = self.open.drain().map(|(_, open)| open.uri).collect();
        uris.sort();
        uris.iter().map(|u| did_close(u)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::language_service::capabilities::SaveSync;

    fn doc(path: &str, incarnation: u64, version: u64, text: &str) -> DesiredDocument {
        DesiredDocument {
            uri: format!("file:///w/{path}"),
            language_id: "rust".into(),
            stamp: Stamp {
                incarnation,
                version,
            },
            text: Arc::new(text.to_owned()),
        }
    }

    fn full() -> DocumentSync {
        DocumentSync::new(TextSync {
            open_close: true,
            change: SyncKind::Full,
            save: None,
        })
    }

    fn methods(notes: &[Notification]) -> Vec<&'static str> {
        notes.iter().map(|n| n.method).collect()
    }

    #[test]
    fn a_new_document_is_opened_once_at_version_one() {
        let mut sync = full();
        let docs = [doc("a.rs", 1, 0, "fn a() {}")];
        let notes = sync.reconcile(&docs);
        assert_eq!(methods(&notes), ["textDocument/didOpen"]);
        let td = &notes[0].params["textDocument"];
        assert_eq!(td["uri"], "file:///w/a.rs");
        assert_eq!(td["languageId"], "rust");
        assert_eq!(td["version"], 1);
        assert_eq!(td["text"], "fn a() {}");
        assert!(
            sync.reconcile(&docs).is_empty(),
            "nothing changed, nothing sent"
        );
        assert_eq!(sync.version_of("file:///w/a.rs"), Some(1));
    }

    #[test]
    fn an_edit_sends_the_whole_text_with_the_next_version() {
        let mut sync = full();
        sync.reconcile(&[doc("a.rs", 1, 0, "a")]);
        let notes = sync.reconcile(&[doc("a.rs", 1, 3, "abc")]);
        assert_eq!(methods(&notes), ["textDocument/didChange"]);
        assert_eq!(notes[0].params["textDocument"]["version"], 2);
        assert_eq!(
            notes[0].params["contentChanges"],
            json!([{ "text": "abc" }])
        );
        // Several buffer versions between two reconciles are one protocol version.
        let notes = sync.reconcile(&[doc("a.rs", 1, 9, "abcdef")]);
        assert_eq!(notes[0].params["textDocument"]["version"], 3);
    }

    #[test]
    fn protocol_versions_map_back_to_editor_states() {
        let mut sync = full();
        sync.reconcile(&[doc("a.rs", 1, 0, "a")]);
        sync.reconcile(&[doc("a.rs", 1, 5, "ab")]);
        let uri = "file:///w/a.rs";
        assert_eq!(
            sync.stamp_of(uri, 1),
            Some(Stamp {
                incarnation: 1,
                version: 0
            })
        );
        assert_eq!(
            sync.stamp_of(uri, 2),
            Some(Stamp {
                incarnation: 1,
                version: 5
            })
        );
        assert_eq!(sync.stamp_of(uri, 3), None);
        for v in 6..60 {
            sync.reconcile(&[doc("a.rs", 1, v, "x")]);
        }
        assert_eq!(sync.stamp_of(uri, 1), None, "old versions are forgotten");
        assert!(sync.stamp_of(uri, sync.version_of(uri).unwrap()).is_some());
    }

    #[test]
    fn a_document_that_goes_away_is_closed() {
        let mut sync = full();
        sync.reconcile(&[doc("a.rs", 1, 0, "a"), doc("b.rs", 2, 0, "b")]);
        let notes = sync.reconcile(&[doc("b.rs", 2, 0, "b")]);
        assert_eq!(methods(&notes), ["textDocument/didClose"]);
        assert_eq!(notes[0].params["textDocument"]["uri"], "file:///w/a.rs");
        assert!(!sync.is_open("file:///w/a.rs"));
        assert_eq!(sync.open_count(), 1);
    }

    #[test]
    fn closes_come_before_opens_and_changes_come_last() {
        let mut sync = full();
        sync.reconcile(&[doc("a.rs", 1, 0, "a"), doc("b.rs", 2, 0, "b")]);
        let notes = sync.reconcile(&[doc("b.rs", 2, 1, "bb"), doc("c.rs", 3, 0, "c")]);
        assert_eq!(
            methods(&notes),
            [
                "textDocument/didClose",
                "textDocument/didOpen",
                "textDocument/didChange"
            ]
        );
    }

    #[test]
    fn a_new_incarnation_or_language_reopens_from_version_one() {
        let mut sync = full();
        sync.reconcile(&[doc("a.rs", 1, 0, "a")]);
        sync.reconcile(&[doc("a.rs", 1, 1, "aa")]);
        let notes = sync.reconcile(&[doc("a.rs", 2, 0, "new life")]);
        assert_eq!(
            methods(&notes),
            ["textDocument/didClose", "textDocument/didOpen"]
        );
        assert_eq!(notes[1].params["textDocument"]["version"], 1);

        let mut other = doc("a.rs", 2, 0, "new life");
        other.language_id = "toml".into();
        let notes = sync.reconcile(&[other]);
        assert_eq!(
            methods(&notes),
            ["textDocument/didClose", "textDocument/didOpen"]
        );
        assert_eq!(notes[1].params["textDocument"]["languageId"], "toml");
    }

    #[test]
    fn two_spellings_of_one_uri_are_one_document() {
        if !cfg!(windows) {
            return;
        }
        let mut sync = full();
        let mut a = doc("a.rs", 1, 0, "a");
        a.uri = "file:///C:/w/a.rs".into();
        sync.reconcile(&[a.clone()]);
        a.uri = "file:///c%3A/W/A.rs".into();
        assert!(sync.reconcile(&[a]).is_empty());
    }

    #[test]
    fn a_server_that_wants_no_changes_still_gets_opens_and_closes() {
        let mut sync = DocumentSync::new(TextSync {
            open_close: true,
            change: SyncKind::None,
            save: None,
        });
        assert_eq!(
            methods(&sync.reconcile(&[doc("a.rs", 1, 0, "a")])),
            ["textDocument/didOpen"]
        );
        assert!(sync.reconcile(&[doc("a.rs", 1, 1, "ab")]).is_empty());
        assert!(
            sync.reconcile(&[doc("a.rs", 1, 1, "ab")]).is_empty(),
            "and does not retry"
        );
        assert_eq!(methods(&sync.reconcile(&[])), ["textDocument/didClose"]);
    }

    #[test]
    fn a_server_that_does_not_track_documents_gets_nothing() {
        let mut sync = DocumentSync::new(TextSync::default());
        assert!(sync.reconcile(&[doc("a.rs", 1, 0, "a")]).is_empty());
        assert_eq!(sync.open_count(), 0);
    }

    #[test]
    fn save_carries_the_text_only_when_asked_for() {
        let text = Arc::new("body".to_owned());
        let mut plain = DocumentSync::new(TextSync {
            open_close: true,
            change: SyncKind::Full,
            save: Some(SaveSync {
                include_text: false,
            }),
        });
        plain.reconcile(&[doc("a.rs", 1, 0, "body")]);
        let note = plain.saved("file:///w/a.rs", &text).unwrap();
        assert_eq!(note.method, "textDocument/didSave");
        assert!(note.params.get("text").is_none());

        let mut with_text = DocumentSync::new(TextSync {
            open_close: true,
            change: SyncKind::Full,
            save: Some(SaveSync { include_text: true }),
        });
        with_text.reconcile(&[doc("a.rs", 1, 0, "body")]);
        assert_eq!(
            with_text.saved("file:///w/a.rs", &text).unwrap().params["text"],
            "body"
        );

        assert!(with_text.saved("file:///w/unopened.rs", &text).is_none());
        assert!(
            full().saved("file:///w/a.rs", &text).is_none(),
            "no save support"
        );
    }

    #[test]
    fn closing_everything_and_forgetting_everything() {
        let mut sync = full();
        sync.reconcile(&[doc("b.rs", 2, 0, "b"), doc("a.rs", 1, 0, "a")]);
        let notes = sync.close_all();
        assert_eq!(
            methods(&notes),
            ["textDocument/didClose", "textDocument/didClose"]
        );
        assert_eq!(notes[0].params["textDocument"]["uri"], "file:///w/a.rs");
        assert_eq!(sync.open_count(), 0);

        sync.reconcile(&[doc("a.rs", 1, 0, "a")]);
        sync.forget_all();
        assert_eq!(
            methods(&sync.reconcile(&[doc("a.rs", 1, 0, "a")])),
            ["textDocument/didOpen"]
        );
    }

    #[test]
    fn the_largest_protocol_version_starts_over_instead_of_overflowing() {
        let mut sync = full();
        sync.reconcile(&[doc("a.rs", 1, 0, "a")]);
        sync.open
            .values_mut()
            .for_each(|open| open.version = i32::MAX);
        let notes = sync.reconcile(&[doc("a.rs", 1, 1, "b")]);
        assert_eq!(
            methods(&notes),
            ["textDocument/didClose", "textDocument/didOpen"]
        );
        assert_eq!(notes[1].params["textDocument"]["version"], 1);
    }
}
