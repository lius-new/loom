use super::capabilities::PositionEncoding;
use super::message::{
    CONTENT_MODIFIED, Id, METHOD_NOT_FOUND, Message, REQUEST_CANCELLED, ResponseError,
};
use super::session::{Event, Output, Session, SessionError, SessionOptions, State, is_stale};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn options() -> SessionOptions {
    SessionOptions {
        root_uri: "file:///work/project".into(),
        root_name: "project".into(),
        process_id: Some(4242),
        client_version: "0.1.1".into(),
        initialization_options: Some(json!({ "cargo": { "features": "all" } })),
        configuration: BTreeMap::from([(
            "rust-analyzer".to_owned(),
            json!({ "check": { "command": "clippy" } }),
        )]),
    }
}

fn started() -> (Session, Output) {
    Session::start(options())
}

fn answer(id: i64, result: Value) -> Message {
    Message::Response {
        id: Id::Number(id),
        outcome: Ok(result),
    }
}

/// A session that has finished its handshake with the given server capabilities.
fn ready(capabilities: Value) -> Session {
    let (mut session, _) = started();
    let out = session.receive(answer(0, json!({ "capabilities": capabilities })));
    assert_eq!(session.state(), &State::Ready, "{out:?}");
    session
}

fn sent_methods(output: &Output) -> Vec<String> {
    output
        .send
        .iter()
        .map(|m| match m {
            Message::Request { method, .. } | Message::Notification { method, .. } => {
                method.clone()
            }
            Message::Response { .. } => "(response)".into(),
        })
        .collect()
}

#[test]
fn the_first_thing_sent_is_an_initialize_request_with_what_loom_can_do() {
    let (session, output) = started();
    assert_eq!(session.state(), &State::Initializing);
    let [Message::Request { id, method, params }] = output.send.as_slice() else {
        panic!("{output:?}");
    };
    assert_eq!((id, method.as_str()), (&Id::Number(0), "initialize"));
    assert_eq!(params["processId"], json!(4242));
    assert_eq!(
        params["clientInfo"],
        json!({ "name": "Loom", "version": "0.1.1" })
    );
    assert_eq!(params["rootUri"], json!("file:///work/project"));
    assert_eq!(
        params["workspaceFolders"],
        json!([{ "uri": "file:///work/project", "name": "project" }])
    );
    assert_eq!(
        params["capabilities"]["workspace"]["workspaceFolders"],
        true
    );
    assert_eq!(
        params["initializationOptions"],
        json!({ "cargo": { "features": "all" } })
    );
    assert_eq!(
        params["capabilities"]["general"]["positionEncodings"],
        json!(["utf-8", "utf-16"])
    );
    assert!(
        params["capabilities"]["textDocument"]["semanticTokens"]["tokenTypes"]
            .as_array()
            .unwrap()
            .contains(&json!("keyword"))
    );
    assert!(output.events.is_empty() && !output.terminate);

    // No initialization options means none are sent.
    let mut plain = options();
    plain.initialization_options = None;
    let (_, output) = Session::start(plain);
    let Message::Request { params, .. } = &output.send[0] else {
        panic!()
    };
    assert!(params.get("initializationOptions").is_none());
}

#[test]
fn requests_wait_for_the_handshake_and_the_initialized_notification_follows_it() {
    let (mut session, _) = started();
    assert_eq!(
        session
            .request("textDocument/hover", json!({}), 1)
            .unwrap_err(),
        SessionError::NotReady
    );
    assert_eq!(
        session
            .notify("textDocument/didOpen", json!({}))
            .unwrap_err(),
        SessionError::NotReady
    );

    let output = session.receive(answer(
        0,
        json!({ "capabilities": { "positionEncoding": "utf-8", "textDocumentSync": 2 }, "serverInfo": { "name": "x" } }),
    ));
    assert_eq!(sent_methods(&output), ["initialized"]);
    assert_eq!(session.state(), &State::Ready);
    let caps = session.capabilities().unwrap();
    assert_eq!(caps.position_encoding, PositionEncoding::Utf8);
    assert_eq!(
        output.events,
        [
            Event::StateChanged(State::Ready),
            Event::Ready(caps.clone())
        ]
    );
    assert!(session.request("a", json!({}), 1).is_ok());
}

