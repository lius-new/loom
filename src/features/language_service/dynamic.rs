//! Per-connection registration IDs and effective static + dynamic providers.
use super::{
    providers::Providers,
    sync::DesiredDocument,
    watch::{Pattern, Watch},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub const WATCH_METHOD: &str = "workspace/didChangeWatchedFiles";

/// Keep this table and the advertised dynamicRegistration flags in step.
pub const PROVIDERS: &[(&str, &str)] = &[
    ("textDocument/hover", "hoverProvider"),
    ("textDocument/completion", "completionProvider"),
    ("textDocument/signatureHelp", "signatureHelpProvider"),
    ("textDocument/definition", "definitionProvider"),
    ("textDocument/declaration", "declarationProvider"),
    ("textDocument/typeDefinition", "typeDefinitionProvider"),
    ("textDocument/implementation", "implementationProvider"),
    ("textDocument/references", "referencesProvider"),
    (
        "textDocument/documentHighlight",
        "documentHighlightProvider",
    ),
    ("textDocument/documentSymbol", "documentSymbolProvider"),
    ("textDocument/rename", "renameProvider"),
    ("textDocument/codeAction", "codeActionProvider"),
    ("textDocument/formatting", "documentFormattingProvider"),
    (
        "textDocument/rangeFormatting",
        "documentRangeFormattingProvider",
    ),
    (
        "textDocument/onTypeFormatting",
        "documentOnTypeFormattingProvider",
    ),
    ("textDocument/inlayHint", "inlayHintProvider"),
    ("textDocument/selectionRange", "selectionRangeProvider"),
    ("textDocument/prepareCallHierarchy", "callHierarchyProvider"),
    ("textDocument/prepareTypeHierarchy", "typeHierarchyProvider"),
    ("textDocument/codeLens", "codeLensProvider"),
    ("workspace/symbol", "workspaceSymbolProvider"),
    ("workspace/executeCommand", "executeCommandProvider"),
];

#[derive(Clone, Debug)]
struct Filter {
    language: Option<String>,
    scheme: Option<String>,
    pattern: Option<Pattern>,
}

impl Filter {
    fn matches(&self, document: &DesiredDocument) -> bool {
        self.language
            .as_ref()
            .is_none_or(|language| language == &document.language_id)
            && self.scheme.as_deref().is_none_or(|scheme| {
                document
                    .uri
                    .split_once(':')
                    .is_some_and(|(found, _)| found == scheme)
            })
            && self.pattern.as_ref().is_none_or(|pattern| {
                super::uri::file_path(&document.uri).is_some_and(|path| pattern.matches(&path))
            })
    }
}

#[derive(Clone, Debug)]
struct Registration {
    method: String,
    providers: Providers,
    selector: Option<Vec<Filter>>,
    watches: Vec<Watch>,
    watch_options: Option<Value>,
}

#[derive(Clone, Debug, Default)]
pub struct Registry {
    entries: BTreeMap<String, Registration>,
}

impl Registry {
    /// A transaction: failure leaves the caller's registry intact.
    pub fn register(&self, params: &Value, root: &Path) -> Result<Self, String> {
        let registrations = params
            .get("registrations")
            .and_then(Value::as_array)
            .ok_or("missing registrations array")?;
        if registrations.len() > 128 {
            return Err("at most 128 dynamic registrations per instance".into());
        }
        let mut next = self.clone();
        let mut seen = BTreeSet::new();
        for registration in registrations {
            let id = registration
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .ok_or("missing registration id")?;
            if !seen.insert(id) {
                return Err(format!("duplicate registration id: {id}"));
            }
            let method = registration
                .get("method")
                .and_then(Value::as_str)
                .ok_or("missing registration method")?;
            if next
                .entries
                .get(id)
                .is_some_and(|previous| previous.method != method)
            {
                return Err(format!("registration method mismatch: {id}"));
            }
            let options = registration
                .get("registerOptions")
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or(json!({}));
            if !options.is_object() {
                return Err("registerOptions must be an object".into());
            }
            let (providers, watches) = if method == WATCH_METHOD {
                (Providers::default(), Watch::parse(&options, root)?)
            } else if let Some((_, property)) = PROVIDERS.iter().find(|(name, _)| *name == method) {
                if method == "textDocument/onTypeFormatting"
                    && options
                        .get("firstTriggerCharacter")
                        .and_then(Value::as_str)
                        .is_none()
                {
                    return Err("onTypeFormatting requires firstTriggerCharacter".into());
                }
                if method == "workspace/executeCommand"
                    && options.get("commands").and_then(Value::as_array).is_none()
                {
                    return Err("executeCommand requires commands".into());
                }
                (Providers::parse(&json!({*property:options})), Vec::new())
            } else {
                return Err(format!("dynamic registration was not offered for {method}"));
            };
            let selector = match options.get("documentSelector") {
                None | Some(Value::Null) => None,
                Some(Value::Array(filters)) if filters.len() <= 128 => Some(
                    filters
                        .iter()
                        .map(|filter| {
                            let object = filter
                                .as_object()
                                .ok_or("document filter must be an object")?;
                            if object.keys().any(|key| {
                                !["language", "scheme", "pattern"].contains(&key.as_str())
                            }) {
                                return Err("unsupported document filter".to_owned());
                            }
                            let text = |key| -> Result<Option<String>, String> {
                                object
                                    .get(key)
                                    .map(|v| {
                                        v.as_str()
                                            .map(str::to_owned)
                                            .ok_or(format!("{key} must be a string"))
                                    })
                                    .transpose()
                            };
                            Ok(Filter {
                                language: text("language")?,
                                scheme: text("scheme")?,
                                pattern: object
                                    .get("pattern")
                                    .map(|p| Pattern::parse(p, root))
                                    .transpose()?,
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                ),
                _ => {
                    return Err(
                        "documentSelector must be null or an array of at most 128 filters".into(),
                    );
                }
            };
            next.entries.insert(
                id.to_owned(),
                Registration {
                    method: method.to_owned(),
                    providers,
                    selector,
                    watches,
                    watch_options: (method == WATCH_METHOD)
                        .then(|| options.get("watchers").cloned().unwrap_or(Value::Null)),
                },
            );
        }
        if next.entries.len() > 128 || next.watches().len() > 256 {
            return Err("at most 256 watchers per instance".into());
        }
        Ok(next)
    }

    pub fn unregister(&self, params: &Value) -> Result<Self, String> {
        // The misspelled field is the normative LSP wire format.
        let registrations = params
            .get("unregisterations")
            .or_else(|| params.get("unregistrations"))
            .and_then(Value::as_array)
            .ok_or("missing unregisterations array")?;
        let mut next = self.clone();
        for registration in registrations {
            let id = registration
                .get("id")
                .and_then(Value::as_str)
                .ok_or("missing unregistration id")?;
            let method = registration
                .get("method")
                .and_then(Value::as_str)
                .ok_or("missing unregistration method")?;
            let entry = next
                .entries
                .get(id)
                .ok_or(format!("unknown registration id: {id}"))?;
            if entry.method != method {
                return Err(format!("registration method mismatch: {id}"));
            }
            next.entries.remove(id);
        }
        Ok(next)
    }

    pub fn effective(
        &self,
        static_providers: &Providers,
        document: Option<&DesiredDocument>,
    ) -> Providers {
        let mut providers = static_providers.clone();
        for registration in self.entries.values() {
            if registration.selector.as_ref().is_none_or(|filters| {
                document
                    .is_some_and(|document| filters.iter().any(|filter| filter.matches(document)))
            }) {
                providers.merge(&registration.providers);
            }
        }
        providers
    }

    pub fn watches(&self) -> Vec<Watch> {
        self.entries
            .values()
            .flat_map(|entry| entry.watches.iter().cloned())
            .collect()
    }

    pub fn ids(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|(id, entry)| format!("{id}: {}", entry.method))
            .collect()
    }
    pub fn watch_signature(&self) -> Vec<(&str, &Value)> {
        self.entries
            .iter()
            .filter_map(|(id, entry)| {
                entry
                    .watch_options
                    .as_ref()
                    .map(|options| (id.as_str(), options))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{sync::Stamp, uri};
    use super::*;
    use std::sync::Arc;

    #[test]
    fn every_advertised_dynamic_provider_can_be_registered_and_removed() {
        let root = std::env::temp_dir();
        let capabilities = super::super::capabilities::client_capabilities(&[], &[]);
        for (method, _) in PROVIDERS {
            let (scope, name) = method.split_once('/').unwrap();
            let name = match name {
                "prepareCallHierarchy" => "callHierarchy",
                "prepareTypeHierarchy" => "typeHierarchy",
                name => name,
            };
            assert_eq!(
                capabilities[scope][name]["dynamicRegistration"], true,
                "{method}"
            );
            let options = json!({"commands":["test.command"],"firstTriggerCharacter":";"});
            let registry=Registry::default().register(&json!({"registrations":[{"id":"test","method":method,"registerOptions":options}]}),&root).unwrap();
            assert_ne!(
                registry.effective(&Providers::default(), None),
                Providers::default(),
                "{method}"
            );
            let registry = registry
                .unregister(&json!({"unregisterations":[{"id":"test","method":method}]}))
                .unwrap();
            assert_eq!(
                registry.effective(&Providers::default(), None),
                Providers::default()
            );
        }
        for name in ["semanticTokens", "diagnostic", "synchronization"] {
            assert_eq!(
                capabilities["textDocument"][name]["dynamicRegistration"],
                false
            );
        }
    }

    #[test]
    fn atomic_ids_selectors_and_static_union() {
        let root = std::env::temp_dir();
        let registered = json!({"registrations":[{"id":"h", "method":"textDocument/hover", "registerOptions":{"documentSelector":[{"language":"rust", "scheme":"file", "pattern":"**/*.rs"}]}}, {"id":"c", "method":"textDocument/completion", "registerOptions":{"triggerCharacters":["."]}}]});
        let empty = Registry::default();
        let registry = empty.register(&registered, &root).unwrap();
        assert!(empty.ids().is_empty());
        let doc = DesiredDocument {
            uri: uri::file_uri(&root.join("a.rs")).unwrap(),
            language_id: "rust".into(),
            stamp: Stamp {
                incarnation: 1,
                version: 0,
            },
            text: Arc::new(String::new()),
        };
        assert!(registry.effective(&Providers::default(), Some(&doc)).hover);
        assert!(
            !registry
                .effective(
                    &Providers::default(),
                    Some(&DesiredDocument {
                        language_id: "other".into(),
                        ..doc.clone()
                    })
                )
                .hover
        );
        let static_providers = Providers::parse(
            &json!({"definitionProvider":true, "completionProvider":{"resolveProvider":true, "triggerCharacters":[":"]}}),
        );
        let effective = registry.effective(&static_providers, Some(&doc));
        assert!(effective.definition && effective.completion.as_ref().unwrap().resolve);
        assert_eq!(effective.completion.unwrap().trigger_characters, [":", "."]);
        assert!(registry.register(&registered, &root).is_ok());
        assert!(registry.register(&json!({"registrations":[{"id":"h","method":"textDocument/hover"},{"id":"h","method":"textDocument/hover"}]}),&root).is_err());
        assert!(registry.register(&json!({"registrations":[{"id":"z", "method":"textDocument/hover"}, {"id":"bad","method":"textDocument/didOpen"}]}), &root).is_err());
        let remaining = registry
            .unregister(&json!({"unregisterations":[{"id":"h","method":"textDocument/hover"}]}))
            .unwrap();
        assert!(!remaining.effective(&static_providers, Some(&doc)).hover);
        assert!(
            remaining
                .effective(&static_providers, Some(&doc))
                .definition
        );
        assert!(
            registry
                .unregister(
                    &json!({"unregisterations":[{"id":"h","method":"textDocument/definition"}]})
                )
                .is_err()
        );
    }
}
