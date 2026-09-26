// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `sugarglider context`: lists and switches contexts, the named window
//! sets. The design is in `docs/specs/contexts.md`, section "Command line".

use std::io::Write;

use clap::{Args, Subcommand};
use serde::Serialize;
use sugarglider::actor::contexts_snapshot::{ContextsSnapshot, Scope};
use sugarglider::actor::reactor::{ContextCommand, ContextRef};
use sugarglider::actor::server::{ContextRequest, Request, Response};
use sugarglider::model::contexts::ContextKey;
use sugarglider::sys::message_port::SendError;

/// What the command says when the server replies with nothing, which is how
/// a server that doesn't know contexts answers (I4).
const OLD_SERVER: &str = "The running Sugarglider doesn't support contexts. Restart it.";

#[derive(Subcommand, Clone, Debug, PartialEq)]
pub enum CmdContext {
    /// List the contexts.
    List(Output),
    /// Print the active context.
    Current(Output),
    /// Create a context from the windows on screen, and switch to it.
    Create {
        /// The new context's name.
        name: String,
    },
    /// Switch to a context.
    Switch {
        /// The context's number from 1 to 9, or its name or part of it.
        query: String,
    },
    /// Show every window.
    Everything,
}

#[derive(Args, Clone, Debug, PartialEq)]
pub struct Output {
    /// Print JSON.
    #[arg(long)]
    json: bool,
}

/// Sends a message to the server and returns its reply.
pub trait Transport {
    fn request(&mut self, message: &[u8]) -> Result<Vec<u8>, SendError>;
}

/// Runs the command on the server that `connect` reaches, or fails with
/// `None` when no server runs. Prints the result to `out`, or the reason for
/// a failure to `err`, and returns the exit status.
pub fn run<T: Transport>(
    command: &CmdContext,
    connect: impl FnOnce() -> Option<T>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> u8 {
    match execute(command, connect) {
        Ok(text) => {
            _ = out.write_all(text.as_bytes());
            0
        }
        Err(reason) => {
            _ = writeln!(err, "{reason}");
            1
        }
    }
}

fn execute<T: Transport>(
    command: &CmdContext,
    connect: impl FnOnce() -> Option<T>,
) -> Result<String, String> {
    let request = match command {
        CmdContext::List(_) => ContextRequest::List,
        CmdContext::Current(_) => ContextRequest::Current,
        CmdContext::Create { name } => {
            ContextRequest::Run(ContextCommand::CreateContext(name.clone()))
        }
        CmdContext::Switch { query } => {
            ContextRequest::Run(ContextCommand::SwitchContext(parse_query(query)?))
        }
        CmdContext::Everything => ContextRequest::Run(ContextCommand::ShowEverything),
    };
    let mut transport = connect().ok_or("Sugarglider isn't running.")?;
    match (send(&mut transport, request)?, command) {
        (
            Response::Contexts(snapshot),
            CmdContext::List(Output { json: true }) | CmdContext::Current(Output { json: true }),
        ) => Ok(json(&snapshot)),
        (Response::Contexts(snapshot), CmdContext::List(_)) => Ok(list(&snapshot)),
        (Response::Contexts(snapshot), CmdContext::Current(_)) => {
            Ok(format!("{}\n", name(&snapshot, snapshot.active)))
        }
        (
            Response::Success,
            CmdContext::Create { .. } | CmdContext::Switch { .. } | CmdContext::Everything,
        ) => Ok(String::new()),
        (Response::Error(reason), _) => Err(reason),
        (response, _) => Err(format!("Unexpected reply from Sugarglider: {response:?}")),
    }
}

/// A whole number names a context by its number, from 1 to 9. Any other
/// text is a name, which the server matches with the switcher's ranking.
fn parse_query(query: &str) -> Result<ContextRef, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() || !trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(ContextRef::Name(query.to_string()));
    }
    match trimmed.parse::<u8>() {
        Ok(number @ 1..=9) => Ok(ContextRef::Number(number)),
        _ => Err(format!("Context numbers go from 1 to 9, not {trimmed}")),
    }
}

fn send(transport: &mut impl Transport, request: ContextRequest) -> Result<Response, String> {
    let message = ron::ser::to_string(&Request::Context(request))
        .map_err(|err| format!("Could not write the request: {err}"))?;
    let reply = transport
        .request(message.as_bytes())
        .map_err(|err| format!("Could not reach Sugarglider: {err}"))?;
    if reply.is_empty() {
        return Err(OLD_SERVER.to_string());
    }
    ron::de::from_bytes(&reply)
        .map_err(|err| format!("Could not read the reply from Sugarglider: {err}"))
}

