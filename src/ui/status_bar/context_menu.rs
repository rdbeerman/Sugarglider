// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The contexts section of the status menu, as data.
//!
//! The status menu builds this section from the published contexts snapshot
//! each time it opens. The design is in `docs/specs/contexts.md`, section
//! "Menu bar".

use livesplit_hotkey::Hotkey;
use serde_json::{Value, json};

use crate::actor::contexts_snapshot::ContextsSnapshot;
use crate::actor::reactor::{self, ContextCommand, ContextRef};
use crate::actor::wm_controller::WmCommand;
use crate::collections::{BTreeMap, HashSet};
use crate::model::contexts::{ContextId, ContextKey, UNSORTED_NAME, fold};
use crate::ui::status_bar::MenuKeyEquivalent;

/// The name of the command that opens the switcher, as a key binding writes
/// it.
const OPEN_CONTEXT_SWITCHER: &str = "open_context_switcher";
/// Whether the `open_context_switcher` command exists. M8 adds it as
/// `ContextCommand::OpenContextSwitcher` and sets this to true, or replaces
/// the arm that reads it. Until then the menu item stays disabled.
const OPEN_CONTEXT_SWITCHER_BUILT: bool = false;

/// What choosing an item of the section does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuAction {
    /// Switches to a named context, to Unsorted, or to Everything.
    Switch(ContextKey),
    /// Creates a context with this name from the windows on screen.
    NewContext(String),
    /// Moves the focused window to the context.
    SendWindowTo(ContextId),
    OpenSwitcher,
}

impl MenuAction {
    /// The command that the item sends. `None` when no command has the name
    /// that the action uses.
    pub fn command(&self) -> Option<WmCommand> {
        let context = |command| Some(WmCommand::ReactorCommand(reactor::Command::Context(command)));
        match self {
            MenuAction::Switch(ContextKey::Named(id)) => {
                context(ContextCommand::SwitchContext(ContextRef::Id(*id)))
            }
            // Unsorted has no reference of its own, and its name is reserved.
            MenuAction::Switch(ContextKey::Unsorted) => context(ContextCommand::SwitchContext(
                ContextRef::Name(UNSORTED_NAME.to_string()),
            )),
            MenuAction::Switch(ContextKey::Everything) => context(ContextCommand::ShowEverything),
            MenuAction::NewContext(name) => context(ContextCommand::CreateContext(name.clone())),
            MenuAction::SendWindowTo(id) => {
                context(ContextCommand::MoveWindowToContext(ContextRef::Id(*id)))
            }
            MenuAction::OpenSwitcher if OPEN_CONTEXT_SWITCHER_BUILT => {
                command_named(json!(OPEN_CONTEXT_SWITCHER))
            }
            // M8 adds the command; nothing names it yet, so the item is
            // disabled.
            MenuAction::OpenSwitcher => None,
        }
    }
}

/// The status menu's enable check: whether the action's command exists.
pub fn command_available(action: &MenuAction) -> bool {
    action.command().is_some()
}

/// The command that a key binding written as `value` names, or `None` when
/// no command reads `value`.
fn command_named(value: Value) -> Option<WmCommand> {
    let command: WmCommand = serde_json::from_value(value.clone()).ok()?;
    // `WmCommand` is untagged, so a command with another name could read the
    // value. Only the named command writes the same value back.
    (serde_json::to_value(&command).ok()? == value).then_some(command)
}

/// An entry of the section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    Item(MenuItem),
    Submenu {
        title: String,
        enabled: bool,
        items: Vec<MenuItem>,
    },
    Separator,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    pub title: String,
    pub action: MenuAction,
    pub checked: bool,
    pub enabled: bool,
    pub key: Option<MenuKeyEquivalent>,
}

/// The key equivalents that the section shows, from the key bindings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextMenuKeys {
    /// For each number that a `switch_context` binding names: the key and
    /// the modifiers of the binding.
    numbers: BTreeMap<u8, MenuKeyEquivalent>,
    show_everything: Option<MenuKeyEquivalent>,
    open_switcher: Option<MenuKeyEquivalent>,
}

impl ContextMenuKeys {
    /// The key equivalents of `bindings`. When several bindings have the same
    /// command, the first one counts.
    pub fn new(bindings: &[(Hotkey, WmCommand)]) -> Self {
        let open_switcher = json!(OPEN_CONTEXT_SWITCHER);
        let mut keys = ContextMenuKeys::default();
        for (hotkey, command) in bindings {
            let Some(key) = MenuKeyEquivalent::from_hotkey(hotkey) else {
                continue;
            };
            match context_command(command) {
                Some(ContextCommand::SwitchContext(ContextRef::Number(number))) => {
                    keys.numbers.entry(*number).or_insert(key);
                }
                Some(ContextCommand::ShowEverything) => {
                    keys.show_everything.get_or_insert(key);
                }
                _ if serde_json::to_value(command).is_ok_and(|value| value == open_switcher) => {
                    keys.open_switcher.get_or_insert(key);
                }
                _ => {}
            }
        }
        keys
    }
}

fn context_command(command: &WmCommand) -> Option<&ContextCommand> {
    match command {
        WmCommand::ReactorCommand(reactor::Command::Context(command)) => Some(command),
        _ => None,
    }
}

