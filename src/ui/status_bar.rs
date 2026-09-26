// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Menu bar icon for displaying the current space ID.

pub(crate) mod context_menu;

use std::cell::RefCell;
use std::ffi::c_void;

pub use context_menu::ContextMenuKeys;
use context_menu::{MenuAction, MenuEntry, MenuItem, command_available, context_menu};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSEventModifierFlags, NSImage, NSMenu, NSMenuDelegate, NSMenuItem, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength,
};
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSData, NSInteger, NSObject, NSObjectProtocol, NSString, ns_string};
use tracing::{Span, debug, warn};

use crate::actor::contexts_snapshot;
use crate::actor::layout::LayoutCommand;
use crate::actor::reactor;
use crate::actor::wm_controller::{self, WmCmd, WmCommand, WmEvent};
use crate::config;
use crate::ui::swift_bridge;

/// Key equivalent info for a menu item (key character and modifier flags).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuKeyEquivalent {
    /// The key character (e.g., "c" for the C key).
    pub key: String,
    /// The modifier flags (Option, Shift, Control, Command).
    pub modifiers: NSEventModifierFlags,
}

impl MenuKeyEquivalent {
    /// Convert a livesplit_hotkey::Hotkey to menu key equivalent format.
    pub fn from_hotkey(hotkey: &livesplit_hotkey::Hotkey) -> Option<Self> {
        let s = hotkey.to_string();
        let mut modifiers = NSEventModifierFlags::empty();

        // Check for modifiers
        if s.contains("Ctrl") {
            modifiers |= NSEventModifierFlags::Control;
        }
        if s.contains("Alt") {
            modifiers |= NSEventModifierFlags::Option;
        }
        if s.contains("Shift") {
            modifiers |= NSEventModifierFlags::Shift;
        }
        if s.contains("Cmd") || s.contains("Super") {
            modifiers |= NSEventModifierFlags::Command;
        }

        // Extract the key name (last part after " + ")
        let key_part = s.rsplit(" + ").next()?;

        // Convert key name to single character for NSMenuItem
        let key = key_part
            .strip_prefix("Key")
            .or_else(|| key_part.strip_prefix("Digit"))
            .map(|k| k.to_lowercase())
            .or_else(|| match key_part {
                "ArrowLeft" => Some("←".to_string()),
                "ArrowRight" => Some("→".to_string()),
                "ArrowUp" => Some("↑".to_string()),
                "ArrowDown" => Some("↓".to_string()),
                "Backslash" => Some("\\".to_string()),
                "Slash" => Some("/".to_string()),
                "Equal" => Some("=".to_string()),
                "Minus" => Some("-".to_string()),
                "Space" => Some(" ".to_string()),
                "Return" | "Enter" => Some("\r".to_string()),
                "Tab" => Some("\t".to_string()),
                "Backspace" => Some("\u{8}".to_string()),
                "Escape" => Some("\u{1b}".to_string()),
                _ => None,
            })?;

        Some(Self { key, modifiers })
    }
}

const SAVE_AND_QUIT_TAG: i64 = 1;
const TOGGLE_GLOBAL_TAG: i64 = 2;
const TOGGLE_SPACE_TAG: i64 = 3;
const FLOAT_WINDOW_TAG: i64 = 4;
const SHOW_PREFERENCES_TAG: i64 = 5;
const CLEAN_UP_SPACE_TAG: i64 = 6;
/// The tag of the first item of the contexts section. Each item of the
/// section has its own tag from here on.
const CONTEXT_ACTION_TAG_BASE: i64 = 1000;

/// `NSControlStateValueOn`, which shows a checkmark on a menu item.
const MENU_ITEM_STATE_ON: NSInteger = 1;

pub struct StatusIcon {
    status_item: Retained<NSStatusItem>,
    mtm: MainThreadMarker,
    menu_handler: Retained<MenuHandler>,
    toggle_item: Retained<NSMenuItem>,
    space_toggle_item: Retained<NSMenuItem>,
    float_window_item: Retained<NSMenuItem>,
    /// Animation frames for the tail swing animation (frame 0 = rest, 1 = right, 2 = left).
    animation_frames: Vec<Retained<NSImage>>,
}

