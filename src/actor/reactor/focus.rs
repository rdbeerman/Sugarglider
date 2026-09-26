// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Focus from outside: when the user focuses a window of another context,
//! Sugarglider switches to that context. The design is in
//! `docs/specs/contexts.md`.

use std::time::{Duration, Instant};

use tracing::{debug, info, warn};

use super::Reactor;
use crate::actor::app::{Quiet, Request, WindowId, pid_t};
use crate::actor::layout::{EventResponse, LayoutEvent};
use crate::collections::HashSet;
use crate::model::contexts::ContextKey;

const FINDER: &str = "com.apple.finder";

/// How long a switch waits at most for its end, in case the events that end it
/// never arrive, for example because an app didn't answer a write.
const GUARD_DEADLINE: Duration = Duration::from_secs(2);

/// What a switch waits for before focus from outside counts again. Until then,
/// activations and main window changes can be the switch's own.
#[derive(Debug, Default)]
pub(super) struct SwitchGuard {
    /// The raise sequence that focuses the switch's window, and the window.
    raise: Option<(u64, WindowId)>,
    /// When the switch raised nothing: the windows it parked whose echo of the
    /// parking write hasn't arrived.
    echoes: HashSet<WindowId>,
    /// Finder, when the switch activated it because no window could take focus,
    /// until its activation arrives.
    finder: Option<pid_t>,
    /// When the switch started to wait.
    pub(super) since: Option<Instant>,
}

impl SwitchGuard {
    fn holds(&self) -> bool {
        self.raise.is_some() || !self.echoes.is_empty() || self.finder.is_some()
    }
}

/// What focus from outside did.
#[derive(Debug, PartialEq)]
enum FocusOutcome {
    /// The window keeps the focus, and the context stays.
    Stays,
    /// Sugarglider switched to a context that shows the window.
    Switched,
    /// Sugarglider raised a member of the active context instead.
    RaisedMember,
    /// Nothing happened, and the focus doesn't count.
    Ignored,
}

impl Reactor {
    /// Handles a window that took focus in a way that counts as the user's.
    /// While a switch is in progress it is ignored. A window the reactor hasn't
    /// seen yet waits until the reactor first sees it.
    pub(super) fn focus_changed(&mut self, wid: WindowId) {
        if !self.contexts_enabled() {
            return;
        }
        if self.switch_guard.holds() {
            debug!(?wid, guard = ?self.switch_guard, "Ignoring focus while a switch is in progress");
            return;
        }
        if !self.windows.contains_key(&wid) {
            debug!(?wid, "Focus on a window not seen yet waits for it");
            self.focus_waiting = Some(wid);
            return;
        }
        self.focus_waiting = None;
        if self.focus_from_outside(wid) == FocusOutcome::Stays {
            self.contexts.window_focused(wid);
        }
    }

    /// Applies focus that waited for windows the reactor sees for the first
    /// time, now that their membership is decided.
    pub(super) fn focus_windows_seen(&mut self, wids: &[WindowId]) {
        let Some(waiting) = self.focus_waiting else { return };
        if !wids.contains(&waiting) {
            return;
        }
        self.focus_waiting = None;
        if self.main_window() == Some(waiting) {
            self.focus_changed(waiting);
        }
    }

    /// When the user focuses a window that isn't a member of its screen's
    /// active context, switches to the most recently used context that holds
    /// it, or to Unsorted. When the window is parked and its app has a visible
    /// member of the active context, raises that member instead, and switches
    /// only when the app has no member there.
    ///
    /// Windows Sugarglider doesn't track, its own windows, a screen that shows
    /// Everything, and focus before startup completes change nothing.
    fn focus_from_outside(&mut self, wid: WindowId) -> FocusOutcome {
        if !self.contexts_in_use() || self.pending_exit.is_some() || !self.startup_complete {
            return FocusOutcome::Stays;
        }
        let own_pid = std::process::id() as pid_t;
        let Some(info) = self.layout_window_info(wid) else {
            return FocusOutcome::Ignored;
        };
        if wid.pid == own_pid || self.layout.is_untracked(&info) {
            return FocusOutcome::Stays;
        }
        let shown = match self.best_space_for_window(&info.frame) {
            Some(space) => self.shown_context(space),
            None => self.contexts.active(),
        };
        if self.shows_under(shown, wid) {
            return FocusOutcome::Stays;
        }
        if self.parked.contains_key(&wid) {
            if let Some(member) = self.visible_member_of_app(wid.pid, shown) {
                info!(?wid, ?member, "Raising the app's member of the active context");
                self.handle_layout_response(EventResponse {
                    focus_window: Some(member),
                    ..Default::default()
                });
                return FocusOutcome::RaisedMember;
            }
            let has_member = self
                .windows
                .keys()
                .any(|&other| other.pid == wid.pid && self.shows_under(shown, other));
            if has_member {
                debug!(?wid, "The app has a member of the active context; not switching");
                return FocusOutcome::Ignored;
            }
        }
        let target = self.contexts.focus_target(self.membership_window(wid));
        info!(?wid, ?target, "Focus from outside the active context; switching");
        _ = self.switch_context_focusing(target, Some(wid));
        FocusOutcome::Switched
    }

