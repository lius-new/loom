//! The interactive requests Loom makes of a server: which method each one is,
//! whether a server can answer it, what its parameters look like, and how the
//! answers of several servers combine.
use super::position::Position;
use super::providers::Providers;
use serde_json::{Map, Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Feature {
    Hover,
    Definition,
    Declaration,
    TypeDefinition,
    Implementation,
    References,
    DocumentHighlight,
    DocumentSymbol,
    WorkspaceSymbol,
    Completion,
    CompletionResolve,
    SignatureHelp,
    PrepareRename,
    Rename,
    CodeAction,
    CodeActionResolve,
    Formatting,
    RangeFormatting,
    OnTypeFormatting,
    InlayHint,
    FoldingRange,
    SelectionRange,
    PrepareCallHierarchy,
    IncomingCalls,
    OutgoingCalls,
    PrepareTypeHierarchy,
    Supertypes,
    Subtypes,
    CodeLens,
    CodeLensResolve,
    ExecuteCommand,
}

/// How the answers of the servers that can reply are combined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Merge {
    /// Every server's answer counts, in listing order.
    All,
    /// The first server (in listing order) with a non-empty answer.
    First,
    /// Only the first server that can answer is asked.
    Primary,
    /// The request is for the one server that produced the item it is about.
    Target,
}

impl Feature {
    pub fn method(self) -> &'static str {
        match self {
            Self::Hover => "textDocument/hover",
            Self::Definition => "textDocument/definition",
            Self::Declaration => "textDocument/declaration",
            Self::TypeDefinition => "textDocument/typeDefinition",
            Self::Implementation => "textDocument/implementation",
            Self::References => "textDocument/references",
            Self::DocumentHighlight => "textDocument/documentHighlight",
            Self::DocumentSymbol => "textDocument/documentSymbol",
            Self::WorkspaceSymbol => "workspace/symbol",
            Self::Completion => "textDocument/completion",
            Self::CompletionResolve => "completionItem/resolve",
            Self::SignatureHelp => "textDocument/signatureHelp",
            Self::PrepareRename => "textDocument/prepareRename",
            Self::Rename => "textDocument/rename",
            Self::CodeAction => "textDocument/codeAction",
            Self::CodeActionResolve => "codeAction/resolve",
            Self::Formatting => "textDocument/formatting",
            Self::RangeFormatting => "textDocument/rangeFormatting",
            Self::OnTypeFormatting => "textDocument/onTypeFormatting",
            Self::InlayHint => "textDocument/inlayHint",
            Self::FoldingRange => "textDocument/foldingRange",
            Self::SelectionRange => "textDocument/selectionRange",
            Self::PrepareCallHierarchy => "textDocument/prepareCallHierarchy",
            Self::IncomingCalls => "callHierarchy/incomingCalls",
            Self::OutgoingCalls => "callHierarchy/outgoingCalls",
            Self::PrepareTypeHierarchy => "textDocument/prepareTypeHierarchy",
            Self::Supertypes => "typeHierarchy/supertypes",
            Self::Subtypes => "typeHierarchy/subtypes",
            Self::CodeLens => "textDocument/codeLens",
            Self::CodeLensResolve => "codeLens/resolve",
            Self::ExecuteCommand => "workspace/executeCommand",
        }
    }

    /// Whether a server with these providers can answer.
    pub fn supported(self, providers: &Providers) -> bool {
        match self {
            Self::Hover => providers.hover,
            Self::Definition => providers.definition,
            Self::Declaration => providers.declaration,
            Self::TypeDefinition => providers.type_definition,
            Self::Implementation => providers.implementation,
            Self::References => providers.references,
            Self::DocumentHighlight => providers.document_highlight,
            Self::DocumentSymbol => providers.document_symbol,
            Self::WorkspaceSymbol => providers.workspace_symbol,
            Self::Completion => providers.completion.is_some(),
            Self::CompletionResolve => providers
                .completion
                .as_ref()
                .is_some_and(|options| options.resolve),
            Self::SignatureHelp => providers.signature_help.is_some(),
            Self::PrepareRename => providers.rename == Some(true),
            Self::Rename => providers.rename.is_some(),
            Self::CodeAction => providers.code_action.is_some(),
            Self::CodeActionResolve => providers
                .code_action
                .as_ref()
                .is_some_and(|options| options.resolve),
            Self::Formatting => providers.formatting,
            Self::RangeFormatting => providers.range_formatting,
            Self::OnTypeFormatting => providers.on_type_formatting.is_some(),
            Self::InlayHint => providers.inlay_hint,
            Self::FoldingRange => providers.folding_range,
            Self::SelectionRange => providers.selection_range,
            Self::PrepareCallHierarchy | Self::IncomingCalls | Self::OutgoingCalls => {
                providers.call_hierarchy
            }
            Self::PrepareTypeHierarchy | Self::Supertypes | Self::Subtypes => {
                providers.type_hierarchy
            }
            Self::CodeLens | Self::CodeLensResolve => providers.code_lens,
            Self::ExecuteCommand => !providers.execute_command.is_empty(),
        }
    }

    pub fn merge(self) -> Merge {
        match self {
            Self::Hover
            | Self::Definition
            | Self::Declaration
            | Self::TypeDefinition
            | Self::Implementation
            | Self::References
            | Self::WorkspaceSymbol
            | Self::Completion
            | Self::CodeAction
            | Self::InlayHint
            | Self::CodeLens => Merge::All,
            Self::DocumentHighlight
            | Self::DocumentSymbol
            | Self::SignatureHelp
            | Self::PrepareRename
            | Self::Rename
            | Self::FoldingRange
            | Self::SelectionRange
            | Self::PrepareCallHierarchy
            | Self::PrepareTypeHierarchy => Merge::First,
            Self::Formatting | Self::RangeFormatting | Self::OnTypeFormatting => Merge::Primary,
            Self::CompletionResolve
            | Self::CodeActionResolve
            | Self::CodeLensResolve
            | Self::IncomingCalls
            | Self::OutgoingCalls
            | Self::Supertypes
            | Self::Subtypes
            | Self::ExecuteCommand => Merge::Target,
        }
    }

    /// Whether the request is about a document (and so waits for its text to be
    /// with the server) rather than about the workspace or an earlier answer.
    pub fn about_document(self) -> bool {
        !matches!(
            self,
            Self::WorkspaceSymbol
                | Self::CompletionResolve
                | Self::CodeActionResolve
                | Self::CodeLensResolve
                | Self::IncomingCalls
                | Self::OutgoingCalls
                | Self::Supertypes
                | Self::Subtypes
                | Self::ExecuteCommand
        )
    }

    /// A newer request of this kind for the same document replaces an older one
    /// still waiting: only the latest position or text matters.
    pub fn replaces_older(self) -> bool {
        matches!(
            self,
            Self::Hover
                | Self::Completion
                | Self::SignatureHelp
                | Self::DocumentHighlight
                | Self::CodeAction
                | Self::InlayHint
                | Self::FoldingRange
                | Self::CodeLens
                | Self::DocumentSymbol
                | Self::PrepareRename
        )
    }
}