impl StatusIcon {
    /// Creates a new menu bar manager.
    pub fn new(
        config: &config::StatusIconExperimental,
        mtm: MainThreadMarker,
        wm_tx: wm_controller::Sender,
        clean_up_keybinding: Option<MenuKeyEquivalent>,
        toggle_floating_keybinding: Option<MenuKeyEquivalent>,
    ) -> Self {
        let status_bar = NSStatusBar::systemStatusBar();
        let status_item = status_bar.statusItemWithLength(NSVariableStatusItemLength);

        // Create animation frames (frame 0 = rest, 1 = swing right, 2 = swing left)
        let animation_frames = vec![
            create_icon_from_svg(
                include_str!("../../site/src/assets/sugarglider-frame0.svg"),
                config.color,
            )
            .expect("Failed to create animation frame 0"),
            create_icon_from_svg(
                include_str!("../../site/src/assets/sugarglider-frame1.svg"),
                config.color,
            )
            .expect("Failed to create animation frame 1"),
            create_icon_from_svg(
                include_str!("../../site/src/assets/sugarglider-frame2.svg"),
                config.color,
            )
            .expect("Failed to create animation frame 2"),
        ];

        // Set initial icon (rest frame)
        if let Some(button) = status_item.button(mtm) {
            button.setImage(Some(&animation_frames[0]));
        }

        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("Sugarglider"));

        // This is needed to be able to manually set the menu item state
        menu.setAutoenablesItems(false);

        let menu_handler = MenuHandler::new(mtm, wm_tx);
        menu.setDelegate(Some(ProtocolObject::from_ref(&*menu_handler)));

        // Global toggle item - "Stop Globally" when enabled, "Start Globally" when disabled
        let toggle_ns_title = ns_string!("Stop Globally");
        let toggle_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &toggle_ns_title,
                Some(sel!(handleAction:)),
                ns_string!(""),
            )
        };
        unsafe { toggle_item.setTarget(Some(&*menu_handler)) };
        toggle_item.setTag(TOGGLE_GLOBAL_TAG as isize);
        menu.addItem(&toggle_item);

        // Space toggle item - "Stop Space" when enabled, "Start Space" when disabled
        let space_toggle_ns_title = ns_string!("Stop Space");
        let space_toggle_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &space_toggle_ns_title,
                Some(sel!(handleAction:)),
                ns_string!(""),
            )
        };
        unsafe { space_toggle_item.setTarget(Some(&*menu_handler)) };
        space_toggle_item.setTag(TOGGLE_SPACE_TAG as isize);
        menu.addItem(&space_toggle_item);

        menu.addItem(&NSMenuItem::separatorItem(mtm));

        // Float window item
        let float_window_key_equiv = toggle_floating_keybinding
            .as_ref()
            .map(|k| NSString::from_str(&k.key))
            .unwrap_or_else(|| NSString::from_str(""));
        let float_window_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                ns_string!("Float Window"),
                Some(sel!(handleAction:)),
                &float_window_key_equiv,
            )
        };
        unsafe { float_window_item.setTarget(Some(&*menu_handler)) };
        float_window_item.setTag(FLOAT_WINDOW_TAG as isize);
        if let Some(ref kb) = toggle_floating_keybinding {
            float_window_item.setKeyEquivalentModifierMask(kb.modifiers);
        }
        menu.addItem(&float_window_item);

        // Clean up space item
        let clean_up_key_equiv = clean_up_keybinding
            .as_ref()
            .map(|k| NSString::from_str(&k.key))
            .unwrap_or_else(|| NSString::from_str(""));
        let clean_up_space_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                ns_string!("Clean Up Space"),
                Some(sel!(handleAction:)),
                &clean_up_key_equiv,
            )
        };
        unsafe { clean_up_space_item.setTarget(Some(&*menu_handler)) };
        clean_up_space_item.setTag(CLEAN_UP_SPACE_TAG as isize);
        // Set modifier mask if we have a keybinding
        if let Some(ref kb) = clean_up_keybinding {
            clean_up_space_item.setKeyEquivalentModifierMask(kb.modifiers);
        }
        menu.addItem(&clean_up_space_item);

        menu.addItem(&NSMenuItem::separatorItem(mtm));

        // Preferences item with ⌘, shortcut
        let preferences_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                ns_string!("Preferences..."),
                Some(sel!(handleAction:)),
                ns_string!(","),
            )
        };
        unsafe { preferences_item.setTarget(Some(&*menu_handler)) };
        preferences_item.setTag(SHOW_PREFERENCES_TAG as isize);
        menu.addItem(&preferences_item);

        menu.addItem(&NSMenuItem::separatorItem(mtm));

        // Quit item
        let save_quit_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                ns_string!("Quit"),
                Some(sel!(handleAction:)),
                ns_string!(""),
            )
        };
        unsafe { save_quit_item.setTarget(Some(&*menu_handler)) };
        save_quit_item.setTag(SAVE_AND_QUIT_TAG as isize);
        menu.addItem(&save_quit_item);

        status_item.setMenu(Some(&menu));

        Self {
            status_item,
            mtm,
            menu_handler,
            toggle_item,
            space_toggle_item,
            float_window_item,
            animation_frames,
        }
    }

    /// Sets the text next to the icon.
    pub fn set_text(&mut self, text: &str) {
        let ns_title = NSString::from_str(text);
        if let Some(button) = self.status_item.button(self.mtm) {
            button.setTitle(&ns_title);
        } else {
            warn!("Could not get button from status item");
        }
    }

    /// Sets the key equivalents that the contexts section shows.
    pub fn set_context_keys(&mut self, keys: ContextMenuKeys) {
        *self.menu_handler.ivars().context_keys.borrow_mut() = keys;
    }

    /// Sets the toggle menu item title.
    pub fn set_toggle_title(&mut self, title: &str) {
        let ns_title = NSString::from_str(title);
        self.toggle_item.setTitle(&ns_title);
    }

    /// Sets the space toggle menu item title.
    pub fn set_space_toggle_title(&mut self, title: &str) {
        let ns_title = NSString::from_str(title);
        self.space_toggle_item.setTitle(&ns_title);
    }

    /// Sets whether the space toggle menu item is enabled.
    pub fn set_space_toggle_enabled(&mut self, enabled: bool) {
        self.space_toggle_item.setEnabled(enabled);
    }

    /// Sets the float window menu item title based on whether the focused window is floating.
    pub fn set_float_window_title(&mut self, is_floating: bool) {
        let ns_title = NSString::from_str(if is_floating {
            "Unfloat Window"
        } else {
            "Float Window"
        });
        self.float_window_item.setTitle(&ns_title);
    }

    /// Sets the icon to the specified animation frame.
    /// Frame 0 = rest position, 1 = swing right, 2 = swing left.
    pub fn set_animation_frame(&mut self, frame_index: usize) {
        let frame_index = frame_index.min(self.animation_frames.len() - 1);
        if let Some(button) = self.status_item.button(self.mtm) {
            button.setImage(Some(&self.animation_frames[frame_index]));
        }
    }
}

