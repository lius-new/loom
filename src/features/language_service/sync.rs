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
/// How many of the most recent texts are kept (shared with the editor, so this
/// costs a pointer each unless the text has since been edited).
const TEXT_HISTORY: usize = 8;

struct Open {
    uri: String,
    language_id: String,
    sent: Stamp,
    version: i32,
    history: VecDeque<(i32, Stamp)>,
    texts: VecDeque<(i32, Arc<String>)>,
}

impl Open {
    fn remember(&mut self, text: &Arc<String>) {
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back((self.version, self.sent));
        if self.texts.len() == TEXT_HISTORY {
            self.texts.pop_front();
        }
        self.texts.push_back((self.version, text.clone()));
    }
}

pub struct DocumentSync {
    sync: TextSync,
    open: HashMap<String, Open>,
    // One counter per connection avoids both version reuse on reopen and an
    // unbounded table of closed URI tombstones. Versions may skip for a URI.
    last_version: i32,
}

/// The connection must be replaced before another document version is sent.
#[derive(Debug, PartialEq, Eq)]
pub struct VersionExhausted;

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
            last_version: 0,
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

    /// The text that was sent as protocol `version` of `uri`, if still remembered.
    pub fn text_of(&self, uri: &str, version: i32) -> Option<Arc<String>> {
        let open = self.open.get(&uri::key(uri))?;
        open.texts
            .iter()
            .find(|(v, _)| *v == version)
            .map(|(_, text)| text.clone())
    }

    /// The notifications that make the server's documents match `desired`: closes
    /// first, then opens, then changes. Documents that already match produce none.
    /// Exhaustion leaves the entire batch unapplied; the caller must start a new
    /// connection, whose instance generation rejects messages from the old one.
    pub fn reconcile(
        &mut self,
        desired: &[DesiredDocument],
    ) -> Result<Vec<Notification>, VersionExhausted> {
        if !self.sync.open_close {
            return Ok(Vec::new());
        }
        let wanted: HashMap<String, &DesiredDocument> =
            desired.iter().map(|d| (uri::key(&d.uri), d)).collect();
        let needed = wanted
            .iter()
            .filter(|(key, document)| {
                self.open.get(*key).is_none_or(|open| {
                    open.sent != document.stamp || open.language_id != document.language_id
                })
            })
            .count();
        if needed > (i32::MAX - self.last_version) as usize {
            return Err(VersionExhausted);
        }
        let mut closes = Vec::new();
        let mut opens = Vec::new();
        let mut changes = Vec::new();

        // A stable order keeps the traffic (and the tests) deterministic.
        let mut gone: Vec<String> = self
            .open
            .iter()
            .filter(|(key, open)| match wanted.get(*key) {
                None => true,
                // A new document under the same name or another language must
                // discard the previous lifetime's version and text histories.
                Some(d) => {
                    d.stamp.incarnation != open.sent.incarnation
                        || d.language_id != open.language_id
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
                    self.last_version += 1;
                    opens.push(did_open(document, self.last_version));
                    let mut open = Open {
                        uri: document.uri.clone(),
                        language_id: document.language_id.clone(),
                        sent: document.stamp,
                        version: self.last_version,
                        history: VecDeque::new(),
                        texts: VecDeque::new(),
                    };
                    open.remember(&document.text);
                    self.open.insert(key.clone(), open);
                }
                Some(open) if open.sent == document.stamp => {}
                Some(open) => {
                    self.last_version += 1;
                    open.version = self.last_version;
                    open.sent = document.stamp;
                    open.remember(&document.text);
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
        Ok(closes)
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

    /// Forgets document mirrors without sending anything. Only a new connection
    /// (a new DocumentSync) may reset the protocol version counter.
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
        let notes = sync.reconcile(&docs).unwrap();
        assert_eq!(methods(&notes), ["textDocument/didOpen"]);
        let td = &notes[0].params["textDocument"];
        assert_eq!(td["uri"], "file:///w/a.rs");
        assert_eq!(td["languageId"], "rust");
        assert_eq!(td["version"], 1);
        assert_eq!(td["text"], "fn a() {}");
        assert!(
            sync.reconcile(&docs).unwrap().is_empty(),
            "nothing changed, nothing sent"
        );
        assert_eq!(sync.version_of("file:///w/a.rs"), Some(1));
    }

    #[test]
    fn an_edit_sends_the_whole_text_with_the_next_version() {
        let mut sync = full();
        sync.reconcile(&[doc("a.rs", 1, 0, "a")]).unwrap();
        let notes = sync.reconcile(&[doc("a.rs", 1, 3, "abc")]).unwrap();
        assert_eq!(methods(&notes), ["textDocument/didChange"]);
        assert_eq!(notes[0].params["textDocument"]["version"], 2);
        assert_eq!(
            notes[0].params["contentChanges"],
            json!([{ "text": "abc" }])
        );
        // Several buffer versions between two reconciles are one protocol version.
        let notes = sync.reconcile(&[doc("a.rs", 1, 9, "abcdef")]).unwrap();
        assert_eq!(notes[0].params["textDocument"]["version"], 3);
    }

    #[test]
    fn protocol_versions_map_back_to_editor_states() {
        let mut sync = full();
        sync.reconcile(&[doc("a.rs", 1, 0, "a")]).unwrap();
        sync.reconcile(&[doc("a.rs", 1, 5, "ab")]).unwrap();
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
            sync.reconcile(&[doc("a.rs", 1, v, "x")]).unwrap();
        }
        assert_eq!(sync.stamp_of(uri, 1), None, "old versions are forgotten");
        assert!(sync.stamp_of(uri, sync.version_of(uri).unwrap()).is_some());
    }

    #[test]
    fn a_document_that_goes_away_is_closed() {
        let mut sync = full();
        sync.reconcile(&[doc("a.rs", 1, 0, "a"), doc("b.rs", 2, 0, "b")])
            .unwrap();
        let notes = sync.reconcile(&[doc("b.rs", 2, 0, "b")]).unwrap();
        assert_eq!(methods(&notes), ["textDocument/didClose"]);
        assert_eq!(notes[0].params["textDocument"]["uri"], "file:///w/a.rs");
        assert!(!sync.is_open("file:///w/a.rs"));
        assert_eq!(sync.open_count(), 1);
    }

    #[test]
    fn closes_come_before_opens_and_changes_come_last() {
        let mut sync = full();
        sync.reconcile(&[doc("a.rs", 1, 0, "a"), doc("b.rs", 2, 0, "b")])
            .unwrap();
        let notes = sync
            .reconcile(&[doc("b.rs", 2, 1, "bb"), doc("c.rs", 3, 0, "c")])
            .unwrap();
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
    fn a_new_incarnation_or_language_reopens_without_reusing_versions() {
        let mut sync = full();
        sync.reconcile(&[doc("a.rs", 1, 0, "a")]).unwrap();
        sync.reconcile(&[doc("a.rs", 1, 1, "aa")]).unwrap();
        let notes = sync.reconcile(&[doc("a.rs", 2, 0, "new life")]).unwrap();
        assert_eq!(
            methods(&notes),
            ["textDocument/didClose", "textDocument/didOpen"]
        );
        assert_eq!(notes[1].params["textDocument"]["version"], 3);
        assert_eq!(sync.stamp_of("file:///w/a.rs", 1), None);
        assert_eq!(sync.text_of("file:///w/a.rs", 2), None);

        let mut other = doc("a.rs", 2, 0, "new life");
        other.language_id = "toml".into();
        let notes = sync.reconcile(&[other]).unwrap();
        assert_eq!(
            methods(&notes),
            ["textDocument/didClose", "textDocument/didOpen"]
        );
        assert_eq!(notes[1].params["textDocument"]["languageId"], "toml");
        assert_eq!(notes[1].params["textDocument"]["version"], 4);
        assert_eq!(sync.stamp_of("file:///w/a.rs", 3), None);
        assert_eq!(sync.text_of("file:///w/a.rs", 3), None);
    }

    #[test]
    fn a_closed_uri_never_reuses_versions_or_retains_old_text() {
        let mut sync = full();
        let old = doc("a.rs", 1, 0, "a");
        sync.reconcile(&[old]).unwrap();
        sync.reconcile(&[doc("a.rs", 1, 1, "aa")]).unwrap();
        sync.reconcile(&[]).unwrap();
        let reopened = doc("a.rs", 2, 0, "aa");
        let notes = sync.reconcile(std::slice::from_ref(&reopened)).unwrap();
        assert_eq!(notes[0].params["textDocument"]["version"], 3);
        for version in [1, 2] {
            assert_eq!(sync.stamp_of(&reopened.uri, version), None);
            assert_eq!(sync.text_of(&reopened.uri, version), None);
        }
        assert_eq!(sync.stamp_of(&reopened.uri, 3), Some(reopened.stamp));
        assert_eq!(sync.text_of(&reopened.uri, 3), Some(reopened.text));
    }

    #[test]
    fn two_spellings_of_one_uri_are_one_document() {
        if !cfg!(windows) {
            return;
        }
        let mut sync = full();
        let mut a = doc("a.rs", 1, 0, "a");
        a.uri = "file:///C:/w/a.rs".into();
        sync.reconcile(&[a.clone()]).unwrap();
        a.uri = "file:///c%3A/W/A.rs".into();
        assert!(sync.reconcile(&[a]).unwrap().is_empty());
    }

    #[test]
    fn a_server_that_wants_no_changes_still_gets_opens_and_closes() {
        let mut sync = DocumentSync::new(TextSync {
            open_close: true,
            change: SyncKind::None,
            save: None,
        });
        assert_eq!(
            methods(&sync.reconcile(&[doc("a.rs", 1, 0, "a")]).unwrap()),
            ["textDocument/didOpen"]
        );
        assert!(
            sync.reconcile(&[doc("a.rs", 1, 1, "ab")])
                .unwrap()
                .is_empty()
        );
        assert!(
            sync.reconcile(&[doc("a.rs", 1, 1, "ab")])
                .unwrap()
                .is_empty(),
            "and does not retry"
        );
        assert_eq!(
            methods(&sync.reconcile(&[]).unwrap()),
            ["textDocument/didClose"]
        );
    }

    #[test]
    fn a_server_that_does_not_track_documents_gets_nothing() {
        let mut sync = DocumentSync::new(TextSync::default());
        assert!(
            sync.reconcile(&[doc("a.rs", 1, 0, "a")])
                .unwrap()
                .is_empty()
        );
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
        plain.reconcile(&[doc("a.rs", 1, 0, "body")]).unwrap();
        let note = plain.saved("file:///w/a.rs", &text).unwrap();
        assert_eq!(note.method, "textDocument/didSave");
        assert!(note.params.get("text").is_none());

        let mut with_text = DocumentSync::new(TextSync {
            open_close: true,
            change: SyncKind::Full,
            save: Some(SaveSync { include_text: true }),
        });
        with_text.reconcile(&[doc("a.rs", 1, 0, "body")]).unwrap();
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
        sync.reconcile(&[doc("b.rs", 2, 0, "b"), doc("a.rs", 1, 0, "a")])
            .unwrap();
        let notes = sync.close_all();
        assert_eq!(
            methods(&notes),
            ["textDocument/didClose", "textDocument/didClose"]
        );
        assert_eq!(notes[0].params["textDocument"]["uri"], "file:///w/a.rs");
        assert_eq!(sync.open_count(), 0);

        let notes = sync.reconcile(&[doc("a.rs", 1, 0, "a")]).unwrap();
        assert_eq!(notes[0].params["textDocument"]["version"], 3);
        sync.forget_all();
        let notes = sync.reconcile(&[doc("a.rs", 1, 0, "a")]).unwrap();
        assert_eq!(methods(&notes), ["textDocument/didOpen"]);
        assert_eq!(notes[0].params["textDocument"]["version"], 4);
    }

    #[test]
    fn exhausted_versions_require_a_new_connection_even_after_closing() {
        let mut sync = full();
        sync.reconcile(&[doc("a.rs", 1, 0, "a")]).unwrap();
        sync.last_version = i32::MAX - 1;
        let notes = sync.reconcile(&[doc("a.rs", 1, 1, "b")]).unwrap();
        assert_eq!(notes[0].params["textDocument"]["version"], i32::MAX);
        assert!(
            sync.reconcile(&[doc("a.rs", 1, 1, "b")])
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            sync.reconcile(&[doc("a.rs", 1, 2, "c")]),
            Err(VersionExhausted)
        );
        assert_eq!(sync.version_of("file:///w/a.rs"), Some(i32::MAX));
        assert_eq!(
            methods(&sync.reconcile(&[]).unwrap()),
            ["textDocument/didClose"]
        );
        assert_eq!(
            sync.reconcile(&[doc("a.rs", 2, 0, "c")]),
            Err(VersionExhausted)
        );
        let notes = full().reconcile(&[doc("a.rs", 2, 0, "c")]).unwrap();
        assert_eq!(
            notes[0].params["textDocument"]["version"], 1,
            "only a new connection resets versions"
        );
    }

    #[test]
    fn exhaustion_leaves_the_whole_batch_unapplied() {
        let mut sync = full();
        let old = doc("a.rs", 1, 0, "old");
        sync.reconcile(std::slice::from_ref(&old)).unwrap();
        sync.last_version = i32::MAX - 1;
        assert_eq!(
            sync.reconcile(&[doc("b.rs", 2, 0, "b"), doc("c.rs", 3, 0, "c")]),
            Err(VersionExhausted)
        );
        assert_eq!(sync.open_count(), 1, "no partial close or open");
        assert_eq!(sync.stamp_of(&old.uri, 1), Some(old.stamp));
        assert_eq!(sync.text_of(&old.uri, 1), Some(old.text));
        let notes = sync.reconcile(&[doc("b.rs", 2, 0, "b")]).unwrap();
        assert_eq!(
            methods(&notes),
            ["textDocument/didClose", "textDocument/didOpen"]
        );
        assert_eq!(notes[1].params["textDocument"]["version"], i32::MAX);
    }
}