#[test]
fn a_server_with_no_capabilities_is_still_usable() {
    let session = ready(json!({}));
    let caps = session.capabilities().unwrap();
    assert_eq!(caps.position_encoding, PositionEncoding::Utf16);
    assert!(caps.semantic_tokens.is_none());
}

#[test]
fn answers_are_routed_by_id_to_the_callers_tag() {
    let mut session = ready(json!({}));
    let (first, out) = session
        .request("textDocument/semanticTokens/full", json!({ "x": 1 }), 11)
        .unwrap();
    let (second, _) = session
        .request("textDocument/hover", json!({}), 22)
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(sent_methods(&out), ["textDocument/semanticTokens/full"]);
    assert_eq!(session.pending_requests(), 2);

    // Answered out of order.
    let out = session.receive(Message::Response {
        id: second.clone(),
        outcome: Ok(json!("h")),
    });
    assert_eq!(
        out.events,
        [Event::Response {
            id: second,
            tag: 22,
            method: "textDocument/hover".into(),
            outcome: Ok(json!("h"))
        }]
    );
    let error = ResponseError::new(CONTENT_MODIFIED, "stale");
    let out = session.receive(Message::Response {
        id: first.clone(),
        outcome: Err(error.clone()),
    });
    assert_eq!(
        out.events,
        [Event::Response {
            id: first,
            tag: 11,
            method: "textDocument/semanticTokens/full".into(),
            outcome: Err(error.clone())
        }]
    );
    assert!(is_stale(&error));
    assert_eq!(session.pending_requests(), 0);
}

#[test]
fn a_cancelled_or_expired_request_answers_at_once_and_a_late_reply_is_dropped() {
    let mut session = ready(json!({}));
    let (id, _) = session.request("a", json!({}), 1).unwrap();
    let out = session.cancel(&id);
    assert_eq!(sent_methods(&out), ["$/cancelRequest"]);
    let Message::Notification { params, .. } = &out.send[0] else {
        panic!()
    };
    assert_eq!(params["id"], json!(1));
    let [
        Event::Response {
            tag,
            outcome: Err(error),
            ..
        },
    ] = out.events.as_slice()
    else {
        panic!("{out:?}")
    };
    assert_eq!((*tag, error.code), (1, REQUEST_CANCELLED));

    // The server answers anyway: nothing happens.
    let out = session.receive(Message::Response {
        id: id.clone(),
        outcome: Ok(json!(1)),
    });
    assert_eq!(out, Output::default());
    assert_eq!(session.late_responses, 1);
    // Cancelling again, or something never asked, does nothing.
    assert_eq!(session.cancel(&id), Output::default());
    assert_eq!(session.expire(&Id::Number(999)), Output::default());

    let (id, _) = session.request("b", json!({}), 2).unwrap();
    let out = session.expire(&id);
    let [
        Event::Response {
            outcome: Err(error),
            ..
        },
    ] = out.events.as_slice()
    else {
        panic!()
    };
    assert!(error.message.contains("timed out"));
}

