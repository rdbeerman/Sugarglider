// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Message server that handles requests from the Glide CLI.

use std::cell::RefCell;
use std::fmt::{Display, Formatter};
use std::future::pending;
use std::rc::Rc;
use std::sync::Arc;

use objc2_service_management::SMAppService;
use serde::{Deserialize, Serialize};
use tracing::{Span, error, info, instrument, warn};

use crate::actor::contexts_snapshot::{
    self, CONTEXTS_OFF, CommandResult, ContextsSnapshot, RequestId,
};
use crate::actor::reactor::ContextCommand;
use crate::actor::wm_controller;
use crate::config::Config;
use crate::sys::message_port::{LocalMessagePort, LocalPortCreateError};

pub const PORT_NAME: &str = "org.glidewm.server";

#[derive(Serialize, Deserialize, Debug)]
pub enum Request {
    Ping(String),
    UpdateConfig(Config),
    Service(ServiceRequest),
    /// Pause (false) or resume (true) global window management.
    SetEnabled(bool),
    /// Reads or changes the contexts, the named window sets.
    Context(ContextRequest),
}

/// A request about contexts.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum ContextRequest {
    /// Every context.
    List,
    /// The active context.
    Current,
    /// Runs a command that switches or changes contexts. The client picks
    /// the id, and asks for the command's result with it.
    Run(RequestId, ContextCommand),
    /// The result of the command that `Run` sent with this id.
    Result(RequestId),
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ServiceRequest {
    Install,
    Uninstall,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Response {
    Pong(String),
    Success,
    Error(String),
    Contexts(ContextsSnapshot),
    /// The command that `Result` asks about hasn't run yet.
    Pending,
}

pub struct MessageServer {
    #[expect(unused)]
    port: LocalMessagePort,
    #[expect(unused)]
    state: Rc<RefCell<State>>,
}

struct State {
    wm_tx: wm_controller::Sender,
    /// Reads the snapshot of the contexts that the reactor published last.
    contexts: Box<dyn Fn() -> Option<Arc<ContextsSnapshot>>>,
}

impl MessageServer {
    pub fn new(name: &str, wm_tx: wm_controller::Sender) -> Result<Self, LocalPortCreateError> {
        let state = Rc::new(RefCell::new(State {
            wm_tx,
            contexts: Box::new(contexts_snapshot::published),
        }));
        let state_ = state.clone();
        Ok(MessageServer {
            port: LocalMessagePort::new(name, move |id, msg| {
                state_.borrow_mut().handle_message(id, msg)
            })?,
            state,
        })
    }

    pub async fn run(self) {
        // For now just don't return.
        pending().await
    }
}

impl State {
    fn handle_message(&mut self, id: i32, message: &[u8]) -> Vec<u8> {
        let Ok(request) = ron::de::from_bytes::<Request>(message) else {
            warn!(
                "Got invalid message with id {id} on port: \"{}\"",
                AsciiEscaped(message)
            );
            return vec![];
        };
        info!("Got message {id} on port: {request:?}");
        let response = self.on_request(request);
        match ron::ser::to_string(&response) {
            Ok(bytes) => bytes.into_bytes(),
            Err(e) => {
                error!("Failed to serialize response: {e}");
                vec![]
            }
        }
    }

    #[instrument(skip(self))]
    fn on_request(&mut self, request: Request) -> Response {
        match request {
            Request::Ping(payload) => {
                let resp = payload.chars().into_iter().rev().collect();
                Response::Pong(resp)
            }
            Request::UpdateConfig(config) => {
                _ = self.wm_tx.send((
                    Span::current(),
                    wm_controller::WmEvent::ConfigUpdated(Arc::new(config)),
                ));
                Response::Success
            }
            Request::SetEnabled(enabled) => {
                _ = self.wm_tx.send((
                    Span::current(),
                    wm_controller::WmEvent::Command(wm_controller::WmCommand::Wm(
                        wm_controller::WmCmd::SetGlobalEnabled(enabled),
                    )),
                ));
                Response::Success
            }
            Request::Context(request) => {
                let snapshot = (self.contexts)();
                let (response, command) = answer_context_request(request, snapshot.as_deref());
                if let Some((request, command)) = command {
                    _ = self.wm_tx.send((
                        Span::current(),
                        wm_controller::WmEvent::ContextCommandRequested(request, command),
                    ));
                }
                response
            }
            Request::Service(ServiceRequest::Install) => {
                // SAFETY: ? Requirements unclear.
                let result = unsafe { SMAppService::mainAppService().registerAndReturnError() };
                match result {
                    Ok(()) => Response::Success,
                    Err(e) => Response::Error(e.to_string()),
                }
            }
            Request::Service(ServiceRequest::Uninstall) => {
                // SAFETY: ? Requirements unclear.
                let result = unsafe { SMAppService::mainAppService().unregisterAndReturnError() };
                match result {
                    Ok(()) => Response::Success,
                    Err(e) => Response::Error(e.to_string()),
                }
            }
        }
    }
}

/// Answers a request about contexts from `snapshot`, the snapshot the
/// reactor published last, without waiting for the reactor (I3). Also
/// returns the command to send to the reactor, as the client sent it. The
/// reactor resolves the context the command names, runs it after the reply,
/// and publishes its result under the request's id.
pub(crate) fn answer_context_request(
    request: ContextRequest,
    snapshot: Option<&ContextsSnapshot>,
) -> (Response, Option<(RequestId, ContextCommand)>) {
    let Some(snapshot) = snapshot else {
        let reason = "Sugarglider hasn't loaded its contexts yet. Try again in a moment.";
        return (Response::Error(reason.to_string()), None);
    };
    match request {
        // A command that the server took before contexts were turned off
        // still has its result.
        ContextRequest::Result(request) => {
            let response = match snapshot.result(request) {
                None => Response::Pending,
                Some(CommandResult { error: None, .. }) => Response::Success,
                Some(CommandResult { error: Some(reason), .. }) => Response::Error(reason.clone()),
            };
            (response, None)
        }
        _ if !snapshot.enabled => (Response::Error(CONTEXTS_OFF.to_string()), None),
        ContextRequest::List => (Response::Contexts(snapshot.clone()), None),
        ContextRequest::Current => (Response::Contexts(snapshot.current()), None),
        ContextRequest::Run(request, command) => (Response::Success, Some((request, command))),
    }
}

pub struct AsciiEscaped<'a>(pub &'a [u8]);