/// The section for `snapshot`, which is empty while contexts are off.
///
/// The checkmark is on what the managed screens show. While no screen shows
/// a managed Space, nothing is checked, and the items that switch or create
/// a context are disabled, because the reactor refuses them then.
///
/// `available` tells whether the command of an action exists. An item whose
/// command doesn't exist is disabled.
pub fn context_menu(
    snapshot: &ContextsSnapshot,
    keys: &ContextMenuKeys,
    available: impl Fn(&MenuAction) -> bool,
) -> Vec<MenuEntry> {
    if !snapshot.enabled {
        return Vec::new();
    }
    let active = snapshot.active;
    let shown = snapshot.shown();
    let item = |title: &str, action: MenuAction, key: Option<MenuKeyEquivalent>| MenuItem {
        title: title.to_string(),
        checked: false,
        enabled: available(&action),
        action,
        key,
    };
    let switch = |title: &str, to: ContextKey, key: Option<MenuKeyEquivalent>| {
        let switch_to = item(title, MenuAction::Switch(to), key);
        MenuEntry::Item(MenuItem {
            checked: shown == Some(to),
            enabled: switch_to.enabled && shown.is_some(),
            ..switch_to
        })
    };

    let mut entries: Vec<MenuEntry> = snapshot
        .contexts
        .iter()
        .map(|context| {
            let key = context.number.and_then(|number| keys.numbers.get(&number).cloned());
            switch(&context.name, ContextKey::Named(context.id), key)
        })
        .collect();
    if snapshot.unsorted.listed {
        entries.push(switch(UNSORTED_NAME, ContextKey::Unsorted, None));
    }
    entries.push(switch(
        "Show Everything",
        ContextKey::Everything,
        keys.show_everything.clone(),
    ));
    entries.push(MenuEntry::Separator);

    let new_context = item(
        "New Context from Current Windows…",
        MenuAction::NewContext(new_context_name(snapshot)),
        None,
    );
    entries.push(MenuEntry::Item(MenuItem {
        enabled: new_context.enabled && shown.is_some(),
        ..new_context
    }));
    let send: Vec<MenuItem> = snapshot
        .contexts
        .iter()
        .map(|context| {
            let send_to = item(&context.name, MenuAction::SendWindowTo(context.id), None);
            MenuItem {
                enabled: send_to.enabled && active != ContextKey::Named(context.id),
                ..send_to
            }
        })
        .collect();
    entries.push(MenuEntry::Submenu {
        title: "Send Window to".to_string(),
        enabled: send.iter().any(|item| item.enabled),
        items: send,
    });
    entries.push(MenuEntry::Item(item(
        "Open Switcher…",
        MenuAction::OpenSwitcher,
        keys.open_switcher.clone(),
    )));
    entries.push(MenuEntry::Separator);
    entries
}

