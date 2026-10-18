//! What Loom tells a server it can do, and what it reads back about the server.
//! Only capabilities that are implemented are declared; a feature the server
//! lacks is simply absent, never an error.
use serde_json::{Value, json};

/// How `character` offsets in positions are counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PositionEncoding {
    Utf8,
    /// The protocol's default, used when the server does not say otherwise.
    #[default]
    Utf16,
}

impl PositionEncoding {
    pub fn name(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf16 => "utf-16",
        }
    }
}

/// How the server wants document changes sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SyncKind {
    /// The server wants no change notifications.
    #[default]
    None,
    Full,
    Incremental,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextSync {
    pub open_close: bool,
    pub change: SyncKind,
    pub save: Option<SaveSync>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveSync {
    pub include_text: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SemanticLegend {
    pub token_types: Vec<String>,
    pub token_modifiers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticTokens {
    pub legend: SemanticLegend,
    pub full: bool,
    pub delta: bool,
    pub range: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerCapabilities {
    pub position_encoding: PositionEncoding,
    pub text_sync: TextSync,
    pub semantic_tokens: Option<SemanticTokens>,
}

/// The server cannot be used as it answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported(pub String);

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl ServerCapabilities {
    /// Reads the `capabilities` object of an initialize result.
    pub fn parse(capabilities: &Value) -> Result<Self, Unsupported> {
        let position_encoding = match capabilities.get("positionEncoding").and_then(Value::as_str) {
            None | Some("utf-16") => PositionEncoding::Utf16,
            Some("utf-8") => PositionEncoding::Utf8,
            // The server must pick from the encodings the client offered.
            Some(other) => {
                return Err(Unsupported(format!(
                    "the server chose the position encoding {other:?}, which Loom did not offer"
                )));
            }
        };
        Ok(Self {
            position_encoding,
            text_sync: text_sync(capabilities.get("textDocumentSync")),
            semantic_tokens: semantic_tokens(capabilities.get("semanticTokensProvider")),
        })
    }
}

fn text_sync(value: Option<&Value>) -> TextSync {
    let kind = |value: Option<&Value>| match value.and_then(Value::as_i64) {
        Some(1) => SyncKind::Full,
        Some(2) => SyncKind::Incremental,
        _ => SyncKind::None,
    };
    match value {
        // A bare number is the change kind, and implies open and close.
        Some(Value::Number(_)) => TextSync {
            open_close: true,
            change: kind(value),
            save: None,
        },
        Some(Value::Object(object)) => TextSync {
            open_close: object
                .get("openClose")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            change: kind(object.get("change")),
            save: match object.get("save") {
                Some(Value::Bool(true)) => Some(SaveSync {
                    include_text: false,
                }),
                Some(Value::Object(save)) => Some(SaveSync {
                    include_text: save
                        .get("includeText")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                }),
                _ => None,
            },
        },
        _ => TextSync::default(),
    }
}

fn semantic_tokens(value: Option<&Value>) -> Option<SemanticTokens> {
    let provider = value?.as_object()?;
    let strings = |key: &str| -> Vec<String> {
        provider
            .get("legend")
            .and_then(|legend| legend.get(key))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let flag = |key: &str| match provider.get(key) {
        Some(Value::Bool(on)) => *on,
        Some(Value::Object(_)) => true,
        _ => false,
    };
    let delta = provider
        .get("full")
        .and_then(|full| full.get("delta"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(SemanticTokens {
        legend: SemanticLegend {
            token_types: strings("tokenTypes"),
            token_modifiers: strings("tokenModifiers"),
        },
        full: flag("full"),
        delta,
        range: flag("range"),
    })
}

/// The semantic token types Loom can map to a highlight class. The names are the
/// protocol's standard ones, so a server may use them without a custom legend.
pub const SUPPORTED_TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "class",
    "enum",
    "interface",
    "struct",
    "typeParameter",
    "parameter",
    "variable",
    "property",
    "enumMember",
    "event",
    "function",
    "method",
    "macro",
    "keyword",
    "modifier",
    "comment",
    "string",
    "number",
    "regexp",
    "operator",
    "decorator",
];

/// The standard semantic token modifiers.
pub const SUPPORTED_TOKEN_MODIFIERS: &[&str] = &[
    "declaration",
    "definition",
    "readonly",
    "static",
    "deprecated",
    "abstract",
    "async",
    "modification",
    "documentation",
    "defaultLibrary",
];

/// The `capabilities` object sent in the initialize request.
pub fn client_capabilities(token_types: &[&str], token_modifiers: &[&str]) -> Value {
    json!({
        "general": { "positionEncodings": ["utf-8", "utf-16"] },
        "textDocument": {
            "synchronization": {
                "dynamicRegistration": false,
                "willSave": false,
                "willSaveWaitUntil": false,
                "didSave": true
            },
            "publishDiagnostics": {
                "relatedInformation": false,
                "versionSupport": true,
                "codeDescriptionSupport": false,
                "dataSupport": false
            },
            "semanticTokens": {
                "dynamicRegistration": false,
                "requests": { "range": false, "full": { "delta": false } },
                "tokenTypes": token_types,
                "tokenModifiers": token_modifiers,
                "formats": ["relative"],
                "overlappingTokenSupport": false,
                "multilineTokenSupport": false,
                "serverCancelSupport": true,
                "augmentsSyntaxTokens": true
            }
        },
        "workspace": {
            "configuration": true,
            "workspaceFolders": false,
            "didChangeConfiguration": { "dynamicRegistration": false },
            "didChangeWatchedFiles": { "dynamicRegistration": false },
            "applyEdit": false,
            "semanticTokens": { "refreshSupport": true }
        },
        "window": {
            "workDoneProgress": true,
            "showMessage": { "messageActionItem": { "additionalPropertiesSupport": false } }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(value: Value) -> ServerCapabilities {
        ServerCapabilities::parse(&value).unwrap()
    }

    #[test]
    fn an_empty_answer_means_utf16_and_no_features() {
        assert_eq!(parse(json!({})), ServerCapabilities::default());
        assert_eq!(parse(json!({})).position_encoding, PositionEncoding::Utf16);
    }

    #[test]
    fn the_position_encoding_is_read_and_unknown_ones_are_refused() {
        assert_eq!(
            parse(json!({"positionEncoding": "utf-8"})).position_encoding,
            PositionEncoding::Utf8
        );
        assert_eq!(
            parse(json!({"positionEncoding": "utf-16"})).position_encoding,
            PositionEncoding::Utf16
        );
        let error = ServerCapabilities::parse(&json!({"positionEncoding": "utf-32"})).unwrap_err();
        assert!(error.0.contains("utf-32"));
        assert_eq!(PositionEncoding::Utf8.name(), "utf-8");
    }

    #[test]
    fn sync_is_a_number_or_an_object() {
        assert_eq!(
            parse(json!({"textDocumentSync": 2})).text_sync,
            TextSync {
                open_close: true,
                change: SyncKind::Incremental,
                save: None
            }
        );
        assert_eq!(
            parse(json!({"textDocumentSync": 1})).text_sync.change,
            SyncKind::Full
        );
        assert_eq!(
            parse(json!({"textDocumentSync": 0})).text_sync.change,
            SyncKind::None
        );
        assert_eq!(
            parse(json!({"textDocumentSync": {"openClose": true, "change": 1, "save": {"includeText": true}}})).text_sync,
            TextSync { open_close: true, change: SyncKind::Full, save: Some(SaveSync { include_text: true }) }
        );
        assert_eq!(
            parse(json!({"textDocumentSync": {"change": 2, "save": true}})).text_sync,
            TextSync {
                open_close: false,
                change: SyncKind::Incremental,
                save: Some(SaveSync {
                    include_text: false
                })
            }
        );
        assert_eq!(
            parse(json!({"textDocumentSync": {"save": false}}))
                .text_sync
                .save,
            None
        );
        assert_eq!(
            parse(json!({"textDocumentSync": "weird"})).text_sync,
            TextSync::default()
        );
    }

    #[test]
    fn semantic_tokens_carry_their_legend_and_modes() {
        let tokens = parse(json!({"semanticTokensProvider": {
            "legend": {"tokenTypes": ["keyword", "function"], "tokenModifiers": ["declaration"]},
            "full": {"delta": true},
            "range": true
        }}))
        .semantic_tokens
        .unwrap();
        assert_eq!(tokens.legend.token_types, ["keyword", "function"]);
        assert_eq!(tokens.legend.token_modifiers, ["declaration"]);
        assert!(tokens.full && tokens.delta && tokens.range);

        let plain = parse(json!({"semanticTokensProvider": {"legend": {}, "full": true}}))
            .semantic_tokens
            .unwrap();
        assert!(plain.full && !plain.delta && !plain.range);
        assert!(plain.legend.token_types.is_empty());

        let range_only = parse(json!({"semanticTokensProvider": {"legend": {}, "range": {}}}))
            .semantic_tokens
            .unwrap();
        assert!(!range_only.full && range_only.range);

        assert!(
            parse(json!({"semanticTokensProvider": true}))
                .semantic_tokens
                .is_none()
        );
        assert!(parse(json!({})).semantic_tokens.is_none());
    }

    #[test]
    fn the_client_declares_only_what_it_implements() {
        let caps = client_capabilities(&["keyword"], &["declaration"]);
        assert_eq!(
            caps["general"]["positionEncodings"],
            json!(["utf-8", "utf-16"])
        );
        assert_eq!(
            caps["textDocument"]["semanticTokens"]["requests"]["full"]["delta"],
            json!(false)
        );
        assert_eq!(
            caps["textDocument"]["semanticTokens"]["requests"]["range"],
            json!(false)
        );
        assert_eq!(
            caps["textDocument"]["semanticTokens"]["overlappingTokenSupport"],
            json!(false)
        );
        assert_eq!(
            caps["textDocument"]["semanticTokens"]["multilineTokenSupport"],
            json!(false)
        );
        assert_eq!(
            caps["textDocument"]["synchronization"]["dynamicRegistration"],
            json!(false)
        );
        assert_eq!(
            caps["workspace"]["didChangeWatchedFiles"]["dynamicRegistration"],
            json!(false)
        );
        assert_eq!(caps["workspace"]["applyEdit"], json!(false));
        assert_eq!(caps["window"]["workDoneProgress"], json!(true));
        assert_eq!(
            caps["workspace"]["semanticTokens"]["refreshSupport"],
            json!(true)
        );
        assert_eq!(caps["workspace"]["configuration"], json!(true));
    }
}
