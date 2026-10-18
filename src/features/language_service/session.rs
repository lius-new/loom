//! One conversation with one language server, as a state machine that does no
//! I/O. The caller feeds it what the server said (or that the connection ended)
//! and sends what it returns, so every rule here is testable without a process.
use super::capabilities::{
    SUPPORTED_TOKEN_MODIFIERS, SUPPORTED_TOKEN_TYPES, ServerCapabilities, client_capabilities,
};
use super::message::{
    CONTENT_MODIFIED, INTERNAL_ERROR, Id, METHOD_NOT_FOUND, Message, REQUEST_CANCELLED,
    ResponseError,
};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// The initialize request is out; nothing else may be asked yet.
    Initializing,
    Ready,
    /// The shutdown request is out; the server is being asked to stop.
    ShuttingDown,
    /// Finished, on request or because the connection ended during shutdown.
    Stopped,
    /// Ended without being asked to: the server crashed, refused, or broke the protocol.
    Failed(String),
}

impl State {
    pub fn is_closed(&self) -> bool {
        matches!(self, Self::Stopped | Self::Failed(_))
    }
}

#[derive(Clone, Debug)]
pub struct SessionOptions {
    pub root_uri: String,
    pub root_name: String,
    pub process_id: Option<u32>,
    pub client_version: String,
    /// Sent as `initializationOptions`.
    pub initialization_options: Option<Value>,
    /// Answers to `workspace/configuration`, by section name.
    pub configuration: BTreeMap<String, Value>,
}

#[derive(Debug, PartialEq)]
pub enum SessionError {
    /// The request needs a server that has finished starting.
    NotReady,
    /// The session has ended.
    Closed,
}

/// Something the application should know or act on.
#[derive(Debug, PartialEq)]
pub enum Event {
    StateChanged(State),
    /// The handshake is done; the answer is in [`Session::capabilities`].
    Ready(ServerCapabilities),
    /// The answer to a request made with [`Session::request`], or its failure.
    Response {
        id: Id,
        /// The caller's own label for the request.
        tag: u64,
        method: String,
        outcome: Result<Value, ResponseError>,
    },
    /// A notification the session does not handle itself, such as diagnostics.
    Notification {
        method: String,
        params: Value,
    },
    /// The server asks for its semantic tokens to be requested again.
    SemanticTokensRefresh,
    /// `window/logMessage`.
    Log {
        level: i64,
        message: String,
    },
    /// `window/showMessage` or `window/showMessageRequest`.
    ShowMessage {
        level: i64,
        message: String,
    },
    /// A request the session refused; kept so a missing feature can be diagnosed.
    Refused {
        method: String,
    },
}

/// What to do after an input: messages to send and events to deliver.
#[derive(Debug, Default, PartialEq)]
pub struct Output {
    pub send: Vec<Message>,
    pub events: Vec<Event>,
    /// The connection should be closed now rather than waited on.
    pub terminate: bool,
}

struct Pending {
    method: String,
    tag: u64,
}

pub struct Session {
    options: SessionOptions,
    state: State,
    next_id: i64,
    initialize_id: Id,
    shutdown_id: Option<Id>,
    pending: HashMap<Id, Pending>,
    capabilities: Option<ServerCapabilities>,
    /// Answers that arrived for requests no longer pending (cancelled or timed out).
    pub late_responses: usize,
}

