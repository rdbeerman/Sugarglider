// Copyright The Sugarglider Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Menu bar icon for displaying the current space ID.

use std::ffi::c_void;

use objc2::rc::Retained;
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSData, NSObject, NSString, ns_string};
use tracing::{Span, debug, warn};

use crate::actor::layout::LayoutCommand;
use crate::actor::reactor;
use crate::actor::wm_controller::{self, WmCmd, WmCommand, WmEvent};
use crate::config;
use crate::ui::swift_bridge;

const SAVE_AND_QUIT_TAG: i64 = 1;
const TOGGLE_GLOBAL_TAG: i64 = 2;
const TOGGLE_SPACE_TAG: i64 = 3;
const FLOAT_WINDOW_TAG: i64 = 4;
const SHOW_PREFERENCES_TAG: i64 = 5;

pub struct StatusIcon {
    status_item: Retained<NSStatusItem>,
    mtm: MainThreadMarker,
    _menu_handler: Retained<MenuHandler>,
    toggle_item: Retained<NSMenuItem>,
    space_toggle_item: Retained<NSMenuItem>,
    /// Animation frames for the tail swing animation (frame 0 = rest, 1 = right, 2 = left).
    animation_frames: Vec<Retained<NSImage>>,
}

impl StatusIcon {
    /// Creates a new menu bar manager.
    pub fn new(
        config: &config::StatusIconExperimental,
        mtm: MainThreadMarker,
        wm_tx: wm_controller::Sender,
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

        // Float window item
        let float_window_item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                ns_string!("Float Window"),
                Some(sel!(handleAction:)),
                ns_string!(""),
            )
        };
        unsafe { float_window_item.setTarget(Some(&*menu_handler)) };
        float_window_item.setTag(FLOAT_WINDOW_TAG as isize);
        menu.addItem(&float_window_item);

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
            _menu_handler: menu_handler,
            toggle_item,
            space_toggle_item,
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
                SHOW_PREFERENCES_TAG => {
                    debug!("Opening preferences window");
                    swift_bridge::show_preferences();
                }
                _ => {
                    warn!("Unknown tag: {}", tag);
                }
            }
        }
    }
);

impl MenuHandler {
    /// Creates the parachute icon from the SVG file
    pub fn new(mtm: MainThreadMarker, wm_tx: wm_controller::Sender) -> Retained<Self> {
        let this = Self::alloc(mtm);
        let this = this.set_ivars(MenuHandlerIvars { wm_tx });
        unsafe { msg_send![super(this), init] }
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