impl Drop for StatusIcon {
    fn drop(&mut self) {
        debug!("Removing menu bar icon");
        let status_bar = NSStatusBar::systemStatusBar();
        status_bar.removeStatusItem(&self.status_item);
    }
}

struct MenuHandlerIvars {
    wm_tx: wm_controller::Sender,
    /// The key equivalents that the contexts section shows.
    context_keys: RefCell<ContextMenuKeys>,
    /// The items of the contexts section, which the next rebuild removes.
    context_items: RefCell<Vec<Retained<NSMenuItem>>>,
    /// The actions of the contexts section's items, by tag, from
    /// `CONTEXT_ACTION_TAG_BASE`.
    context_actions: RefCell<Vec<MenuAction>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuHandlerIvars]
    struct MenuHandler;

    impl MenuHandler {
        #[unsafe(method(handleAction:))]
        fn handle_action(&self, sender: &NSObject) {
            let tag = unsafe {
                let tag: i64 = msg_send![sender, tag];
                tag
            };
            debug!("Menu action received with tag: {}", tag);
            let wm_tx = &self.ivars().wm_tx;
            match tag {
                SAVE_AND_QUIT_TAG => {
                    debug!("Sending SaveAndExit command");
                    let _ = wm_tx.send((
                        Span::current(),
                        WmEvent::Command(WmCommand::ReactorCommand(
                            reactor::Command::Reactor(reactor::ReactorCommand::SaveAndExit),
                        )),
                    ));
                }
                TOGGLE_GLOBAL_TAG => {
                    debug!("Sending ToggleGlobalEnabled command");
                    let _ = wm_tx.send((
                        Span::current(),
                        WmEvent::Command(WmCommand::Wm(WmCmd::ToggleGlobalEnabled)),
                    ));
                }
                TOGGLE_SPACE_TAG => {
                    debug!("Sending ToggleSpaceActivated command");
                    let _ = wm_tx.send((
                        Span::current(),
                        WmEvent::Command(WmCommand::Wm(WmCmd::ToggleSpaceActivated)),
                    ));
                }
                FLOAT_WINDOW_TAG => {
                    debug!("Sending ToggleWindowFloating command");
                    let _ = wm_tx.send((
                        Span::current(),
                        WmEvent::Command(WmCommand::ReactorCommand(
                            reactor::Command::Layout(LayoutCommand::ToggleWindowFloating),
                        )),
                    ));
                }
                CLEAN_UP_SPACE_TAG => {
                    debug!("Sending CleanUpSpace command");
                    let _ = wm_tx.send((
                        Span::current(),
                        WmEvent::Command(WmCommand::ReactorCommand(
                            reactor::Command::Layout(LayoutCommand::CleanUpSpace),
                        )),
                    ));
                }
                SHOW_PREFERENCES_TAG => {
                    debug!("Opening preferences window");
                    swift_bridge::show_preferences();
                }
                tag if tag >= CONTEXT_ACTION_TAG_BASE => {
                    self.run_context_action(tag - CONTEXT_ACTION_TAG_BASE);
                }
                _ => {
                    warn!("Unknown tag: {}", tag);
                }
            }
        }
    }

    unsafe impl NSObjectProtocol for MenuHandler {}

    unsafe impl NSMenuDelegate for MenuHandler {
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self, menu: &NSMenu) {
            self.rebuild_contexts_section(menu);
        }
    }
);

