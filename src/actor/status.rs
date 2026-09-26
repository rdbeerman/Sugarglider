// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Manages the status bar icon.

use std::sync::Arc;
use std::time::Duration;

use objc2::MainThreadMarker;
use tracing::instrument;

use crate::actor::contexts_snapshot::ContextsSnapshot;
use crate::actor::layout::LayoutCommand;
use crate::actor::reactor::Command as ReactorCommand;
use crate::actor::wm_controller::{self, WmCommand};
use crate::config::Config;
use crate::model::contexts::ContextKey;
use crate::sys::screen::{SpaceId, get_active_space_number};
use crate::sys::timer::Timer;
use crate::ui::status_bar::{ContextMenuKeys, MenuKeyEquivalent, StatusIcon};
use crate::{actor, trace_call};

/// Animation frame sequence: rest -> right -> rest -> left -> repeat
const ANIMATION_SEQUENCE: [usize; 4] = [0, 1, 0, 2];
/// Duration of each animation frame in milliseconds.
const ANIMATION_FRAME_DURATION_MS: u64 = 75;
/// Number of animation cycles to run.
const ANIMATION_CYCLES: usize = 4;

#[derive(Debug)]
pub enum Event {
    // Note: These should not be filtered for active (they should all be Some)
    // so we can always show the user the current space id.
    SpaceChanged(Vec<Option<SpaceId>>),
    FocusedScreenChanged,
    GlobalEnabledChanged(bool),
    SpaceEnabledChanged(bool),
    /// The focused window's floating state changed.
    FocusedWindowFloatingChanged(bool),
    ConfigUpdated(Arc<Config>),
    /// Trigger the tail swing animation (e.g., after clean up space).
    Animate,
    /// The reactor published a new snapshot of the contexts.
    ContextsChanged(Arc<ContextsSnapshot>),
}

/// Animation state for the status icon tail swing.
struct AnimationState {
    /// Timer for driving the animation.
    timer: Timer,
    /// Current position in the animation sequence.
    sequence_index: usize,
    /// Number of cycles completed.
    cycles_completed: usize,
}

pub struct Status {
    config: Arc<Config>,
    rx: Receiver,
    icon: Option<StatusIcon>,
    mtm: MainThreadMarker,
    wm_tx: wm_controller::Sender,
    /// Current animation state, if any.
    animation: Option<AnimationState>,
    /// The active Space's number, while the setting to show it is on.
    space_number: Option<usize>,
    /// The name of the context that the title shows.
    context_name: Option<String>,
}

pub type Sender = actor::Sender<Event>;
pub type Receiver = actor::Receiver<Event>;

impl Status {
    pub fn new(
        config: Arc<Config>,
        rx: Receiver,
        mtm: MainThreadMarker,
        wm_tx: wm_controller::Sender,
    ) -> Self {
        let mut this = Self {
            icon: None,
            config,
            rx,
            mtm,
            wm_tx,
            animation: None,
            space_number: None,
            context_name: None,
        };
        this.apply_config();
        this.update_toggle_title(true);
        this
    }

    fn apply_config(&mut self) {
        let icon = self.icon.take();
        if self.config.settings.status_icon.enable {
            self.icon = icon.or_else(|| {
                let clean_up_kb = Self::find_clean_up_keybinding(&self.config);
                let toggle_floating_kb = Self::find_toggle_floating_keybinding(&self.config);
                Some(StatusIcon::new(
                    &self.config.settings.experimental.status_icon,
                    self.mtm,
                    self.wm_tx.clone(),
                    clean_up_kb,
                    toggle_floating_kb,
                ))
            });
        }
        if let Some(icon) = &mut self.icon {
            icon.set_context_keys(ContextMenuKeys::new(&self.config.keys));
        }
        self.update_space();
    }

    /// Find the keybinding for CleanUpSpace command in the config.
    fn find_clean_up_keybinding(config: &Config) -> Option<MenuKeyEquivalent> {
        for (hotkey, cmd) in &config.keys {
            if matches!(
                cmd,
                WmCommand::ReactorCommand(ReactorCommand::Layout(LayoutCommand::CleanUpSpace))
            ) {
                return MenuKeyEquivalent::from_hotkey(hotkey);
            }
        }
        None
    }