/// A name that no context has: "Context" and a number, from one more than
/// the number of contexts. Names are compared as R4 compares them.
fn new_context_name(snapshot: &ContextsSnapshot) -> String {
    let taken: HashSet<String> =
        snapshot.contexts.iter().map(|context| fold(&context.name)).collect();
    (snapshot.contexts.len() + 1..)
        .map(|number| format!("Context {number}"))
        .find(|name| !taken.contains(&fold(name)))
        .expect("some number gives a free name")
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use objc2_app_kit::NSEventModifierFlags;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::actor::app::WindowId;
    use crate::actor::contexts_snapshot::ScreenContext;
    use crate::actor::wm_controller::WmCmd;
    use crate::model::contexts::{Contexts, Scope, WindowDesc};

    /// Comms (1), Relax (2), and Build (3), with `active` active and
    /// `unsorted` unsorted windows.
    fn snapshot(active: Option<&str>, unsorted: usize) -> ContextsSnapshot {
        let mut contexts = Contexts::new();
        for name in ["Comms", "Relax", "Build"] {
            contexts.create(name).unwrap();
        }
        let key = match active {
            Some("Unsorted") => ContextKey::Unsorted,
            Some(name) => ContextKey::Named(contexts.by_name(name).unwrap().id),
            None => ContextKey::Everything,
        };
        contexts.switch_to(key).unwrap();
        let screens = vec![ScreenContext { id: 1, shows: key }];
        ContextsSnapshot::new(&contexts, Scope::Global, screens, unsorted)
    }

    fn id(snapshot: &ContextsSnapshot, name: &str) -> ContextId {
        snapshot.contexts.iter().find(|context| context.name == name).unwrap().id
    }

    fn ctrl_alt(key: &str) -> Option<MenuKeyEquivalent> {
        Some(MenuKeyEquivalent {
            key: key.to_string(),
            modifiers: NSEventModifierFlags::Control | NSEventModifierFlags::Option,
        })
    }

    /// ⌃⌥1 and ⌃⌥2 switch to contexts 1 and 2, ⌃⌥0 shows Everything, and
    /// ⌃⌥Space opens the switcher.
    fn keys() -> ContextMenuKeys {
        ContextMenuKeys {
            numbers: [(1, ctrl_alt("1").unwrap()), (2, ctrl_alt("2").unwrap())].into(),
            show_everything: ctrl_alt("0"),
            open_switcher: ctrl_alt(" "),
        }
    }

    fn all(_: &MenuAction) -> bool {
        true
    }

    fn item(title: &str, action: MenuAction) -> MenuItem {
        MenuItem {
            title: title.to_string(),
            action,
            checked: false,
            enabled: true,
            key: None,
        }
    }

    fn items(entries: &[MenuEntry]) -> Vec<&MenuItem> {
        entries
            .iter()
            .flat_map(|entry| match entry {
                MenuEntry::Item(item) => vec![item],
                MenuEntry::Submenu { items, .. } => items.iter().collect(),
                MenuEntry::Separator => vec![],
            })
            .collect()
    }

    fn checked(entries: &[MenuEntry]) -> Vec<&str> {
        items(entries)
            .into_iter()
            .filter(|item| item.checked)
            .map(|item| item.title.as_str())
            .collect()
    }

    fn titles(entries: &[MenuEntry]) -> Vec<&str> {
        entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Item(item) => item.title.as_str(),
                MenuEntry::Submenu { title, .. } => title.as_str(),
                MenuEntry::Separator => "-",
            })
            .collect()
    }

    fn submenu(entries: &[MenuEntry]) -> (bool, &[MenuItem]) {
        entries
            .iter()
            .find_map(|entry| match entry {
                MenuEntry::Submenu { enabled, items, .. } => Some((*enabled, items.as_slice())),
                _ => None,
            })
            .unwrap()
    }

    fn json(command: Option<WmCommand>) -> Value {
        serde_json::to_value(command.unwrap()).unwrap()
    }

    /// R28. With contexts off, the menu gains nothing.
    #[test]
    fn with_contexts_off_the_section_is_empty() {
        assert_eq!(
            Vec::<MenuEntry>::new(),
            context_menu(&ContextsSnapshot::off(), &keys(), all)
        );
    }

    /// Each context, with a checkmark on the active one and its number's
    /// binding as the key equivalent, then Show Everything, then the
    /// actions. The active context can't take the focused window.
    #[test]
    fn the_section_lists_each_context_and_the_actions() {
        let snapshot = snapshot(Some("Relax"), 0);
        let [comms, relax, build] = ["Comms", "Relax", "Build"].map(|name| id(&snapshot, name));

        assert_eq!(
            vec![
                MenuEntry::Item(MenuItem {
                    key: ctrl_alt("1"),
                    ..item("Comms", MenuAction::Switch(ContextKey::Named(comms)))
                }),
                MenuEntry::Item(MenuItem {
                    key: ctrl_alt("2"),
                    checked: true,
                    ..item("Relax", MenuAction::Switch(ContextKey::Named(relax)))
                }),
                MenuEntry::Item(item("Build", MenuAction::Switch(ContextKey::Named(build)))),
                MenuEntry::Item(MenuItem {
                    key: ctrl_alt("0"),
                    ..item("Show Everything", MenuAction::Switch(ContextKey::Everything))
                }),
                MenuEntry::Separator,
                MenuEntry::Item(item(
                    "New Context from Current Windows…",
                    MenuAction::NewContext("Context 4".to_string())
                )),
                MenuEntry::Submenu {
                    title: "Send Window to".to_string(),
                    enabled: true,
                    items: vec![
                        item("Comms", MenuAction::SendWindowTo(comms)),
                        MenuItem {
                            enabled: false,
                            ..item("Relax", MenuAction::SendWindowTo(relax))
                        },
                        item("Build", MenuAction::SendWindowTo(build)),
                    ],
                },
                MenuEntry::Item(MenuItem {
                    key: ctrl_alt(" "),
                    ..item("Open Switcher…", MenuAction::OpenSwitcher)
                }),
                MenuEntry::Separator,
            ],
            context_menu(&snapshot, &keys(), all)
        );
    }

    /// Under Everything, Show Everything has the checkmark, and every
    /// context can take the focused window.
    #[test]
    fn under_everything_show_everything_is_checked() {
        let entries = context_menu(&snapshot(None, 0), &keys(), all);

        assert_eq!(vec!["Show Everything"], checked(&entries));
        let (enabled, send) = submenu(&entries);
        assert!(enabled);
        assert!(send.iter().all(|item| item.enabled));
    }

    /// R29. Unsorted is listed, after the contexts, while the snapshot lists
    /// it, which is while it has windows. An active Unsorted without windows
    /// isn't listed, so the menu offers no switch that the reactor would
    /// refuse, and nothing is checked.
    #[test]
    fn unsorted_is_listed_while_the_snapshot_lists_it() {
        let listed = |snapshot: &ContextsSnapshot| {
            titles(&context_menu(snapshot, &keys(), all)).contains(&"Unsorted")
        };
        assert!(!listed(&snapshot(None, 0)));
        assert!(!listed(&snapshot(Some("Comms"), 0)));

        let with_windows = context_menu(&snapshot(Some("Comms"), 2), &keys(), all);
        assert_eq!(
            vec!["Comms", "Relax", "Build", "Unsorted", "Show Everything"],
            titles(&with_windows)[..5].to_vec()
        );
        assert_eq!(vec!["Comms"], checked(&with_windows));

        let active = context_menu(&snapshot(Some("Unsorted"), 2), &keys(), all);
        let unsorted = items(&active).into_iter().find(|item| item.title == "Unsorted");
        assert_eq!(
            Some(&MenuItem {
                checked: true,
                ..item("Unsorted", MenuAction::Switch(ContextKey::Unsorted))
            }),
            unsorted
        );
        assert_eq!(vec!["Unsorted"], checked(&active));

        let empty = snapshot(Some("Unsorted"), 0);
        assert!(!listed(&empty));
        assert!(checked(&context_menu(&empty, &keys(), all)).is_empty());
        let mut unlisted = snapshot(Some("Comms"), 2);
        unlisted.unsorted.listed = false;
        assert!(!listed(&unlisted));
    }

    /// Items whose command doesn't exist are disabled: the whole Send Window
    /// to submenu, and Open Switcher. The other items stay enabled.
    #[test]
    fn items_without_a_command_are_disabled() {
        let entries = context_menu(&snapshot(Some("Comms"), 1), &keys(), |action| {
            !matches!(action, MenuAction::SendWindowTo(_) | MenuAction::OpenSwitcher)
        });

        let (enabled, send) = submenu(&entries);
        assert!(!enabled);
        assert_eq!(3, send.len());
        assert!(send.iter().all(|item| !item.enabled));
        let disabled: Vec<&str> = items(&entries)
            .into_iter()
            .filter(|item| !item.enabled && !matches!(item.action, MenuAction::SendWindowTo(_)))
            .map(|item| item.title.as_str())
            .collect();
        assert_eq!(vec!["Open Switcher…"], disabled);
    }

    /// Without contexts, the section offers Show Everything, checked, and a
    /// new context, and Send Window to has nothing to offer.
    #[test]
    fn without_contexts_send_window_to_is_disabled() {
        let mut contexts = Contexts::new();
        contexts.switch_to(ContextKey::Everything).unwrap();
        let screens = vec![ScreenContext {
            id: 1,
            shows: ContextKey::Everything,
        }];
        let snapshot = ContextsSnapshot::new(&contexts, Scope::Global, screens, 0);

        let entries = context_menu(&snapshot, &ContextMenuKeys::default(), all);

        assert_eq!(
            vec![
                "Show Everything",
                "-",
                "New Context from Current Windows…",
                "Send Window to",
                "Open Switcher…",
                "-"
            ],
            titles(&entries)
        );
        assert_eq!(vec!["Show Everything"], checked(&entries));
        assert_eq!((false, &[][..]), submenu(&entries));
        assert!(items(&entries).iter().all(|item| item.key.is_none()));
        assert!(items(&entries).contains(&&item(
            "New Context from Current Windows…",
            MenuAction::NewContext("Context 1".to_string())
        )));
    }

    /// R4. A new context's name is one no context has, ignoring case and
    /// accents.
    #[test]
    fn a_new_context_gets_a_free_name() {
        let mut contexts = Contexts::new();
        for name in ["Comms", "cöntext 3", "Context 4"] {
            contexts.create(name).unwrap();
        }
        let snapshot = ContextsSnapshot::new(&contexts, Scope::Global, Vec::new(), 0);

        assert_eq!("Context 5", new_context_name(&snapshot));
        assert_eq!("Context 1", new_context_name(&ContextsSnapshot::off()));
    }

    /// A context's key equivalent is the key and the modifiers of the first
    /// `switch_context` binding of its number, so the menu shows the
    /// shortcut that works. Show Everything takes the key of its binding.
    /// Other bindings don't count.
    #[test]
    fn key_equivalents_come_from_the_bindings() {
        let hotkey = |text: &str| Hotkey::from_str(text).unwrap();
        let context = |command| WmCommand::ReactorCommand(reactor::Command::Context(command));
        let switch = |reference| context(ContextCommand::SwitchContext(reference));
        let bindings = vec![
            (hotkey("Ctrl + Alt + Digit1"), switch(ContextRef::Number(1))),
            (hotkey("Meta + Digit1"), switch(ContextRef::Number(1))),
            (hotkey("Ctrl + Shift + KeyQ"), switch(ContextRef::Number(2))),
            (hotkey("Alt + Digit3"), switch(ContextRef::Name("Build".into()))),
            (
                hotkey("Ctrl + Alt + Digit0"),
                context(ContextCommand::ShowEverything),
            ),
            (hotkey("Alt + KeyT"), WmCommand::Wm(WmCmd::ToggleGlobalEnabled)),
        ];

        let keys = ContextMenuKeys::new(&bindings);

        assert_eq!(
            ContextMenuKeys {
                numbers: [
                    (1, ctrl_alt("1").unwrap()),
                    (
                        2,
                        MenuKeyEquivalent {
                            key: "q".to_string(),
                            modifiers: NSEventModifierFlags::Control | NSEventModifierFlags::Shift,
                        }
                    ),
                ]
                .into(),
                show_everything: ctrl_alt("0"),
                open_switcher: None,
            },
            keys
        );
    }

    /// The items send the context commands by id, Unsorted by its reserved
    /// name, and Everything as `show_everything`, in the form a key binding
    /// writes them.
    #[test]
    fn items_send_the_context_commands() {
        let snapshot = snapshot(None, 0);
        let comms = id(&snapshot, "Comms");

        assert_eq!(
            serde_json::json!({ "switch_context": { "id": comms } }),
            json(MenuAction::Switch(ContextKey::Named(comms)).command())
        );
        assert_eq!(
            serde_json::json!({ "switch_context": "Unsorted" }),
            json(MenuAction::Switch(ContextKey::Unsorted).command())
        );
        assert_eq!(
            serde_json::json!("show_everything"),
            json(MenuAction::Switch(ContextKey::Everything).command())
        );
        assert_eq!(
            serde_json::json!({ "create_context": "Context 4" }),
            json(MenuAction::NewContext("Context 4".into()).command())
        );
    }

    /// A command is found by the name and argument a key binding gives it.
    /// A name that no command has finds nothing.
    #[test]
    fn a_command_is_found_by_its_name() {
        let by_id = serde_json::json!({ "switch_context": { "id": 7 } });
        assert_eq!(by_id, json(command_named(by_id.clone())));
        let everything = serde_json::json!("show_everything");
        assert_eq!(everything, json(command_named(everything.clone())));
        assert!(command_named(serde_json::json!("no_such_command")).is_none());
        assert!(command_named(serde_json::json!({ "no_such_command": { "id": 7 } })).is_none());
    }

    /// The snapshot of `contexts` on one screen that shows the active
    /// context, with `unsorted` unsorted windows.
    fn on_screen(contexts: &Contexts, unsorted: usize) -> ContextsSnapshot {
        let screens = vec![ScreenContext {
            id: 1,
            shows: contexts.active(),
        }];
        ContextsSnapshot::new(contexts, Scope::Global, screens, unsorted)
    }

    /// The check that the status menu makes: whether the action's command
    /// exists.
    fn exists(action: &MenuAction) -> bool {
        command_available(action)
    }

    fn hotkey(text: &str) -> Hotkey {
        Hotkey::from_str(text).unwrap()
    }

    fn context(command: ContextCommand) -> WmCommand {
        WmCommand::ReactorCommand(reactor::Command::Context(command))
    }

    fn switch_to_number(number: u8) -> WmCommand {
        context(ContextCommand::SwitchContext(ContextRef::Number(number)))
    }

    fn key(key: &str, modifiers: NSEventModifierFlags) -> MenuKeyEquivalent {
        MenuKeyEquivalent {
            key: key.to_string(),
            modifiers,
        }
    }

    /// The command of a key binding written as `command` in the config
    /// file, in JSON.
    fn bound(command: &str) -> Value {
        #[derive(serde::Deserialize)]
        struct Binding {
            command: WmCommand,
        }
        let binding: Binding = toml::from_str(&format!("command = {command}")).unwrap();
        serde_json::to_value(binding.command).unwrap()
    }

    /// Menu bar, R29. Under Everything, Show Everything has the checkmark,
    /// Unsorted is listed while it has windows, and every context can take
    /// the focused window.
    #[test]
    fn under_everything_show_everything_is_checked_and_unsorted_is_listed() {
        let snapshot = snapshot(None, 2);
        let [comms, relax, build] = ["Comms", "Relax", "Build"].map(|name| id(&snapshot, name));

        assert_eq!(
            vec![
                MenuEntry::Item(MenuItem {
                    key: ctrl_alt("1"),
                    ..item("Comms", MenuAction::Switch(ContextKey::Named(comms)))
                }),
                MenuEntry::Item(MenuItem {
                    key: ctrl_alt("2"),
                    ..item("Relax", MenuAction::Switch(ContextKey::Named(relax)))
                }),
                MenuEntry::Item(item("Build", MenuAction::Switch(ContextKey::Named(build)))),
                MenuEntry::Item(item("Unsorted", MenuAction::Switch(ContextKey::Unsorted))),
                MenuEntry::Item(MenuItem {
                    key: ctrl_alt("0"),
                    checked: true,
                    ..item("Show Everything", MenuAction::Switch(ContextKey::Everything))
                }),
                MenuEntry::Separator,
                MenuEntry::Item(item(
                    "New Context from Current Windows…",
                    MenuAction::NewContext("Context 4".to_string())
                )),
                MenuEntry::Submenu {
                    title: "Send Window to".to_string(),
                    enabled: true,
                    items: vec![
                        item("Comms", MenuAction::SendWindowTo(comms)),
                        item("Relax", MenuAction::SendWindowTo(relax)),
                        item("Build", MenuAction::SendWindowTo(build)),
                    ],
                },
                MenuEntry::Item(MenuItem {
                    key: ctrl_alt(" "),
                    ..item("Open Switcher…", MenuAction::OpenSwitcher)
                }),
                MenuEntry::Separator,
            ],
            context_menu(&snapshot, &keys(), all)
        );
    }

    /// Menu bar, R16. In each state exactly the active entry has the
    /// checkmark, and its item stays enabled, because switching to the
    /// active context applies it again.
    #[test]
    fn only_the_active_entry_is_checked_and_it_stays_enabled() {
        for (active, unsorted, expected) in [
            (None, 0, "Show Everything"),
            (None, 2, "Show Everything"),
            (Some("Comms"), 0, "Comms"),
            (Some("Relax"), 2, "Relax"),
            (Some("Build"), 0, "Build"),
            (Some("Unsorted"), 2, "Unsorted"),
        ] {
            let snapshot = snapshot(active, unsorted);
            let entries = context_menu(&snapshot, &keys(), exists);

            let checked: Vec<(&str, &MenuAction, bool)> = items(&entries)
                .into_iter()
                .filter(|item| item.checked)
                .map(|item| (item.title.as_str(), &item.action, item.enabled))
                .collect();
            assert_eq!(
                vec![(expected, &MenuAction::Switch(snapshot.active), true)],
                checked,
                "{active:?} with {unsorted} unsorted windows"
            );
        }
    }

    /// Menu bar, with the spec's term: the active context is the context a
    /// screen shows. While the screens show Everything and the model's
    /// active context stays, during a quit that waits for parked windows
    /// (R32) or before the Space change that turns a Space off (R33), the
    /// checkmark is on Show Everything. The reactor publishes this snapshot,
    /// as `a_quit_that_waits_publishes_everything_on_each_screen` and the
    /// `ShowEverythingOn` step of `a_snapshot_is_published_after_each_kind_of_change`
    /// show.
    #[test]
    fn while_the_screens_show_everything_show_everything_is_checked() {
        let mut contexts = Contexts::new();
        let comms = contexts.create("Comms").unwrap();
        contexts.switch_to(ContextKey::Named(comms)).unwrap();
        let screens = vec![ScreenContext {
            id: 1,
            shows: ContextKey::Everything,
        }];
        let everything_shown = ContextsSnapshot::new(&contexts, Scope::Global, screens, 0);

        let entries = context_menu(&everything_shown, &keys(), all);

        assert_eq!(vec!["Show Everything"], checked(&entries));
    }

    /// Menu bar, with the coordinator's decision for a desktop without a
    /// managed Space: after Stop Globally, or at the login window, the
    /// snapshot has no screens. Then no item is checked, whichever context
    /// is active, and the items that switch or create a context are
    /// disabled. Send Window to still works.
    #[test]
    fn with_no_managed_space_nothing_is_checked_and_switches_are_disabled() {
        for active in [Some("Comms"), Some("Unsorted"), None] {
            let snapshot = ContextsSnapshot {
                screens: Vec::new(),
                ..snapshot(active, 2)
            };

            let entries = context_menu(&snapshot, &keys(), all);

            assert!(checked(&entries).is_empty(), "{active:?}");
            let enabled: Vec<&str> = items(&entries)
                .into_iter()
                .filter(|item| item.enabled && !matches!(item.action, MenuAction::SendWindowTo(_)))
                .map(|item| item.title.as_str())
                .collect();
            assert_eq!(vec!["Open Switcher…"], enabled, "{active:?}");
            assert!(submenu(&entries).0, "{active:?}");
        }
    }

    /// Menu bar, with the coordinator's decision for mixed screens: while
    /// one screen shows Everything and another the context, the context
    /// has the checkmark.
    #[test]
    fn with_mixed_screens_the_context_a_screen_shows_is_checked() {
        let mut snapshot = snapshot(Some("Relax"), 0);
        let relax = ContextKey::Named(id(&snapshot, "Relax"));
        snapshot.screens = vec![
            ScreenContext {
                id: 1,
                shows: ContextKey::Everything,
            },
            ScreenContext { id: 2, shows: relax },
        ];

        assert_eq!(vec!["Relax"], checked(&context_menu(&snapshot, &keys(), all)));
    }

    /// Menu bar. Send Window to offers each named context in order, never
    /// Unsorted or Everything, and not the active context, which already
    /// shows the window. With only the active context to offer, the submenu
    /// is disabled.
    #[test]
    fn send_window_to_offers_each_named_context_but_the_active_one() {
        for (active, enabled) in [
            (None, [true, true, true]),
            (Some("Comms"), [false, true, true]),
            (Some("Build"), [true, true, false]),
            (Some("Unsorted"), [true, true, true]),
        ] {
            let snapshot = snapshot(active, 2);
            let entries = context_menu(&snapshot, &keys(), all);

            let (submenu_enabled, send) = submenu(&entries);
            let expected: Vec<MenuItem> = ["Comms", "Relax", "Build"]
                .into_iter()
                .zip(enabled)
                .map(|(name, enabled)| MenuItem {
                    enabled,
                    ..item(name, MenuAction::SendWindowTo(id(&snapshot, name)))
                })
                .collect();
            assert_eq!(expected, send, "{active:?}");
            assert!(submenu_enabled, "{active:?}");
        }

        let mut contexts = Contexts::new();
        let solo = contexts.create("Solo").unwrap();
        contexts.switch_to(ContextKey::Named(solo)).unwrap();
        let entries = context_menu(&on_screen(&contexts, 0), &keys(), all);
        let only_active = [MenuItem {
            enabled: false,
            ..item("Solo", MenuAction::SendWindowTo(solo))
        }];
        assert_eq!((false, &only_active[..]), submenu(&entries));
    }

    /// Menu bar, R5. A context's key equivalent follows its number. A
    /// context that gives its number to another loses the key with it, a
    /// number that no binding names gives no key, a binding of a number
    /// that no context has shows nowhere, and the tenth context has no
    /// number and so no key.
    #[test]
    fn key_equivalents_follow_the_context_numbers() {
        let names = [
            "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten",
        ];
        let mut contexts = Contexts::new();
        for name in names {
            contexts.create(name).unwrap();
        }
        let three = contexts.by_name("Three").unwrap().id;
        contexts.set_number(three, Some(1)).unwrap();
        contexts.switch_to(ContextKey::Everything).unwrap();
        let snapshot = on_screen(&contexts, 1);
        let numbers: Vec<Option<u8>> =
            snapshot.contexts.iter().map(|context| context.number).collect();
        assert_eq!(
            vec![
                None,
                Some(2),
                Some(1),
                Some(4),
                Some(5),
                Some(6),
                Some(7),
                Some(8),
                Some(9),
                None
            ],
            numbers
        );
        let bindings: Vec<(Hotkey, WmCommand)> = [1, 2, 3, 5]
            .into_iter()
            .map(|n| (hotkey(&format!("Ctrl + Alt + Digit{n}")), switch_to_number(n)))
            .collect();

        let entries = context_menu(&snapshot, &ContextMenuKeys::new(&bindings), all);

        let with_keys: Vec<(&str, Option<MenuKeyEquivalent>)> = items(&entries)
            .into_iter()
            .filter(|item| item.key.is_some())
            .map(|item| (item.title.as_str(), item.key.clone()))
            .collect();
        assert_eq!(
            vec![
                ("Two", ctrl_alt("2")),
                ("Three", ctrl_alt("1")),
                ("Five", ctrl_alt("5"))
            ],
            with_keys
        );
    }

    /// Menu bar. A number's key equivalent carries the modifiers of its
    /// binding, whichever they are, and none for a binding without
    /// modifiers. `Option` in a binding is Alt. Show Everything takes the
    /// key and the modifiers of its binding.
    #[test]
    fn key_equivalents_carry_the_modifiers_of_each_binding() {
        let bindings = vec![
            (hotkey("Ctrl + Digit1"), switch_to_number(1)),
            (hotkey("Alt + Shift + Digit2"), switch_to_number(2)),
            (hotkey("Option + Digit3"), switch_to_number(3)),
            (hotkey("Ctrl + Alt + Shift + Digit4"), switch_to_number(4)),
            (hotkey("Digit5"), switch_to_number(5)),
            (hotkey("Shift + Digit0"), context(ContextCommand::ShowEverything)),
        ];

        assert_eq!(
            ContextMenuKeys {
                numbers: [
                    (1, key("1", NSEventModifierFlags::Control)),
                    (
                        2,
                        key("2", NSEventModifierFlags::Option | NSEventModifierFlags::Shift)
                    ),
                    (3, key("3", NSEventModifierFlags::Option)),
                    (
                        4,
                        key(
                            "4",
                            NSEventModifierFlags::Control
                                | NSEventModifierFlags::Option
                                | NSEventModifierFlags::Shift
                        )
                    ),
                    (5, key("5", NSEventModifierFlags::empty())),
                ]
                .into(),
                show_everything: Some(key("0", NSEventModifierFlags::Shift)),
                open_switcher: None,
            },
            ContextMenuKeys::new(&bindings)
        );
    }

    /// Menu bar. A binding with ⌘, which the config writes as `Meta`, shows
    /// ⌘ on its item, for a number and for Show Everything.
    #[test]
    fn a_command_key_binding_shows_the_command_modifier() {
        let bindings = vec![
            (hotkey("Meta + Digit1"), switch_to_number(1)),
            (hotkey("Ctrl + Meta + Digit2"), switch_to_number(2)),
            (hotkey("Meta + Digit0"), context(ContextCommand::ShowEverything)),
        ];

        assert_eq!(
            ContextMenuKeys {
                numbers: [
                    (1, key("1", NSEventModifierFlags::Command)),
                    (
                        2,
                        key(
                            "2",
                            NSEventModifierFlags::Control | NSEventModifierFlags::Command
                        )
                    ),
                ]
                .into(),
                show_everything: Some(key("0", NSEventModifierFlags::Command)),
                open_switcher: None,
            },
            ContextMenuKeys::new(&bindings)
        );
    }

    /// Menu bar. Show Everything takes the first of its bindings. Bindings
    /// that name a context by id or by name, and the other context commands,
    /// give no key equivalent.
    #[test]
    fn show_everything_takes_its_first_binding_and_other_commands_give_no_key() {
        let seven: ContextId = serde_json::from_value(serde_json::json!(7)).unwrap();
        let bindings = vec![
            (
                hotkey("Ctrl + Alt + Digit0"),
                context(ContextCommand::ShowEverything),
            ),
            (hotkey("Alt + Digit0"), context(ContextCommand::ShowEverything)),
            (
                hotkey("Ctrl + Alt + Digit7"),
                context(ContextCommand::SwitchContext(ContextRef::Id(seven))),
            ),
            (
                hotkey("Ctrl + Alt + Digit8"),
                context(ContextCommand::SwitchContext(ContextRef::Name("8".into()))),
            ),
            (
                hotkey("Ctrl + Alt + Tab"),
                context(ContextCommand::PreviousContext),
            ),
            (
                hotkey("Ctrl + Alt + KeyN"),
                context(ContextCommand::CreateContext("New".into())),
            ),
        ];

        assert_eq!(
            ContextMenuKeys {
                numbers: BTreeMap::default(),
                show_everything: ctrl_alt("0"),
                open_switcher: None,
            },
            ContextMenuKeys::new(&bindings)
        );
    }

    /// R4. New Context from Current Windows… names the context "Context
    /// <n>" from one more than the number of contexts, and skips each name
    /// that a context has in another case or with accents. The model's name
    /// check takes the name it gives, and refuses the names it skips. Pins
    /// the commit's naming, which no rule states.
    #[test]
    fn new_context_names_skip_case_and_accent_variants() {
        for (names, skipped, expected) in [
            (&[][..], &[][..], "Context 1"),
            (&["Comms", "Relax"][..], &[][..], "Context 3"),
            (&["Comms", "context 3"][..], &["Context 3"][..], "Context 4"),
            (&["Cöntéxt 3", "Relax"][..], &["Context 3"][..], "Context 4"),
            (&["Comms", "ÇONTEXT 3"][..], &["Context 3"][..], "Context 4"),
            (
                &["Context 2", "context 3", "CONTEXT 4"][..],
                &["Context 4"][..],
                "Context 5",
            ),
            (&["Context 1"][..], &[][..], "Context 2"),
            (&["Context 5"][..], &[][..], "Context 2"),
        ] {
            let mut contexts = Contexts::new();
            for name in names {
                contexts.create(name).unwrap();
            }
            contexts.switch_to(ContextKey::Everything).unwrap();
            let snapshot = on_screen(&contexts, 0);

            let entries = context_menu(&snapshot, &keys(), all);

            let offered: Vec<&MenuAction> = items(&entries)
                .into_iter()
                .map(|item| &item.action)
                .filter(|action| matches!(action, MenuAction::NewContext(_)))
                .collect();
            let expected_action = MenuAction::NewContext(expected.to_string());
            assert_eq!(vec![&expected_action], offered, "{names:?}");
            assert!(contexts.clone().create(expected).is_ok(), "{names:?}");
            for name in skipped {
                assert!(contexts.clone().create(name).is_err(), "{name}");
            }
        }

        // A deleted context frees its name, and the count starts lower.
        let mut contexts = Contexts::new();
        for name in ["Context 1", "Context 2", "Context 3"] {
            contexts.create(name).unwrap();
        }
        contexts.delete(contexts.by_name("Context 1").unwrap().id).unwrap();
        assert_eq!("Context 4", new_context_name(&on_screen(&contexts, 0)));
    }

    /// Menu bar. With the check that the status menu makes, an item is
    /// enabled exactly when its command exists, except that the active
    /// context can't take the focused window. Switching and creating always
    /// have a command. The Send Window to submenu is enabled while one of
    /// its items is.
    #[test]
    fn with_the_menu_check_an_item_is_enabled_when_its_command_exists() {
        for (active, unsorted) in [(None, 0), (Some("Comms"), 2), (Some("Unsorted"), 0)] {
            let snapshot = snapshot(active, unsorted);

            let entries = context_menu(&snapshot, &keys(), exists);

            for item in items(&entries) {
                let to_active = matches!(
                    item.action,
                    MenuAction::SendWindowTo(id) if ContextKey::Named(id) == snapshot.active
                );
                assert_eq!(
                    exists(&item.action) && !to_active,
                    item.enabled,
                    "{active:?}: {}",
                    item.title
                );
                if matches!(item.action, MenuAction::Switch(_) | MenuAction::NewContext(_)) {
                    assert!(exists(&item.action), "{active:?}: {}", item.title);
                }
            }
            let (enabled, send) = submenu(&entries);
            assert_eq!(send.iter().any(|item| item.enabled), enabled, "{active:?}");
        }
    }

    /// Menu bar. The items send the commands that key bindings in the
    /// config give: each context by its id, Unsorted by its reserved name,
    /// Everything as `show_everything`, and a new context by its name. Send
    /// Window to and Open Switcher send `move_window_to_context` with the
    /// context's id and `open_context_switcher`, once those commands exist.
    #[test]
    fn the_items_send_the_commands_that_key_bindings_give() {
        let snapshot = snapshot(Some("Comms"), 2);
        let [comms, relax, build] = ["Comms", "Relax", "Build"].map(|name| id(&snapshot, name));

        let entries = context_menu(&snapshot, &keys(), all);

        let sent: Vec<(&str, Value)> = entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(item) if item.action != MenuAction::OpenSwitcher => {
                    Some((item.title.as_str(), json(item.action.command())))
                }
                _ => None,
            })
            .collect();
        let by_id =
            |id: ContextId| bound(&format!("{{ switch_context = {{ id = {} }} }}", id.get()));
        assert_eq!(
            vec![
                ("Comms", by_id(comms)),
                ("Relax", by_id(relax)),
                ("Build", by_id(build)),
                ("Unsorted", bound(r#"{ switch_context = "Unsorted" }"#)),
                ("Show Everything", bound(r#""show_everything""#)),
                (
                    "New Context from Current Windows…",
                    bound(r#"{ create_context = "Context 4" }"#)
                ),
            ],
            sent
        );
        assert_eq!(
            serde_json::json!({ "move_window_to_context": { "id": relax } }),
            json(MenuAction::SendWindowTo(relax).command()),
        );
        // M8 adds the command; the item is disabled until then.
        assert!(MenuAction::OpenSwitcher.command().is_none());
    }

    /// The status menu's own check, `command_available`, with the published
    /// snapshot: every item is enabled except Open Switcher, whose command M8
    /// adds. Send Window to now sends a typed command, so its submenu and
    /// every item but the active context's are enabled.
    #[test]
    fn the_status_menu_enables_every_item_but_open_switcher() {
        let entries = context_menu(&snapshot(Some("Unsorted"), 2), &keys(), command_available);

        let disabled: Vec<&str> = items(&entries)
            .into_iter()
            .filter(|item| !item.enabled)
            .map(|item| item.title.as_str())
            .collect();
        assert_eq!(vec!["Open Switcher…"], disabled);
        let (enabled, send) = submenu(&entries);
        assert!(enabled);
        assert_eq!(3, send.len());
        assert!(send.iter().all(|item| item.enabled));
    }

    /// Menu bar, R23. A context whose windows are all closed is still
    /// listed and can still take the focused window, whether its record
    /// waits to learn if its app quit or is empty after the app quit.
    #[test]
    fn contexts_whose_windows_are_closed_are_still_listed() {
        let window = |pid: i32, title: &str| WindowDesc {
            wid: WindowId::new(pid, 1),
            bundle_id: Some(format!("app.{pid}")),
            app_name: Some(format!("App {pid}")),
            title: title.to_string(),
            window_server_id: None,
        };
        let mut contexts = Contexts::new();
        let [comms, relax, build] =
            ["Comms", "Relax", "Build"].map(|name| contexts.create(name).unwrap());
        contexts.add_window(comms, &window(1, "Mail")).unwrap();
        contexts.add_window(relax, &window(2, "Music")).unwrap();
        contexts.add_window(build, &window(3, "Terminal")).unwrap();
        contexts.window_closed(WindowId::new(1, 1));
        contexts.window_closed(WindowId::new(2, 1));
        contexts.app_terminated(2);
        contexts.switch_to(ContextKey::Named(build)).unwrap();
        let snapshot = on_screen(&contexts, 0);
        let open: Vec<usize> = snapshot.contexts.iter().map(|context| context.windows).collect();
        assert_eq!(vec![0, 0, 1], open);

        let entries = context_menu(&snapshot, &keys(), all);

        assert_eq!(
            vec![
                "Comms",
                "Relax",
                "Build",
                "Show Everything",
                "-",
                "New Context from Current Windows…",
                "Send Window to",
                "Open Switcher…",
                "-"
            ],
            titles(&entries)
        );
        let (enabled, send) = submenu(&entries);
        let offered: Vec<(&str, bool)> =
            send.iter().map(|item| (item.title.as_str(), item.enabled)).collect();
        assert!(enabled);
        assert_eq!(vec![("Comms", true), ("Relax", true), ("Build", false)], offered);
    }

    /// R4. A context may have a name like one of the section's items, and
    /// its item still switches to that context. Names that fold to
    /// "Everything" or "Unsorted" are refused, so no context's item stands
    /// for a built-in entry.
    #[test]
    fn a_context_named_like_an_item_switches_to_that_context() {
        let mut contexts = Contexts::new();
        for name in ["ÉVERYTHING", " unsorted "] {
            assert!(contexts.create(name).is_err(), "{name}");
        }
        let show = contexts.create("Show Everything").unwrap();
        let unsorted_work = contexts.create("Unsorted work").unwrap();
        contexts.switch_to(ContextKey::Everything).unwrap();

        let entries = context_menu(&on_screen(&contexts, 1), &keys(), all);

        let switches: Vec<(&str, &MenuAction, bool)> = items(&entries)
            .into_iter()
            .filter(|item| matches!(item.action, MenuAction::Switch(_)))
            .map(|item| (item.title.as_str(), &item.action, item.checked))
            .collect();
        assert_eq!(
            vec![
                (
                    "Show Everything",
                    &MenuAction::Switch(ContextKey::Named(show)),
                    false
                ),
                (
                    "Unsorted work",
                    &MenuAction::Switch(ContextKey::Named(unsorted_work)),
                    false
                ),
                ("Unsorted", &MenuAction::Switch(ContextKey::Unsorted), false),
                (
                    "Show Everything",
                    &MenuAction::Switch(ContextKey::Everything),
                    true
                ),
            ],
            switches
        );
    }
}