fn position_json(position: Position) -> Value {
    json!({ "line": position.line, "character": position.character })
}

/// The parameters of a request. `start` and `end` are in the server's encoding.
/// `extra` holds the parts that depend on the request (`context`, `options`,
/// `newName`, `ch`, `query`, `positions`); for the requests that continue an
/// earlier answer it *is* the parameters.
pub fn params(feature: Feature, uri: &str, start: Position, end: Position, extra: &Value) -> Value {
    let document = json!({ "uri": uri });
    let mut object = Map::new();
    match feature {
        Feature::WorkspaceSymbol => {}
        Feature::CompletionResolve
        | Feature::CodeActionResolve
        | Feature::CodeLensResolve
        | Feature::ExecuteCommand => return extra.clone(),
        Feature::IncomingCalls
        | Feature::OutgoingCalls
        | Feature::Supertypes
        | Feature::Subtypes => return json!({ "item": extra }),
        Feature::DocumentSymbol
        | Feature::FoldingRange
        | Feature::CodeLens
        | Feature::Formatting => {
            object.insert("textDocument".into(), document);
        }
        Feature::CodeAction | Feature::RangeFormatting | Feature::InlayHint => {
            object.insert("textDocument".into(), document);
            object.insert(
                "range".into(),
                json!({ "start": position_json(start), "end": position_json(end) }),
            );
        }
        Feature::SelectionRange => {
            object.insert("textDocument".into(), document);
            object.insert("positions".into(), json!([position_json(start)]));
        }
        _ => {
            object.insert("textDocument".into(), document);
            object.insert("position".into(), position_json(start));
        }
    }
    if let Some(extra) = extra.as_object() {
        for (key, value) in extra {
            object.insert(key.clone(), value.clone());
        }
    }
    if feature == Feature::References && !object.contains_key("context") {
        object.insert("context".into(), json!({ "includeDeclaration": true }));
    }
    if feature == Feature::CodeAction && !object.contains_key("context") {
        object.insert("context".into(), json!({ "diagnostics": [] }));
    }
    if matches!(
        feature,
        Feature::Formatting | Feature::RangeFormatting | Feature::OnTypeFormatting
    ) && !object.contains_key("options")
    {
        object.insert(
            "options".into(),
            json!({ "tabSize": 4, "insertSpaces": true }),
        );
    }
    Value::Object(object)
}