    /// Find the keybinding for ToggleWindowFloating command in the config.
    fn find_toggle_floating_keybinding(config: &Config) -> Option<MenuKeyEquivalent> {
        for (hotkey, cmd) in &config.keys {
            if matches!(
                cmd,
                WmCommand::ReactorCommand(ReactorCommand::Layout(
                    LayoutCommand::ToggleWindowFloating
                ))
            ) {
                return MenuKeyEquivalent::from_hotkey(hotkey);
            }
        }
        None
    }

    pub async fn run(mut self) {
        if self.icon.is_none() {
            return;
        }

        loop {
            // If we have an active animation, poll both the event channel and animation timer
            if let Some(ref mut anim) = self.animation {
                tokio::select! {
                    biased;

                    Some((span, event)) = self.rx.recv() => {
                        let _guard = span.enter();
                        self.handle_event(event);
                    }

                    Some(_) = anim.timer.next() => {
                        self.advance_animation();
                    }
                }
            } else {
                // No animation, just wait for events
                let Some((span, event)) = self.rx.recv().await else {
                    break;
                };
                let _guard = span.enter();
                self.handle_event(event);
            }
        }
    }

    #[instrument(skip(self))]
    fn handle_event(&mut self, event: Event) {
        match event {
            Event::SpaceChanged(_) | Event::FocusedScreenChanged => self.update_space(),
            Event::GlobalEnabledChanged(enabled) => self.update_toggle_title(enabled),
            Event::SpaceEnabledChanged(enabled) => self.update_space_toggle_title(enabled),
            Event::FocusedWindowFloatingChanged(is_floating) => {
                self.update_float_window_title(is_floating)
            }
            Event::ConfigUpdated(config) => {
                if self.config.settings.experimental.status_icon
                    != config.settings.experimental.status_icon
                {
                    // Remove icon so it gets recreated.
                    self.icon.take();
                }
                self.config = config;
                self.apply_config();
            }
            Event::Animate => self.start_animation(),
            Event::ContextsChanged(snapshot) => {
                self.context_name = shown_context_name(&snapshot).map(str::to_string);
                self.update_title();
            }
        }
    }

    fn update_space(&mut self) {
        if self.icon.is_none() {
            return;
        }
        self.space_number = if self.config.settings.experimental.status_icon.space_index {
            // TODO: Move this off the main thread.
            trace_call!(get_active_space_number())
        } else {
            None
        };
        self.update_title();
    }

    fn update_title(&mut self) {
        let Some(icon) = &mut self.icon else { return };
        icon.set_text(&status_title(self.space_number, self.context_name.as_deref()));
    }

    fn update_toggle_title(&mut self, enabled: bool) {
        let Some(icon) = &mut self.icon else { return };
        icon.set_toggle_title(if enabled {
            "Stop Globally"
        } else {
            "Start Globally"
        });
        icon.set_space_toggle_enabled(enabled);

        // Animate the icon when tiling is enabled
        if enabled {
            self.start_animation();
        }
    }

    fn update_space_toggle_title(&mut self, enabled: bool) {
        let Some(icon) = &mut self.icon else { return };
        icon.set_space_toggle_title(if enabled { "Stop Space" } else { "Start Space" });
    }

    fn update_float_window_title(&mut self, is_floating: bool) {
        let Some(icon) = &mut self.icon else { return };
        icon.set_float_window_title(is_floating);
    }

    /// Starts the tail swing animation.
    fn start_animation(&mut self) {
        // Don't restart if already animating
        if self.animation.is_some() {
            return;
        }

        let frame_duration = Duration::from_millis(ANIMATION_FRAME_DURATION_MS);
        self.animation = Some(AnimationState {
            timer: Timer::repeating(frame_duration, frame_duration),
            sequence_index: 0,
            cycles_completed: 0,
        });

        // Set initial frame
        if let Some(icon) = &mut self.icon {
            icon.set_animation_frame(ANIMATION_SEQUENCE[0]);
        }
    }