impl Display for AsciiEscaped<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(f, "{}", std::ascii::escape_default(*byte))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::actor::contexts_snapshot::{
        ContextSummary, EverythingSummary, Scope, ScreenContext, UnsortedSummary,
    };
    use crate::actor::reactor::ContextRef;
    use crate::model::contexts::{ContextId, ContextKey, Contexts};

    fn read(text: &str) -> ContextRequest {
        match ron::de::from_str::<Request>(text).unwrap() {
            Request::Context(request) => request,
            other => panic!("{other:?}"),
        }
    }

    fn run(command: ContextCommand) -> ContextRequest {
        ContextRequest::Run(RequestId(7), command)
    }

    fn switch(reference: ContextRef) -> ContextCommand {
        ContextCommand::SwitchContext(reference)
    }

    fn name(name: &str) -> ContextRef {
        ContextRef::Name(name.to_string())
    }

    fn result(request: u64, error: Option<&str>) -> CommandResult {
        CommandResult {
            request: RequestId(request),
            error: error.map(str::to_string),
        }
    }

    /// Comms (1) and Client work (2), with Comms active and 2 unsorted
    /// windows.
    fn contexts() -> (Contexts, ContextsSnapshot) {
        let mut contexts = Contexts::new();
        let comms = contexts.create("Comms").unwrap();
        contexts.create("Client work").unwrap();
        contexts.switch_to(ContextKey::Named(comms)).unwrap();
        let screens = vec![ScreenContext {
            id: 1,
            shows: ContextKey::Named(comms),
        }];
        let snapshot = ContextsSnapshot::new(&contexts, screens, 2);
        (contexts, snapshot)
    }

    /// I2. Each context request, and each form of the tagged `ContextRef`,
    /// survives the RON round trip between the command line and the
    /// server. A bare integer is a number, `Id(7)` is an id, and a string is
    /// a name, even when it holds digits.
    #[test]
    fn context_requests_survive_a_ron_round_trip() {
        let seven: ContextId = serde_json::from_value(serde_json::json!(7)).unwrap();
        for request in [
            ContextRequest::List,
            ContextRequest::Current,
            run(switch(ContextRef::Number(7))),
            run(switch(name("7"))),
            run(switch(name("Client work"))),
            run(switch(ContextRef::Id(seven))),
            run(ContextCommand::ShowEverything),
            run(ContextCommand::PreviousContext),
            run(ContextCommand::CreateContext("Client work".into())),
            run(ContextCommand::AddWindowToContext(name("Comms"))),
            ContextRequest::Result(RequestId(u64::MAX)),
        ] {
            let text = ron::ser::to_string(&Request::Context(request.clone())).unwrap();
            assert_eq!(request, read(&text), "{text}");
        }
        assert_eq!(ContextRequest::List, read("Context(List)"));
        assert_eq!(
            run(switch(ContextRef::Number(7))),
            read("Context(Run(7, switch_context(7)))")
        );
        assert_eq!(
            run(switch(name("7"))),
            read("Context(Run(7, switch_context(\"7\")))")
        );
        assert_eq!(
            run(switch(ContextRef::Id(seven))),
            read("Context(Run(7, switch_context(Id(7))))")
        );
        assert_eq!(
            run(ContextCommand::CreateContext("X".into())),
            read("Context(Run(7, create_context(\"X\")))")
        );
        assert_eq!(ContextRequest::Result(RequestId(7)), read("Context(Result(7))"));
    }

    /// I2. The replies survive the RON round trip, the snapshot included.
    #[test]
    fn responses_survive_a_ron_round_trip() {
        let (_, snapshot) = contexts();
        for response in [
            Response::Success,
            Response::Pending,
            Response::Error("No context has the number 4".into()),
            Response::Contexts(snapshot),
            Response::Contexts(ContextsSnapshot::off()),
        ] {
            let text = ron::ser::to_string(&response).unwrap();
            assert_eq!(response, ron::de::from_str::<Response>(&text).unwrap(), "{text}");
        }
    }

    /// I3. `List` and `Current` reply from the snapshot at once and send
    /// nothing.
    #[test]
    fn list_and_current_reply_from_the_snapshot() {
        let (_, snapshot) = contexts();

        assert_eq!(
            (Response::Contexts(snapshot.clone()), None),
            answer_context_request(ContextRequest::List, Some(&snapshot))
        );
        assert_eq!(
            (Response::Contexts(snapshot.current()), None),
            answer_context_request(ContextRequest::Current, Some(&snapshot))
        );
    }

    /// `Run` replies at once that the server took the command, and sends it
    /// with its request id as the client wrote it. The server resolves no
    /// name, number, or id and checks no new name, because the snapshot can
    /// be older than the reactor's state: the reactor does both when it runs
    /// the command.
    #[test]
    fn run_takes_the_command_at_once_and_sends_it_as_written() {
        let (_, snapshot) = contexts();
        let gone: ContextId = serde_json::from_value(serde_json::json!(99)).unwrap();

        for command in [
            switch(name("cli")),
            switch(name("uns")),
            switch(name("Sugarglider")),
            switch(name(" ")),
            switch(ContextRef::Number(2)),
            switch(ContextRef::Number(9)),
            switch(ContextRef::Id(gone)),
            ContextCommand::ShowEverything,
            ContextCommand::PreviousContext,
            ContextCommand::CreateContext("comms".into()),
            ContextCommand::CreateContext("Everything".into()),
            ContextCommand::AddWindowToContext(name("Comms")),
            ContextCommand::MoveWindowToContext(ContextRef::Number(2)),
            ContextCommand::RemoveWindowFromContext,
            ContextCommand::ToggleWindowPinned,
        ] {
            let request = ContextRequest::Run(RequestId(3), command.clone());
            assert_eq!(
                (Response::Success, Some((RequestId(3), command))),
                answer_context_request(request, Some(&snapshot))
            );
        }
    }

    /// `Result` replies with the result the reactor published for the
    /// request id: `Success` when the command ran, the reason when it did
    /// nothing, and `Pending` while the snapshot has no result for the id,
    /// either because the command hasn't run yet or because newer results
    /// pushed its result out. It sends nothing.
    #[test]
    fn result_replies_with_the_published_result_or_pending() {
        let (_, snapshot) = contexts();
        let snapshot = ContextsSnapshot {
            results: vec![
                result(1, None),
                result(2, Some("No context matches \"x\"")),
                result(3, Some("No Space is managed right now")),
            ],
            ..snapshot
        };
        let answer =
            |request| answer_context_request(ContextRequest::Result(request), Some(&snapshot));

        assert_eq!((Response::Success, None), answer(RequestId(1)));
        assert_eq!(
            (Response::Error("No context matches \"x\"".into()), None),
            answer(RequestId(2))
        );
        assert_eq!(
            (Response::Error("No Space is managed right now".into()), None),
            answer(RequestId(3))
        );
        assert_eq!((Response::Pending, None), answer(RequestId(4)));
    }

    /// I3. The server sends the command to the window manager with its
    /// request id, and the window manager passes both to the reactor. A read
    /// sends nothing.
    #[test]
    fn run_sends_the_command_to_the_window_manager() {
        let (_, snapshot) = contexts();
        let snapshot = Arc::new(snapshot);
        let (wm_tx, mut wm_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut state = State {
            wm_tx,
            contexts: Box::new(move || Some(snapshot.clone())),
        };

        let response = state.on_request(Request::Context(run(switch(name("cli")))));

        assert_eq!(Response::Success, response);
        let (_, event) = wm_rx.try_recv().unwrap();
        let wm_controller::WmEvent::ContextCommandRequested(request, command) = event else {
            panic!("{event:?}");
        };
        assert_eq!((RequestId(7), switch(name("cli"))), (request, command));
        for read in [
            ContextRequest::List,
            ContextRequest::Current,
            ContextRequest::Result(RequestId(7)),
        ] {
            state.on_request(Request::Context(read));
        }
        assert!(wm_rx.try_recv().is_err());
    }

    /// R28. With contexts off, or before the reactor published anything,
    /// every request but `Result` fails and sends nothing. While contexts
    /// are off, `Result` still gives the result of a command that the
    /// server took before they were turned off.
    #[test]
    fn contexts_that_are_off_or_not_loaded_answer_nothing() {
        let off = ContextsSnapshot::off();
        for request in [
            ContextRequest::List,
            ContextRequest::Current,
            run(ContextCommand::ShowEverything),
        ] {
            let (response, sent) = answer_context_request(request.clone(), Some(&off));
            assert_eq!(None, sent);
            assert_eq!(Response::Error(CONTEXTS_OFF.into()), response);
            let (response, sent) = answer_context_request(request, None);
            assert_eq!(None, sent);
            assert!(matches!(response, Response::Error(_)), "{response:?}");
        }
        assert!(CONTEXTS_OFF.starts_with("Contexts are off."));

        let turned_off = ContextsSnapshot {
            results: vec![result(5, Some(CONTEXTS_OFF))],
            ..ContextsSnapshot::off()
        };
        let answer =
            |request| answer_context_request(ContextRequest::Result(request), Some(&turned_off));
        assert_eq!(
            (Response::Error(CONTEXTS_OFF.into()), None),
            answer(RequestId(5))
        );
        assert_eq!((Response::Pending, None), answer(RequestId(6)));
        assert!(matches!(
            answer_context_request(ContextRequest::Result(RequestId(5)), None),
            (Response::Error(_), None)
        ));
    }

    /// The requests of Sugarglider before contexts, as an older client sends
    /// them and an older server reads them.
    #[derive(Serialize, Deserialize, Debug)]
    enum RequestBeforeContexts {
        Ping(String),
        UpdateConfig(Config),
        Service(ServiceRequestBeforeContexts),
        SetEnabled(bool),
    }

    #[derive(Serialize, Deserialize, Debug)]
    enum ServiceRequestBeforeContexts {
        Install,
        Uninstall,
    }

    /// The replies of Sugarglider before contexts.
    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    enum ResponseBeforeContexts {
        Pong(String),
        Success,
        Error(String),
    }

    /// A name with a quote, a backslash, a newline, and letters outside
    /// ASCII.
    const ODD_NAME: &str = "Say \"hi\" \\ to Café ☕\nnow";

    fn context_id(id: u32) -> ContextId {
        serde_json::from_value(serde_json::json!(id)).unwrap()
    }

    /// A context request for each form of every command.
    fn every_context_request() -> Vec<ContextRequest> {
        vec![
            ContextRequest::List,
            ContextRequest::Current,
            run(switch(ContextRef::Number(1))),
            run(switch(ContextRef::Number(9))),
            run(switch(name("Client work"))),
            run(switch(name(ODD_NAME))),
            run(switch(name(""))),
            run(switch(name("Id(7)"))),
            run(switch(ContextRef::Id(context_id(7)))),
            run(ContextCommand::ShowEverything),
            run(ContextCommand::PreviousContext),
            run(ContextCommand::CreateContext("Client work".into())),
            run(ContextCommand::CreateContext(ODD_NAME.into())),
            run(ContextCommand::CreateContext(String::new())),
            run(ContextCommand::AddWindowToContext(name("Client work"))),
            ContextRequest::Result(RequestId(0)),
            ContextRequest::Result(RequestId(u64::MAX)),
        ]
    }

    /// I2. Every request survives the RON round trip, names that need
    /// escaping included. A name that looks like a tagged id stays a name.
    #[test]
    fn every_request_survives_a_ron_round_trip() {
        let mut requests = vec![
            Request::Ping(ODD_NAME.into()),
            Request::Service(ServiceRequest::Install),
            Request::Service(ServiceRequest::Uninstall),
            Request::SetEnabled(true),
            Request::SetEnabled(false),
        ];
        requests.extend(every_context_request().into_iter().map(Request::Context));
        for request in requests {
            let text = ron::ser::to_string(&request).unwrap();
            let back: Request = ron::de::from_str(&text).unwrap();
            assert_eq!(format!("{request:?}"), format!("{back:?}"), "{text}");
        }
        assert_eq!(
            run(switch(name("Id(7)"))),
            read("Context(Run(7, switch_context(\"Id(7)\")))")
        );

        let config = Config::default();
        let text = ron::ser::to_string(&Request::UpdateConfig(Config::default())).unwrap();
        let Request::UpdateConfig(back) = ron::de::from_str(&text).unwrap() else {
            panic!("{text}")
        };
        assert_eq!(
            serde_json::to_value(&config).unwrap(),
            serde_json::to_value(&back).unwrap()
        );
    }

    /// A snapshot with every kind of entry on some screen, a context without
    /// a number, names that need escaping, and command results.
    fn snapshot_of_every_kind() -> ContextsSnapshot {
        ContextsSnapshot {
            enabled: true,
            scope: Scope::Global,
            active: ContextKey::Named(context_id(2)),
            screens: vec![
                ScreenContext {
                    id: 1,
                    shows: ContextKey::Named(context_id(2)),
                },
                ScreenContext {
                    id: 2,
                    shows: ContextKey::Everything,
                },
                ScreenContext {
                    id: 4,
                    shows: ContextKey::Unsorted,
                },
            ],
            contexts: vec![
                ContextSummary {
                    id: context_id(1),
                    name: ODD_NAME.into(),
                    number: None,
                    last_used: 0,
                    apps: vec![],
                    windows: 0,
                    members: vec![],
                },
                ContextSummary {
                    id: context_id(2),
                    name: "Café".into(),
                    number: Some(9),
                    last_used: 7,
                    apps: vec!["Zed".into(), "Microsoft Teams (work or school)".into()],
                    windows: 3,
                    members: vec![],
                },
            ],
            unsorted: UnsortedSummary {
                listed: true,
                windows: 4,
                last_used: 6,
            },
            everything: EverythingSummary { last_used: 5 },
            results: vec![result(1, None), result(u64::MAX, Some(ODD_NAME))],
        }
    }

    /// I2. Every reply survives the RON round trip, including a snapshot
    /// with each kind of entry.
    #[test]
    fn every_response_survives_a_ron_round_trip() {
        let (_, snapshot) = contexts();
        let mut everything = snapshot.clone();
        everything.active = ContextKey::Everything;
        everything.screens[0].shows = ContextKey::Everything;
        for response in [
            Response::Pong(ODD_NAME.into()),
            Response::Success,
            Response::Pending,
            Response::Error(ODD_NAME.into()),
            Response::Contexts(snapshot_of_every_kind()),
            Response::Contexts(snapshot.current()),
            Response::Contexts(everything.current()),
            Response::Contexts(ContextsSnapshot::off()),
        ] {
            let text = ron::ser::to_string(&response).unwrap();
            assert_eq!(response, ron::de::from_str::<Response>(&text).unwrap(), "{text}");
        }
    }

    /// I2. A client from before contexts writes the requests it shares with
    /// this server exactly as this server's client does, and reads this
    /// server's replies to them. It pings, pauses, and sends a config.
    #[test]
    fn an_older_client_is_answered_as_before() {
        use RequestBeforeContexts as Old;
        for (old, new) in [
            (Old::Ping(ODD_NAME.into()), Request::Ping(ODD_NAME.into())),
            (
                Old::Service(ServiceRequestBeforeContexts::Install),
                Request::Service(ServiceRequest::Install),
            ),
            (
                Old::Service(ServiceRequestBeforeContexts::Uninstall),
                Request::Service(ServiceRequest::Uninstall),
            ),
            (Old::SetEnabled(true), Request::SetEnabled(true)),
            (Old::SetEnabled(false), Request::SetEnabled(false)),
        ] {
            let text = ron::ser::to_string(&old).unwrap();
            assert_eq!(text, ron::ser::to_string(&new).unwrap());
            let read: Request = ron::de::from_str(&text).unwrap();
            assert_eq!(format!("{new:?}"), format!("{read:?}"));
        }
        for (old, new) in [
            (
                ResponseBeforeContexts::Pong("olleh".into()),
                Response::Pong("olleh".into()),
            ),
            (ResponseBeforeContexts::Success, Response::Success),
            (
                ResponseBeforeContexts::Error("No".into()),
                Response::Error("No".into()),
            ),
        ] {
            assert_eq!(
                ron::ser::to_string(&old).unwrap(),
                ron::ser::to_string(&new).unwrap()
            );
        }

        let (wm_tx, mut wm_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut state = State {
            wm_tx,
            contexts: Box::new(|| None),
        };
        let mut send = |request: Old| -> ResponseBeforeContexts {
            let message = ron::ser::to_string(&request).unwrap();
            let reply = state.handle_message(0, message.as_bytes());
            ron::de::from_bytes(&reply).unwrap()
        };
        assert_eq!(
            ResponseBeforeContexts::Pong("olleh".into()),
            send(Old::Ping("hello".into()))
        );
        assert_eq!(ResponseBeforeContexts::Success, send(Old::SetEnabled(false)));
        assert_eq!(
            ResponseBeforeContexts::Success,
            send(Old::UpdateConfig(Config::default()))
        );
        let (_, event) = wm_rx.try_recv().unwrap();
        assert!(
            matches!(
                event,
                wm_controller::WmEvent::Command(wm_controller::WmCommand::Wm(
                    wm_controller::WmCmd::SetGlobalEnabled(false)
                ))
            ),
            "{event:?}"
        );
        let (_, event) = wm_rx.try_recv().unwrap();
        assert!(
            matches!(event, wm_controller::WmEvent::ConfigUpdated(_)),
            "{event:?}"
        );
        assert!(wm_rx.try_recv().is_err());
    }

    /// I4. A server from before contexts can't read any context request, so
    /// it replies with nothing.
    #[test]
    fn an_older_server_cannot_read_a_context_request() {
        for request in every_context_request() {
            let text = ron::ser::to_string(&Request::Context(request)).unwrap();
            assert!(
                ron::de::from_str::<RequestBeforeContexts>(&text).is_err(),
                "{text}"
            );
        }
    }

    /// I3, I4. The server reads context requests in RON and replies in RON.
    /// `List`, `Current`, and `Result` reply from the snapshot, and `Run`
    /// sends one command. A message it can't read gets an empty reply and
    /// sends nothing. That includes a `Run` without a request id, as the
    /// command line before command results sends it.
    #[test]
    fn the_server_answers_context_requests_in_ron() {
        let (_, snapshot) = contexts();
        let snapshot = ContextsSnapshot {
            results: vec![result(4, Some("No context matches \"x\""))],
            ..snapshot
        };
        let published = Arc::new(snapshot.clone());
        let (wm_tx, mut wm_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut state = State {
            wm_tx,
            contexts: Box::new(move || Some(published.clone())),
        };
        let mut send = |message: &[u8]| state.handle_message(0, message);
        let reply = |bytes: Vec<u8>| ron::de::from_bytes::<Response>(&bytes).unwrap();

        assert_eq!(
            Response::Contexts(snapshot.clone()),
            reply(send(b"Context(List)"))
        );
        assert_eq!(
            Response::Contexts(snapshot.current()),
            reply(send(b"Context(Current)"))
        );
        assert_eq!(
            Response::Success,
            reply(send(b"Context(Run(9, switch_context(\"cli\")))"))
        );
        assert_eq!(Response::Pending, reply(send(b"Context(Result(9))")));
        assert_eq!(
            Response::Error("No context matches \"x\"".into()),
            reply(send(b"Context(Result(4))"))
        );
        for unreadable in [
            &b"Context(Delete)"[..],
            b"Context(Run(switch_context(\"cli\")))",
            b"Context(Run(9, unknown_command(1)))",
            b"Context(Run(9, switch_context(300)))",
            b"Context(Result(-1))",
            b"",
            b"\xff\xfe",
        ] {
            assert!(send(unreadable).is_empty(), "{}", AsciiEscaped(unreadable));
        }

        let (_, event) = wm_rx.try_recv().unwrap();
        let wm_controller::WmEvent::ContextCommandRequested(request, command) = event else {
            panic!("{event:?}");
        };
        assert_eq!((RequestId(9), switch(name("cli"))), (request, command));
        assert!(wm_rx.try_recv().is_err());
    }
}