/// Whether an answer says nothing, for [`Merge::First`].
pub fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.is_empty(),
        Value::Object(object) => object.is_empty(),
        Value::String(text) => text.is_empty(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    #[test]
    fn a_positional_request_names_the_document_and_the_position() {
        let p = params(
            Feature::Hover,
            "file:///a.rs",
            at(2, 5),
            at(2, 5),
            &Value::Null,
        );
        assert_eq!(p["textDocument"]["uri"], "file:///a.rs");
        assert_eq!(p["position"], json!({ "line": 2, "character": 5 }));
    }

    #[test]
    fn references_include_the_declaration_unless_told_otherwise() {
        let default = params(Feature::References, "u", at(0, 0), at(0, 0), &Value::Null);
        assert_eq!(default["context"]["includeDeclaration"], true);
        let custom = params(
            Feature::References,
            "u",
            at(0, 0),
            at(0, 0),
            &json!({ "context": { "includeDeclaration": false } }),
        );
        assert_eq!(custom["context"]["includeDeclaration"], false);
    }

    #[test]
    fn ranged_requests_carry_a_range_and_the_extras() {
        let p = params(
            Feature::CodeAction,
            "u",
            at(1, 0),
            at(1, 4),
            &json!({ "context": { "diagnostics": [], "only": ["quickfix"] } }),
        );
        assert_eq!(p["range"]["end"]["character"], 4);
        assert_eq!(p["context"]["only"], json!(["quickfix"]));
        let format = params(Feature::Formatting, "u", at(0, 0), at(0, 0), &Value::Null);
        assert_eq!(format["options"]["tabSize"], 4);
        assert!(format.get("position").is_none());
    }

    #[test]
    fn follow_up_requests_use_the_earlier_answer_as_it_was() {
        let item = json!({ "label": "x", "data": 1 });
        assert_eq!(
            params(Feature::CompletionResolve, "", at(0, 0), at(0, 0), &item),
            item
        );
        assert_eq!(
            params(Feature::IncomingCalls, "", at(0, 0), at(0, 0), &item),
            json!({ "item": item })
        );
        assert_eq!(
            params(
                Feature::WorkspaceSymbol,
                "",
                at(0, 0),
                at(0, 0),
                &json!({ "query": "ab" })
            ),
            json!({ "query": "ab" })
        );
    }

    #[test]
    fn selection_ranges_ask_about_a_list_of_positions() {
        let p = params(
            Feature::SelectionRange,
            "u",
            at(3, 1),
            at(3, 1),
            &Value::Null,
        );
        assert_eq!(p["positions"], json!([{ "line": 3, "character": 1 }]));
    }

    #[test]
    fn support_follows_the_providers() {
        let none = Providers::default();
        for feature in [
            Feature::Hover,
            Feature::Completion,
            Feature::Rename,
            Feature::CodeLens,
        ] {
            assert!(!feature.supported(&none), "{feature:?}");
        }
        let some = Providers::parse(&json!({
            "hoverProvider": true,
            "renameProvider": { "prepareProvider": false },
            "completionProvider": { "resolveProvider": true },
        }));
        assert!(Feature::Hover.supported(&some));
        assert!(Feature::Rename.supported(&some));
        assert!(!Feature::PrepareRename.supported(&some));
        assert!(Feature::CompletionResolve.supported(&some));
        assert!(!Feature::CodeActionResolve.supported(&some));
    }

    #[test]
    fn empty_answers_are_recognised() {
        for empty in [json!(null), json!([]), json!({}), json!("")] {
            assert!(is_empty(&empty));
        }
        assert!(!is_empty(&json!([1])));
        assert!(!is_empty(&json!(0)));
    }
}
