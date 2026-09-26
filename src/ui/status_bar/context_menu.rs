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

/// The name of the command that moves the focused window to a context, as a
/// key binding writes it.
const MOVE_WINDOW_TO_CONTEXT: &str = "move_window_to_context";
/// The name of the command that opens the switcher, as a key binding writes
/// it.
const OPEN_CONTEXT_SWITCHER: &str = "open_context_switcher";

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
                command_named(json!({ MOVE_WINDOW_TO_CONTEXT: { "id": id } }))
            }
            MenuAction::OpenSwitcher => command_named(json!(OPEN_CONTEXT_SWITCHER)),
        }
    }
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
    /// For each number that a `switch_context` binding names: the number,
    /// with the modifiers of the binding.
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
                    keys.numbers.entry(*number).or_insert(MenuKeyEquivalent {
                        key: number.to_string(),
                        modifiers: key.modifiers,
                    });
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
    let item = |title: &str, action: MenuAction, key: Option<MenuKeyEquivalent>| MenuItem {
        title: title.to_string(),
        checked: false,
        enabled: available(&action),
        action,
        key,
    };
    let switch = |title: &str, to: ContextKey, key: Option<MenuKeyEquivalent>| {
        MenuEntry::Item(MenuItem {
            checked: active == to,
            ..item(title, MenuAction::Switch(to), key)
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
    if active == ContextKey::Unsorted || snapshot.unsorted.windows > 0 {
        entries.push(switch(UNSORTED_NAME, ContextKey::Unsorted, None));
    }
    entries.push(switch(
        "Show Everything",
        ContextKey::Everything,
        keys.show_everything.clone(),
    ));
    entries.push(MenuEntry::Separator);

    let name = new_context_name(snapshot);
    entries.push(MenuEntry::Item(item(
        "New Context from Current Windows…",
        MenuAction::NewContext(name),
        None,
    )));
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
    use crate::actor::contexts_snapshot::ScreenContext;
    use crate::actor::wm_controller::WmCmd;
    use crate::model::contexts::Contexts;

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
        ContextsSnapshot::new(&contexts, screens, unsorted)
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

    /// R29. Unsorted is listed, after the contexts, while it has windows or
    /// is active.
    #[test]
    fn unsorted_is_listed_while_it_has_windows_or_is_active() {
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

        let active = context_menu(&snapshot(Some("Unsorted"), 0), &keys(), all);
        let unsorted = items(&active).into_iter().find(|item| item.title == "Unsorted");
        assert_eq!(
            Some(&MenuItem {
                checked: true,
                ..item("Unsorted", MenuAction::Switch(ContextKey::Unsorted))
            }),
            unsorted
        );
        assert_eq!(vec!["Unsorted"], checked(&active));
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
        let snapshot = ContextsSnapshot::new(&contexts, Vec::new(), 0);

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
        let snapshot = ContextsSnapshot::new(&contexts, Vec::new(), 0);

        assert_eq!("Context 5", new_context_name(&snapshot));
        assert_eq!("Context 1", new_context_name(&ContextsSnapshot::off()));
    }

    /// A context's key equivalent is its number, with the modifiers of the
    /// first `switch_context` binding of that number. Show Everything takes
    /// the key of its binding. Other bindings don't count.
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
                            key: "2".to_string(),
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
}
