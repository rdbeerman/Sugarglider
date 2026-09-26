// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Switching between contexts, the named window sets that each have their own
//! layout. A switch shows one context's windows and parks every other window.
//! The design is in `docs/specs/contexts.md`.

use std::io;
use std::time::{Instant, SystemTime};

use objc2_core_foundation::CGSize;
use tracing::{debug, error, info, warn};

use super::{ContextCommand, ContextRef, Reactor};
use crate::actor::app::{WindowId, pid_t};
use crate::actor::contexts_store::{ContextsStore, Loaded, empty_contexts_after};
use crate::actor::layout::{ActiveContext, EventResponse, LayoutEvent};
use crate::model::contexts::{
    ContextError, ContextId, ContextKey, Contexts, RecordLink, SwitchInput, SwitchPlan,
    SwitchScreen, plan_switch, rank,
};
use crate::sys::screen::SpaceId;

/// A visible screen, with its Space and size, and the context it shows.
#[derive(Clone, Copy, Debug)]
struct ShownSpace {
    screen: usize,
    space: SpaceId,
    size: CGSize,
    key: ContextKey,
}

/// Why contexts are applied.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Apply {
    /// The user switches to this context. If the journal can't be written,
    /// nothing changes.
    Switch(ContextKey),
    /// The active context is applied again. If the journal can't be written,
    /// the Spaces still show their contexts, and nothing is parked.
    Again,
}

impl Reactor {
    pub(super) fn contexts_enabled(&self) -> bool {
        self.config.settings.experimental.contexts.enable
    }

    /// Whether applying contexts can change anything. Without contexts and
    /// parked windows, Spaces are shown exactly as without the feature.
    fn contexts_in_use(&self) -> bool {
        self.contexts_enabled()
            && (!self.contexts.contexts().is_empty()
                || self.contexts.active() != ContextKey::Everything
                || !self.parked.is_empty())
    }

    /// The context the Space shows when `active` is the active context. With
    /// contexts off, while quitting, on a Space about to be turned off, and in
    /// place of a context that doesn't exist, that is Everything.
    fn shown_with(&self, space: SpaceId, active: ContextKey) -> ContextKey {
        if !self.contexts_enabled()
            || self.pending_exit.is_some()
            || self.showing_everything.contains(&space)
        {
            return ContextKey::Everything;
        }
        if let ContextKey::Named(id) = active
            && self.contexts.get(id).is_none()
        {
            error!(
                ?active,
                "Showing Everything in place of a context that doesn't exist"
            );
            return ContextKey::Everything;
        }
        active
    }

    /// The context the Space shows.
    pub(super) fn shown_context(&self, space: SpaceId) -> ContextKey {
        self.shown_with(space, self.contexts.active())
    }

    /// Whether the window may have a tile in the layout the Space shows.
    /// Under Everything every window may; under a context only its members
    /// may.
    pub(super) fn may_tile(&self, space: SpaceId, wid: WindowId) -> bool {
        match self.shown_context(space) {
            ContextKey::Everything => true,
            key => self.contexts.is_member(key, wid),
        }
    }

    /// The visible screens and the contexts they show when `active` is the
    /// active context.
    fn shown_spaces(&self, active: ContextKey) -> Vec<ShownSpace> {
        self.screens
            .iter()
            .enumerate()
            .filter_map(|(screen, info)| {
                let space = info.space?;
                Some(ShownSpace {
                    screen,
                    space,
                    size: info.frame.size,
                    key: self.shown_with(space, active),
                })
            })
            .collect()
    }

    /// The context and its open members, as the layout needs it.
    fn active_context(&self, key: ContextKey) -> ActiveContext {
        if key == ContextKey::Everything {
            return ActiveContext::EVERYTHING;
        }
        ActiveContext {
            key,
            members: self
                .windows
                .keys()
                .copied()
                .filter(|&wid| self.contexts.is_member(key, wid))
                .collect(),
        }
    }

    /// Makes each Space's context the one its layout shows, and returns the
    /// layout's responses.
    fn expose(&mut self, spaces: &[ShownSpace]) -> Option<EventResponse> {
        spaces
            .iter()
            .map(|shown| {
                let context = self.active_context(shown.key);
                let event = LayoutEvent::SpaceExposed(shown.space, shown.size, context);
                self.layout.handle_event(event)
            })
            .reduce(EventResponse::coalesce)
    }

    /// Shows each visible Space's context in the layout, and applies the
    /// active context again when contexts are in use. Returns the layout's
    /// response to the exposure for the caller to handle.
    pub(super) fn show_visible_spaces(&mut self) -> Option<EventResponse> {
        if self.contexts_in_use() {
            return self.apply(Apply::Again).ok().and_then(|(_, response)| response);
        }
        let spaces = self.shown_spaces(self.contexts.active());
        self.expose(&spaces)
    }

    /// Describes the windows on the visible screens for `plan_switch`. A
    /// window on no screen counts as on the first visible screen.
    fn switch_input(&self, spaces: &[ShownSpace]) -> SwitchInput {
        let mut input = SwitchInput {
            screens: spaces
                .iter()
                .map(|shown| SwitchScreen {
                    active: shown.key,
                    windows: vec![],
                })
                .collect(),
        };
        if spaces.is_empty() {
            return input;
        }
        let own_pid = std::process::id() as pid_t;
        let mut wids: Vec<WindowId> = self
            .windows
            .keys()
            .copied()
            .filter(|wid| self.apps.contains_key(&wid.pid))
            .collect();
        wids.sort();
        for wid in wids {
            let Some(info) = self.layout_window_info(wid) else {
                continue;
            };
            let slot = match self.best_screen_idx_for_window(&info.frame) {
                Some(screen) => {
                    let Some(slot) = spaces.iter().position(|shown| shown.screen == screen) else {
                        continue;
                    };
                    slot
                }
                None => 0,
            };
            let visible = self.windows[&wid]
                .window_server_id
                .is_some_and(|wsid| self.visible_windows.contains(&wsid));
            let mut window = self.contexts.switch_window(wid);
            window.parked = self.parked.contains_key(&wid);
            window.own = wid.pid == own_pid;
            window.untracked = self.layout.is_untracked(&info);
            // Minimized windows, windows of hidden apps, and windows on
            // Spaces nobody sees are all outside the visible windows, and the
            // plan leaves each of them where it is.
            window.unseen_space = !visible;
            input.screens[slot].windows.push(window);
        }
        input
    }

    /// Applies contexts to the visible Spaces in the order a switch takes:
    /// the journal entries of the windows it parks, then the layouts, then
    /// the members put back and laid out, then the parking. Returns the plan
    /// and the layout's response to the exposure, which the caller handles.
    fn apply(&mut self, apply: Apply) -> io::Result<(SwitchPlan, Option<EventResponse>)> {
        let active = match apply {
            Apply::Switch(target) => target,
            Apply::Again => self.contexts.active(),
        };
        let spaces = self.shown_spaces(active);
        let plan = plan_switch(&self.switch_input(&spaces));
        let parking = match self.journal_parking(&plan.park) {
            Ok(parking) => parking,
            Err(err) if matches!(apply, Apply::Switch(_)) => return Err(err),
            Err(err) => {
                error!("Could not write the parked-window journal, so nothing is parked: {err}");
                vec![]
            }
        };
        if let Apply::Switch(target) = apply
            && let Err(err) = self.contexts.switch_to(target)
        {
            error!(?target, "Could not switch: {err}");
        }
        let response = self.expose(&spaces);
        // Windows put back still count as parked here, so that the layout
        // sees them at their frames from before parking, and never as tabs
        // that share a corner.
        let mut pids: Vec<pid_t> = self.apps.keys().copied().collect();
        pids.sort();
        for pid in pids {
            self.send_visible_windows_to_layout(pid);
        }
        let released = self.release_parked(&plan.unpark);
        self.put_back_unplaced(&released);
        self.update_layout(&[], true);
        self.move_to_corners(parking);
        Ok((plan, response))
    }

    /// Switches to `target` on every screen. The switch focuses the most
    /// recently focused window that shows. If the journal can't be written,
    /// the old context stays.
    fn switch_context(&mut self, target: ContextKey) {
        if !self.contexts_enabled() {
            debug!(?target, "Ignoring a context switch while contexts are off");
            return;
        }
        if self.pending_exit.is_some() {
            info!(?target, "Ignoring a context switch while quitting");
            return;
        }
        if let ContextKey::Named(id) = target
            && self.contexts.get(id).is_none()
        {
            warn!(?target, "Ignoring a switch to a context that doesn't exist");
            return;
        }
        let start = Instant::now();
        self.layout.cancel_interactive_state();
        self.in_drag = false;
        self.resizing_window = None;
        self.title_bar_drag = None;
        match self.apply(Apply::Switch(target)) {
            Ok((plan, response)) => {
                self.save_contexts();
                let mut response = response.unwrap_or_default();
                if let Some(focus) = plan.focus
                    && self.main_window() != Some(focus)
                {
                    response.focus_window = Some(focus);
                }
                self.handle_layout_response(response);
                info!(
                    ?target,
                    parked = plan.park.len(),
                    put_back = plan.unpark.len(),
                    elapsed = ?start.elapsed(),
                    "Switched context"
                );
            }
            Err(err) => {
                error!(
                    ?target,
                    "Could not write the parked-window journal, so the context stays: {err}"
                );
            }
        }
    }

