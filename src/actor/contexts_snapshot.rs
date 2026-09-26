// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The contexts as threads other than the reactor's see them.
//!
//! The reactor owns the contexts. After every change it publishes a
//! [`ContextsSnapshot`] here, and the message server answers the command line
//! from the snapshot without waiting for the reactor. The design is in
//! `docs/specs/contexts.md`, section "IPC".

use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};

use crate::model::contexts::{
    Context, ContextId, ContextKey, Contexts, EVERYTHING_NAME, MemberRecord, UNSORTED_NAME,
};

/// The app name that stands in when a record names no app.
pub const UNKNOWN_APP: &str = "Unknown app";

/// Why a context request or command does nothing while contexts are off.
pub const CONTEXTS_OFF: &str = "Contexts are off. Turn them on with enable = true under \
                                [settings.experimental.contexts] in the config file.";

/// How many command results the snapshot keeps, the newest ones.
pub const MAX_COMMAND_RESULTS: usize = 32;

/// Names a command that a client sent, so that it can ask for the command's
/// result. The client picks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub u64);

/// The result of a command that a client sent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandResult {
    pub request: RequestId,
    /// Why the command did nothing, or `None` when it ran.
    pub error: Option<String>,
}

/// Which screens a switch changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// A switch changes every screen.
    #[default]
    Global,
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "ContextKey")]
enum ContextKeyDef {
    Everything,
    Unsorted,
    Named(ContextId),
}

/// The contexts, as the reactor last published them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextsSnapshot {
    /// Whether contexts are on. When they are off, nothing else is filled in.
    pub enabled: bool,
    pub scope: Scope,
    /// The active context.
    #[serde(with = "ContextKeyDef")]
    pub active: ContextKey,
    /// What each visible screen shows, main screen first.
    pub screens: Vec<ScreenContext>,
    /// Every named context, in the model's order.
    pub contexts: Vec<ContextSummary>,
    pub unsorted: UnsortedSummary,
    pub everything: EverythingSummary,
    /// The results of the last commands that clients sent with a request id,
    /// oldest first, at most [`MAX_COMMAND_RESULTS`].
    pub results: Vec<CommandResult>,
}