fn name(snapshot: &ContextsSnapshot, key: ContextKey) -> &str {
    snapshot.name(key).unwrap_or("Unknown context")
}

/// One line per entry: a star on the active one, the number, the name, and
/// how many windows are open, with their apps. Unsorted is listed while it
/// has windows or is active.
fn list(snapshot: &ContextsSnapshot) -> String {
    let windows = |count: usize| match count {
        1 => "1 window".to_string(),
        count => format!("{count} windows"),
    };
    let mut rows: Vec<(ContextKey, Option<u8>, String)> = snapshot
        .contexts
        .iter()
        .map(|context| {
            let mut detail = windows(context.windows);
            if !context.apps.is_empty() {
                detail = format!("{detail}  {}", context.apps.join(", "));
            }
            (ContextKey::Named(context.id), context.number, detail)
        })
        .collect();
    if snapshot.unsorted.windows > 0 || snapshot.active == ContextKey::Unsorted {
        rows.push((ContextKey::Unsorted, None, windows(snapshot.unsorted.windows)));
    }
    rows.push((ContextKey::Everything, None, String::new()));
    let width = rows.iter().map(|(key, ..)| name(snapshot, *key).chars().count()).max();
    let width = width.unwrap_or(0);
    let mut text = String::new();
    for (key, number, detail) in rows {
        let marker = if key == snapshot.active { '*' } else { ' ' };
        let number = number.map_or(" ".to_string(), |n| n.to_string());
        let line = format!("{marker} {number} {:<width$}  {detail}", name(snapshot, key));
        text.push_str(line.trim_end());
        text.push('\n');
    }
    text
}

/// The shape in the spec's "Command line" section.
#[derive(Serialize)]
struct SnapshotJson<'a> {
    scope: Scope,
    screens: Vec<ScreenJson<'a>>,
    contexts: Vec<ContextJson<'a>>,
    unsorted: usize,
}

#[derive(Serialize)]
struct ScreenJson<'a> {
    id: u32,
    active: &'a str,
}

#[derive(Serialize)]
struct ContextJson<'a> {
    name: &'a str,
    number: Option<u8>,
    active: bool,
    apps: &'a [String],
    windows: usize,
}

