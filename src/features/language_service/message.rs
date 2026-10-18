//! JSON-RPC 2.0 messages as the language server protocol uses them.
use serde_json::{Map, Value, json};
use std::fmt;

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INTERNAL_ERROR: i64 = -32603;
/// The request was cancelled by the client.
pub const REQUEST_CANCELLED: i64 = -32800;
/// The text changed while the server was working; the answer is stale.
pub const CONTENT_MODIFIED: i64 = -32801;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Id {
    Number(i64),
    String(String),
}

impl Id {
    fn to_json(&self) -> Value {
        match self {
            Self::Number(number) => json!(number),
            Self::String(text) => json!(text),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl ResponseError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

impl fmt::Display for ResponseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (code {})", self.message, self.code)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Request {
        id: Id,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Response {
        id: Id,
        outcome: Result<Value, ResponseError>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    NotJson(String),
    NotAnObject,
    /// An error response that answers no request, such as a parse error.
    Unaddressed,
    Invalid(&'static str),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotJson(error) => write!(f, "the message is not JSON: {error}"),
            Self::NotAnObject => f.write_str("the message is not a JSON object"),
            Self::Unaddressed => f.write_str("a response with no id"),
            Self::Invalid(why) => write!(f, "invalid message: {why}"),
        }
    }
}

impl std::error::Error for ParseError {}

fn id_of(value: &Value) -> Result<Option<Id>, ParseError> {
    match value {
        Value::Null => Ok(None),
        Value::Number(n) => n
            .as_i64()
            .map(|n| Some(Id::Number(n)))
            .ok_or(ParseError::Invalid("a non-integer id")),
        Value::String(text) => Ok(Some(Id::String(text.clone()))),
        _ => Err(ParseError::Invalid(
            "an id that is neither number nor string",
        )),
    }
}

impl Message {
    pub fn parse(bytes: &[u8]) -> Result<Self, ParseError> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|error| ParseError::NotJson(error.to_string()))?;
        let Value::Object(mut object) = value else {
            return Err(ParseError::NotAnObject);
        };
        let method = match object.remove("method") {
            None => None,
            Some(Value::String(method)) => Some(method),
            Some(_) => return Err(ParseError::Invalid("a method that is not a string")),
        };
        let params = object.remove("params").unwrap_or(Value::Null);
        let id = object.get("id").map(id_of).transpose()?.flatten();
        let has_id_field = object.contains_key("id");
        match (method, id) {
            (Some(method), Some(id)) => Ok(Self::Request { id, method, params }),
            (Some(method), None) if !has_id_field => Ok(Self::Notification { method, params }),
            (Some(_), None) => Err(ParseError::Invalid("a request with a null id")),
            (None, Some(id)) => {
                if let Some(error) = object.remove("error") {
                    let Value::Object(mut error) = error else {
                        return Err(ParseError::Invalid("an error that is not an object"));
                    };
                    let code = error
                        .remove("code")
                        .and_then(|c| c.as_i64())
                        .ok_or(ParseError::Invalid("an error without a code"))?;
                    let message = match error.remove("message") {
                        Some(Value::String(message)) => message,
                        _ => String::new(),
                    };
                    return Ok(Self::Response {
                        id,
                        outcome: Err(ResponseError {
                            code,
                            message,
                            data: error.remove("data"),
                        }),
                    });
                }
                Ok(Self::Response {
                    id,
                    outcome: Ok(object.remove("result").unwrap_or(Value::Null)),
                })
            }
            (None, None) if has_id_field && object.contains_key("error") => {
                Err(ParseError::Unaddressed)
            }
            (None, None) => Err(ParseError::Invalid("neither a method nor an id")),
        }
    }

    pub fn to_value(&self) -> Value {
        let mut object = Map::new();
        object.insert("jsonrpc".into(), json!("2.0"));
        match self {
            Self::Request { id, method, params } => {
                object.insert("id".into(), id.to_json());
                object.insert("method".into(), json!(method));
                if !params.is_null() {
                    object.insert("params".into(), params.clone());
                }
            }
            Self::Notification { method, params } => {
                object.insert("method".into(), json!(method));
                if !params.is_null() {
                    object.insert("params".into(), params.clone());
                }
            }
            Self::Response { id, outcome } => {
                object.insert("id".into(), id.to_json());
                match outcome {
                    Ok(result) => {
                        object.insert("result".into(), result.clone());
                    }
                    Err(error) => {
                        let mut body = Map::new();
                        body.insert("code".into(), json!(error.code));
                        body.insert("message".into(), json!(error.message));
                        if let Some(data) = &error.data {
                            body.insert("data".into(), data.clone());
                        }
                        object.insert("error".into(), Value::Object(body));
                    }
                }
            }
        }
        Value::Object(object)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&self.to_value()).expect("a JSON value always serializes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Result<Message, ParseError> {
        Message::parse(text.as_bytes())
    }