#[test]
fn every_server_request_gets_an_answer() {
    let mut session = ready(json!({}));
    let ask = |session: &mut Session, id: i64, method: &str, params: Value| {
        session.receive(Message::Request {
            id: Id::Number(id),
            method: method.into(),
            params,
        })
    };
    let result = |output: &Output| match output.send.as_slice() {
        [Message::Response { outcome, .. }] => outcome.clone(),
        other => panic!("expected one response, got {other:?}"),
    };

    // Configuration is answered per item, in order, null where nothing is set.
    let out = ask(
        &mut session,
        1,
        "workspace/configuration",
        json!({ "items": [{ "section": "rust-analyzer" }, { "section": "other" }, {}] }),
    );
    assert_eq!(
        result(&out),
        Ok(json!([
            { "check": { "command": "clippy" } },
            null,
            { "rust-analyzer": { "check": { "command": "clippy" } } }
        ]))
    );
    assert_eq!(
        result(&ask(&mut session, 2, "workspace/configuration", json!({}))),
        Ok(json!([]))
    );

    let out = ask(
        &mut session,
        3,
        "workspace/semanticTokens/refresh",
        Value::Null,
    );
    assert_eq!(result(&out), Ok(Value::Null));
    assert_eq!(out.events, [Event::SemanticTokensRefresh]);

    let out = ask(&mut session, 4, "workspace/diagnostic/refresh", Value::Null);
    assert_eq!(result(&out), Ok(Value::Null));
    assert_eq!(out.events, [Event::DiagnosticsRefresh]);

    for method in [
        "window/workDoneProgress/create",
        "workspace/inlayHint/refresh",
    ] {
        assert_eq!(
            result(&ask(&mut session, 4, method, json!({}))),
            Ok(Value::Null),
            "{method}"
        );
    }
    assert_eq!(
        result(&ask(
            &mut session,
            5,
            "workspace/workspaceFolders",
            Value::Null
        )),
        Ok(json!([{ "uri": "file:///work/project", "name": "project" }]))
    );
    // These two are for the application to answer: nothing is sent until it does.
    for (n, method) in [(6, "workspace/applyEdit"), (7, "window/showDocument")] {
        let out = ask(&mut session, n, method, json!({ "edit": {} }));
        assert!(out.send.is_empty(), "{method}");
        assert!(matches!(
            out.events.as_slice(),
            [Event::ServerRequest { method: asked, .. }] if asked == method
        ));
        let answered = session.respond(Id::Number(n), Ok(json!({ "applied": true })));
        assert_eq!(result(&answered), Ok(json!({ "applied": true })));
    }
    let out = ask(
        &mut session,
        8,
        "window/showMessageRequest",
        json!({ "type": 2, "message": "pick" }),
    );
    assert_eq!(result(&out), Ok(Value::Null));
    assert_eq!(
        out.events,
        [Event::ShowMessage {
            level: 2,
            message: "pick".into()
        }]
    );

    // Registration is answered asynchronously after the coordinator commits it.
    for method in ["client/registerCapability", "client/unregisterCapability"] {
        let out = ask(&mut session, 9, method, json!({}));
        assert!(out.send.is_empty());
        assert!(
            matches!(out.events.as_slice(), [Event::ServerRequest{method: found, ..}] if found == method)
        );
    }
    {
        let method = "textDocument/unheardOf";
        let out = ask(&mut session, 9, method, json!({}));
        let Err(error) = result(&out) else {
            panic!("{method} should be refused")
        };
        assert_eq!(error.code, METHOD_NOT_FOUND, "{method}");
        assert_eq!(
            out.events,
            [Event::Refused {
                method: method.into()
            }]
        );
    }

    // The answer carries the request's own id, whatever its type.
    let out = session.receive(Message::Request {
        id: Id::String("srv-1".into()),
        method: "x".into(),
        params: Value::Null,
    });
    let [Message::Response { id, .. }] = out.send.as_slice() else {
        panic!()
    };
    assert_eq!(id, &Id::String("srv-1".into()));
}

#[test]
fn server_requests_are_answered_during_the_handshake_too() {
    let (mut session, _) = started();
    let out = session.receive(Message::Request {
        id: Id::Number(77),
        method: "workspace/configuration".into(),
        params: json!({ "items": [{ "section": "rust-analyzer" }] }),
    });
    assert_eq!(sent_methods(&out), ["(response)"]);
    assert_eq!(session.state(), &State::Initializing);
}