fn json(snapshot: &ContextsSnapshot) -> String {
    let view = SnapshotJson {
        scope: snapshot.scope,
        screens: snapshot
            .screens
            .iter()
            .map(|screen| ScreenJson {
                id: screen.id,
                active: name(snapshot, screen.shows),
            })
            .collect(),
        contexts: snapshot
            .contexts
            .iter()
            .map(|context| ContextJson {
                name: &context.name,
                number: context.number,
                active: snapshot.active == ContextKey::Named(context.id),
                apps: &context.apps,
                windows: context.windows,
            })
            .collect(),
        unsorted: snapshot.unsorted.windows,
    };
    let mut text = serde_json::to_string_pretty(&view).expect("the snapshot serializes");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use sugarglider::actor::contexts_snapshot::{
        ContextSummary, EverythingSummary, ScreenContext, UnsortedSummary,
    };
    use sugarglider::model::contexts::ContextId;

    use super::*;
    use crate::{Command, Opt};

    fn id(id: u32) -> ContextId {
        serde_json::from_value(json!(id)).unwrap()
    }

    fn summary(id: u32, name: &str, apps: &[&str], windows: usize) -> ContextSummary {
        ContextSummary {
            id: self::id(id),
            name: name.into(),
            number: u8::try_from(id).ok(),
            last_used: 0,
            apps: apps.iter().map(|app| app.to_string()).collect(),
            windows,
        }
    }

    /// The spec's example: Comms is active, and 3 windows are unsorted.
    fn snapshot() -> ContextsSnapshot {
        ContextsSnapshot {
            enabled: true,
            scope: Scope::Global,
            active: ContextKey::Named(id(1)),
            screens: vec![ScreenContext {
                id: 1,
                shows: ContextKey::Named(id(1)),
            }],
            contexts: vec![
                summary(1, "Comms", &["WhatsApp", "Microsoft Teams"], 2),
                summary(2, "Relax", &["WhatsApp", "Google Chrome"], 2),
            ],
            unsorted: UnsortedSummary { windows: 3, last_used: 0 },
            everything: EverythingSummary { last_used: 0 },
        }
    }

    fn parse(args: &[&str]) -> Result<CmdContext, clap::Error> {
        let args = ["sugarglider", "context"].iter().chain(args);
        match Opt::try_parse_from(args)?.command {
            Command::Context(command) => Ok(command),
            _ => panic!("not a context command"),
        }
    }

    /// A server that answers every request with the same bytes.
    struct Server {
        reply: Vec<u8>,
        requests: Vec<ContextRequest>,
    }

    impl Transport for &mut Server {
        fn request(&mut self, message: &[u8]) -> Result<Vec<u8>, SendError> {
            match ron::de::from_bytes(message).unwrap() {
                Request::Context(request) => self.requests.push(request),
                other => panic!("{other:?}"),
            }
            Ok(self.reply.clone())
        }
    }

    struct Ran {
        status: u8,
        out: String,
        err: String,
        requests: Vec<ContextRequest>,
    }

    /// Runs `sugarglider context <args>` against a server that replies with
    /// `reply`, or with no server when `reply` is `None`.
    fn run_raw(args: &[&str], reply: Option<Vec<u8>>) -> Ran {
        let command = parse(args).unwrap();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut server = reply.map(|reply| Server { reply, requests: Vec::new() });
        let status = run(&command, || server.as_mut(), &mut out, &mut err);
        Ran {
            status,
            out: String::from_utf8(out).unwrap(),
            err: String::from_utf8(err).unwrap(),
            requests: server.map(|server| server.requests).unwrap_or_default(),
        }
    }

    fn run_with(args: &[&str], reply: Response) -> Ran {
        run_raw(args, Some(ron::ser::to_string(&reply).unwrap().into_bytes()))
    }

    fn switch(reference: ContextRef) -> ContextRequest {
        ContextRequest::Run(ContextCommand::SwitchContext(reference))
    }

    #[test]
    fn subcommands_parse() {
        let output = |json| Output { json };
        for (args, command) in [
            (&["list"][..], CmdContext::List(output(false))),
            (&["list", "--json"], CmdContext::List(output(true))),
            (&["current"], CmdContext::Current(output(false))),
            (&["current", "--json"], CmdContext::Current(output(true))),
            (
                &["create", "Client work"],
                CmdContext::Create { name: "Client work".into() },
            ),
            (&["switch", "2"], CmdContext::Switch { query: "2".into() }),
            (&["everything"], CmdContext::Everything),
        ] {
            assert_eq!(command, parse(args).unwrap(), "{args:?}");
        }
        for args in [
            &["create"][..],
            &["switch"],
            &["switch", "a", "b"],
            &["list", "--yaml"],
            &["add", "Comms"],
        ] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }

    /// A whole number from 1 to 9 is a number, and other text is a name.
    #[test]
    fn a_query_is_a_number_or_a_name() {
        assert_eq!(Ok(ContextRef::Number(2)), parse_query("2"));
        assert_eq!(Ok(ContextRef::Number(9)), parse_query(" 09 "));
        assert_eq!(Ok(ContextRef::Name("cli".into())), parse_query("cli"));
        assert_eq!(Ok(ContextRef::Name("2nd".into())), parse_query("2nd"));
        assert_eq!(Ok(ContextRef::Name("-1".into())), parse_query("-1"));
        for query in ["0", "10", "300"] {
            assert_eq!(
                Err(format!("Context numbers go from 1 to 9, not {query}")),
                parse_query(query)
            );
        }
    }

    #[test]
    fn list_prints_one_line_per_entry() {
        let ran = run_with(&["list"], Response::Contexts(snapshot()));

        assert_eq!(
            "* 1 Comms       2 windows  WhatsApp, Microsoft Teams\n\
            \x20 2 Relax       2 windows  WhatsApp, Google Chrome\n\
            \x20   Unsorted    3 windows\n\
            \x20   Everything\n",
            ran.out
        );
        assert_eq!("", ran.err);
        assert_eq!(0, ran.status);
        assert_eq!(vec![ContextRequest::List], ran.requests);
    }

    /// R29. Unsorted is listed while it has windows or is active.
    #[test]
    fn list_leaves_out_an_empty_unsorted() {
        let mut snapshot = snapshot();
        snapshot.unsorted.windows = 0;
        snapshot.contexts[0].windows = 1;
        snapshot.contexts[0].apps.truncate(1);
        snapshot.contexts[1].number = None;
        assert_eq!(
            "* 1 Comms       1 window  WhatsApp\n\
            \x20   Relax       2 windows  WhatsApp, Google Chrome\n\
            \x20   Everything\n",
            run_with(&["list"], Response::Contexts(snapshot.clone())).out
        );
        snapshot.active = ContextKey::Unsorted;
        assert_eq!(
            "  1 Comms       1 window  WhatsApp\n\
            \x20   Relax       2 windows  WhatsApp, Google Chrome\n\
            *   Unsorted    0 windows\n\
            \x20   Everything\n",
            run_with(&["list"], Response::Contexts(snapshot)).out
        );
    }

    /// The JSON is the shape in the spec's "Command line" section.
    #[test]
    fn list_json_prints_the_spec_shape() {
        let ran = run_with(&["list", "--json"], Response::Contexts(snapshot()));

        let printed: serde_json::Value = serde_json::from_str(&ran.out).unwrap();
        assert_eq!(
            json!({
              "scope": "global",
              "screens": [{ "id": 1, "active": "Comms" }],
              "contexts": [
                { "name": "Comms", "number": 1, "active": true,
                  "apps": ["WhatsApp", "Microsoft Teams"], "windows": 2 },
                { "name": "Relax", "number": 2, "active": false,
                  "apps": ["WhatsApp", "Google Chrome"], "windows": 2 }
              ],
              "unsorted": 3
            }),
            printed
        );
        assert_eq!("", ran.err);
        assert_eq!(0, ran.status);
    }

    #[test]
    fn current_prints_the_active_context() {
        let ran = run_with(&["current"], Response::Contexts(snapshot().current()));
        assert_eq!("Comms\n", ran.out);
        assert_eq!(0, ran.status);
        assert_eq!(vec![ContextRequest::Current], ran.requests);

        let mut everything = snapshot();
        everything.active = ContextKey::Everything;
        everything.screens[0].shows = ContextKey::Everything;
        let ran = run_with(&["current"], Response::Contexts(everything.current()));
        assert_eq!("Everything\n", ran.out);

        let ran = run_with(&["current", "--json"], Response::Contexts(snapshot().current()));
        let printed: serde_json::Value = serde_json::from_str(&ran.out).unwrap();
        assert_eq!(json!("Comms"), printed["contexts"][0]["name"]);
        assert_eq!(1, printed["contexts"].as_array().unwrap().len());
    }

    /// Commands print nothing when the server takes them.
    #[test]
    fn commands_send_their_request_and_print_nothing() {
        for (args, request) in [
            (
                &["create", "Client work"][..],
                ContextRequest::Run(ContextCommand::CreateContext("Client work".into())),
            ),
            (&["switch", "2"], switch(ContextRef::Number(2))),
            (&["switch", "cli"], switch(ContextRef::Name("cli".into()))),
            (
                &["everything"],
                ContextRequest::Run(ContextCommand::ShowEverything),
            ),
        ] {
            let ran = run_with(args, Response::Success);
            assert_eq!(("", "", 0), (&*ran.out, &*ran.err, ran.status), "{args:?}");
            assert_eq!(vec![request], ran.requests);
        }
    }

    /// Failures print the reason to stderr and exit with status 1.
    #[test]
    fn failures_print_the_reason_and_exit_with_1() {
        let error = Response::Error("No context has the number 3".into());
        let not_running = run_raw(&["list"], None);
        let old_server = run_raw(&["list"], Some(Vec::new()));
        let garbage = run_raw(&["current"], Some(b"Nonsense".to_vec()));
        let out_of_range = run_with(&["switch", "12"], Response::Success);
        for (ran, reason) in [
            (
                run_with(&["switch", "3"], error),
                "No context has the number 3\n",
            ),
            (not_running, "Sugarglider isn't running.\n"),
            (
                old_server,
                "The running Sugarglider doesn't support contexts. Restart it.\n",
            ),
            (out_of_range, "Context numbers go from 1 to 9, not 12\n"),
            (
                run_with(&["list"], Response::Success),
                "Unexpected reply from Sugarglider: Success\n",
            ),
        ] {
            assert_eq!((1, "", reason), (ran.status, &*ran.out, &*ran.err));
        }
        assert_eq!(1, garbage.status);
        assert!(garbage.err.starts_with("Could not read the reply from Sugarglider"));
    }

    /// A number out of range fails before anything is sent.
    #[test]
    fn a_bad_number_sends_nothing() {
        assert!(run_with(&["switch", "0"], Response::Success).requests.is_empty());
    }
}
