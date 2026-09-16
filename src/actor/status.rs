// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Manages the status bar icon.

use std::sync::Arc;
use std::time::Duration;

use objc2::MainThreadMarker;
use tracing::instrument;

use crate::actor::wm_controller;
use crate::config::Config;
use crate::sys::screen::{SpaceId, get_active_space_number};
use crate::sys::timer::Timer;
use crate::ui::status_bar::StatusIcon;
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
    ConfigUpdated(Arc<Config>),
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
        };
        this.apply_config();
        this.update_toggle_title(true);
        this
    }

    fn apply_config(&mut self) {
        let icon = self.icon.take();
        if self.config.settings.status_icon.enable {
            self.icon = icon.or_else(|| {
                Some(StatusIcon::new(
                    &self.config.settings.experimental.status_icon,
                    self.mtm,
                    self.wm_tx.clone(),
                ))
            });
        }
        self.update_space();
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
        }
    }

    fn update_space(&mut self) {
        let Some(icon) = &mut self.icon else { return };
        if self.config.settings.experimental.status_icon.space_index {
            // TODO: Move this off the main thread.
            let label = trace_call!(get_active_space_number())
                .map(|n| n.to_string())
                .unwrap_or_default();
            icon.set_text(&label);
        } else {
            icon.set_text("");
        }
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