    /// Applies the context each visible Space shows again, and handles the
    /// layout's response.
    pub(super) fn apply_again(&mut self) {
        if let Ok((_, Some(response))) = self.apply(Apply::Again) {
            self.handle_layout_response(response);
        }
    }

    /// Shows every window on the visible ones of `spaces`, which Sugarglider
    /// is about to stop managing. The active context doesn't change, and the
    /// next space change applies it again.
    pub(super) fn show_everything_on(&mut self, spaces: &[SpaceId]) {
        if !self.contexts_in_use() {
            return;
        }
        let visible: Vec<SpaceId> = self
            .screens
            .iter()
            .filter_map(|screen| screen.space)
            .filter(|space| spaces.contains(space))
            .collect();
        if visible.is_empty() {
            return;
        }
        info!(?visible, "Showing every window before Spaces are turned off");
        self.showing_everything.extend(visible);
        self.apply_again();
    }

    pub(super) fn handle_context_command(&mut self, command: ContextCommand) {
        match command {
            ContextCommand::SwitchContext(reference) => match self.resolve(&reference) {
                Some(key) => self.switch_context(key),
                None => warn!(?reference, "No context matches"),
            },
            ContextCommand::ShowEverything => self.switch_context(ContextKey::Everything),
            ContextCommand::PreviousContext => match self.contexts.previous() {
                Some(key) => self.switch_context(key),
                None => debug!("There is no previous context"),
            },
        }
    }

    /// The context that a command names. A name takes the best match of the
    /// switcher's ranking.
    fn resolve(&self, reference: &ContextRef) -> Option<ContextKey> {
        match reference {
            ContextRef::Number(number) => {
                self.contexts.by_number(*number).map(|context| ContextKey::Named(context.id))
            }
            ContextRef::Id(id) => {
                self.contexts.get(*id).map(|context| ContextKey::Named(context.id))
            }
            ContextRef::Name(name) if name.trim().is_empty() => None,
            ContextRef::Name(name) => rank(name, &self.contexts, true).first().map(|(key, _)| *key),
        }
    }

    /// Applies the active context after contexts were turned on, or shows
    /// every window after they were turned off.
    pub(super) fn contexts_turned_on_or_off(&mut self) {
        if self.contexts_enabled() {
            info!("Contexts are on");
            if !self.contexts_in_use() {
                return;
            }
        } else {
            info!("Contexts are off; showing every window");
        }
        self.apply_again();
        if !self.contexts_enabled() {
            let rest: Vec<WindowId> = self.parked.keys().copied().collect();
            if !rest.is_empty() {
                self.unpark_windows(&rest);
            }
        }
    }

