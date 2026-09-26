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

use crate::actor::reactor::{ContextCommand, ContextRef};
use crate::model::contexts::{
    self, CONTEXTS_FILE_VERSION, Context, ContextError, ContextId, ContextKey, Contexts,
    EVERYTHING_NAME, MemberRecord, NameMatch, UNSORTED_NAME,
};

/// The app name that stands in when a record names no app.
pub const UNKNOWN_APP: &str = "Unknown app";

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

    /// Ranks the entries for a query, as the switcher does.
    pub fn rank(&self, query: &str) -> Vec<(ContextKey, NameMatch)> {
        contexts::rank(query, &self.model(), self.unsorted.windows > 0)
    }

    /// The entry that a reference names. A name takes the best match of the
    /// switcher's ranking. `Ok(None)` means that no entry matches the name
    /// here, and the reactor resolves it against its own state, which can be
    /// newer than the snapshot.
    pub fn resolve(&self, reference: &ContextRef) -> Result<Option<ContextKey>, String> {
        match reference {
            ContextRef::Number(number) => self
                .contexts
                .iter()
                .find(|context| context.number == Some(*number))
                .map(|context| Some(ContextKey::Named(context.id)))
                .ok_or_else(|| format!("No context has the number {number}")),
            ContextRef::Id(id) => self
                .get(*id)
                .map(|context| Some(ContextKey::Named(context.id)))
                .ok_or_else(|| ContextError::NoSuchContext.to_string()),
            ContextRef::Name(name) if name.trim().is_empty() => {
                Err("Give the name or the number of a context".to_string())
            }
            ContextRef::Name(name) => Ok(self.rank(name).first().map(|(key, _)| *key)),
        }
    }

    /// Checks a command against the snapshot, and replaces the context it
    /// names with the id of the context the snapshot resolves it to (I3). A
    /// name that the snapshot can't resolve stays for the reactor. Returns
    /// the reason when the command can't run.
    pub fn resolve_command(&self, command: ContextCommand) -> Result<ContextCommand, String> {
        match command {
            ContextCommand::SwitchContext(reference) => Ok(match self.resolve(&reference)? {
                Some(ContextKey::Named(id)) => ContextCommand::SwitchContext(ContextRef::Id(id)),
                // Unsorted has no reference of its own, and its name is
                // reserved.
                Some(ContextKey::Unsorted) => {
                    ContextCommand::SwitchContext(ContextRef::Name(UNSORTED_NAME.to_string()))
                }
                Some(ContextKey::Everything) => ContextCommand::ShowEverything,
                None => ContextCommand::SwitchContext(reference),
            }),
            ContextCommand::CreateContext(name) => match self.model().create(&name) {
                Ok(_) => Ok(ContextCommand::CreateContext(name)),
                Err(err) => Err(err.to_string()),
            },
            command @ (ContextCommand::ShowEverything
            | ContextCommand::PreviousContext
            | ContextCommand::AddWindowToContext(_)
            | ContextCommand::MoveWindowToContext(_)
            | ContextCommand::RemoveWindowFromContext
            | ContextCommand::ToggleWindowPinned) => Ok(command),
        }
    }

    /// A model of the named contexts with the same ids, names, numbers, and
    /// order of use, so that the model's ranking and name checks run on the
    /// snapshot. `contexts.json` has no use numbers for Unsorted and
    /// Everything, so every context starts unused, and the switches are made
    /// again in the order the snapshot gives.
    fn model(&self) -> Contexts {
        let contexts: Vec<Context> = self
            .contexts
            .iter()
            .map(|context| Context {
                id: context.id,
                name: context.name.clone(),
                number: context.number,
                members: Vec::new(),
                last_used: 0,
            })
            .collect();
        let mut model: Contexts = serde_json::from_value(serde_json::json!({
            "version": CONTEXTS_FILE_VERSION,
            "contexts": contexts,
        }))
        .expect("the contexts of a model load");
        let mut uses: Vec<(u64, ContextKey)> = self
            .contexts
            .iter()
            .map(|context| (context.last_used, ContextKey::Named(context.id)))
            .chain([
                (self.unsorted.last_used, ContextKey::Unsorted),
                (self.everything.last_used, ContextKey::Everything),
            ])
            .filter(|(used, _)| *used > 0)
            .collect();
        uses.sort_by_key(|(used, _)| *used);
        for (_, key) in uses {
            model.switch_to(key).expect("the model has every context of the snapshot");
        }
        model
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

    fn switch(reference: ContextRef) -> ContextCommand {
        ContextCommand::SwitchContext(reference)
    }

    fn name(name: &str) -> ContextRef {
        ContextRef::Name(name.to_string())
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

    /// The snapshot carries the use numbers of every entry, so ranking it
    /// breaks ties exactly as ranking the model does, Unsorted and
    /// Everything included (R19).
    #[test]
    fn ranking_the_snapshot_gives_the_models_order() {
        let mut contexts = three();
        contexts.create("Ultra").unwrap();
        contexts.create("Everyday").unwrap();
        for key in [
            ContextKey::Everything,
            named(&contexts, "Community"),
            ContextKey::Unsorted,
            named(&contexts, "Everyday"),
            named(&contexts, "Comms"),
        ] {
            contexts.switch_to(key).unwrap();
        }
        let snapshot = snapshot(&contexts, 1);

        for query in ["", "u", "e", "c", "com", "cli", "cw", "ev", "nothing"] {
            assert_eq!(
                contexts::rank(query, &contexts, true),
                snapshot.rank(query),
                "query {query:?}"
            );
        }
        assert_eq!(
            vec![
                named(&contexts, "Comms"),
                named(&contexts, "Everyday"),
                ContextKey::Unsorted,
                named(&contexts, "Community"),
                ContextKey::Everything,
                named(&contexts, "Client work"),
                named(&contexts, "Ultra"),
            ],
            snapshot.rank("").into_iter().map(|(key, _)| key).collect::<Vec<_>>()
        );
    }

    /// R29. Unsorted ranks only while it has windows.
    #[test]
    fn ranking_lists_unsorted_only_while_it_has_windows() {
        let contexts = three();
        let has = |snapshot: &ContextsSnapshot| {
            snapshot.rank("").iter().any(|(key, _)| *key == ContextKey::Unsorted)
        };
        assert!(!has(&snapshot(&contexts, 0)));
        assert!(has(&snapshot(&contexts, 1)));
    }

    /// I3. A number and an id resolve to their context, and a name to the
    /// best match of the switcher's ranking.
    #[test]
    fn a_query_resolves_by_number_exact_name_and_ranked_name() {
        let mut contexts = three();
        contexts.switch_to(named(&contexts, "Community")).unwrap();
        contexts.switch_to(named(&contexts, "Comms")).unwrap();
        let snapshot = snapshot(&contexts, 0);
        let comms = named(&contexts, "Comms");
        let community = named(&contexts, "Community");
        let client = named(&contexts, "Client work");
        let ContextKey::Named(community_id) = community else {
            unreachable!()
        };

        assert_eq!(Ok(Some(community)), snapshot.resolve(&ContextRef::Number(2)));
        assert_eq!(
            Ok(Some(community)),
            snapshot.resolve(&ContextRef::Id(community_id))
        );
        assert_eq!(Ok(Some(community)), snapshot.resolve(&name("COMMUNITY")));
        assert_eq!(Ok(Some(client)), snapshot.resolve(&name("cli")));
        assert_eq!(Ok(Some(client)), snapshot.resolve(&name("cw")));
        // "com" starts both; Comms was used last.
        assert_eq!(Ok(Some(comms)), snapshot.resolve(&name("com")));
        assert_eq!(
            Ok(Some(ContextKey::Everything)),
            snapshot.resolve(&name("every"))
        );
    }

    /// I3. A name that matches nothing stays for the reactor. A number or an
    /// id that names no context, and a blank name, fail.
    #[test]
    fn a_query_that_names_no_context() {
        let mut contexts = three();
        let gone = contexts.create("Gone").unwrap();
        contexts.delete(gone).unwrap();
        let snapshot = snapshot(&contexts, 0);

        assert_eq!(Ok(None), snapshot.resolve(&name("Sugarglider")));
        assert_eq!(
            Err("No context has the number 4".to_string()),
            snapshot.resolve(&ContextRef::Number(4))
        );
        assert_eq!(
            Err("No such context".to_string()),
            snapshot.resolve(&ContextRef::Id(gone))
        );
        assert_eq!(
            Err("Give the name or the number of a context".to_string()),
            snapshot.resolve(&name("  "))
        );
        // Unsorted has no windows, so the reserved name matches nothing.
        assert_eq!(Ok(None), snapshot.resolve(&name("unsorted")));
    }

    /// I3. A resolved switch names its context by id, so a rename before the
    /// reactor runs it can't redirect it. Unsorted goes by its reserved
    /// name, and Everything by its own command.
    #[test]
    fn a_switch_is_sent_with_the_resolved_context() {
        let contexts = three();
        let snapshot = snapshot(&contexts, 1);
        let ContextKey::Named(client) = named(&contexts, "Client work") else {
            unreachable!()
        };

        assert_eq!(
            Ok(switch(ContextRef::Id(client))),
            snapshot.resolve_command(switch(name("cli")))
        );
        assert_eq!(
            Ok(switch(ContextRef::Id(client))),
            snapshot.resolve_command(switch(ContextRef::Number(3)))
        );
        assert_eq!(
            Ok(switch(name("Unsorted"))),
            snapshot.resolve_command(switch(name("uns")))
        );
        assert_eq!(
            Ok(ContextCommand::ShowEverything),
            snapshot.resolve_command(switch(name("Everything")))
        );
        assert_eq!(
            Ok(switch(name("New one"))),
            snapshot.resolve_command(switch(name("New one")))
        );
        assert!(snapshot.resolve_command(switch(ContextRef::Number(9))).is_err());
        for command in [
            ContextCommand::ShowEverything,
            ContextCommand::PreviousContext,
        ] {
            assert_eq!(Ok(command.clone()), snapshot.resolve_command(command));
        }
    }

    /// R4. A new context's name is checked as the model checks it.
    #[test]
    fn a_new_context_needs_a_free_name() {
        let snapshot = snapshot(&three(), 0);
        let create = |name: &str| ContextCommand::CreateContext(name.to_string());

        assert_eq!(
            Ok(create("Sugarglider")),
            snapshot.resolve_command(create("Sugarglider"))
        );
        assert_eq!(
            Err("A context named \"Comms\" already exists".to_string()),
            snapshot.resolve_command(create(" cómms "))
        );
        assert_eq!(
            Err("\"Unsorted\" is a reserved name".to_string()),
            snapshot.resolve_command(create("Unsorted"))
        );
        assert_eq!(
            Err("A context name can't be empty".to_string()),
            snapshot.resolve_command(create(" "))
        );
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

    #[test]
    fn the_snapshot_survives_a_ron_round_trip() {
        let mut contexts = three();
        contexts.switch_to(ContextKey::Unsorted).unwrap();
        for snapshot in [ContextsSnapshot::off(), snapshot(&contexts, 3)] {
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