#[test]
fn notifications_are_routed_and_the_noisy_ones_are_dropped() {
    let mut session = ready(json!({}));
    let note = |method: &str, params: Value| Message::Notification {
        method: method.into(),
        params,
    };
    assert_eq!(
        session
            .receive(note(
                "window/logMessage",
                json!({ "type": 3, "message": "indexing" })
            ))
            .events,
        [Event::Log {
            level: 3,
            message: "indexing".into()
        }]
    );
    assert_eq!(
        session
            .receive(note(
                "window/showMessage",
                json!({ "type": 1, "message": "bad" })
            ))
            .events,
        [Event::ShowMessage {
            level: 1,
            message: "bad".into()
        }]
    );
    for quiet in [
        "$/progress",
        "$/logTrace",
        "$/cancelRequest",
        "telemetry/event",
    ] {
        assert_eq!(
            session.receive(note(quiet, json!({}))),
            Output::default(),
            "{quiet}"
        );
    }
    let params = json!({ "uri": "file:///a.rs", "diagnostics": [] });
    assert_eq!(
        session
            .receive(note("textDocument/publishDiagnostics", params.clone()))
            .events,
        [Event::Notification {
            method: "textDocument/publishDiagnostics".into(),
            params
        }]
    );
    // A log message without fields still arrives.
    assert_eq!(
        session.receive(note("window/logMessage", json!({}))).events,
        [Event::Log {
            level: 4,
            message: String::new()
        }]
    );
}

#[test]
fn a_failed_handshake_ends_the_session_with_the_reason() {
    for (response, expected) in [
        (
            Message::Response {
                id: Id::Number(0),
                outcome: Err(ResponseError::new(-32603, "no workspace")),
            },
            "refused to initialize",
        ),
        (
            answer(
                0,
                json!({ "capabilities": { "positionEncoding": "utf-32" } }),
            ),
            "utf-32",
        ),
    ] {
        let (mut session, _) = started();
        let out = session.receive(response);
        let State::Failed(reason) = session.state().clone() else {
            panic!("{:?}", session.state())
        };
        assert!(reason.contains(expected), "{reason}");
        assert!(out.terminate);
        assert!(
            out.send.is_empty(),
            "nothing more is sent to a server that was refused"
        );
        assert_eq!(out.events, [Event::StateChanged(State::Failed(reason))]);
        assert_eq!(
            session.request("a", json!({}), 1).unwrap_err(),
            SessionError::Closed
        );
        assert_eq!(
            session.receive(answer(0, json!({}))),
            Output::default(),
            "a closed session ignores input"
        );
    }
}

#[test]
fn a_connection_that_ends_fails_the_session_and_every_waiting_request() {
    let mut session = ready(json!({}));
    session.request("a", json!({}), 1).unwrap();
    session.request("b", json!({}), 2).unwrap();
    let out = session.transport_closed(Some("exited with code 101".into()));
    let tags: Vec<u64> = out
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Response {
                tag,
                outcome: Err(_),
                ..
            } => Some(*tag),
            _ => None,
        })
        .collect();
    assert_eq!(tags, [1, 2], "in request order");
    assert_eq!(
        out.events.last(),
        Some(&Event::StateChanged(State::Failed(
            "exited with code 101".into()
        )))
    );
    assert!(out.terminate);
    assert_eq!(session.pending_requests(), 0);
    // Cleanup is idempotent.
    assert_eq!(session.transport_closed(None), Output::default());
    assert_eq!(session.begin_shutdown(), Output::default());
    assert_eq!(session.fail("again".into()), Output::default());
    assert!(matches!(session.state(), State::Failed(reason) if reason.contains("101")));

    // Ending during the handshake fails it as well.
    let (mut session, _) = started();
    session.transport_closed(None);
    assert!(
        matches!(session.state(), State::Failed(reason) if reason.contains("closed the connection"))
    );
}

#[test]
fn shutdown_asks_politely_then_exits_and_cancels_what_was_waiting() {
    let mut session = ready(json!({}));
    session.request("a", json!({}), 7).unwrap();
    let out = session.begin_shutdown();
    assert_eq!(session.state(), &State::ShuttingDown);
    assert_eq!(sent_methods(&out), ["shutdown"]);
    let [
        Event::Response {
            tag,
            outcome: Err(error),
            ..
        },
        Event::StateChanged(State::ShuttingDown),
    ] = out.events.as_slice()
    else {
        panic!("{out:?}")
    };
    assert_eq!((*tag, error.code), (7, REQUEST_CANCELLED));
    assert!(!out.terminate);
    assert_eq!(
        session.request("b", json!({}), 8).unwrap_err(),
        SessionError::NotReady
    );
    assert_eq!(
        session.begin_shutdown(),
        Output::default(),
        "asking twice is harmless"
    );

    let Message::Request { id, .. } = &out.send[0] else {
        panic!()
    };
    let out = session.receive(Message::Response {
        id: id.clone(),
        outcome: Ok(Value::Null),
    });
    assert_eq!(sent_methods(&out), ["exit"]);
    assert_eq!(session.state(), &State::Stopped);
    assert_eq!(out.events, [Event::StateChanged(State::Stopped)]);
    // The process ending afterwards changes nothing.
    assert_eq!(session.transport_closed(None), Output::default());
    assert_eq!(session.state(), &State::Stopped);
}

