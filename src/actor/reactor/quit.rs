// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Quitting. Before Sugarglider quits, it puts every parked window back and
//! waits until every window in the journal has reported its frame, for at
//! most 2 seconds.

use std::time::{Duration, Instant};

use tracing::{debug, error, info, warn};

use super::Reactor;
use crate::actor::app::{WindowId, pid_t};
use crate::sys::window_server::WindowServerId;

/// How long a quit waits for windows to come back from parking.
const EXIT_DEADLINE: Duration = Duration::from_secs(2);

/// A quit that waits for windows to come back from parking.
#[derive(Debug)]
pub(super) struct PendingExit {
    since: Instant,
    /// The journal entries that must go before the quit.
    waiting: Vec<(pid_t, WindowServerId)>,
}

impl Reactor {
    /// Saves the state and quits. While contexts are in use, every visible
    /// Space shows Everything first, which puts the parked windows back. The
    /// quit waits until every window that the journal lists, and that the
    /// reactor knows, has reported its frame, or until the first visibility
    /// refresh 2 seconds after `now`. The saved active context doesn't
    /// change.
    pub(super) fn save_and_exit(&mut self, now: Instant) {
        if self.pending_exit.is_some() {
            debug!("Already waiting to quit");
            return;
        }
        let waiting = self.journaled_known_windows();
        if waiting.is_empty() {
            self.exit_now();
            return;
        }
        info!(
            count = waiting.len(),
            parked = self.parked.len(),
            "Waiting for windows to come back from parking before quitting"
        );
        self.pending_exit = Some(PendingExit { since: now, waiting });
        if self.parked.is_empty() && !self.contexts_in_use() {
            return;
        }
        self.layout.cancel_interactive_state();
        self.in_drag = false;
        self.resizing_window = None;
        self.title_bar_drag = None;
        self.apply_again();
        let rest: Vec<WindowId> = self.parked.keys().copied().collect();
        if !rest.is_empty() {
            self.unpark_windows(&rest);
        }
    }

    /// The journal entries of windows the reactor knows. The entries of apps
    /// that haven't registered can only be put back at the next launch.
    fn journaled_known_windows(&self) -> Vec<(pid_t, WindowServerId)> {
        self.journal
            .entries()
            .iter()
            .filter(|entry| {
                self.windows.iter().any(|(wid, window)| {
                    wid.pid == entry.pid && window.window_server_id == Some(entry.window_server_id)
                })
            })
            .map(|entry| (entry.pid, entry.window_server_id))
            .collect()
    }

    /// Quits if a quit is waiting and every window it waits for is back or
    /// gone.
    pub(super) fn exit_if_windows_are_back(&mut self) {
        let Some(pending) = &self.pending_exit else { return };
        if pending.waiting.iter().any(|&(pid, wsid)| self.journal.get(pid, wsid).is_some()) {
            return;
        }
        info!("Every window is back from parking; quitting");
        self.exit_now();
    }

    /// Quits if a quit has waited for 2 seconds at `now`. The journal keeps
    /// the windows that aren't back, and the next launch puts them back.
    pub(super) fn exit_deadline_tick(&mut self, now: Instant) {
        let Some(pending) = &self.pending_exit else { return };
        if now.saturating_duration_since(pending.since) < EXIT_DEADLINE {
            return;
        }
        warn!(
            waiting = self.journal.entries().len(),
            "Quitting before every parked window is back; the journal keeps them"
        );
        self.exit_now();
    }

    fn exit_now(&mut self) {
        self.pending_exit = None;
        if self.contexts_enabled() {
            self.save_contexts();
        }
        let code = match &self.layout_file {
            Some(path) => match self.layout.save(path.clone()) {
                Ok(()) => 0,
                Err(err) => {
                    error!("Could not save layout: {err}");
                    3
                }
            },
            None => 0,
        };
        (self.exit)(code);
    }
}