/// The context a visible screen shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenContext {
    /// The screen's position in the reactor's list of screens, from 1 for the
    /// main screen.
    pub id: u32,
    /// The active context, or Everything while the screen's Space shows every
    /// window, for example during a quit.
    #[serde(with = "ContextKeyDef")]
    pub shows: ContextKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSummary {
    pub id: ContextId,
    pub name: String,
    pub number: Option<u8>,
    /// The use number of the last switch to the context. Larger is later.
    pub last_used: u64,
    /// The app names of the open member windows, each once, in member order.
    pub apps: Vec<String>,
    /// How many member windows are open.
    pub windows: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsortedSummary {
    /// How many tracked windows on the visible Spaces are in no named context
    /// and not pinned.
    pub windows: usize,
    /// The use number of the last switch to Unsorted.
    pub last_used: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EverythingSummary {
    /// The use number of the last switch to Everything.
    pub last_used: u64,
}

impl ContextsSnapshot {
    /// The snapshot while contexts are off.
    pub fn off() -> Self {
        ContextsSnapshot {
            enabled: false,
            scope: Scope::Global,
            active: ContextKey::Everything,
            screens: Vec::new(),
            contexts: Vec::new(),
            unsorted: UnsortedSummary::default(),
            everything: EverythingSummary::default(),
            results: Vec::new(),
        }
    }

    /// The snapshot of `contexts`, with the facts that only the reactor
    /// knows: what each visible screen shows, and how many windows are
    /// unsorted.
    pub fn new(contexts: &Contexts, screens: Vec<ScreenContext>, unsorted_windows: usize) -> Self {
        ContextsSnapshot {
            enabled: true,
            scope: Scope::Global,
            active: contexts.active(),
            screens,
            contexts: contexts.contexts().iter().map(ContextSummary::new).collect(),
            unsorted: UnsortedSummary {
                windows: unsorted_windows,
                last_used: contexts.last_used(ContextKey::Unsorted),
            },
            everything: EverythingSummary {
                last_used: contexts.last_used(ContextKey::Everything),
            },
            results: Vec::new(),
        }
    }

    pub fn get(&self, id: ContextId) -> Option<&ContextSummary> {
        self.contexts.iter().find(|context| context.id == id)
    }

    /// The name of an entry. `None` for a named context the snapshot doesn't
    /// have.
    pub fn name(&self, key: ContextKey) -> Option<&str> {
        match key {
            ContextKey::Everything => Some(EVERYTHING_NAME),
            ContextKey::Unsorted => Some(UNSORTED_NAME),
            ContextKey::Named(id) => self.get(id).map(|context| context.name.as_str()),
        }
    }

    /// The snapshot with only the named contexts that are active or that a
    /// screen shows.
    pub fn current(&self) -> Self {
        let shown = |id: ContextId| {
            let key = ContextKey::Named(id);
            self.active == key || self.screens.iter().any(|screen| screen.shows == key)
        };
        ContextsSnapshot {
            contexts: self.contexts.iter().filter(|c| shown(c.id)).cloned().collect(),
            ..self.clone()
        }
    }

    /// The result of the command sent with `request`, while the snapshot
    /// keeps it.
    pub fn result(&self, request: RequestId) -> Option<&CommandResult> {
        self.results.iter().rev().find(|result| result.request == request)
    }
}

impl ContextSummary {
    fn new(context: &Context) -> Self {
        let open: Vec<&MemberRecord> =
            context.members.iter().filter(|record| record.window().is_some()).collect();
        let mut apps: Vec<String> = Vec::new();
        for record in &open {
            let app = app_name(record);
            if !apps.contains(&app) {
                apps.push(app);
            }
        }
        ContextSummary {
            id: context.id,
            name: context.name.clone(),
            number: context.number,
            last_used: context.last_used,
            apps,
            windows: open.len(),
        }
    }
}

/// The record's app name, or its bundle id when the name is unknown, or
/// [`UNKNOWN_APP`].
pub fn app_name(record: &MemberRecord) -> String {
    record
        .app_name
        .as_deref()
        .or(record.bundle_id.as_deref())
        .unwrap_or(UNKNOWN_APP)
        .to_string()
}

static PUBLISHED: OnceLock<RwLock<Arc<ContextsSnapshot>>> = OnceLock::new();

/// Makes `snapshot` the one that [`published`] returns. The reactor calls
/// this after every change.
pub fn publish(snapshot: Arc<ContextsSnapshot>) {
    let mut snapshot = Some(snapshot);
    let lock = PUBLISHED.get_or_init(|| RwLock::new(snapshot.take().expect("not taken yet")));
    if let Some(snapshot) = snapshot {
        *lock.write().unwrap() = snapshot;
    }
}

/// The snapshot the reactor published last, or `None` before the first.
pub fn published() -> Option<Arc<ContextsSnapshot>> {
    PUBLISHED.get().map(|lock| lock.read().unwrap().clone())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::actor::app::WindowId;
    use crate::model::contexts::WindowDesc;

    fn window(pid: i32, idx: u32, bundle_id: Option<&str>, app_name: Option<&str>) -> WindowDesc {
        WindowDesc {
            wid: WindowId::new(pid, idx),
            bundle_id: bundle_id.map(str::to_string),
            app_name: app_name.map(str::to_string),
            title: format!("Window{idx}"),
            window_server_id: None,
        }
    }

    fn named(contexts: &Contexts, name: &str) -> ContextKey {
        ContextKey::Named(contexts.by_name(name).unwrap().id)
    }

    fn snapshot(contexts: &Contexts, unsorted_windows: usize) -> ContextsSnapshot {
        let screens = vec![ScreenContext {
            id: 1,
            shows: contexts.active(),
        }];
        ContextsSnapshot::new(contexts, screens, unsorted_windows)
    }

    /// Comms (1), Community (2), and Client work (3), none of them used.
    fn three() -> Contexts {
        let mut contexts = Contexts::new();
        for name in ["Comms", "Community", "Client work"] {
            contexts.create(name).unwrap();
        }
        contexts
    }

    /// The apps are the open members' apps, each once, by name, bundle id,
    /// or "Unknown app". Closed and gone windows count toward neither the
    /// apps nor the windows.
    #[test]
    fn a_context_lists_the_apps_and_the_number_of_its_open_windows() {
        let mut contexts = Contexts::new();
        let id = contexts.create("Comms").unwrap();
        let members = [
            window(1, 1, Some("net.whatsapp.WhatsApp"), Some("WhatsApp")),
            window(2, 2, Some("com.microsoft.teams2"), None),
            window(1, 3, Some("net.whatsapp.WhatsApp"), Some("WhatsApp")),
            window(3, 4, None, None),
            window(4, 5, Some("com.apple.mail"), Some("Mail")),
            window(5, 6, Some("com.apple.iCal"), Some("Calendar")),
        ];
        for member in &members {
            contexts.add_window(id, member).unwrap();
        }
        contexts.window_closed(WindowId::new(4, 5));
        contexts.window_closed(WindowId::new(5, 6));
        contexts.app_terminated(5);
        contexts.set_number(id, Some(3)).unwrap();

        let snapshot = snapshot(&contexts, 2);

        assert_eq!(
            vec![ContextSummary {
                id,
                name: "Comms".into(),
                number: Some(3),
                last_used: 0,
                apps: vec![
                    "WhatsApp".into(),
                    "com.microsoft.teams2".into(),
                    UNKNOWN_APP.into()
                ],
                windows: 4,
            }],
            snapshot.contexts
        );
        assert!(snapshot.enabled);
        assert_eq!(ContextKey::Everything, snapshot.active);
        assert_eq!(2, snapshot.unsorted.windows);
    }

    #[test]
    fn current_keeps_only_the_active_context() {
        let mut contexts = three();
        let community = named(&contexts, "Community");
        contexts.switch_to(community).unwrap();
        let snapshot = snapshot(&contexts, 2);

        let current = snapshot.current();

        assert_eq!(vec![snapshot.contexts[1].clone()], current.contexts);
        assert_eq!(
            ContextsSnapshot {
                contexts: snapshot.contexts.clone(),
                ..current.clone()
            },
            snapshot
        );
        contexts.switch_to(ContextKey::Unsorted).unwrap();
        assert!(self::snapshot(&contexts, 2).current().contexts.is_empty());
    }

    #[test]
    fn names_of_entries() {
        let contexts = three();
        let snapshot = snapshot(&contexts, 0);
        assert_eq!(Some("Everything"), snapshot.name(ContextKey::Everything));
        assert_eq!(Some("Unsorted"), snapshot.name(ContextKey::Unsorted));
        assert_eq!(Some("Community"), snapshot.name(named(&contexts, "Community")));
    }

    /// A command's result is found by its request id. When two results have
    /// the same id, the newer one counts.
    #[test]
    fn a_result_is_found_by_its_request_id() {
        let result = |request: u64, error: Option<&str>| CommandResult {
            request: RequestId(request),
            error: error.map(str::to_string),
        };
        let snapshot = ContextsSnapshot {
            results: vec![
                result(7, Some("No context matches \"x\"")),
                result(8, None),
                result(7, None),
            ],
            ..snapshot(&three(), 0)
        };

        assert_eq!(Some(&result(8, None)), snapshot.result(RequestId(8)));
        assert_eq!(Some(&result(7, None)), snapshot.result(RequestId(7)));
        assert_eq!(None, snapshot.result(RequestId(9)));
    }

    #[test]
    fn the_snapshot_survives_a_ron_round_trip() {
        let mut contexts = three();
        contexts.switch_to(ContextKey::Unsorted).unwrap();
        let with_results = ContextsSnapshot {
            results: vec![
                CommandResult {
                    request: RequestId(1),
                    error: None,
                },
                CommandResult {
                    request: RequestId(u64::MAX),
                    error: Some("No context matches \"x\"".into()),
                },
            ],
            ..snapshot(&contexts, 3)
        };
        for snapshot in [
            ContextsSnapshot::off(),
            snapshot(&contexts, 3),
            with_results,
        ] {
            let ron = ron::ser::to_string(&snapshot).unwrap();
            assert_eq!(snapshot, ron::de::from_str::<ContextsSnapshot>(&ron).unwrap());
        }
    }

    /// I1. The only test that touches the process-wide snapshot.
    #[test]
    fn the_published_snapshot_is_the_last_one() {
        let first = Arc::new(snapshot(&three(), 1));
        let second = Arc::new(ContextsSnapshot::off());
        publish(first.clone());
        assert_eq!(Some(first), published());
        publish(second.clone());
        assert_eq!(Some(second), published());
    }
}