#[test]
fn a_connection_that_ends_while_shutting_down_is_a_normal_stop() {
    let mut session = ready(json!({}));
    session.begin_shutdown();
    let out = session.transport_closed(Some("exit".into()));
    assert_eq!(session.state(), &State::Stopped);
    assert_eq!(out.events, [Event::StateChanged(State::Stopped)]);
}

#[test]
fn a_server_that_is_still_starting_cannot_be_asked_to_shut_down() {
    let (mut session, _) = started();
    let out = session.begin_shutdown();
    assert_eq!(session.state(), &State::Stopped);
    assert!(out.terminate && out.send.is_empty());
}

#[test]
fn request_ids_never_repeat_and_never_collide_with_the_handshake() {
    let mut session = ready(json!({}));
    let mut seen = vec![Id::Number(0)];
    for tag in 0..50 {
        let (id, _) = session.request("m", json!({}), tag).unwrap();
        assert!(!seen.contains(&id));
        seen.push(id);
    }
    // The shutdown request takes the next one.
    let out = session.begin_shutdown();
    let shutdown = out.send.iter().find_map(|m| match m {
        Message::Request { id, .. } => Some(id.clone()),
        _ => None,
    });
    assert!(!seen.contains(&shutdown.unwrap()));
}

#[test]
fn an_answer_to_nothing_is_counted_not_acted_on() {
    let mut session = ready(json!({}));
    assert_eq!(session.receive(answer(555, json!(1))), Output::default());
    assert_eq!(session.late_responses, 1);
}

#[test]
fn progress_reports_are_read_for_every_kind_and_other_payloads_are_ignored() {
    use crate::features::language_service::session::{ProgressUpdate, parse_progress};
    let begin = parse_progress(&json!({ "token": "t", "value": {
        "kind": "begin", "title": "Indexing", "message": "a", "percentage": 5 } }));
    assert_eq!(
        begin,
        Some((
            Id::String("t".into()),
            ProgressUpdate::Begin {
                title: "Indexing".into(),
                message: Some("a".into()),
                percentage: Some(5),
                cancellable: None,
            }
        ))
    );
    let report =
        parse_progress(&json!({ "token": 7, "value": { "kind": "report", "percentage": 400 } }));
    assert_eq!(
        report,
        Some((
            Id::Number(7),
            ProgressUpdate::Report {
                message: None,
                percentage: Some(100),
                cancellable: None,
            }
        )),
        "numeric tokens work and the percentage is capped"
    );
    assert_eq!(
        parse_progress(&json!({ "token": "t", "value": { "kind": "end" } })),
        Some((
            Id::String("t".into()),
            ProgressUpdate::End { message: None }
        ))
    );
    // Partial results and malformed reports are not ours.
    assert_eq!(
        parse_progress(&json!({ "token": "t", "value": [1, 2] })),
        None
    );
    assert_eq!(
        parse_progress(&json!({ "token": "t", "value": { "kind": "begin" } })),
        None
    );
    assert_eq!(parse_progress(&json!({ "value": { "kind": "end" } })), None);
    let cancel = parse_progress(
        &json!({ "token": "t", "value": { "kind": "report", "cancellable": false } }),
    );
    assert!(matches!(
        cancel,
        Some((
            _,
            ProgressUpdate::Report {
                cancellable: Some(false),
                ..
            }
        ))
    ));
    assert_eq!(
        parse_progress(&json!({ "token": 1.5, "value": { "kind": "end" } })),
        None
    );
}