    /// Advances the animation to the next frame.
    fn advance_animation(&mut self) {
        let Some(anim) = &mut self.animation else {
            return;
        };

        // Advance to next frame in sequence
        anim.sequence_index += 1;
        if anim.sequence_index >= ANIMATION_SEQUENCE.len() {
            anim.sequence_index = 0;
            anim.cycles_completed += 1;
        }

        // Check if animation is complete
        if anim.cycles_completed >= ANIMATION_CYCLES {
            // Reset to rest frame and stop animation
            if let Some(icon) = &mut self.icon {
                icon.set_animation_frame(0);
            }
            self.animation = None;
            return;
        }

        // Update frame
        if let Some(icon) = &mut self.icon {
            icon.set_animation_frame(ANIMATION_SEQUENCE[anim.sequence_index]);
        }
    }
}

/// The name of the context that the status item shows: the active context.
/// `None` under Everything, and while contexts are off.
fn shown_context_name(snapshot: &ContextsSnapshot) -> Option<&str> {
    if !snapshot.enabled {
        return None;
    }
    match snapshot.active {
        ContextKey::Everything => None,
        key => snapshot.name(key),
    }
}

/// The text next to the status icon: the Space's number, the context's name,
/// or both, as in "2 · Comms".
fn status_title(space_number: Option<usize>, context_name: Option<&str>) -> String {
    match (space_number, context_name) {
        (Some(number), Some(name)) => format!("{number} · {name}"),
        (Some(number), None) => number.to_string(),
        (None, Some(name)) => name.to_string(),
        (None, None) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::actor::contexts_snapshot::ScreenContext;
    use crate::model::contexts::Contexts;

    /// Comms and Relax, with `active` active.
    fn snapshot(active: Option<&str>) -> ContextsSnapshot {
        let mut contexts = Contexts::new();
        let comms = contexts.create("Comms").unwrap();
        let relax = contexts.create("Relax").unwrap();
        let key = match active {
            Some("Comms") => ContextKey::Named(comms),
            Some("Relax") => ContextKey::Named(relax),
            Some("Unsorted") => ContextKey::Unsorted,
            Some(other) => panic!("no context {other}"),
            None => ContextKey::Everything,
        };
        contexts.switch_to(key).unwrap();
        let screens = vec![ScreenContext { id: 1, shows: key }];
        ContextsSnapshot::new(&contexts, screens, 1)
    }

    fn title(space_number: Option<usize>, snapshot: &ContextsSnapshot) -> String {
        status_title(space_number, shown_context_name(snapshot))
    }

    /// The title names the active context, and adds nothing under
    /// Everything.
    #[test]
    fn the_title_names_the_active_context() {
        assert_eq!("Comms", title(None, &snapshot(Some("Comms"))));
        assert_eq!("Relax", title(None, &snapshot(Some("Relax"))));
        assert_eq!("Unsorted", title(None, &snapshot(Some("Unsorted"))));
        assert_eq!("", title(None, &snapshot(None)));
    }

    /// With the Space index on, the title shows the Space's number before
    /// the context's name, and only the number under Everything.
    #[test]
    fn the_title_shows_the_space_number_before_the_context() {
        assert_eq!("2 · Comms", title(Some(2), &snapshot(Some("Comms"))));
        assert_eq!("2", title(Some(2), &snapshot(None)));
    }

    /// R28. With contexts off, the title is the Space's number while the
    /// Space index is on, and empty otherwise, as without contexts.
    #[test]
    fn with_contexts_off_the_title_is_as_without_contexts() {
        let off = ContextsSnapshot::off();
        assert_eq!("", title(None, &off));
        assert_eq!("3", title(Some(3), &off));
        let disabled = ContextsSnapshot {
            enabled: false,
            ..snapshot(Some("Comms"))
        };
        assert_eq!("3", title(Some(3), &disabled));
    }
}