    /// Deletes a context. Its windows stay open, and the ones that were only
    /// in it become unsorted. When it was active, Unsorted shows first, and
    /// then the context's layouts go.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "no command deletes a context yet")
    )]
    pub(super) fn delete_context(&mut self, id: ContextId) -> Result<(), ContextError> {
        let was_active = self.contexts.active() == ContextKey::Named(id);
        self.contexts.delete(id)?;
        if was_active {
            self.apply_again();
        }
        self.layout.remove_context_layouts(id);
        if self.contexts_enabled() {
            self.save_contexts();
        }
        Ok(())
    }

    /// Writes `contexts.json`. A failure is logged.
    pub(super) fn save_contexts(&self) {
        if let Err(err) = self.contexts_store.save(&self.contexts, self.boot_id.as_deref()) {
            error!("Could not write the contexts: {err}");
        }
    }

    /// Saves the contexts when an app that has member records quits, so the
    /// records keep the windows' last titles.
    pub(super) fn app_quit(&mut self, pid: pid_t) {
        if !self.contexts_enabled() {
            return;
        }
        let has_records = self
            .contexts
            .contexts()
            .iter()
            .flat_map(|context| &context.members)
            .chain(self.contexts.pinned())
            .any(|record| {
                matches!(record.link, RecordLink::Live(wid) | RecordLink::Pending(wid)
                    if wid.pid == pid)
            });
        if has_records {
            self.save_contexts();
        }
    }

    /// Reads the contexts from `store`, which later saves go to.
    ///
    /// Window server ids are valid only within one boot of the Mac, so the
    /// contexts forget their saved ids when `boot_id` differs from the boot
    /// that saved them. Once the contexts are read, the layouts of contexts
    /// that no longer exist are dropped. When the file can't be read, the
    /// layouts stay, and new contexts take ids after theirs.
    pub(super) fn open_contexts(
        &mut self,
        store: ContextsStore,
        boot_id: Option<String>,
        now: SystemTime,
    ) {
        match store.load(now) {
            Loaded::Read { mut contexts, boot_id: saved } => {
                if saved.is_none() || saved != boot_id {
                    info!("The contexts were saved in another boot; forgetting window server ids");
                    contexts.forget_window_server_ids();
                }
                self.layout.retain_context_layouts(|id| contexts.get(id).is_some());
                self.contexts = contexts;
            }
            Loaded::Missing => {
                self.layout.retain_context_layouts(|_| false);
                self.contexts = Contexts::new();
            }
            Loaded::Unreadable => {
                self.contexts = empty_contexts_after(self.layout.context_ids().max());
            }
        }
        self.contexts_store = store;
        self.boot_id = boot_id;
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant, SystemTime};

    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use tempfile::TempDir;
    use test_log::test;
    use tokio::sync::mpsc;

    use super::super::testing::*;
    use super::super::{
        Command, ContextCommand, ContextRef, Event, Reactor, ReactorCommand, Requested,
    };
    use crate::actor::app::{Quiet, Request, WindowId};
    use crate::actor::contexts_store::{ContextsStore, Loaded};
    use crate::actor::layout::{LayoutCommand, LayoutEvent, LayoutManager};
    use crate::actor::parked_journal::{FailingWrites, JournalEntry, ParkedJournal};
    use crate::actor::raise;
    use crate::config::Config;
    use crate::model::Direction;
    use crate::model::contexts::{ContextId, ContextKey, WindowDesc};
    use crate::sys::app::WindowInfo;
    use crate::sys::event::MouseState;
    use crate::sys::screen::{CoordinateConverter, SpaceId};
    use crate::sys::window_server::{WindowServerId, WindowServerInfo, WindowsOnScreen};

    fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
        CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
    }

    fn wid(idx: u32) -> WindowId {
        WindowId::new(1, idx)
    }

    fn space() -> SpaceId {
        SpaceId::new(1)
    }

    /// A screen on which up to four windows side by side get tiles with
    /// whole-number frames.
    fn screen() -> CGRect {
        rect(0., 0., 1200., 1000.)
    }

    /// The corner that parks a window of `size` on `screen()`.
    fn corner(size: CGSize) -> CGRect {
        CGRect {
            origin: CGPoint::new(1199., 999.),
            size,
        }
    }

    /// The config of a test reactor, with contexts on or off.
    fn config(contexts: bool) -> Arc<Config> {
        let mut config = Config::default();
        config.settings.default_disable = false;
        config.settings.animate = false;
        config.settings.experimental.contexts.enable = contexts;
        Arc::new(config)
    }

    fn screens(frames: Vec<CGRect>, spaces: Vec<Option<SpaceId>>) -> Event {
        Event::ScreenParametersChanged {
            bounds: frames.clone(),
            scale_factors: vec![1.0; frames.len()],
            frames,
            spaces,
            converter: CoordinateConverter::default(),
            on_screen: Default::default(),
        }
    }

    fn frame_writes(requests: &[Request], wid: WindowId) -> Vec<CGRect> {
        requests
            .iter()
            .filter_map(|request| match request {
                Request::SetWindowFrame(request_wid, frame, _) if *request_wid == wid => {
                    Some(*frame)
                }
                _ => None,
            })
            .collect()
    }

    /// A reactor with contexts on, its journal and `contexts.json` in a
    /// temporary directory.
    struct Setup {
        reactor: Reactor,
        apps: Apps,
        dir: TempDir,
    }

    impl Setup {
        /// App 1's `windows` windows tiled side by side on one screen.
        fn new(windows: usize) -> Setup {
            let mut s = Setup::on(vec![screen()], vec![Some(space())]);
            s.reactor.handle_events(s.apps.make_app(1, make_windows(windows)));
            s.reactor.handle_event(Event::StartupComplete);
            s.apps.simulate_until_quiet(&mut s.reactor);
            s
        }

        /// A reactor with contexts on that no app has reached yet.
        fn on(frames: Vec<CGRect>, spaces: Vec<Option<SpaceId>>) -> Setup {
            let dir = TempDir::new().unwrap();
            let mut reactor = Reactor::new_for_test(LayoutManager::new_for_test());
            reactor.journal =
                ParkedJournal::open(dir.path().join("parked.json"), SystemTime::now());
            reactor.open_contexts(
                ContextsStore::new(dir.path().join("contexts.json")),
                Some("boot".into()),
                SystemTime::now(),
            );
            reactor.handle_event(Event::ConfigChanged(config(true)));
            reactor.handle_event(screens(frames, spaces));
            Setup {
                reactor,
                apps: Apps::new(),
                dir,
            }
        }

        fn desc(&self, wid: WindowId) -> WindowDesc {
            let window = &self.reactor.windows[&wid];
            WindowDesc {
                wid,
                bundle_id: Some(format!("com.testapp{}", wid.pid)),
                app_name: Some(format!("TestApp{}", wid.pid)),
                title: window.title.expose_secret().clone(),
                window_server_id: window.window_server_id,
            }
        }

        /// Creates a context that holds `members`.
        fn create(&mut self, name: &str, members: &[WindowId]) -> ContextKey {
            let id = self.reactor.contexts.create(name).unwrap();
            for &wid in members {
                self.add(ContextKey::Named(id), wid);
            }
            ContextKey::Named(id)
        }

        fn add(&mut self, key: ContextKey, wid: WindowId) {
            let ContextKey::Named(id) = key else { panic!("{key:?}") };
            let desc = self.desc(wid);
            self.reactor.contexts.add_window(id, &desc).unwrap();
        }

        /// Sends the command that switches to `key`, as a key binding does.
        fn command(&mut self, key: ContextKey) {
            let command = match key {
                ContextKey::Everything => ContextCommand::ShowEverything,
                ContextKey::Unsorted => {
                    ContextCommand::SwitchContext(ContextRef::Name("Unsorted".into()))
                }
                ContextKey::Named(id) => ContextCommand::SwitchContext(ContextRef::Id(id)),
            };
            self.reactor.handle_event(Event::Command(Command::Context(command)));
        }

        /// Switches to `key`, and lets the apps answer.
        fn switch(&mut self, key: ContextKey) {
            self.command(key);
            self.apps.simulate_until_quiet(&mut self.reactor);
        }

        fn frame(&self, wid: WindowId) -> CGRect {
            self.apps.windows[&wid].frame
        }

        fn frames(&self, wids: &[WindowId]) -> Vec<(WindowId, CGRect)> {
            wids.iter().map(|&wid| (wid, self.frame(wid))).collect()
        }

        fn tiles_on(&self, space: SpaceId, screen: CGRect) -> Vec<(WindowId, CGRect)> {
            let mut tiles =
                self.reactor.layout.calculate_layout(space, screen, &self.reactor.config);
            tiles.sort_by_key(|(wid, _)| *wid);
            tiles
        }

        fn tiles(&self) -> Vec<(WindowId, CGRect)> {
            self.tiles_on(space(), screen())
        }

        fn parked(&self) -> Vec<WindowId> {
            let mut parked: Vec<WindowId> = self.reactor.parked.keys().copied().collect();
            parked.sort();
            parked
        }

        fn journal_on_disk(&self) -> Vec<JournalEntry> {
            ParkedJournal::open(self.dir.path().join("parked.json"), SystemTime::now())
                .entries()
                .to_vec()
        }

        fn saved_active(&self) -> ContextKey {
            match ContextsStore::new(self.dir.path().join("contexts.json")).load(SystemTime::now())
            {
                Loaded::Read { contexts, .. } => contexts.active(),
                other => panic!("{other:?}"),
            }
        }

        /// Focuses the window and moves it, as the user does with a key.
        fn move_window(&mut self, wid: WindowId, direction: Direction) {
            self.reactor.send_layout_event(LayoutEvent::WindowFocused(vec![space()], wid));
            self.reactor
                .handle_event(Event::Command(Command::Layout(LayoutCommand::MoveNode(
                    direction,
                ))));
            self.apps.simulate_until_quiet(&mut self.reactor);
        }

        /// Closes the window. Its app forgets it, and the reactor learns that
        /// it was destroyed.
        fn close(&mut self, wid: WindowId) {
            self.apps.windows.remove(&wid);
            self.reactor.handle_event(Event::WindowDestroyed(wid));
        }
    }

    /// A window server snapshot that lists the windows at their frames.
    fn on_screen(s: &Setup, wids: &[WindowId]) -> WindowsOnScreen {
        WindowsOnScreen::new(
            wids.iter()
                .map(|&wid| WindowServerInfo {
                    id: s.reactor.windows[&wid].window_server_id.unwrap(),
                    pid: wid.pid,
                    layer: 0,
                    frame: s.frame(wid),
                })
                .collect(),
        )
    }

    fn entry(idx: u32, frame: CGRect) -> JournalEntry {
        JournalEntry {
            pid: 1,
            bundle_id: Some("com.testapp1".into()),
            window_server_id: WindowServerId::new(idx),
            title: format!("Window{idx}"),
            frame: frame.into(),
        }
    }

    /// R12, L1. The regression test for keeping layouts across switches.
    #[test]
    fn r12_switching_away_from_a_context_and_back_gives_the_same_frames() {
        let mut s = Setup::new(4);
        let all = [wid(1), wid(2), wid(3), wid(4)];
        let everything = vec![
            (wid(1), rect(0., 0., 300., 1000.)),
            (wid(2), rect(300., 0., 300., 1000.)),
            (wid(3), rect(600., 0., 300., 1000.)),
            (wid(4), rect(900., 0., 300., 1000.)),
        ];
        assert_eq!(everything, s.frames(&all));
        let c = s.create("C", &[wid(1), wid(2), wid(3)]);
        let d = s.create("D", &[wid(2), wid(3), wid(4)]);

        s.switch(c);
        s.move_window(wid(1), Direction::Right);
        let in_c = vec![
            (wid(1), rect(400., 0., 400., 1000.)),
            (wid(2), rect(0., 0., 400., 1000.)),
            (wid(3), rect(800., 0., 400., 1000.)),
        ];
        assert_eq!(in_c, s.frames(&[wid(1), wid(2), wid(3)]));
        assert_eq!(in_c, s.tiles());
        assert_eq!(corner(CGSize::new(300., 1000.)), s.frame(wid(4)));
        // The parked window reaches the layout again and gets no tile in C.
        s.reactor.update_visible_windows();
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(in_c, s.tiles());

        s.switch(d);
        s.move_window(wid(4), Direction::Left);
        let in_d = vec![
            (wid(2), rect(0., 0., 400., 1000.)),
            (wid(3), rect(800., 0., 400., 1000.)),
            (wid(4), rect(400., 0., 400., 1000.)),
        ];
        assert_eq!(in_d, s.frames(&[wid(2), wid(3), wid(4)]));
        assert_eq!(corner(CGSize::new(400., 1000.)), s.frame(wid(1)));

        for _ in 0..2 {
            s.switch(c);
            assert_eq!(in_c, s.frames(&[wid(1), wid(2), wid(3)]));
            assert_eq!(vec![wid(4)], s.parked());
            s.switch(d);
            assert_eq!(in_d, s.frames(&[wid(2), wid(3), wid(4)]));
            assert_eq!(vec![wid(1)], s.parked());
        }
        s.switch(ContextKey::Everything);
        assert_eq!(everything, s.frames(&all));
        assert!(s.parked().is_empty());
        assert!(s.journal_on_disk().is_empty());
    }

    /// R13, R14. The window of Sugarglider itself, a panel the layout leaves
    /// alone, a minimized window, and a window on a screen whose Space is
    /// off all stay.
    #[test]
    fn r13_r14_a_switch_parks_every_non_member_on_the_visible_spaces_and_nothing_else() {
        let right = rect(1200., 0., 1200., 1000.);
        let mut s = Setup::on(vec![screen(), right], vec![Some(space()), None]);
        let own_pid = std::process::id() as i32;
        let own = WindowId::new(own_pid, 1);
        let own_window = WindowInfo {
            sys_id: Some(WindowServerId::new(50)),
            ..make_window(1)
        };
        s.reactor.handle_events(s.apps.make_app(own_pid, vec![own_window]));
        let on_right = WindowInfo {
            frame: rect(1300., 100., 50., 50.),
            ..make_window(5)
        };
        let mut windows = make_windows(4);
        windows.push(on_right);
        let mut launch = s.apps.make_app(1, windows);
        // Window 4 is a panel on a layer of its own.
        for event in &mut launch {
            if let Event::WindowsOnScreenUpdated { on_screen, .. } = event {
                on_screen.info[3].layer = 1;
            }
        }
        s.reactor.handle_events(launch);
        s.reactor.handle_event(Event::StartupComplete);
        // Window 3 is minimized.
        let visible = |id: u32, layer: i32, frame: CGRect| WindowServerInfo {
            id: WindowServerId::new(id),
            pid: if id == 50 { own_pid } else { 1 },
            layer,
            frame,
        };
        s.reactor.handle_event(Event::WindowsOnScreenUpdated {
            pid: None,
            on_screen: WindowsOnScreen::new(vec![
                visible(50, 0, rect(100., 100., 50., 50.)),
                visible(1, 0, rect(100., 100., 50., 50.)),
                visible(2, 0, rect(200., 100., 50., 50.)),
                visible(4, 1, rect(400., 100., 50., 50.)),
                visible(5, 0, rect(1300., 100., 50., 50.)),
            ]),
        });
        s.reactor.update_visible_windows();
        s.apps.simulate_until_quiet(&mut s.reactor);
        let tiles = vec![
            (own, rect(0., 0., 400., 1000.)),
            (wid(1), rect(400., 0., 400., 1000.)),
            (wid(2), rect(800., 0., 400., 1000.)),
        ];
        let mut before = s.tiles();
        before.sort_by_key(|(wid, _)| *wid != own);
        assert_eq!(tiles, before);
        let untouched = [own, wid(3), wid(4), wid(5)].map(|wid| (wid, s.frame(wid)));
        let c = s.create("C", &[wid(1)]);

        s.switch(c);

        assert_eq!(vec![wid(2)], s.parked());
        // The bottom right corner would reach into the screen on the right.
        assert_eq!(rect(-399., 999., 400., 1000.), s.frame(wid(2)));
        assert_eq!(
            untouched,
            [own, wid(3), wid(4), wid(5)].map(|wid| (wid, s.frame(wid)))
        );
        assert_eq!(vec![entry(2, rect(800., 0., 400., 1000.))], s.journal_on_disk());
    }

    /// L8. The window joins C while D is active, so C's layout has no node
    /// for it.
    #[test]
    fn l8_a_member_without_a_node_in_the_targets_layout_is_put_back_and_tiled() {
        let mut s = Setup::new(3);
        let c = s.create("C", &[wid(1)]);
        let d = s.create("D", &[wid(2), wid(3)]);
        s.switch(c);
        s.switch(d);
        assert_eq!(vec![wid(1)], s.parked());
        s.add(c, wid(3));

        s.switch(c);

        let tiles = vec![
            (wid(1), rect(0., 0., 600., 1000.)),
            (wid(3), rect(600., 0., 600., 1000.)),
        ];
        assert_eq!(tiles, s.tiles());
        assert_eq!(tiles, s.frames(&[wid(1), wid(3)]));
        assert_eq!(vec![wid(2)], s.parked());
        assert_eq!(corner(CGSize::new(600., 1000.)), s.frame(wid(2)));
        assert_eq!(vec![entry(2, rect(0., 0., 600., 1000.))], s.journal_on_disk());
    }

    /// Makes window 1 float at the frame it had before it was tiled,
    /// `(100, 100, 50, 50)`.
    fn float_window_1(s: &mut Setup) {
        s.reactor.handle_event(Event::ApplicationGloballyActivated(1));
        s.reactor.send_layout_event(LayoutEvent::WindowFocused(vec![space()], wid(1)));
        s.reactor.handle_event(Event::Command(Command::Layout(
            LayoutCommand::ToggleWindowFloating,
        )));
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(rect(100., 100., 50., 50.), s.frame(wid(1)));
    }

    /// H3.
    #[test]
    fn h3_a_floating_member_comes_back_at_its_journal_frame() {
        let mut s = Setup::new(2);
        float_window_1(&mut s);
        let floating = rect(100., 100., 50., 50.);
        let c = s.create("C", &[wid(2)]);
        let d = s.create("D", &[wid(1), wid(2)]);
        s.switch(c);
        assert_eq!(corner(floating.size), s.frame(wid(1)));
        assert_eq!(vec![entry(1, floating)], s.journal_on_disk());

        s.command(d);

        let requests = s.apps.requests();
        assert_eq!(vec![floating], frame_writes(&requests, wid(1)));
        for event in s.apps.simulate_events_for_requests(requests) {
            s.reactor.handle_event(event);
        }
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(floating, s.frame(wid(1)));
        assert_eq!(vec![(wid(2), screen())], s.tiles());
        assert!(s.parked().is_empty());
        assert!(s.journal_on_disk().is_empty());
    }

    /// H4.
    #[test]
    fn h4_toggling_between_two_contexts_ten_times_quickly_ends_at_the_right_frames() {
        let mut s = Setup::new(3);
        let c = s.create("C", &[wid(1), wid(2)]);
        let d = s.create("D", &[wid(2), wid(3)]);
        s.switch(c);
        s.switch(d);
        let in_d = vec![
            (wid(2), rect(0., 0., 600., 1000.)),
            (wid(3), rect(600., 0., 600., 1000.)),
        ];
        assert_eq!(in_d, s.frames(&[wid(2), wid(3)]));

        for _ in 0..10 {
            s.command(c);
            s.command(d);
        }
        s.apps.simulate_until_quiet(&mut s.reactor);

        assert_eq!(in_d, s.frames(&[wid(2), wid(3)]));
        assert_eq!(corner(CGSize::new(600., 1000.)), s.frame(wid(1)));
        assert_eq!(vec![wid(1)], s.parked());
        assert_eq!(vec![entry(1, rect(0., 0., 600., 1000.))], s.journal_on_disk());
        s.switch(c);
        assert_eq!(
            vec![
                (wid(1), rect(0., 0., 600., 1000.)),
                (wid(2), rect(600., 0., 600., 1000.)),
            ],
            s.frames(&[wid(1), wid(2)])
        );
        assert_eq!(vec![wid(3)], s.parked());
    }

    /// H5.
    #[test]
    fn h5_a_parked_window_that_closes_during_a_switch_cycle_leaves_no_empty_tile() {
        let mut s = Setup::new(3);
        let c = s.create("C", &[wid(1)]);
        let d = s.create("D", &[wid(2), wid(3)]);
        s.switch(d);
        s.switch(c);
        assert_eq!(vec![wid(2), wid(3)], s.parked());
        assert_eq!(s.frame(wid(2)), s.frame(wid(3)), "both share one corner");

        s.close(wid(2));
        s.switch(d);

        assert_eq!(vec![(wid(3), screen())], s.tiles());
        assert_eq!(screen(), s.frame(wid(3)));
        assert_eq!(vec![wid(1)], s.parked());
        assert_eq!(vec![entry(1, screen())], s.journal_on_disk());
    }

    /// R12, R30.
    #[test]
    fn r12_r30_a_failed_journal_write_stops_the_switch_and_keeps_the_old_context() {
        let mut s = Setup::new(2);
        let c = s.create("C", &[wid(1)]);
        let frames = s.frames(&[wid(1), wid(2)]);
        let txids = [wid(1), wid(2)].map(|wid| s.reactor.windows[&wid].last_sent_txid);

        let failing = FailingWrites::start(s.dir.path());
        s.command(c);
        drop(failing);

        assert!(s.apps.requests().is_empty());
        assert_eq!(ContextKey::Everything, s.reactor.contexts.active());
        assert_eq!(None, s.reactor.contexts.previous());
        assert!(s.parked().is_empty());
        assert_eq!(frames, s.tiles());
        assert_eq!(
            txids,
            [wid(1), wid(2)].map(|wid| s.reactor.windows[&wid].last_sent_txid)
        );
        assert_eq!(0, s.reactor.layout.context_ids().count());
        assert!(!s.dir.path().join("contexts.json").exists());
        assert!(s.journal_on_disk().is_empty());

        s.switch(c);
        assert_eq!(vec![wid(2)], s.parked());
        assert_eq!(c, s.saved_active());
    }

    /// R27.
    #[test]
    fn r27_showing_everything_puts_every_window_back() {
        let mut s = Setup::new(3);
        float_window_1(&mut s);
        let all = [wid(1), wid(2), wid(3)];
        let everything = s.frames(&all);
        let c = s.create("C", &[wid(2)]);
        let d = s.create("D", &[wid(3)]);
        s.switch(c);
        s.switch(d);
        assert_eq!(vec![wid(1), wid(2)], s.parked());

        s.switch(ContextKey::Everything);

        assert_eq!(everything, s.frames(&all));
        assert!(s.parked().is_empty());
        assert!(s.journal_on_disk().is_empty());
        assert_eq!(ContextKey::Everything, s.saved_active());
    }

    /// L10, R7.
    #[test]
    fn l10_r7_a_window_moved_to_another_screen_under_c_stays_there_under_d() {
        let left = screen();
        let right = rect(1200., 0., 1200., 1000.);
        let mut s = Setup::on(vec![left, right], vec![Some(space()), Some(SpaceId::new(2))]);
        let on_right = WindowInfo {
            frame: rect(1300., 100., 50., 50.),
            ..make_window(3)
        };
        s.reactor
            .handle_events(s.apps.make_app(1, vec![make_window(1), make_window(2), on_right]));
        s.reactor.handle_event(Event::StartupComplete);
        s.apps.simulate_until_quiet(&mut s.reactor);
        let c = s.create("C", &[wid(1), wid(2), wid(3)]);
        let d = s.create("D", &[wid(1), wid(2), wid(3)]);
        s.switch(d);
        s.switch(c);
        assert_eq!(
            vec![
                (wid(1), rect(0., 0., 600., 1000.)),
                (wid(2), rect(600., 0., 600., 1000.)),
            ],
            s.tiles_on(space(), left)
        );

        // The user drags window 2 onto the right screen.
        let dropped = rect(1400., 100., 600., 1000.);
        let txid = s.reactor.windows[&wid(2)].last_sent_txid;
        s.apps.windows.get_mut(&wid(2)).unwrap().frame = dropped;
        s.reactor.handle_event(Event::WindowFrameChanged(
            wid(2),
            dropped,
            txid,
            Requested(false),
            Some(MouseState::Up),
        ));
        s.apps.simulate_until_quiet(&mut s.reactor);
        let right_tiles = vec![
            (wid(2), rect(1800., 0., 600., 1000.)),
            (wid(3), rect(1200., 0., 600., 1000.)),
        ];
        assert_eq!(vec![(wid(1), left)], s.tiles_on(space(), left));
        assert_eq!(right_tiles, s.tiles_on(SpaceId::new(2), right));

        s.switch(d);

        // D's layout on the right screen takes window 2 in the order of the
        // windows' first frames.
        let right_tiles = vec![
            (wid(2), rect(1200., 0., 600., 1000.)),
            (wid(3), rect(1800., 0., 600., 1000.)),
        ];
        assert_eq!(vec![(wid(1), left)], s.tiles_on(space(), left));
        assert_eq!(right_tiles, s.tiles_on(SpaceId::new(2), right));
        assert_eq!(right_tiles, s.frames(&[wid(2), wid(3)]));
        assert_eq!(left, s.frame(wid(1)));
        assert!(s.parked().is_empty());
    }

    /// R10, L2.
    #[test]
    fn r10_l2_a_space_change_makes_the_contexts_layout_and_applies_it_in_the_same_event() {
        let mut s = Setup::new(3);
        // Window 3 is on another Space.
        let snapshot = on_screen(&s, &[wid(1), wid(2)]);
        s.reactor
            .handle_event(Event::WindowsOnScreenUpdated { pid: None, on_screen: snapshot });
        s.reactor.update_visible_windows();
        s.apps.simulate_until_quiet(&mut s.reactor);
        let c = s.create("C", &[wid(1)]);
        s.switch(c);
        assert_eq!(vec![(wid(1), screen())], s.tiles());
        assert_eq!(vec![wid(2)], s.parked());

        // The user moves to Space 2, which shows windows 1 and 3.
        let space2 = SpaceId::new(2);
        let snapshot = on_screen(&s, &[wid(1), wid(3)]);
        s.reactor.handle_event(Event::SpaceChanged(vec![Some(space2)], snapshot));

        assert_eq!(vec![(wid(1), screen())], s.tiles_on(space2, screen()));
        assert_eq!(vec![wid(2), wid(3)], s.parked());
        let requests = s.apps.requests();
        assert_eq!(
            vec![corner(CGSize::new(400., 1000.))],
            frame_writes(&requests, wid(3))
        );
        assert!(frame_writes(&requests, wid(1)).is_empty());
        assert!(s.reactor.layout.context_ids().all(|id| ContextKey::Named(id) == c));
    }

    /// R16.
    #[test]
    fn r16_switching_to_the_active_context_parks_windows_that_drifted_in() {
        let mut s = Setup::new(2);
        let c = s.create("C", &[wid(1)]);
        s.switch(c);
        assert_eq!(vec![wid(2)], s.parked());
        // A new window of the app shows up. Nothing parks it.
        let new = WindowInfo {
            frame: rect(300., 100., 50., 50.),
            ..make_window(3)
        };
        s.apps.windows.insert(
            wid(3),
            WindowState {
                frame: new.frame,
                ..Default::default()
            },
        );
        s.reactor.handle_event(Event::WindowCreated(wid(3), new, MouseState::Up));
        s.reactor.handle_event(Event::WindowsOnScreenUpdated {
            pid: Some(1),
            on_screen: WindowsOnScreen::new(
                (1..=3)
                    .map(|idx| WindowServerInfo {
                        id: WindowServerId::new(idx),
                        pid: 1,
                        layer: 0,
                        frame: s.frame(wid(idx)),
                    })
                    .collect(),
            ),
        });
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(vec![wid(2)], s.parked());

        s.switch(c);

        assert_eq!(vec![wid(2), wid(3)], s.parked());
        assert_eq!(corner(CGSize::new(50., 50.)), s.frame(wid(3)));
        assert_eq!(vec![(wid(1), screen())], s.tiles());
    }

    /// R18, R19.
    #[test]
    fn r18_previous_context_goes_back_to_the_context_used_before() {
        let mut s = Setup::new(2);
        let c = s.create("C", &[wid(1)]);
        let d = s.create("D", &[wid(2)]);
        let previous = |s: &mut Setup| {
            s.reactor
                .handle_event(Event::Command(Command::Context(ContextCommand::PreviousContext)));
            s.apps.simulate_until_quiet(&mut s.reactor);
        };
        previous(&mut s);
        assert_eq!(ContextKey::Everything, s.reactor.contexts.active());

        s.switch(c);
        s.switch(d);
        let used = |s: &Setup, key| s.reactor.contexts.last_used(key);
        assert!(used(&s, d) > used(&s, c) && used(&s, c) > 0, "R19");
        previous(&mut s);
        assert_eq!(c, s.reactor.contexts.active());
        assert_eq!(vec![wid(2)], s.parked());
        previous(&mut s);
        assert_eq!(d, s.reactor.contexts.active());
        assert_eq!(vec![wid(1)], s.parked());
        s.switch(d);
        previous(&mut s);
        assert_eq!(c, s.reactor.contexts.active());
    }

    /// R29, R3. A pinned window shows under every context.
    #[test]
    fn r29_unsorted_shows_the_windows_in_no_context_and_the_pinned_ones() {
        let mut s = Setup::new(3);
        let c = s.create("C", &[wid(1)]);
        let desc = s.desc(wid(3));
        s.reactor.contexts.pin(&desc);

        s.switch(ContextKey::Unsorted);
        assert_eq!(ContextKey::Unsorted, s.reactor.contexts.active());
        assert_eq!(vec![wid(1)], s.parked());
        assert_eq!(
            vec![
                (wid(2), rect(0., 0., 600., 1000.)),
                (wid(3), rect(600., 0., 600., 1000.)),
            ],
            s.tiles()
        );

        s.switch(c);
        assert_eq!(vec![wid(2)], s.parked());
        assert_eq!(
            vec![
                (wid(1), rect(0., 0., 600., 1000.)),
                (wid(3), rect(600., 0., 600., 1000.)),
            ],
            s.tiles()
        );
    }

    /// R12, step 5.
    #[test]
    fn r12_a_switch_focuses_the_targets_most_recently_focused_window() {
        let mut s = Setup::new(3);
        let focus = |s: &mut Setup, wid: WindowId| {
            s.reactor.handle_event(Event::ApplicationActivated(1, Quiet::No));
            s.reactor.handle_event(Event::ApplicationGloballyActivated(1));
            s.reactor
                .handle_event(Event::ApplicationMainWindowChanged(1, Some(wid), Quiet::No));
        };
        focus(&mut s, wid(2));
        focus(&mut s, wid(1));
        focus(&mut s, wid(3));
        let c = s.create("C", &[wid(1), wid(2)]);
        let (raise_manager_tx, mut raise_manager_rx) = mpsc::unbounded_channel();
        s.reactor.raise_manager_tx = raise_manager_tx;

        s.switch(c);

        let mut focused = vec![];
        while let Ok((_, event)) = raise_manager_rx.try_recv() {
            if let raise::Event::RaiseRequest(request) = event {
                focused.extend(request.focus_window.map(|(wid, _)| wid));
            }
        }
        assert_eq!(vec![wid(1)], focused);
    }

    #[test]
    fn a_switch_during_a_drag_ends_the_drag_first() {
        let mut s = Setup::new(2);
        let c = s.create("C", &[wid(1)]);
        s.reactor.handle_event(Event::LeftMouseDown(
            CGPoint::new(300., 500.),
            Some(WindowServerId::new(1)),
        ));
        assert!(s.reactor.in_drag);

        s.switch(c);

        assert!(!s.reactor.in_drag);
        assert!(!s.reactor.layout.has_interactive_state());
        assert!(s.reactor.title_bar_drag.is_none());
        assert_eq!(vec![(wid(1), screen())], s.tiles());
        s.reactor.handle_event(Event::LeftMouseDragged(CGPoint::new(900., 500.)));
        s.reactor.handle_event(Event::MouseUp);
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(vec![(wid(1), screen())], s.tiles());
        assert_eq!(screen(), s.frame(wid(1)));
    }

    #[test]
    fn a_switch_to_a_context_that_doesnt_exist_changes_nothing() {
        let mut s = Setup::new(2);
        let c = s.create("C", &[wid(1)]);
        let ContextKey::Named(id) = c else { unreachable!() };
        s.reactor.contexts.delete(id).unwrap();

        s.command(c);
        s.reactor
            .handle_event(Event::Command(Command::Context(ContextCommand::SwitchContext(
                ContextRef::Number(1),
            ))));

        assert!(s.apps.requests().is_empty());
        assert!(s.parked().is_empty());
        assert_eq!(0, s.reactor.layout.context_ids().count());
    }

    #[test]
    fn a_command_names_a_context_by_number_name_or_id() {
        let mut s = Setup::new(3);
        let c = s.create("Client work", &[wid(1)]);
        let d = s.create("Comms", &[wid(2)]);
        let run = |s: &mut Setup, reference: ContextRef| {
            s.reactor.handle_event(Event::Command(Command::Context(
                ContextCommand::SwitchContext(reference),
            )));
            s.apps.simulate_until_quiet(&mut s.reactor);
            s.reactor.contexts.active()
        };
        assert_eq!(d, run(&mut s, ContextRef::Number(2)));
        assert_eq!(c, run(&mut s, ContextRef::Name("cli".into())));
        assert_eq!(d, run(&mut s, ContextRef::Name("COMMS".into())));
        assert_eq!(
            ContextKey::Unsorted,
            run(&mut s, ContextRef::Name("unsorted".into()))
        );
        let ContextKey::Named(id) = c else { unreachable!() };
        assert_eq!(c, run(&mut s, ContextRef::Id(id)));
        assert_eq!(c, run(&mut s, ContextRef::Name("nothing like it".into())));
    }

    /// R28.
    #[test]
    fn r28_with_contexts_off_nothing_changes() {
        let dir = TempDir::new().unwrap();
        // Contexts from a session with contexts on, with C active.
        let mut saved = crate::model::contexts::Contexts::new();
        let id = saved.create("C").unwrap();
        saved.switch_to(ContextKey::Named(id)).unwrap();
        let mut reactor = Reactor::new_for_test(LayoutManager::new_for_test());
        reactor.journal = ParkedJournal::open(dir.path().join("parked.json"), SystemTime::now());
        reactor.open_contexts(
            ContextsStore::new(dir.path().join("contexts.json")),
            Some("boot".into()),
            SystemTime::now(),
        );
        reactor.contexts = saved;
        let mut apps = Apps::new();
        reactor.handle_event(screens(vec![screen()], vec![Some(space())]));
        reactor.handle_events(apps.make_app(1, make_windows(2)));
        reactor.handle_event(Event::StartupComplete);
        apps.simulate_until_quiet(&mut reactor);
        let desc = WindowDesc {
            wid: wid(1),
            bundle_id: Some("com.testapp1".into()),
            app_name: Some("TestApp1".into()),
            title: "Window1".into(),
            window_server_id: Some(WindowServerId::new(1)),
        };
        reactor.contexts.add_window(id, &desc).unwrap();
        let tiles = vec![
            (wid(1), rect(0., 0., 600., 1000.)),
            (wid(2), rect(600., 0., 600., 1000.)),
        ];

        for command in [
            ContextCommand::SwitchContext(ContextRef::Id(id)),
            ContextCommand::SwitchContext(ContextRef::Number(1)),
            ContextCommand::ShowEverything,
            ContextCommand::PreviousContext,
        ] {
            reactor.handle_event(Event::Command(Command::Context(command)));
            assert!(apps.requests().is_empty());
        }
        reactor.handle_event(Event::SpaceChanged(
            vec![Some(SpaceId::new(2))],
            Default::default(),
        ));
        apps.simulate_until_quiet(&mut reactor);
        let shorter = rect(0., 0., 1200., 900.);
        reactor.handle_event(screens(vec![shorter], vec![Some(space())]));
        apps.simulate_until_quiet(&mut reactor);
        reactor.handle_event(screens(vec![screen()], vec![Some(space())]));
        apps.simulate_until_quiet(&mut reactor);

        let mut after = reactor.layout.calculate_layout(space(), screen(), &reactor.config);
        after.sort_by_key(|(wid, _)| *wid);
        assert_eq!(tiles, after);
        assert_eq!(
            tiles,
            [wid(1), wid(2)].map(|wid| (wid, apps.windows[&wid].frame))
        );
        assert!(reactor.parked.is_empty());
        assert!(reactor.layout.serialize_to_string().contains(",context_layouts:{},"));
        assert!(
            fs::read_dir(dir.path()).unwrap().next().is_none(),
            "no file is written"
        );

        let exits = Arc::new(Mutex::new(vec![]));
        let caught = exits.clone();
        reactor.layout_file = Some(dir.path().join("layout.ron"));
        reactor.exit = Box::new(move |code| caught.lock().unwrap().push(code));
        reactor.handle_event(Event::Command(Command::Reactor(ReactorCommand::SaveAndExit)));
        assert_eq!(vec![0], *exits.lock().unwrap());
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(vec!["layout.ron"], names);
    }

    /// R33, R28.
    #[test]
    fn turning_contexts_off_shows_every_window_and_turning_them_on_applies_the_context() {
        let mut s = Setup::new(3);
        let all = [wid(1), wid(2), wid(3)];
        let everything = s.frames(&all);
        let c = s.create("C", &[wid(1)]);
        s.switch(c);
        assert_eq!(vec![wid(2), wid(3)], s.parked());

        s.reactor.handle_event(Event::ConfigChanged(config(false)));
        s.apps.simulate_until_quiet(&mut s.reactor);

        assert_eq!(everything, s.frames(&all));
        assert_eq!(everything, s.tiles());
        assert!(s.parked().is_empty());
        assert!(s.journal_on_disk().is_empty());
        assert_eq!(c, s.reactor.contexts.active());
        assert_eq!(c, s.saved_active());

        s.reactor.handle_event(Event::ConfigChanged(config(true)));
        s.apps.simulate_until_quiet(&mut s.reactor);

        assert_eq!(vec![wid(2), wid(3)], s.parked());
        assert_eq!(vec![(wid(1), screen())], s.tiles());
        assert_eq!(screen(), s.frame(wid(1)));
    }

    #[test]
    fn contexts_json_is_written_after_each_switch_and_when_an_app_with_members_quits() {
        let mut s = Setup::new(2);
        let path = s.dir.path().join("contexts.json");
        assert!(!path.exists());
        let c = s.create("C", &[wid(1)]);

        s.switch(c);
        assert_eq!(c, s.saved_active());
        s.switch(ContextKey::Everything);
        assert_eq!(ContextKey::Everything, s.saved_active());

        fs::remove_file(&path).unwrap();
        s.reactor.handle_events(s.apps.make_app(2, vec![]));
        s.reactor.handle_event(Event::ApplicationThreadTerminated(2));
        assert!(!path.exists(), "app 2 has no member records");
        s.reactor.handle_event(Event::ApplicationThreadTerminated(1));
        assert!(path.exists());
        let written: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(serde_json::json!("boot"), written["boot_id"]);
        assert_eq!(
            serde_json::json!("Window1"),
            written["contexts"][0]["members"][0]["title"]
        );
    }

    /// Saves contexts with one member record, as a boot named `boot` did.
    fn saved_by_boot(s: &Setup, boot: Option<&str>) -> ContextsStore {
        let mut contexts = crate::model::contexts::Contexts::new();
        let id = contexts.create("C").unwrap();
        let desc = s.desc(wid(1));
        contexts.add_window(id, &desc).unwrap();
        let store = ContextsStore::new(s.dir.path().join("contexts.json"));
        store.save(&contexts, boot).unwrap();
        ContextsStore::new(s.dir.path().join("contexts.json"))
    }

    fn saved_window_server_id(s: &Setup) -> Option<WindowServerId> {
        s.reactor.contexts.contexts()[0].members[0].window_server_id
    }

    /// R22.
    #[test]
    fn r22_contexts_saved_in_another_boot_forget_their_window_server_ids() {
        let mut s = Setup::new(1);
        let now = SystemTime::now();

        let store = saved_by_boot(&s, Some("boot"));
        s.reactor.open_contexts(store, Some("boot".into()), now);
        assert_eq!(Some(WindowServerId::new(1)), saved_window_server_id(&s));

        let store = saved_by_boot(&s, Some("earlier boot"));
        s.reactor.open_contexts(store, Some("boot".into()), now);
        assert_eq!(None, saved_window_server_id(&s));

        let store = saved_by_boot(&s, None);
        s.reactor.open_contexts(store, Some("boot".into()), now);
        assert_eq!(None, saved_window_server_id(&s));

        let store = saved_by_boot(&s, Some("boot"));
        s.reactor.open_contexts(store, None, now);
        assert_eq!(None, saved_window_server_id(&s));
    }

    /// L2.
    #[test]
    fn l2_loading_drops_the_layouts_of_deleted_contexts_only_when_the_file_is_read() {
        let mut s = Setup::new(2);
        let c = s.create("C", &[wid(1)]);
        let d = s.create("D", &[wid(2)]);
        s.switch(c);
        s.switch(d);
        s.switch(ContextKey::Everything);
        let (ContextKey::Named(c_id), ContextKey::Named(d_id)) = (c, d) else {
            unreachable!()
        };
        let ids = |s: &Setup| {
            let mut ids: Vec<_> = s.reactor.layout.context_ids().collect();
            ids.sort();
            ids.dedup();
            ids
        };
        assert_eq!(vec![c_id, d_id], ids(&s));
        let path = s.dir.path().join("contexts.json");
        let now = SystemTime::now();

        fs::write(&path, "{ not json").unwrap();
        s.reactor
            .open_contexts(ContextsStore::new(path.clone()), Some("boot".into()), now);
        assert_eq!(vec![c_id, d_id], ids(&s));
        assert!(s.reactor.contexts.contexts().is_empty());
        let new = s.reactor.contexts.create("New").unwrap();
        assert!(new > d_id, "{new:?}");

        let mut kept = crate::model::contexts::Contexts::new();
        assert_eq!(c_id, kept.create("C").unwrap());
        ContextsStore::new(path.clone()).save(&kept, Some("boot")).unwrap();
        s.reactor
            .open_contexts(ContextsStore::new(path.clone()), Some("boot".into()), now);
        assert_eq!(vec![c_id], ids(&s));

        fs::remove_file(&path).unwrap();
        s.reactor.open_contexts(ContextsStore::new(path), Some("boot".into()), now);
        assert!(ids(&s).is_empty());
    }

    /// Each form of `ContextRef` survives the RON round trip that recordings
    /// take. A bare integer is a number, and `Id(7)` is an id.
    #[test]
    fn every_context_ref_survives_a_ron_round_trip() {
        let id: ContextId = serde_json::from_value(serde_json::json!(7)).unwrap();
        let command =
            |reference| Event::Command(Command::Context(ContextCommand::SwitchContext(reference)));
        let read = |text: &str| match ron::de::from_str(text).unwrap() {
            Event::Command(Command::Context(ContextCommand::SwitchContext(reference))) => reference,
            other => panic!("{other:?}"),
        };
        for reference in [
            ContextRef::Number(7),
            ContextRef::Name("Comms".into()),
            ContextRef::Name("7".into()),
            ContextRef::Id(id),
        ] {
            let text = ron::ser::to_string(&command(reference.clone())).unwrap();
            assert_eq!(reference, read(&text), "{text}");
        }
        assert_eq!(ContextRef::Number(7), read("Command(switch_context(7))"));
        assert_eq!(ContextRef::Id(id), read("Command(switch_context(Id(7)))"));
        assert_eq!(ContextRef::Id(id), read("Command(switch_context(id(7)))"));
        assert!(ron::de::from_str::<Event>("Command(switch_context(Other(7)))").is_err());
        for command in [
            ContextCommand::ShowEverything,
            ContextCommand::PreviousContext,
        ] {
            let event = Event::Command(Command::Context(command.clone()));
            let text = ron::ser::to_string(&event).unwrap();
            let Event::Command(Command::Context(back)) = ron::de::from_str(&text).unwrap() else {
                panic!("{text}");
            };
            assert_eq!(command, back);
        }
    }

    /// Makes the reactor save its layout in the temporary directory, and
    /// returns the exit codes it quits with.
    fn catch_exits(s: &mut Setup) -> Arc<Mutex<Vec<i32>>> {
        let exits = Arc::new(Mutex::new(vec![]));
        let caught = exits.clone();
        s.reactor.layout_file = Some(s.dir.path().join("layout.ron"));
        s.reactor.exit = Box::new(move |code| caught.lock().unwrap().push(code));
        exits
    }

    fn save_and_exit(s: &mut Setup) {
        s.reactor
            .handle_event(Event::Command(Command::Reactor(ReactorCommand::SaveAndExit)));
    }

    /// R32.
    #[test]
    fn r32_quitting_puts_every_parked_window_back_and_quits_after_the_last_is_back() {
        let mut s = Setup::new(3);
        let all = [wid(1), wid(2), wid(3)];
        let everything = s.frames(&all);
        let c = s.create("C", &[wid(1)]);
        s.switch(c);
        let exits = catch_exits(&mut s);

        save_and_exit(&mut s);

        let requests = s.apps.requests();
        assert_eq!(vec![everything[1].1], frame_writes(&requests, wid(2)));
        assert_eq!(vec![everything[2].1], frame_writes(&requests, wid(3)));
        assert!(exits.lock().unwrap().is_empty());
        // Contexts commands wait for the quit.
        s.command(ContextKey::Everything);
        s.command(c);
        assert!(s.apps.requests().is_empty());
        assert!(s.parked().is_empty());

        let mut echoes = s.apps.simulate_events_for_requests(requests).into_iter();
        let first = echoes.next().unwrap();
        s.reactor.handle_event(first);
        assert!(exits.lock().unwrap().is_empty());
        for event in echoes {
            s.reactor.handle_event(event);
        }

        assert_eq!(vec![0], *exits.lock().unwrap());
        assert_eq!(everything, s.frames(&all));
        assert!(s.journal_on_disk().is_empty());
        assert_eq!(c, s.saved_active());
        let saved = fs::read_to_string(s.dir.path().join("layout.ron")).unwrap();
        assert!(saved.contains("context_layouts:{((1),Named(1))"), "{saved}");
        s.reactor.exit_deadline_tick(Instant::now() + Duration::from_secs(10));
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(vec![0], *exits.lock().unwrap(), "the quit happens once");
    }

    /// R32.
    #[test]
    fn r32_when_the_deadline_passes_first_the_journal_stays() {
        let mut s = Setup::new(3);
        let c = s.create("C", &[wid(1)]);
        s.switch(c);
        let journal = s.journal_on_disk();
        assert_eq!(2, journal.len());
        let exits = catch_exits(&mut s);
        let start = Instant::now();

        save_and_exit(&mut s);
        save_and_exit(&mut s);
        s.reactor.exit_deadline_tick(start + Duration::from_millis(1900));
        assert!(exits.lock().unwrap().is_empty());
        s.reactor.exit_deadline_tick(Instant::now() + Duration::from_secs(2));

        assert_eq!(vec![0], *exits.lock().unwrap());
        assert_eq!(journal, s.journal_on_disk());
        assert_eq!(c, s.saved_active());
        assert!(s.dir.path().join("layout.ron").exists());
    }

    /// R32.
    #[test]
    fn r32_with_nothing_parked_quitting_is_at_once() {
        let mut s = Setup::new(2);
        let exits = catch_exits(&mut s);

        save_and_exit(&mut s);

        assert_eq!(vec![0], *exits.lock().unwrap());
        assert!(s.apps.requests().is_empty());
        assert_eq!(ContextKey::Everything, s.saved_active());
    }

    /// R33.
    #[test]
    fn r33_turning_off_shows_everything_first_and_turning_on_applies_the_context_again() {
        let mut s = Setup::new(3);
        let all = [wid(1), wid(2), wid(3)];
        let everything = s.frames(&all);
        let c = s.create("C", &[wid(1)]);
        s.switch(c);
        assert_eq!(vec![wid(2), wid(3)], s.parked());

        s.reactor.handle_event(Event::ShowEverythingOn(vec![space()]));
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(everything, s.frames(&all));
        assert!(s.parked().is_empty());
        assert!(s.journal_on_disk().is_empty());
        assert_eq!(c, s.reactor.contexts.active());

        s.reactor.handle_event(Event::SpaceChanged(vec![None], Default::default()));
        let requests = s.apps.requests();
        assert!(all.iter().all(|&wid| frame_writes(&requests, wid).is_empty()));
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(everything, s.frames(&all));

        s.reactor
            .handle_event(Event::SpaceChanged(vec![Some(space())], on_screen(&s, &all)));
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(vec![wid(2), wid(3)], s.parked());
        assert_eq!(vec![(wid(1), screen())], s.tiles());
    }

    /// R33.
    #[test]
    fn r33_a_space_change_to_none_from_the_login_window_changes_nothing() {
        let mut s = Setup::new(3);
        let c = s.create("C", &[wid(1)]);
        s.switch(c);
        let frames = s.frames(&[wid(1), wid(2), wid(3)]);
        let journal = s.journal_on_disk();

        s.reactor.handle_event(Event::SpaceChanged(vec![None], Default::default()));
        s.apps.simulate_until_quiet(&mut s.reactor);

        assert_eq!(frames, s.frames(&[wid(1), wid(2), wid(3)]));
        assert_eq!(vec![wid(2), wid(3)], s.parked());
        assert_eq!(journal, s.journal_on_disk());

        let snapshot = on_screen(&s, &[wid(1), wid(2), wid(3)]);
        s.reactor.handle_event(Event::SpaceChanged(vec![Some(space())], snapshot));
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(frames, s.frames(&[wid(1), wid(2), wid(3)]));
        assert_eq!(vec![wid(2), wid(3)], s.parked());
        assert_eq!(vec![(wid(1), screen())], s.tiles());
    }

    /// R33, R7.
    #[test]
    fn r33_turning_off_one_space_shows_everything_only_there() {
        let left = screen();
        let right = rect(1200., 0., 1200., 1000.);
        let space2 = SpaceId::new(2);
        let mut s = Setup::on(vec![left, right], vec![Some(space()), Some(space2)]);
        let at = |x: f64, idx| WindowInfo {
            frame: rect(x, 100., 50., 50.),
            ..make_window(idx)
        };
        s.reactor.handle_events(
            s.apps.make_app(1, vec![at(100., 1), at(200., 2), at(1300., 3), at(1400., 4)]),
        );
        s.reactor.handle_event(Event::StartupComplete);
        s.apps.simulate_until_quiet(&mut s.reactor);
        let right_everything = s.frames(&[wid(3), wid(4)]);
        let c = s.create("C", &[wid(1), wid(3)]);
        s.switch(c);
        assert_eq!(vec![wid(2), wid(4)], s.parked());

        s.reactor.handle_event(Event::ShowEverythingOn(vec![space2]));
        s.apps.simulate_until_quiet(&mut s.reactor);

        assert_eq!(vec![wid(2)], s.parked());
        assert_eq!(right_everything, s.frames(&[wid(3), wid(4)]));
        assert_eq!(right_everything, s.tiles_on(space2, right));
        assert_eq!(vec![(wid(1), left)], s.tiles_on(space(), left));
        assert_eq!(c, s.reactor.contexts.active());
    }

    /// Counts the errors logged while `f` runs.
    fn count_errors(f: impl FnOnce()) -> usize {
        use std::sync::atomic::{AtomicUsize, Ordering};

        use tracing_subscriber::layer::{Context, SubscriberExt};

        struct ErrorCounter(Arc<AtomicUsize>);
        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for ErrorCounter {
            fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
                if *event.metadata().level() == tracing::Level::ERROR {
                    self.0.fetch_add(1, Ordering::Relaxed);
                }
            }
        }

        let count = Arc::new(AtomicUsize::new(0));
        let subscriber = tracing_subscriber::registry().with(ErrorCounter(count.clone()));
        tracing::subscriber::with_default(subscriber, f);
        count.load(Ordering::Relaxed)
    }

    /// R6, L2.
    #[test]
    fn r6_deleting_the_active_context_shows_unsorted_first_without_errors() {
        let mut s = Setup::new(3);
        let c = s.create("C", &[wid(1), wid(2)]);
        let d = s.create("D", &[wid(2)]);
        s.switch(d);
        s.switch(c);
        assert_eq!(vec![wid(3)], s.parked());
        let ContextKey::Named(c_id) = c else { unreachable!() };

        let errors = count_errors(|| s.reactor.delete_context(c_id).unwrap());
        s.apps.simulate_until_quiet(&mut s.reactor);

        assert_eq!(0, errors);
        assert_eq!(ContextKey::Unsorted, s.reactor.contexts.active());
        assert_eq!(ContextKey::Unsorted, s.saved_active());
        assert!(s.reactor.layout.context_ids().all(|id| id != c_id));
        assert_eq!(vec![wid(2)], s.parked());
        let shown = vec![
            (wid(1), rect(0., 0., 600., 1000.)),
            (wid(3), rect(600., 0., 600., 1000.)),
        ];
        assert_eq!(shown, s.tiles());
        assert_eq!(shown, s.frames(&[wid(1), wid(3)]));
    }

    /// R6, L2.
    #[test]
    fn r6_deleting_a_context_that_is_not_active_moves_nothing() {
        let mut s = Setup::new(2);
        let c = s.create("C", &[wid(1)]);
        s.switch(c);
        s.switch(ContextKey::Everything);
        let frames = s.frames(&[wid(1), wid(2)]);
        let ContextKey::Named(c_id) = c else { unreachable!() };

        let errors = count_errors(|| s.reactor.delete_context(c_id).unwrap());

        assert_eq!(0, errors);
        assert!(s.apps.requests().is_empty());
        assert_eq!(frames, s.frames(&[wid(1), wid(2)]));
        assert_eq!(0, s.reactor.layout.context_ids().count());
        assert_eq!(ContextKey::Everything, s.saved_active());
    }

    /// L8. The app quits and starts again while another context is active,
    /// and its new window joins C there.
    #[test]
    fn l8_a_window_of_a_relaunched_app_is_tiled_when_its_context_becomes_active() {
        let mut s = Setup::new(1);
        let window = |idx: u32| WindowInfo {
            sys_id: Some(WindowServerId::new(20 + idx)),
            frame: rect(700., 100., 50., 50.),
            ..make_window(idx as usize)
        };
        s.reactor.handle_events(s.apps.make_app(2, vec![window(1)]));
        s.reactor.handle_event(Event::WindowsOnScreenUpdated {
            pid: None,
            on_screen: on_screen(&s, &[wid(1), WindowId::new(2, 1)]),
        });
        s.apps.simulate_until_quiet(&mut s.reactor);
        let c = s.create("C", &[wid(1), WindowId::new(2, 1)]);
        let d = s.create("D", &[wid(1)]);
        s.switch(c);
        s.switch(d);
        assert_eq!(vec![WindowId::new(2, 1)], s.parked());

        s.reactor.handle_event(Event::ApplicationTerminated(2));
        s.reactor.handle_event(Event::ApplicationThreadTerminated(2));
        s.apps.windows.remove(&WindowId::new(2, 1));
        s.reactor.handle_events(s.apps.make_app(3, vec![window(1)]));
        let relaunched = WindowId::new(3, 1);
        s.reactor.handle_event(Event::WindowsOnScreenUpdated {
            pid: None,
            on_screen: on_screen(&s, &[wid(1), relaunched]),
        });
        s.apps.simulate_until_quiet(&mut s.reactor);
        s.add(c, relaunched);

        s.switch(c);

        let tiles = vec![
            (wid(1), rect(0., 0., 600., 1000.)),
            (relaunched, rect(600., 0., 600., 1000.)),
        ];
        assert_eq!(tiles, s.tiles());
        assert_eq!(tiles, s.frames(&[wid(1), relaunched]));
        assert!(s.parked().is_empty());
    }

    /// L6.
    #[test]
    fn l6_a_minimized_member_leaves_the_layout_and_stays_a_member() {
        let mut s = Setup::new(3);
        let c = s.create("C", &[wid(1), wid(2)]);
        s.switch(c);
        assert_eq!(vec![wid(3)], s.parked());

        // Window 2 is minimized.
        s.reactor.handle_event(Event::WindowsOnScreenUpdated {
            pid: None,
            on_screen: on_screen(&s, &[wid(1), wid(3)]),
        });
        s.reactor.update_visible_windows();
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(vec![(wid(1), screen())], s.tiles());
        s.switch(ContextKey::Everything);
        s.switch(c);
        assert_eq!(vec![(wid(1), screen())], s.tiles());
        assert_eq!(vec![wid(3)], s.parked());
        assert!(s.reactor.contexts.is_member(c, wid(2)));

        // It comes back.
        s.reactor.handle_event(Event::WindowsOnScreenUpdated {
            pid: None,
            on_screen: on_screen(&s, &[wid(1), wid(2), wid(3)]),
        });
        s.reactor.update_visible_windows();
        s.apps.simulate_until_quiet(&mut s.reactor);
        let tiles = vec![
            (wid(1), rect(0., 0., 600., 1000.)),
            (wid(2), rect(600., 0., 600., 1000.)),
        ];
        assert_eq!(tiles, s.tiles());
        assert_eq!(tiles, s.frames(&[wid(1), wid(2)]));
    }

    /// L2, L8, L9. Floating a window under C leaves its node in Everything's
    /// layout for another screen size.
    #[test]
    fn l2_a_display_change_leaves_a_window_that_floats_out_of_the_new_sizes_layout() {
        let mut s = Setup::new(2);
        let shorter = rect(0., 0., 1200., 800.);
        let display = |s: &Setup, frame| match screens(vec![frame], vec![Some(space())]) {
            Event::ScreenParametersChanged {
                frames,
                bounds,
                spaces,
                scale_factors,
                converter,
                ..
            } => Event::ScreenParametersChanged {
                frames,
                bounds,
                spaces,
                scale_factors,
                converter,
                on_screen: on_screen(s, &[wid(1), wid(2)]),
            },
            _ => unreachable!(),
        };
        // Everything gets a layout of its own for each screen size.
        s.move_window(wid(1), Direction::Right);
        s.reactor.handle_event(display(&s, shorter));
        s.apps.simulate_until_quiet(&mut s.reactor);
        s.move_window(wid(1), Direction::Left);
        s.reactor.handle_event(display(&s, screen()));
        s.apps.simulate_until_quiet(&mut s.reactor);
        let c = s.create("C", &[wid(1), wid(2)]);
        s.switch(c);
        float_window_1(&mut s);
        let floating = rect(100., 100., 50., 50.);
        s.switch(ContextKey::Everything);
        assert_eq!(vec![(wid(2), screen())], s.tiles());

        s.reactor.handle_event(display(&s, shorter));
        s.apps.simulate_until_quiet(&mut s.reactor);

        assert_eq!(vec![(wid(2), shorter)], s.tiles_on(space(), shorter));
        assert_eq!(shorter, s.frame(wid(2)));
        assert_eq!(floating, s.frame(wid(1)));
    }
}