impl MenuHandler {
    /// Creates the parachute icon from the SVG file
    pub fn new(mtm: MainThreadMarker, wm_tx: wm_controller::Sender) -> Retained<Self> {
        let this = Self::alloc(mtm);
        let this = this.set_ivars(MenuHandlerIvars {
            wm_tx,
            context_keys: RefCell::default(),
            context_items: RefCell::default(),
            context_actions: RefCell::default(),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn run_context_action(&self, index: i64) {
        let actions = &self.ivars().context_actions;
        let action = usize::try_from(index).ok().and_then(|i| actions.borrow().get(i).cloned());
        let Some(action) = action else {
            warn!("Unknown contexts menu item: {index}");
            return;
        };
        let Some(command) = action.command() else {
            warn!(?action, "No command for the contexts menu item");
            return;
        };
        debug!(?command, "Sending the contexts menu item's command");
        let _ = self.ivars().wm_tx.send((Span::current(), WmEvent::Command(command)));
    }

    /// Replaces the contexts section at the top of `menu` with one built from
    /// the published contexts snapshot.
    fn rebuild_contexts_section(&self, menu: &NSMenu) {
        let ivars = self.ivars();
        for item in ivars.context_items.take() {
            menu.removeItem(&item);
        }
        let entries = match contexts_snapshot::published() {
            Some(snapshot) => context_menu(&snapshot, &ivars.context_keys.borrow(), command_available),
            None => Vec::new(),
        };
        let mut actions = Vec::new();
        let items: Vec<Retained<NSMenuItem>> =
            entries.iter().map(|entry| self.ns_menu_entry(entry, &mut actions)).collect();
        for (index, item) in items.iter().enumerate() {
            menu.insertItem_atIndex(item, index as NSInteger);
        }
        *ivars.context_items.borrow_mut() = items;
        *ivars.context_actions.borrow_mut() = actions;
    }

    fn ns_menu_entry(
        &self,
        entry: &MenuEntry,
        actions: &mut Vec<MenuAction>,
    ) -> Retained<NSMenuItem> {
        let mtm = self.mtm();
        match entry {
            MenuEntry::Separator => NSMenuItem::separatorItem(mtm),
            MenuEntry::Item(item) => self.ns_menu_item(item, actions),
            MenuEntry::Submenu { title, enabled, items } => {
                let title = NSString::from_str(title);
                let submenu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &title);
                submenu.setAutoenablesItems(false);
                for item in items {
                    submenu.addItem(&self.ns_menu_item(item, actions));
                }
                let parent = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &title,
                        None,
                        ns_string!(""),
                    )
                };
                parent.setSubmenu(Some(&submenu));
                parent.setEnabled(*enabled);
                parent
            }
        }
    }

    /// An item whose tag is its action's place in `actions`, from
    /// `CONTEXT_ACTION_TAG_BASE`.
    fn ns_menu_item(&self, item: &MenuItem, actions: &mut Vec<MenuAction>) -> Retained<NSMenuItem> {
        let key = NSString::from_str(item.key.as_ref().map_or("", |key| key.key.as_str()));
        let ns_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(self.mtm()),
                &NSString::from_str(&item.title),
                Some(sel!(handleAction:)),
                &key,
            )
        };
        unsafe { ns_item.setTarget(Some(self)) };
        ns_item.setTag((CONTEXT_ACTION_TAG_BASE + actions.len() as i64) as isize);
        actions.push(item.action.clone());
        if let Some(key) = &item.key {
            ns_item.setKeyEquivalentModifierMask(key.modifiers);
        }
        if item.checked {
            let _: () = unsafe { msg_send![&*ns_item, setState: MENU_ITEM_STATE_ON] };
        }
        ns_item.setEnabled(item.enabled);
        ns_item
    }
}

fn create_icon_from_svg(svg_data: &str, use_color: bool) -> Option<Retained<NSImage>> {
    let ns_data =
        unsafe { NSData::dataWithBytes_length(svg_data.as_ptr() as *const c_void, svg_data.len()) };

    let Some(image) = NSImage::initWithData(NSImage::alloc(), &ns_data) else {
        return None;
    };

    // Set the image size to be appropriate for menu bar (16x16 points)
    image.setSize(CGSize { width: 16.0, height: 16.0 });

    if !use_color {
        // Set as template image so it follows system appearance
        image.setTemplate(true);
    }

    Some(image)
}