impl Session {
    /// Begins a session: the returned output holds the initialize request.
    pub fn start(options: SessionOptions) -> (Self, Output) {
        let initialize_id = Id::Number(0);
        let mut params = Map::new();
        params.insert("processId".into(), json!(options.process_id));
        params.insert(
            "clientInfo".into(),
            json!({ "name": "Loom", "version": options.client_version }),
        );
        params.insert("rootUri".into(), json!(options.root_uri));
        params.insert("rootPath".into(), Value::Null);
        params.insert(
            "capabilities".into(),
            client_capabilities(SUPPORTED_TOKEN_TYPES, SUPPORTED_TOKEN_MODIFIERS),
        );
        params.insert("trace".into(), json!("off"));
        if let Some(options) = &options.initialization_options {
            params.insert("initializationOptions".into(), options.clone());
        }
        let request = Message::Request {
            id: initialize_id.clone(),
            method: "initialize".into(),
            params: Value::Object(params),
        };
        let session = Self {
            options,
            state: State::Initializing,
            next_id: 1,
            initialize_id,
            shutdown_id: None,
            pending: HashMap::new(),
            capabilities: None,
            late_responses: 0,
        };
        (
            session,
            Output {
                send: vec![request],
                ..Output::default()
            },
        )
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn capabilities(&self) -> Option<&ServerCapabilities> {
        self.capabilities.as_ref()
    }

    pub fn pending_requests(&self) -> usize {
        self.pending.len()
    }

    /// Asks the server something. `tag` comes back with the answer.
    pub fn request(
        &mut self,
        method: &str,
        params: Value,
        tag: u64,
    ) -> Result<(Id, Output), SessionError> {
        self.require_ready()?;
        let id = Id::Number(self.next_id);
        self.next_id += 1;
        self.pending.insert(
            id.clone(),
            Pending {
                method: method.to_owned(),
                tag,
            },
        );
        let output = Output {
            send: vec![Message::Request {
                id: id.clone(),
                method: method.to_owned(),
                params,
            }],
            ..Output::default()
        };
        Ok((id, output))
    }

    pub fn notify(&mut self, method: &str, params: Value) -> Result<Output, SessionError> {
        self.require_ready()?;
        Ok(Output {
            send: vec![Message::Notification {
                method: method.to_owned(),
                params,
            }],
            ..Output::default()
        })
    }

    fn require_ready(&self) -> Result<(), SessionError> {
        match self.state {
            State::Ready => Ok(()),
            State::Initializing | State::ShuttingDown => Err(SessionError::NotReady),
            State::Stopped | State::Failed(_) => Err(SessionError::Closed),
        }
    }

    /// Gives up on a request: tells the server and answers the caller at once. A
    /// server may ignore the cancellation; its late answer is dropped.
    pub fn cancel(&mut self, id: &Id) -> Output {
        self.abandon(
            id,
            ResponseError::new(REQUEST_CANCELLED, "the request was cancelled"),
        )
    }

    /// Like [`cancel`](Self::cancel), for a request that took too long.
    pub fn expire(&mut self, id: &Id) -> Output {
        self.abandon(
            id,
            ResponseError::new(INTERNAL_ERROR, "the request timed out"),
        )
    }

    fn abandon(&mut self, id: &Id, error: ResponseError) -> Output {
        let mut output = Output::default();
        if let Some(pending) = self.pending.remove(id) {
            output.send.push(Message::Notification {
                method: "$/cancelRequest".into(),
                params: json!({ "id": id_json(id) }),
            });
            output.events.push(Event::Response {
                id: id.clone(),
                tag: pending.tag,
                method: pending.method,
                outcome: Err(error),
            });
        }
        output
    }

    /// Handles one message from the server.
    pub fn receive(&mut self, message: Message) -> Output {
        if self.state.is_closed() {
            return Output::default();
        }
        match message {
            Message::Response { id, outcome } => self.response(id, outcome),
            Message::Request { id, method, params } => self.server_request(id, &method, &params),
            Message::Notification { method, params } => self.notification(method, params),
        }
    }

    fn response(&mut self, id: Id, outcome: Result<Value, ResponseError>) -> Output {
        if id == self.initialize_id && self.state == State::Initializing {
            return self.initialized(outcome);
        }
        if Some(&id) == self.shutdown_id.as_ref() {
            // The server agreed to stop (or failed to; either way it is time to leave).
            self.state = State::Stopped;
            return Output {
                send: vec![Message::Notification {
                    method: "exit".into(),
                    params: Value::Null,
                }],
                events: vec![Event::StateChanged(State::Stopped)],
                terminate: false,
            };
        }
        match self.pending.remove(&id) {
            Some(pending) => Output {
                events: vec![Event::Response {
                    id,
                    tag: pending.tag,
                    method: pending.method,
                    outcome,
                }],
                ..Output::default()
            },
            None => {
                self.late_responses += 1;
                Output::default()
            }
        }
    }

    fn initialized(&mut self, outcome: Result<Value, ResponseError>) -> Output {
        let result = match outcome {
            Ok(result) => result,
            Err(error) => return self.fail(format!("the server refused to initialize: {error}")),
        };
        let capabilities =
            match ServerCapabilities::parse(result.get("capabilities").unwrap_or(&Value::Null)) {
                Ok(capabilities) => capabilities,
                Err(error) => return self.fail(error.to_string()),
            };
        self.capabilities = Some(capabilities.clone());
        self.state = State::Ready;
        Output {
            send: vec![Message::Notification {
                method: "initialized".into(),
                params: json!({}),
            }],
            events: vec![
                Event::StateChanged(State::Ready),
                Event::Ready(capabilities),
            ],
            terminate: false,
        }
    }

    fn notification(&mut self, method: String, params: Value) -> Output {
        let events = match method.as_str() {
            "window/logMessage" => vec![Event::Log {
                level: level(&params),
                message: text(&params),
            }],
            "window/showMessage" => vec![Event::ShowMessage {
                level: level(&params),
                message: text(&params),
            }],
            // Progress, tracing and the server's own cancellations carry nothing
            // this client acts on.
            "$/progress" | "$/logTrace" | "$/cancelRequest" | "telemetry/event" => Vec::new(),
            _ => vec![Event::Notification { method, params }],
        };
        Output {
            events,
            ..Output::default()
        }
    }

    /// Answers a request the server made of us. Every request gets an answer: a
    /// server waiting for one may otherwise stall.
    fn server_request(&mut self, id: Id, method: &str, params: &Value) -> Output {
        let mut events = Vec::new();
        let outcome: Result<Value, ResponseError> = match method {
            "workspace/configuration" => {
                let items = params.get("items").and_then(Value::as_array);
                Ok(Value::Array(
                    items
                        .map(|items| {
                            items
                                .iter()
                                .map(|item| {
                                    item.get("section")
                                        .and_then(Value::as_str)
                                        .and_then(|section| self.options.configuration.get(section))
                                        .cloned()
                                        .unwrap_or(Value::Null)
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                ))
            }
            "workspace/semanticTokens/refresh" => {
                events.push(Event::SemanticTokensRefresh);
                Ok(Value::Null)
            }
            // Accepted and ignored: Loom shows no progress yet (P4f).
            "window/workDoneProgress/create" => Ok(Value::Null),
            "workspace/diagnostic/refresh"
            | "workspace/codeLens/refresh"
            | "workspace/inlayHint/refresh"
            | "workspace/inlineValue/refresh"
            | "workspace/foldingRange/refresh" => Ok(Value::Null),
            "window/showMessageRequest" => {
                events.push(Event::ShowMessage {
                    level: level(params),
                    message: text(params),
                });
                Ok(Value::Null)
            }
            "window/showDocument" => Ok(json!({ "success": false })),
            "workspace/workspaceFolders" => Ok(json!([
                { "uri": self.options.root_uri, "name": self.options.root_name }
            ])),
            "workspace/applyEdit" => Ok(json!({
                "applied": false,
                "failureReason": "Loom does not apply edits requested by a language server"
            })),
            "client/registerCapability" | "client/unregisterCapability" => {
                events.push(Event::Refused {
                    method: method.to_owned(),
                });
                Err(ResponseError::new(
                    METHOD_NOT_FOUND,
                    "Loom did not offer dynamic registration",
                ))
            }
            other => {
                events.push(Event::Refused {
                    method: other.to_owned(),
                });
                Err(ResponseError::new(
                    METHOD_NOT_FOUND,
                    format!("method not found: {other}"),
                ))
            }
        };
        Output {
            send: vec![Message::Response { id, outcome }],
            events,
            terminate: false,
        }
    }

    /// Starts an orderly stop. A session that is not ready cannot be stopped
    /// politely (the protocol forbids it before the handshake), so it is ended.
    pub fn begin_shutdown(&mut self) -> Output {
        match self.state {
            State::Ready => {
                let id = Id::Number(self.next_id);
                self.next_id += 1;
                self.shutdown_id = Some(id.clone());
                self.state = State::ShuttingDown;
                let mut output =
                    self.fail_pending(REQUEST_CANCELLED, "the server is shutting down");
                output.send.push(Message::Request {
                    id,
                    method: "shutdown".into(),
                    params: Value::Null,
                });
                output.events.push(Event::StateChanged(State::ShuttingDown));
                output
            }
            State::Initializing => {
                self.state = State::Stopped;
                Output {
                    events: vec![Event::StateChanged(State::Stopped)],
                    terminate: true,
                    ..Output::default()
                }
            }
            State::ShuttingDown | State::Stopped | State::Failed(_) => Output::default(),
        }
    }

    /// The connection ended: end of file, a crash, or a write that failed.
    pub fn transport_closed(&mut self, reason: Option<String>) -> Output {
        match self.state {
            State::Stopped | State::Failed(_) => Output::default(),
            State::ShuttingDown => {
                self.state = State::Stopped;
                Output {
                    events: vec![Event::StateChanged(State::Stopped)],
                    ..Output::default()
                }
            }
            State::Initializing | State::Ready => {
                self.fail(reason.unwrap_or_else(|| "the server closed the connection".into()))
            }
        }
    }

    /// Ends the session as failed: every pending request is answered with an error.
    pub fn fail(&mut self, reason: String) -> Output {
        if self.state.is_closed() {
            return Output::default();
        }
        self.state = State::Failed(reason.clone());
        let mut output = self.fail_pending(INTERNAL_ERROR, &reason);
        output
            .events
            .push(Event::StateChanged(State::Failed(reason)));
        output.terminate = true;
        output
    }

    fn fail_pending(&mut self, code: i64, message: &str) -> Output {
        let mut ids: Vec<Id> = self.pending.keys().cloned().collect();
        ids.sort_by_key(|id| match id {
            Id::Number(n) => *n,
            Id::String(_) => i64::MAX,
        });
        let events = ids
            .into_iter()
            .filter_map(|id| {
                self.pending.remove(&id).map(|pending| Event::Response {
                    id,
                    tag: pending.tag,
                    method: pending.method,
                    outcome: Err(ResponseError::new(code, message)),
                })
            })
            .collect();
        Output {
            events,
            ..Output::default()
        }
    }
}

fn id_json(id: &Id) -> Value {
    match id {
        Id::Number(number) => json!(number),
        Id::String(text) => json!(text),
    }
}

fn level(params: &Value) -> i64 {
    params.get("type").and_then(Value::as_i64).unwrap_or(4)
}

fn text(params: &Value) -> String {
    params
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// A response the server may send for a request whose text has since changed.
pub fn is_stale(error: &ResponseError) -> bool {
    error.code == CONTENT_MODIFIED
}