    /// The most recently focused window of the app that is a member of `key`,
    /// is in the visible-window set, and isn't parked.
    fn visible_member_of_app(&self, pid: pid_t, key: ContextKey) -> Option<WindowId> {
        self.windows
            .iter()
            .filter(|&(&wid, window)| {
                wid.pid == pid
                    && !self.parked.contains_key(&wid)
                    && self.shows_under(key, wid)
                    && window
                        .window_server_id
                        .is_some_and(|wsid| self.visible_windows.contains(&wsid))
            })
            .map(|(&wid, _)| wid)
            .max_by_key(|&wid| (self.contexts.last_focus(wid), wid))
    }

    /// After windows were parked, handles the layout's `response` and raises
    /// `focus`, unless it is the main window already and `always` is false.
    /// When there is no window to focus, activates Finder instead. Focus from
    /// outside counts again when that ends.
    pub(super) fn focus_after_parking(
        &mut self,
        mut response: EventResponse,
        focus: Option<WindowId>,
        always: bool,
        parked: &[WindowId],
    ) {
        if let Some(focus) = focus
            && (always || self.main_window() != Some(focus))
        {
            response.focus_window = Some(focus);
        }
        let raised = response.focus_window;
        let sequence = self.handle_layout_response(response);
        let finder = match focus {
            Some(_) => None,
            None => self.activate_finder(),
        };
        self.guard_switch(sequence.zip(raised), parked, finder);
    }

    /// Starts waiting for the end of a switch: its raise sequence, which
    /// focuses `raise`'s window, or, when it raised nothing, the echo of every
    /// window it parked. With `finder`, the wait also lasts until Finder's
    /// activation arrives.
    fn guard_switch(
        &mut self,
        raise: Option<(u64, WindowId)>,
        parked: &[WindowId],
        finder: Option<pid_t>,
    ) {
        let echoes = match raise {
            Some(_) => HashSet::default(),
            None => parked.iter().copied().collect(),
        };
        self.switch_guard = SwitchGuard {
            raise,
            echoes,
            finder,
            since: Some(Instant::now()),
        };
        debug!(guard = ?self.switch_guard, "Waiting for the switch to end");
    }

    /// Stops waiting for the end of a switch that started to wait 2 seconds or
    /// more before `now`. The reactor's visibility refresh calls this.
    pub(super) fn guard_deadline_tick(&mut self, now: Instant) {
        let guard = &self.switch_guard;
        if guard.holds()
            && guard
                .since
                .is_some_and(|since| now.saturating_duration_since(since) >= GUARD_DEADLINE)
        {
            warn!(
                ?guard,
                "The switch didn't end in time; focus from outside counts again"
            );
            self.switch_guard = SwitchGuard::default();
        }
    }

    /// A raise sequence reported a completed raise of `window`, or with `None`,
    /// that it failed or timed out. A sequence at least as new as the switch's
    /// ends the wait for it, because a request identical to the queued one
    /// replaces it.
    pub(super) fn raise_ended(&mut self, sequence_id: u64, window: Option<WindowId>) {
        if let Some((sequence, focus)) = self.switch_guard.raise
            && sequence_id >= sequence
            && window.is_none_or(|window| window == focus)
        {
            self.switch_guard.raise = None;
            self.log_guard_end();
        }
    }

    /// The echo of the last frame write to the window arrived.
    pub(super) fn frame_write_echoed(&mut self, wid: WindowId) {
        if self.switch_guard.echoes.remove(&wid) {
            self.log_guard_end();
        }
    }

    /// The app became active.
    pub(super) fn app_activated(&mut self, pid: pid_t) {
        if self.switch_guard.finder == Some(pid) {
            self.switch_guard.finder = None;
            self.log_guard_end();
        }
    }

    /// Stops waiting on a window that is gone.
    pub(super) fn guarded_window_gone(&mut self, wid: WindowId) {
        let guard = &mut self.switch_guard;
        let echoed = guard.echoes.remove(&wid);
        let raised = guard.raise.take_if(|(_, focus)| *focus == wid).is_some();
        if echoed || raised {
            self.log_guard_end();
        }
    }

    /// Stops waiting on an app that is gone.
    pub(super) fn guarded_app_gone(&mut self, pid: pid_t) {
        let guard = &mut self.switch_guard;
        let before = guard.echoes.len();
        guard.echoes.retain(|wid| wid.pid != pid);
        let raised = guard.raise.take_if(|(_, focus)| focus.pid == pid).is_some();
        let finder = guard.finder.take_if(|finder| *finder == pid).is_some();
        if raised || finder || guard.echoes.len() != before {
            self.log_guard_end();
        }
    }

    fn log_guard_end(&self) {
        if !self.switch_guard.holds() {
            debug!("The switch has ended; focus from outside counts again");
        }
    }

    /// When a switch leaves no window to focus, activates Finder quietly, so
    /// that keystrokes don't go to a parked window. Returns Finder's pid when
    /// its activation is to come.
    pub(super) fn activate_finder(&mut self) -> Option<pid_t> {
        let (&pid, app) = self
            .apps
            .iter()
            .find(|(_, app)| app.info.bundle_id.as_deref() == Some(FINDER))?;
        if self.main_window_tracker.frontmost_app() == Some(pid) {
            return None;
        }
        info!(pid, "No window can take focus; activating Finder");
        app.handle.send(Request::Activate(Quiet::Yes)).ok()?;
        Some(pid)
    }

    /// Selects the window in the layouts of the visible Spaces.
    pub(super) fn select_in_layout(&mut self, wid: WindowId) {
        let spaces = self.screens.iter().flat_map(|screen| screen.space).collect();
        self.send_layout_event(LayoutEvent::WindowFocused(spaces, wid));
    }
}
