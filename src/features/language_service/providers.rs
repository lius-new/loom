//! Which interactive features a server offers, read from the `capabilities` of
//! its initialize result. A provider is `true`, an options object, or absent;
//! anything else counts as absent, so a server that sends nonsense for one
//! feature loses that feature and nothing more.
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompletionOptions {
    pub trigger_characters: Vec<String>,
    /// `completionItem/resolve` fills in the documentation and edits on demand.
    pub resolve: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignatureOptions {
    pub trigger_characters: Vec<String>,
    pub retrigger_characters: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeActionOptions {
    pub resolve: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OnTypeFormatting {
    pub first: String,
    pub more: Vec<String>,
}

/// What the server can answer. Every field is false or `None` for a server that
/// does not say.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Providers {
    pub hover: bool,
    pub definition: bool,
    pub declaration: bool,
    pub type_definition: bool,
    pub implementation: bool,
    pub references: bool,
    pub document_highlight: bool,
    pub document_symbol: bool,
    pub workspace_symbol: bool,
    pub completion: Option<CompletionOptions>,
    pub signature_help: Option<SignatureOptions>,
    /// `Some(true)` when the server also answers `prepareRename`.
    pub rename: Option<bool>,
    pub code_action: Option<CodeActionOptions>,
    pub formatting: bool,
    pub range_formatting: bool,
    pub on_type_formatting: Option<OnTypeFormatting>,
    pub inlay_hint: bool,
    pub folding_range: bool,
    pub selection_range: bool,
    pub call_hierarchy: bool,
    pub type_hierarchy: bool,
    pub code_lens: bool,
    pub execute_command: Vec<String>,
    /// The server answers `textDocument/diagnostic` (pull diagnostics).
    pub pull_diagnostics: bool,
}

impl Providers {
    pub fn parse(capabilities: &Value) -> Self {
        let get = |key: &str| capabilities.get(key);
        let on = |key: &str| enabled(get(key));
        Self {
            hover: on("hoverProvider"),
            definition: on("definitionProvider"),
            declaration: on("declarationProvider"),
            type_definition: on("typeDefinitionProvider"),
            implementation: on("implementationProvider"),
            references: on("referencesProvider"),
            document_highlight: on("documentHighlightProvider"),
            document_symbol: on("documentSymbolProvider"),
            workspace_symbol: on("workspaceSymbolProvider"),
            completion: get("completionProvider")
                .and_then(Value::as_object)
                .map(|options| CompletionOptions {
                    trigger_characters: strings(options.get("triggerCharacters")),
                    resolve: flag(options.get("resolveProvider")),
                }),
            signature_help: get("signatureHelpProvider").and_then(Value::as_object).map(
                |options| SignatureOptions {
                    trigger_characters: strings(options.get("triggerCharacters")),
                    retrigger_characters: strings(options.get("retriggerCharacters")),
                },
            ),
            rename: match get("renameProvider") {
                Some(Value::Bool(true)) => Some(false),
                Some(Value::Object(options)) => Some(flag(options.get("prepareProvider"))),
                _ => None,
            },
            code_action: match get("codeActionProvider") {
                Some(Value::Bool(true)) => Some(CodeActionOptions::default()),
                Some(Value::Object(options)) => Some(CodeActionOptions {
                    resolve: flag(options.get("resolveProvider")),
                }),
                _ => None,
            },
            formatting: on("documentFormattingProvider"),
            range_formatting: on("documentRangeFormattingProvider"),
            on_type_formatting: get("documentOnTypeFormattingProvider")
                .and_then(Value::as_object)
                .and_then(|options| {
                    Some(OnTypeFormatting {
                        first: options.get("firstTriggerCharacter")?.as_str()?.to_owned(),
                        more: strings(options.get("moreTriggerCharacter")),
                    })
                }),
            inlay_hint: on("inlayHintProvider"),
            folding_range: on("foldingRangeProvider"),
            selection_range: on("selectionRangeProvider"),
            call_hierarchy: on("callHierarchyProvider"),
            type_hierarchy: on("typeHierarchyProvider"),
            code_lens: get("codeLensProvider").is_some_and(Value::is_object),
            execute_command: get("executeCommandProvider")
                .map(|options| strings(options.get("commands")))
                .unwrap_or_default(),
            pull_diagnostics: get("diagnosticProvider").is_some_and(Value::is_object),
        }
    }
}

fn enabled(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Bool(true) | Value::Object(_)))
}

fn flag(value: Option<&Value>) -> bool {
    matches!(value, Some(Value::Bool(true)))
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_server_that_says_nothing_offers_nothing() {
        assert_eq!(Providers::parse(&json!({})), Providers::default());
    }

    #[test]
    fn providers_are_true_or_an_options_object() {
        let p = Providers::parse(&json!({
            "hoverProvider": true,
            "definitionProvider": { "workDoneProgress": true },
            "referencesProvider": false,
            "declarationProvider": "nonsense",
            "renameProvider": { "prepareProvider": true },
            "codeActionProvider": { "resolveProvider": true },
            "completionProvider": { "triggerCharacters": [".", ":"], "resolveProvider": true },
            "signatureHelpProvider": { "triggerCharacters": ["("], "retriggerCharacters": [","] },
            "documentOnTypeFormattingProvider": { "firstTriggerCharacter": "}", "moreTriggerCharacter": [";"] },
            "executeCommandProvider": { "commands": ["a.b", "c.d"] },
            "diagnosticProvider": { "interFileDependencies": false, "workspaceDiagnostics": false },
            "codeLensProvider": { "resolveProvider": false },
        }));
        assert!(p.hover && p.definition);
        assert!(!p.references && !p.declaration);
        assert_eq!(p.rename, Some(true));
        assert_eq!(p.code_action, Some(CodeActionOptions { resolve: true }));
        let completion = p.completion.unwrap();
        assert_eq!(completion.trigger_characters, [".", ":"]);
        assert!(completion.resolve);
        assert_eq!(p.signature_help.unwrap().retrigger_characters, [","]);
        assert_eq!(p.on_type_formatting.unwrap().first, "}");
        assert_eq!(p.execute_command, ["a.b", "c.d"]);
        assert!(p.pull_diagnostics && p.code_lens);
    }

    #[test]
    fn rename_without_prepare_is_still_rename() {
        assert_eq!(
            Providers::parse(&json!({ "renameProvider": true })).rename,
            Some(false)
        );
        assert_eq!(
            Providers::parse(&json!({ "renameProvider": false })).rename,
            None
        );
    }
}
