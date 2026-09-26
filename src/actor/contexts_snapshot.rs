// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The contexts as threads other than the reactor's see them.
//!
//! The reactor owns the contexts. After every change it publishes a
//! [`ContextsSnapshot`] here, and the message server answers the command line
//! from the snapshot without waiting for the reactor. The design is in
//! `docs/specs/contexts.md`, section "IPC".
//!
//! The command line and the server can be different versions, so every field
//! but an id has a default: a reader fills in the fields that a writer of
//! another version left out, and skips the fields it doesn't know.

use std::sync::{Arc, OnceLock, PoisonError, RwLock};

use serde::{Deserialize, Serialize};

use crate::actor::app::WindowId;
use crate::model::contexts::{
    Context, ContextError, ContextId, ContextKey, Contexts, EVERYTHING_NAME, MemberRecord, NameMatch,
    Query, UNSORTED_NAME, rank_entries, resolve_name,
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
    #[serde(default)]
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
#[serde(default)]
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
#[serde(default)]
pub struct ScreenContext {
    /// The screen's position in the reactor's list of screens, from 1 for the
    /// main screen. It changes when displays are added, removed, or
    /// rearranged. Per-screen scope (M9) may name screens by display id
    /// instead.
    pub id: u32,
    /// The active context, or Everything while the screen's Space shows every
    /// window, for example during a quit.
    #[serde(with = "ContextKeyDef")]
    pub shows: ContextKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextSummary {
    pub id: ContextId,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub number: Option<u8>,
    /// The use number of the last switch to the context. Larger is later.
    #[serde(default)]
    pub last_used: u64,
    /// The app names of the open member windows, each once, in member order,
    /// then those of the open pinned windows.
    #[serde(default)]
    pub apps: Vec<String>,
    /// How many member windows are open, wherever they are: on another
    /// Space, minimized, or parked. Pinned windows are members of every
    /// context and count too.
    #[serde(default)]
    pub windows: usize,
    /// Every member record, in the model's order. This is what the switcher
    /// edits, and what `sugarglider context forget` names by index.
    #[serde(default)]
    pub members: Vec<MemberSummary>,
}

/// One member record of a context, as the command line and the switcher see
/// it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MemberSummary {
    /// The record's index in `Context.members`, which the edit command and
    /// `sugarglider context forget` take.
    pub record: usize,
    /// The record's app name, or its bundle id when the name is unknown, or
    /// [`UNKNOWN_APP`].
    pub app: String,
    pub title: String,
    /// The record's open window, or `None` when its window is gone: the
    /// record is empty or pending (R23).
    pub window: Option<WindowId>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UnsortedSummary {
    /// Whether Unsorted is listed among the contexts to switch to: while it
    /// has windows. The menu, the command line, and the switcher list it
    /// by this field.
    pub listed: bool,
    /// How many tracked windows on the visible Spaces, parked ones included,
    /// are in no named context and not pinned. Unlike a context's count, it
    /// leaves out minimized windows and windows on other Spaces. It is 0
    /// until the first context exists.
    pub windows: usize,
    /// The use number of the last switch to Unsorted.
    pub last_used: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EverythingSummary {
    /// The use number of the last switch to Everything.
    pub last_used: u64,
}

impl Default for ContextsSnapshot {
    fn default() -> Self {
        Self::off()
    }
}

impl Default for ScreenContext {
    fn default() -> Self {
        ScreenContext {
            id: 0,
            shows: ContextKey::Everything,
        }
    }
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
    /// unsorted. Unsorted is listed while that number isn't 0.
    pub fn new(contexts: &Contexts, screens: Vec<ScreenContext>, unsorted_windows: usize) -> Self {
        let summary = |context| ContextSummary::new(context, contexts.pinned());
        ContextsSnapshot {
            enabled: true,
            scope: Scope::Global,
            active: contexts.active(),
            screens,
            contexts: contexts.contexts().iter().map(summary).collect(),
            unsorted: UnsortedSummary {
                listed: unsorted_windows > 0,
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

    /// What the managed screens show: the first screen's entry that isn't
    /// Everything, or Everything while every screen shows it. `None` while
    /// no screen shows a managed Space, and while contexts are off. The
    /// status title and the menu's checkmark follow it.
    pub fn shown(&self) -> Option<ContextKey> {
        if !self.enabled || self.screens.is_empty() {
            return None;
        }
        let mut shown = self.screens.iter().map(|screen| screen.shows);
        let context = shown.find(|key| *key != ContextKey::Everything);
        Some(context.unwrap_or(ContextKey::Everything))
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

    /// Ranks the entries for a query, best first, as [`crate::model::contexts::rank`]
    /// ranks the live contexts: the named contexts, Unsorted while it is
    /// listed, and Everything. Ties go to the most recently used entry.
    pub fn rank(&self, query: &str) -> Vec<(ContextKey, NameMatch)> {
        let mut entries: Vec<(ContextKey, &str, u64)> = self
            .contexts
            .iter()
            .map(|context| {
                (ContextKey::Named(context.id), context.name.as_str(), context.last_used)
            })
            .collect();
        if self.unsorted.listed {
            entries.push((
                ContextKey::Unsorted,
                UNSORTED_NAME,
                self.unsorted.last_used,
            ));
        }
        entries.push((
            ContextKey::Everything,
            EVERYTHING_NAME,
            self.everything.last_used,
        ));
        rank_entries(query, entries)
    }

    /// The entry that a query names, as [`crate::model::contexts::resolve`]
    /// names it against the live contexts. A client resolves a query here
    /// before it sends a command that needs to name a record, so a snapshot
    /// that lags behind the reactor fails instead of naming another record.
    pub fn resolve(&self, query: Query<'_>) -> Result<ContextKey, ContextError> {
        match query {
            Query::Number(number) => self
                .contexts
                .iter()
                .find(|context| context.number == Some(number))
                .map(|context| ContextKey::Named(context.id))
                .ok_or(ContextError::NoContextNumbered(number)),
            Query::Id(id) => self
                .get(id)
                .map(|context| ContextKey::Named(context.id))
                .ok_or(ContextError::NoSuchContext),
            Query::Name(name) if name.trim().is_empty() => Err(ContextError::NoQuery),
            Query::Name(name) => resolve_name(
                name,
                self.rank(name),
                self.unsorted.listed && !self.contexts.is_empty(),
            ),
        }
    }
}

impl ContextSummary {
    /// The summary of `context`, whose members include the `pinned`
    /// windows.
    pub(crate) fn new(context: &Context, pinned: &[MemberRecord]) -> Self {
        let mut open: Vec<WindowId> = Vec::new();
        let mut apps: Vec<String> = Vec::new();
        for record in context.members.iter().chain(pinned) {
            let Some(wid) = record.window() else { continue };
            if open.contains(&wid) {
                continue;
            }
            open.push(wid);
            let app = app_name(record);
            if !apps.contains(&app) {
                apps.push(app);
            }
        }
        let members = context
            .members
            .iter()
            .enumerate()
            .map(|(record, member)| MemberSummary {
                record,
                app: app_name(member),
                title: member.title.clone(),
                window: member.window(),
            })
            .collect();
        ContextSummary {
            id: context.id,
            name: context.name.clone(),
            number: context.number,
            last_used: context.last_used,
            apps,
            windows: open.len(),
            members,
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
        *lock.write().unwrap_or_else(PoisonError::into_inner) = snapshot;
    }
}

/// The snapshot the reactor published last, or `None` before the first.
/// The message server calls this on the main thread, so it never panics.
pub fn published() -> Option<Arc<ContextsSnapshot>> {
    PUBLISHED
        .get()
        .map(|lock| lock.read().unwrap_or_else(PoisonError::into_inner).clone())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::model::contexts::{Query, WindowDesc, rank, resolve};

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
    /// apps nor the windows. Every record, open or not, is listed with the
    /// index that names it.
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
                members: vec![
                    MemberSummary {
                        record: 0,
                        app: "WhatsApp".into(),
                        title: "Window1".into(),
                        window: Some(WindowId::new(1, 1)),
                    },
                    MemberSummary {
                        record: 1,
                        app: "com.microsoft.teams2".into(),
                        title: "Window2".into(),
                        window: Some(WindowId::new(2, 2)),
                    },
                    MemberSummary {
                        record: 2,
                        app: "WhatsApp".into(),
                        title: "Window3".into(),
                        window: Some(WindowId::new(1, 3)),
                    },
                    MemberSummary {
                        record: 3,
                        app: UNKNOWN_APP.into(),
                        title: "Window4".into(),
                        window: Some(WindowId::new(3, 4)),
                    },
                    MemberSummary {
                        record: 4,
                        app: "Mail".into(),
                        title: "Window5".into(),
                        window: None,
                    },
                    MemberSummary {
                        record: 5,
                        app: "Calendar".into(),
                        title: "Window6".into(),
                        window: None,
                    },
                ],
            }],
            snapshot.contexts
        );
        assert!(snapshot.enabled);
        assert_eq!(ContextKey::Everything, snapshot.active);
        assert_eq!(2, snapshot.unsorted.windows);
        assert!(snapshot.unsorted.listed);
    }

    /// R3. A pinned window is a member of every context, so its window and
    /// its app count in every context, and once in a context that also has
    /// a record of it. A closed pinned window doesn't count.
    #[test]
    fn a_pinned_window_counts_in_every_context() {
        let mut contexts = Contexts::new();
        let comms = contexts.create("Comms").unwrap();
        let empty = contexts.create("Empty").unwrap();
        let mail = window(1, 1, Some("com.apple.mail"), Some("Mail"));
        let music = window(2, 1, Some("com.apple.Music"), Some("Music"));
        let notes = window(3, 1, Some("com.apple.Notes"), Some("Notes"));
        contexts.add_window(comms, &mail).unwrap();
        contexts.add_window(comms, &music).unwrap();
        contexts.pin(&music);
        contexts.pin(&notes);
        contexts.window_closed(notes.wid);

        let snapshot = snapshot(&contexts, 0);

        let counts: Vec<(ContextId, Vec<&str>, usize)> = snapshot
            .contexts
            .iter()
            .map(|c| (c.id, c.apps.iter().map(String::as_str).collect(), c.windows))
            .collect();
        assert_eq!(
            vec![(comms, vec!["Mail", "Music"], 2), (empty, vec!["Music"], 1)],
            counts
        );
        assert!(!snapshot.unsorted.listed);
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

    /// What the screens show, for the status title and the menu: the first
    /// screen that shows a context other than Everything, else Everything,
    /// and nothing while no screen shows a managed Space or contexts are
    /// off. The active context doesn't count, because it may not show.
    #[test]
    fn the_shown_entry_is_the_first_one_that_a_screen_shows_other_than_everything() {
        let contexts = three();
        let comms = named(&contexts, "Comms");
        let client = named(&contexts, "Client work");
        let on = |shows: &[ContextKey]| ContextsSnapshot {
            active: comms,
            screens: shows
                .iter()
                .zip(1..)
                .map(|(&shows, id)| ScreenContext { id, shows })
                .collect(),
            ..snapshot(&contexts, 0)
        };
        let everything = ContextKey::Everything;

        assert_eq!(None, on(&[]).shown());
        assert_eq!(Some(everything), on(&[everything]).shown());
        assert_eq!(Some(everything), on(&[everything, everything]).shown());
        assert_eq!(Some(comms), on(&[comms]).shown());
        assert_eq!(Some(comms), on(&[everything, comms]).shown());
        assert_eq!(Some(client), on(&[client, comms]).shown());
        assert_eq!(
            Some(ContextKey::Unsorted),
            on(&[everything, ContextKey::Unsorted]).shown()
        );
        let off = ContextsSnapshot { enabled: false, ..on(&[comms]) };
        assert_eq!(None, off.shown());
    }

    /// A snapshot resolves a query as the model resolves it against the
    /// live contexts. A client resolves a name against the snapshot before
    /// it sends a command that names a record, so the two must agree on
    /// numbers, ids, exact and partial names, Unsorted while it is listed,
    /// and the reasons for failure.
    #[test]
    fn a_query_resolves_against_the_snapshot_as_it_does_against_the_contexts() {
        let mut contexts = three();
        let comms = contexts.by_name("Comms").unwrap().id;
        let client = contexts.by_name("Client work").unwrap().id;
        contexts.switch_to(ContextKey::Named(client)).unwrap();
        let missing: ContextId = serde_json::from_value(serde_json::json!(99)).unwrap();

        for unsorted in [0, 2] {
            let snapshot = snapshot(&contexts, unsorted);
            for query in [
                Query::Number(1),
                Query::Number(9),
                Query::Id(comms),
                Query::Id(missing),
                Query::Name("comms"),
                Query::Name("cli"),
                Query::Name("nothing"),
                Query::Name("Unsorted"),
                Query::Name("Everything"),
                Query::Name("  "),
            ] {
                assert_eq!(
                    resolve(query, &contexts, unsorted > 0),
                    snapshot.resolve(query),
                    "{query:?} with {unsorted} unsorted windows"
                );
            }
        }
    }

    /// The snapshot ranks as the model ranks the live contexts: the same
    /// entries for a query, and the most recently used entry first among
    /// equals.
    #[test]
    fn the_snapshot_ranks_as_the_model_ranks() {
        let mut contexts = three();
        let client = contexts.by_name("Client work").unwrap().id;
        contexts.switch_to(ContextKey::Named(client)).unwrap();
        let snapshot = snapshot(&contexts, 2);

        for query in ["", "c", "cli", "work", "zzz"] {
            assert_eq!(rank(query, &contexts, true), snapshot.rank(query), "{query:?}");
        }
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

    /// A snapshot as a server of M5c writes it, before command results and
    /// Unsorted's listing, reads with those fields at their defaults.
    #[test]
    fn a_snapshot_from_an_older_server_reads_with_defaults() {
        let older = r#"(
            enabled: true,
            scope: global,
            active: Named(2),
            screens: [(id: 1, shows: Named(2))],
            contexts: [(id: 2, name: "Comms", number: Some(1), last_used: 3, apps: ["Mail"], windows: 1)],
            unsorted: (windows: 2, last_used: 1),
            everything: (last_used: 0),
        )"#;

        let snapshot: ContextsSnapshot = ron::de::from_str(older).unwrap();

        assert_eq!(1, snapshot.contexts.len());
        assert_eq!("Comms", snapshot.contexts[0].name);
        assert_eq!(2, snapshot.unsorted.windows);
        assert!(!snapshot.unsorted.listed);
        assert!(snapshot.results.is_empty());
        let empty: ContextsSnapshot = ron::de::from_str("()").unwrap();
        assert_eq!(ContextsSnapshot::off(), empty);
    }

    /// A snapshot from a newer server, with fields this version doesn't
    /// know, reads without them. A member record that names only some of
    /// its keys reads with the rest at their defaults, because a server
    /// and a client can be different versions.
    #[test]
    fn a_snapshot_from_a_newer_server_reads_without_its_new_fields() {
        let newer = r#"(
            enabled: true,
            scope: global,
            active: Unsorted,
            screens: [(id: 1, shows: Unsorted, display_id: 42)],
            contexts: [(id: 2, name: "Comms", hotkey: "⌃⌥1",
                        members: [(record: 1, title: "Inbox")])],
            unsorted: (listed: true, windows: 1, last_used: 4, titles: ["Notes"]),
            everything: (last_used: 0),
            results: [(request: 9, error: None, finished_at: 12)],
            focused_screen: 1,
        )"#;

        let snapshot: ContextsSnapshot = ron::de::from_str(newer).unwrap();

        assert_eq!(ContextKey::Unsorted, snapshot.active);
        assert_eq!(ContextKey::Unsorted, snapshot.screens[0].shows);
        assert_eq!(
            ("Comms", 0),
            (&*snapshot.contexts[0].name, snapshot.contexts[0].windows)
        );
        assert_eq!(
            vec![MemberSummary {
                record: 1,
                app: String::new(),
                title: "Inbox".into(),
                window: None,
            }],
            snapshot.contexts[0].members
        );
        assert!(snapshot.unsorted.listed);
        assert_eq!(
            Some(&CommandResult {
                request: RequestId(9),
                error: None
            }),
            snapshot.result(RequestId(9))
        );
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
