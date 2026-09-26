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
    use crate::actor::contexts_snapshot::ScreenContext;
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
}
