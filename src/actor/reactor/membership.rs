// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Membership: which contexts a window belongs to. The reactor decides it
//! when it first sees a window, and keeps it current as windows change and
//! close. The design is in `docs/specs/contexts.md`.

use tracing::{debug, error, info};

use super::Reactor;
use crate::actor::app::{WindowId, pid_t};
use crate::model::contexts::{Arrival, ContextKey, MatchPass, WindowDesc, plan_switch};

impl Reactor {
    /// Decides the membership of windows that the reactor sees for the first
    /// time (R38). The reactor's windows are the windows it has seen, so a
    /// window that comes back from being minimized, from a hidden app, or
    /// from another Space is not new.
    ///
    /// Windows found before `StartupComplete` were open before Sugarglider
    /// started. They rejoin the contexts whose records they match, or stay
    /// unsorted. Later windows rejoin the contexts whose records they match
    /// (R21), or else join the context that their screen shows (R20).
    ///
    /// Sugarglider's own windows, windows the layout doesn't track, and
    /// windows of apps that haven't registered get no membership. With
    /// contexts off nothing is decided; the windows rejoin their contexts
    /// when contexts are turned on.
    pub(super) fn windows_seen(&mut self, wids: &[WindowId]) {
        if !self.contexts_enabled() {
            return;
        }
        let own_pid = std::process::id() as pid_t;
        let windows: Vec<WindowDesc> = wids
            .iter()
            .filter(|wid| wid.pid != own_pid && self.apps.contains_key(&wid.pid))
            .filter(|&&wid| {
                self.layout_window_info(wid)
                    .is_some_and(|info| !self.layout.is_untracked(&info))
            })
            .filter_map(|&wid| self.window_desc(wid))
            .collect();
        if windows.is_empty() {
            return;
        }
        let changed = if self.startup_complete {
            self.windows_appeared(&windows)
        } else {
            let matches = self.contexts.rejoin_all(&windows, MatchPass::Arrival);
            let rejoined = matches.iter().filter(|found| !found.is_empty()).count();
            if rejoined > 0 {
                info!(rejoined, "Windows open at launch rejoined their contexts");
            }
            rejoined > 0
        };
        if changed {
            self.save_contexts();
        }
    }

    /// R20, R21. Matches windows that appeared after startup together
    /// against the member records, and has each window that matches nothing
    /// join the context its screen shows. Returns whether any window joined
    /// or rejoined a context.
    fn windows_appeared(&mut self, windows: &[WindowDesc]) -> bool {
        let mut by_context: Vec<(ContextKey, Vec<WindowDesc>)> = vec![];
        for window in windows {
            let key = self.arrival_context(window.wid);
            match by_context.iter_mut().find(|(other, _)| *other == key) {
                Some((_, group)) => group.push(window.clone()),
                None => by_context.push((key, vec![window.clone()])),
            }
        }
        let mut changed = false;
        for (key, group) in by_context {
            let arrivals = self.contexts.windows_appeared(&group, key);
            for (window, arrival) in group.iter().zip(arrivals) {
                debug!(wid = ?window.wid, ?arrival, "A new window appeared");
                changed |= matches!(arrival, Arrival::Rejoined(_) | Arrival::Joined(_));
            }
        }
        changed
    }

    /// The context a new window joins when it matches no record: the one
    /// the screen it appears on shows. A window on no managed screen joins
    /// the active context.
    fn arrival_context(&self, wid: WindowId) -> ContextKey {
        match self.layout_frame(wid).and_then(|frame| self.best_space_for_window(&frame)) {
            Some(space) => self.shown_context(space),
            None => self.contexts.active(),
        }
    }

    /// Parks the windows of `pid` that must not show (R13), with their
    /// journal entries written first (R30). The windows that R14 names are
    /// left alone, and so is the main window, whose focus R24 handles. Does
    /// nothing while contexts aren't in use and while quitting.
    pub(super) fn park_what_must_not_show(&mut self, pid: pid_t) {
        if !self.contexts_in_use() || self.pending_exit.is_some() {
            return;
        }
        let spaces = self.shown_spaces(self.contexts.active());
        let main_window = self.main_window();
        let park: Vec<WindowId> = plan_switch(&self.switch_input(&spaces))
            .park
            .into_iter()
            .filter(|wid| wid.pid == pid && Some(*wid) != main_window)
            .collect();
        if park.is_empty() {
            return;
        }
        match self.journal_parking(&park) {
            Ok(parking) => {
                let parked = self.move_to_corners(parking);
                info!(?parked, "Parking windows that must not show");
            }
            Err(err) => {
                error!("Could not write the parked-window journal, so nothing is parked: {err}");
            }
        }
    }
}
