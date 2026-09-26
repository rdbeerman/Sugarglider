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

use crate::actor::contexts_snapshot::{self, ContextsSnapshot};
use crate::actor::reactor::{self, ContextCommand};
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
    /// Runs a command that switches or changes contexts.
    Run(ContextCommand),
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
                if let Some(command) = command {
                    _ = self.wm_tx.send((
                        Span::current(),
                        wm_controller::WmEvent::Command(wm_controller::WmCommand::ReactorCommand(
                            reactor::Command::Context(command),
                        )),
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
/// returns the command to send to the reactor, which runs it after the
/// reply.
pub(crate) fn answer_context_request(
    request: ContextRequest,
    snapshot: Option<&ContextsSnapshot>,
) -> (Response, Option<ContextCommand>) {
    let Some(snapshot) = snapshot else {
        let reason = "Sugarglider hasn't loaded its contexts yet. Try again in a moment.";
        return (Response::Error(reason.to_string()), None);
    };
    if !snapshot.enabled {
        let reason = "Contexts are off. Turn them on with enable = true under \
                      [settings.experimental.contexts] in the config file.";
        return (Response::Error(reason.to_string()), None);
    }
    match request {
        ContextRequest::List => (Response::Contexts(snapshot.clone()), None),
        ContextRequest::Current => (Response::Contexts(snapshot.current()), None),
        ContextRequest::Run(command) => match snapshot.resolve_command(command) {
            Ok(command) => (Response::Success, Some(command)),
            Err(reason) => (Response::Error(reason), None),
        },
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
        ContextRequest::Run(command)
    }

    fn switch(reference: ContextRef) -> ContextCommand {
        ContextCommand::SwitchContext(reference)
    }

    fn name(name: &str) -> ContextRef {
        ContextRef::Name(name.to_string())
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

    fn id(contexts: &Contexts, name: &str) -> ContextId {
        contexts.by_name(name).unwrap().id
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
        ] {
            let text = ron::ser::to_string(&Request::Context(request.clone())).unwrap();
            assert_eq!(request, read(&text), "{text}");
        }
        assert_eq!(ContextRequest::List, read("Context(List)"));
        assert_eq!(
            run(switch(ContextRef::Number(7))),
            read("Context(Run(switch_context(7)))")
        );
        assert_eq!(
            run(switch(name("7"))),
            read("Context(Run(switch_context(\"7\")))")
        );
        assert_eq!(
            run(switch(ContextRef::Id(seven))),
            read("Context(Run(switch_context(Id(7))))")
        );
        assert_eq!(
            run(ContextCommand::CreateContext("X".into())),
            read("Context(Run(create_context(\"X\")))")
        );
    }

    /// I2. The replies survive the RON round trip, the snapshot included.
    #[test]
    fn responses_survive_a_ron_round_trip() {
        let (_, snapshot) = contexts();
        for response in [
            Response::Success,
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

    /// I3. `Run` replies at once and sends the command with the context the
    /// snapshot resolves. A name that matches nothing goes to the reactor
    /// as it is.
    #[test]
    fn run_sends_the_command_with_the_resolved_context() {
        let (contexts, snapshot) = contexts();
        let client = ContextRef::Id(id(&contexts, "Client work"));
        let answer = |command| answer_context_request(run(command), Some(&snapshot));

        for (command, sent) in [
            (switch(name("cli")), switch(client.clone())),
            (switch(ContextRef::Number(2)), switch(client.clone())),
            (switch(name("New")), switch(name("New"))),
            (switch(name("unsorted")), switch(name("Unsorted"))),
            (switch(name("everything")), ContextCommand::ShowEverything),
            (ContextCommand::ShowEverything, ContextCommand::ShowEverything),
            (
                ContextCommand::CreateContext("New".into()),
                ContextCommand::CreateContext("New".into()),
            ),
        ] {
            assert_eq!((Response::Success, Some(sent)), answer(command));
        }
    }

    /// I3. A lookup that fails other than by name, and a name that R4
    /// refuses, reply with the reason and send nothing.
    #[test]
    fn run_replies_with_the_reason_when_the_command_cant_run() {
        let (_, snapshot) = contexts();
        let answer = |command| answer_context_request(run(command), Some(&snapshot));
        let error = |reason: &str| (Response::Error(reason.to_string()), None);

        assert_eq!(
            error("No context has the number 3"),
            answer(switch(ContextRef::Number(3)))
        );
        assert_eq!(
            error("A context named \"Comms\" already exists"),
            answer(ContextCommand::CreateContext("comms".into()))
        );
        assert_eq!(
            error("\"Everything\" is a reserved name"),
            answer(ContextCommand::CreateContext("Everything".into()))
        );
    }

    /// I3. The server sends a resolved command to the window manager, which
    /// passes it to the reactor. A read sends nothing.
    #[test]
    fn run_sends_the_command_to_the_window_manager() {
        let (contexts, snapshot) = contexts();
        let snapshot = Arc::new(snapshot);
        let (wm_tx, mut wm_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut state = State {
            wm_tx,
            contexts: Box::new(move || Some(snapshot.clone())),
        };

        let response = state.on_request(Request::Context(run(switch(name("cli")))));

        assert_eq!(Response::Success, response);
        let (_, event) = wm_rx.try_recv().unwrap();
        let wm_controller::WmEvent::Command(wm_controller::WmCommand::ReactorCommand(
            reactor::Command::Context(command),
        )) = event
        else {
            panic!("{event:?}");
        };
        assert_eq!(switch(ContextRef::Id(id(&contexts, "Client work"))), command);
        let response = state.on_request(Request::Context(ContextRequest::List));
        assert!(matches!(response, Response::Contexts(_)), "{response:?}");
        assert!(wm_rx.try_recv().is_err());
    }

    /// R28. With contexts off, or before the reactor published anything,
    /// every request fails and sends nothing.
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
            assert!(
                matches!(&response, Response::Error(reason) if reason.starts_with("Contexts are off.")),
                "{response:?}"
            );
            let (response, sent) = answer_context_request(request, None);
            assert_eq!(None, sent);
            assert!(matches!(response, Response::Error(_)), "{response:?}");
        }
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
            read("Context(Run(switch_context(\"Id(7)\")))")
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
    /// a number, and names that need escaping.
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
                },
                ContextSummary {
                    id: context_id(2),
                    name: "Café".into(),
                    number: Some(9),
                    last_used: 7,
                    apps: vec!["Zed".into(), "Microsoft Teams (work or school)".into()],
                    windows: 3,
                },
            ],
            unsorted: UnsortedSummary { windows: 4, last_used: 6 },
            everything: EverythingSummary { last_used: 5 },
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

    /// I3. The server reads context requests in RON and replies in RON.
    /// `List` and `Current` reply from the snapshot, and `Run` sends one
    /// command. A message it can't read gets an empty reply and sends
    /// nothing.
    #[test]
    fn the_server_answers_context_requests_in_ron() {
        let (contexts, snapshot) = contexts();
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
            Response::Error("No context has the number 5".into()),
            reply(send(b"Context(Run(switch_context(5)))"))
        );
        assert_eq!(
            Response::Success,
            reply(send(b"Context(Run(switch_context(\"cli\")))"))
        );
        for unreadable in [
            &b"Context(Delete)"[..],
            b"Context(Run(delete_context(1)))",
            b"Context(Run(switch_context(300)))",
            b"",
            b"\xff\xfe",
        ] {
            assert!(send(unreadable).is_empty(), "{}", AsciiEscaped(unreadable));
        }

        let (_, event) = wm_rx.try_recv().unwrap();
        let wm_controller::WmEvent::Command(wm_controller::WmCommand::ReactorCommand(
            reactor::Command::Context(command),
        )) = event
        else {
            panic!("{event:?}");
        };
        assert_eq!(switch(ContextRef::Id(id(&contexts, "Client work"))), command);
        assert!(wm_rx.try_recv().is_err());
    }

    /// Comms (1, active), Client work (2), and Relax (no number), with 2
    /// unsorted windows, and the id of a deleted context.
    fn three_contexts() -> (Contexts, ContextsSnapshot, ContextId) {
        let mut contexts = Contexts::new();
        let comms = contexts.create("Comms").unwrap();
        contexts.create("Client work").unwrap();
        let relax = contexts.create("Relax").unwrap();
        let gone = contexts.create("Gone").unwrap();
        contexts.delete(gone).unwrap();
        contexts.set_number(relax, None).unwrap();
        contexts.switch_to(ContextKey::Named(comms)).unwrap();
        let screens = vec![ScreenContext {
            id: 1,
            shows: ContextKey::Named(comms),
        }];
        let snapshot = ContextsSnapshot::new(&contexts, screens, 2);
        (contexts, snapshot, gone)
    }

    /// I3, R4, R29. How the server resolves each form of reference against
    /// the snapshot, and which names it refuses for a new context. A name
    /// that matches nothing goes to the reactor as it is, and every other
    /// failed lookup fails at once.
    #[test]
    fn run_resolves_each_form_of_reference_as_this_table_gives() {
        let (contexts, snapshot, gone) = three_contexts();
        let comms = switch(ContextRef::Id(id(&contexts, "Comms")));
        let client = switch(ContextRef::Id(id(&contexts, "Client work")));
        let relax = switch(ContextRef::Id(id(&contexts, "Relax")));
        let unsorted = switch(name("Unsorted"));
        let everything = ContextCommand::ShowEverything;
        let create = |name: &str| ContextCommand::CreateContext(name.to_string());
        let no_name = "Give the name or the number of a context";
        let taken = "A context named \"Comms\" already exists";
        let table: Vec<(ContextCommand, Result<ContextCommand, &str>)> = vec![
            (switch(ContextRef::Number(1)), Ok(comms.clone())),
            (switch(ContextRef::Number(2)), Ok(client.clone())),
            (switch(ContextRef::Number(3)), Err("No context has the number 3")),
            (switch(ContextRef::Number(0)), Err("No context has the number 0")),
            (switch(ContextRef::Number(9)), Err("No context has the number 9")),
            (switch(ContextRef::Id(id(&contexts, "Relax"))), Ok(relax.clone())),
            (switch(ContextRef::Id(gone)), Err("No such context")),
            (switch(name("Comms")), Ok(comms.clone())),
            (switch(name("  COMMS  ")), Ok(comms.clone())),
            (switch(name("cómms")), Ok(comms.clone())),
            (switch(name("Client work")), Ok(client.clone())),
            (switch(name("cli")), Ok(client.clone())),
            (switch(name("cw")), Ok(client.clone())),
            (switch(name("work")), Ok(client.clone())),
            (switch(name("rlx")), Ok(relax.clone())),
            (switch(name("Everything")), Ok(everything.clone())),
            (switch(name("ÉVERYTHING")), Ok(everything.clone())),
            (switch(name("Unsorted")), Ok(unsorted.clone())),
            (switch(name("uns")), Ok(unsorted.clone())),
            (switch(name("Sugarglider")), Ok(switch(name("Sugarglider")))),
            (switch(name("7")), Ok(switch(name("7")))),
            (switch(name("--")), Ok(switch(name("--")))),
            (switch(name("")), Err(no_name)),
            (switch(name(" \t ")), Err(no_name)),
            (everything.clone(), Ok(everything.clone())),
            (
                ContextCommand::PreviousContext,
                Ok(ContextCommand::PreviousContext),
            ),
            (create("Work"), Ok(create("Work"))),
            (create(ODD_NAME), Ok(create(ODD_NAME))),
            (create("comms"), Err(taken)),
            (create(" Cómms "), Err(taken)),
            (create("everything"), Err("\"everything\" is a reserved name")),
            (create(" UNSORTED "), Err("\"UNSORTED\" is a reserved name")),
            (create(""), Err("A context name can't be empty")),
            (create(" \t "), Err("A context name can't be empty")),
        ];
        for (command, expected) in table {
            let expected = match expected {
                Ok(sent) => (Response::Success, Some(sent)),
                Err(reason) => (Response::Error(reason.to_string()), None),
            };
            assert_eq!(
                expected,
                answer_context_request(run(command.clone()), Some(&snapshot)),
                "{command:?}"
            );
        }
    }
}