    #[test]
    fn the_three_kinds_are_told_apart() {
        assert_eq!(
            parse(r#"{"jsonrpc":"2.0","id":7,"method":"m","params":{"a":1}}"#).unwrap(),
            Message::Request {
                id: Id::Number(7),
                method: "m".into(),
                params: json!({"a": 1})
            }
        );
        assert_eq!(
            parse(r#"{"jsonrpc":"2.0","method":"n"}"#).unwrap(),
            Message::Notification {
                method: "n".into(),
                params: Value::Null
            }
        );
        assert_eq!(
            parse(r#"{"jsonrpc":"2.0","id":"abc","result":[1,2]}"#).unwrap(),
            Message::Response {
                id: Id::String("abc".into()),
                outcome: Ok(json!([1, 2]))
            }
        );
        assert_eq!(
            parse(r#"{"id":3,"error":{"code":-32601,"message":"nope","data":9}}"#).unwrap(),
            Message::Response {
                id: Id::Number(3),
                outcome: Err(ResponseError {
                    code: METHOD_NOT_FOUND,
                    message: "nope".into(),
                    data: Some(json!(9))
                })
            }
        );
    }

    #[test]
    fn a_null_result_is_a_successful_answer() {
        assert_eq!(
            parse(r#"{"id":1,"result":null}"#).unwrap(),
            Message::Response {
                id: Id::Number(1),
                outcome: Ok(Value::Null)
            }
        );
        assert_eq!(
            parse(r#"{"id":1}"#).unwrap(),
            Message::Response {
                id: Id::Number(1),
                outcome: Ok(Value::Null)
            },
            "a response with neither field still answers the request"
        );
    }

    #[test]
    fn malformed_messages_are_errors() {
        assert!(matches!(parse("not json"), Err(ParseError::NotJson(_))));
        assert_eq!(parse("[1]").unwrap_err(), ParseError::NotAnObject);
        assert_eq!(
            parse("{}").unwrap_err(),
            ParseError::Invalid("neither a method nor an id")
        );
        assert_eq!(
            parse(r#"{"method":5}"#).unwrap_err(),
            ParseError::Invalid("a method that is not a string")
        );
        assert_eq!(
            parse(r#"{"id":1.5,"method":"m"}"#).unwrap_err(),
            ParseError::Invalid("a non-integer id")
        );
        assert_eq!(
            parse(r#"{"id":[1],"method":"m"}"#).unwrap_err(),
            ParseError::Invalid("an id that is neither number nor string")
        );
        assert_eq!(
            parse(r#"{"id":null,"method":"m"}"#).unwrap_err(),
            ParseError::Invalid("a request with a null id")
        );
        assert_eq!(
            parse(r#"{"id":null,"error":{"code":-32700,"message":"x"}}"#).unwrap_err(),
            ParseError::Unaddressed
        );
        assert_eq!(
            parse(r#"{"id":1,"error":3}"#).unwrap_err(),
            ParseError::Invalid("an error that is not an object")
        );
        assert_eq!(
            parse(r#"{"id":1,"error":{"message":"x"}}"#).unwrap_err(),
            ParseError::Invalid("an error without a code")
        );
    }

    #[test]
    fn messages_serialize_to_what_they_parse_from() {
        let messages = [
            Message::Request {
                id: Id::Number(1),
                method: "textDocument/hover".into(),
                params: json!({"position": {"line": 0, "character": 2}}),
            },
            Message::Request {
                id: Id::String("s".into()),
                method: "shutdown".into(),
                params: Value::Null,
            },
            Message::Notification {
                method: "exit".into(),
                params: Value::Null,
            },
            Message::Response {
                id: Id::Number(2),
                outcome: Ok(json!({"x": [1, null]})),
            },
            Message::Response {
                id: Id::Number(3),
                outcome: Err(ResponseError::new(INTERNAL_ERROR, "boom")),
            },
        ];
        for message in messages {
            assert_eq!(Message::parse(&message.to_bytes()).unwrap(), message);
        }
        let bytes = Message::Notification {
            method: "exit".into(),
            params: Value::Null,
        }
        .to_bytes();
        assert!(!String::from_utf8(bytes.clone()).unwrap().contains("params"));
        assert!(
            String::from_utf8(bytes)
                .unwrap()
                .contains("\"jsonrpc\":\"2.0\"")
        );
    }

    #[test]
    fn errors_display_their_code() {
        assert_eq!(
            ResponseError::new(CONTENT_MODIFIED, "stale").to_string(),
            "stale (code -32801)"
        );
    }
}
