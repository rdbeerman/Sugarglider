// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Creating a context from the windows on screen.

use tracing::{debug, info, warn};

use super::{ContextCommand, ContextRef, Reactor};
use crate::actor::app::{WindowId, pid_t};
use crate::model::contexts::WindowDesc;

impl Reactor {
    /// Creates a context whose members are the windows that show on the
    /// visible Spaces, and switches to it. Its layout starts as the layout
    /// the Spaces show (L3). Pinned windows get no record, because they are
    /// members of every context already (R3).
    pub(super) fn create_context(&mut self, name: &str) {
        if !self.contexts_enabled() {
            debug!(name, "Ignoring a new context while contexts are off");
            return;
        }
        if self.pending_exit.is_some() {
            info!(name, "Ignoring a new context while quitting");
            return;
        }
        let id = match self.contexts.create(name) {
            Ok(id) => id,
            Err(err) => {
                warn!(name, "Could not create a context: {err}");
                return;
            }
        };
        let members: Vec<WindowDesc> = self
            .windows_on_visible_spaces(|wid| {
                !self.parked.contains_key(&wid) && !self.contexts.is_pinned(wid)
            })
            .into_iter()
            .filter_map(|wid| self.window_desc(wid))
            .collect();
        for member in &members {
            self.contexts.add_window(id, member).expect("the context was just created");
        }
        info!(name, members = members.len(), "Created a context");
        self.save_contexts();
        self.handle_context_command(ContextCommand::SwitchContext(ContextRef::Id(id)));
    }

    /// The windows on the visible Spaces for which `keep` returns true, in id
    /// order. These are the windows of running apps that the window server
    /// lists as visible, on a screen that shows a Space, or on no screen
    /// while one does, as a switch finds them. Sugarglider's own windows and
    /// windows the layout doesn't track are left out. `keep` runs before the
    /// checks that copy the window's details.
    pub(super) fn windows_on_visible_spaces(
        &self,
        keep: impl Fn(WindowId) -> bool,
    ) -> Vec<WindowId> {
        let own_pid = std::process::id() as pid_t;
        let any_space = self.screens.iter().any(|screen| screen.space.is_some());
        let mut wids: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(wid, window)| {
                wid.pid != own_pid
                    && self.apps.contains_key(&wid.pid)
                    && window
                        .window_server_id
                        .is_some_and(|wsid| self.visible_windows.contains(&wsid))
            })
            .map(|(wid, _)| *wid)
            .filter(|&wid| keep(wid))
            .filter(|&wid| {
                let Some(info) = self.layout_window_info(wid) else {
                    return false;
                };
                let on_space = match self.best_screen_idx_for_window(&info.frame) {
                    Some(screen) => self.screens[screen].space.is_some(),
                    None => any_space,
                };
                on_space && !self.layout.is_untracked(&info)
            })
            .collect();
        wids.sort();
        wids
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::sync::Arc;
    use std::time::SystemTime;

    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;
    use test_log::test;

    use super::super::testing::*;
    use super::super::{Command, ContextCommand, ContextRef, Event, Reactor};
    use crate::actor::app::WindowId;
    use crate::actor::contexts_store::{ContextsStore, Loaded};
    use crate::actor::layout::LayoutManager;
    use crate::actor::parked_journal::ParkedJournal;
    use crate::actor::server::{ContextRequest, Response, answer_context_request};
    use crate::config::Config;
    use crate::model::contexts::{ContextId, ContextKey, Contexts};
    use crate::sys::app::WindowInfo;
    use crate::sys::screen::{CoordinateConverter, SpaceId};
    use crate::sys::window_server::{WindowServerId, WindowServerInfo, WindowsOnScreen};

    pub fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
        CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
    }

    pub fn wid(idx: u32) -> WindowId {
        WindowId::new(1, idx)
    }

    pub fn space() -> SpaceId {
        SpaceId::new(1)
    }

    pub fn screen() -> CGRect {
        rect(0., 0., 1200., 1000.)
    }

    pub fn config(contexts: bool) -> Arc<Config> {
        let mut config = Config::default();
        config.settings.default_disable = false;
        config.settings.animate = false;
        config.settings.experimental.contexts.enable = contexts;
        Arc::new(config)
    }

    pub fn screens(frames: Vec<CGRect>, spaces: Vec<Option<SpaceId>>) -> Event {
        Event::ScreenParametersChanged {
            bounds: frames.clone(),
            scale_factors: vec![1.0; frames.len()],
            frames,
            spaces,
            converter: CoordinateConverter::default(),
            on_screen: Default::default(),
        }
    }

    pub fn context(command: ContextCommand) -> Event {
        Event::Command(Command::Context(command))
    }

    /// A reactor with contexts on, its journal and `contexts.json` in a
    /// temporary directory.
    pub struct Setup {
        pub reactor: Reactor,
        pub apps: Apps,
        pub dir: TempDir,
    }

    impl Setup {
        /// App 1's `windows` windows tiled side by side on one screen.
        pub fn new(windows: usize) -> Setup {
            let mut s = Setup::on(vec![screen()], vec![Some(space())]);
            s.reactor.handle_events(s.apps.make_app(1, make_windows(windows)));
            s.reactor.handle_event(Event::StartupComplete);
            s.apps.simulate_until_quiet(&mut s.reactor);
            s
        }

        /// A reactor with contexts on that no app has reached yet.
        pub fn on(frames: Vec<CGRect>, spaces: Vec<Option<SpaceId>>) -> Setup {
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

        /// Sends a context command, and lets the apps answer.
        pub fn run(&mut self, command: ContextCommand) {
            self.reactor.handle_event(context(command));
            self.apps.simulate_until_quiet(&mut self.reactor);
        }

        pub fn create(&mut self, name: &str) {
            self.run(ContextCommand::CreateContext(name.into()));
        }

        pub fn id(&self, name: &str) -> ContextId {
            self.reactor.contexts.by_name(name).unwrap().id
        }

        /// The open member windows of the named context.
        pub fn members(&self, name: &str) -> Vec<WindowId> {
            let context = self.reactor.contexts.by_name(name).unwrap();
            context.members.iter().filter_map(|record| record.window()).collect()
        }

        pub fn frames(&self, wids: &[WindowId]) -> Vec<(WindowId, CGRect)> {
            wids.iter().map(|&wid| (wid, self.apps.windows[&wid].frame)).collect()
        }

        pub fn parked(&self) -> Vec<WindowId> {
            let mut parked: Vec<WindowId> = self.reactor.parked.keys().copied().collect();
            parked.sort();
            parked
        }

        pub fn saved(&self) -> Contexts {
            match ContextsStore::new(self.dir.path().join("contexts.json")).load(SystemTime::now())
            {
                Loaded::Read { contexts, .. } => contexts,
                other => panic!("{other:?}"),
            }
        }
    }

    /// M5c. The members are the windows that show on the visible Spaces.
    /// Sugarglider's own window, a panel the layout leaves alone, a
    /// minimized window, and a window on a screen whose Space is off stay
    /// out.
    #[test]
    fn a_new_context_holds_the_tracked_windows_on_the_visible_spaces() {
        let right = rect(1200., 0., 1200., 1000.);
        let mut s = Setup::on(vec![screen(), right], vec![Some(space()), None]);
        let own_pid = std::process::id() as i32;
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

        s.create("Work");

        assert_eq!(vec![wid(1), wid(2)], s.members("Work"));
        assert_eq!(ContextKey::Named(s.id("Work")), s.reactor.contexts.active());
        assert!(s.parked().is_empty());
    }

    /// M5c, L3. Under a context, the new context takes the windows that
    /// show. A parked window stays parked and out, and a pinned window gets
    /// no record (R3). The windows keep their frames.
    #[test]
    fn a_new_context_keeps_what_the_screen_shows() {
        let mut s = Setup::new(4);
        let c = s.reactor.contexts.create("C").unwrap();
        let d = s.reactor.contexts.create("D").unwrap();
        for (id, idx) in [(c, 1), (c, 2), (d, 3)] {
            let desc = s.reactor.window_desc(wid(idx)).unwrap();
            s.reactor.contexts.add_window(id, &desc).unwrap();
        }
        let pinned = s.reactor.window_desc(wid(4)).unwrap();
        s.reactor.contexts.pin(&pinned);
        s.run(ContextCommand::SwitchContext(ContextRef::Id(c)));
        assert_eq!(vec![wid(3)], s.parked());
        let shown = [wid(1), wid(2), wid(4)];
        let frames = s.frames(&shown);

        s.create("New");

        assert_eq!(vec![wid(1), wid(2)], s.members("New"));
        assert_eq!(ContextKey::Named(s.id("New")), s.reactor.contexts.active());
        assert_eq!(vec![wid(3)], s.parked());
        assert_eq!(frames, s.frames(&shown));
        let mut tiles = s.reactor.layout.calculate_layout(space(), screen(), &s.reactor.config);
        tiles.sort_by_key(|(wid, _)| *wid);
        assert_eq!(frames, tiles);
        assert_eq!(vec![wid(1), wid(2)], s.members("C"));
    }

    /// The new context and its members are saved, and it is saved active.
    #[test]
    fn a_new_context_is_saved() {
        let mut s = Setup::new(2);

        s.create("Work");

        let saved = s.saved();
        let work = saved.by_name("Work").unwrap();
        assert_eq!(Some(1), work.number);
        assert_eq!(
            vec!["Window1", "Window2"],
            work.members.iter().map(|m| m.title.as_str()).collect::<Vec<_>>()
        );
        assert_eq!(ContextKey::Named(work.id), saved.active());
    }

    /// R4. A name that is taken, reserved, or empty creates nothing and
    /// switches nowhere.
    #[test]
    fn a_new_context_needs_a_free_name() {
        let mut s = Setup::new(2);
        s.create("Work");
        s.run(ContextCommand::ShowEverything);

        for name in ["work", "Everything", "  "] {
            s.create(name);
        }

        assert_eq!(1, s.reactor.contexts.contexts().len());
        assert_eq!(ContextKey::Everything, s.reactor.contexts.active());
    }

    /// R28. With contexts off, nothing is created.
    #[test]
    fn no_context_is_created_while_contexts_are_off() {
        let mut s = Setup::new(2);
        s.reactor.handle_event(Event::ConfigChanged(config(false)));

        s.create("Work");

        assert!(s.reactor.contexts.contexts().is_empty());
        assert_eq!(ContextKey::Everything, s.reactor.contexts.active());
    }

    /// M5c, I3. `sugarglider context create "Client work"` followed at once
    /// by `sugarglider context switch "Client work"`. The server answers
    /// both from a snapshot that doesn't have the new context yet, so it
    /// sends the name as it is, and the reactor resolves it to the new
    /// context.
    #[test]
    fn a_switch_right_after_a_create_goes_to_the_new_context() {
        let mut s = Setup::new(2);
        s.create("Comms");
        s.run(ContextCommand::ShowEverything);
        let stale = s.reactor.published_contexts.clone().unwrap();
        let answer = |command| answer_context_request(ContextRequest::Run(command), Some(&stale));
        let by_name = ContextCommand::SwitchContext(ContextRef::Name("Client work".into()));

        let (reply, create) = answer(ContextCommand::CreateContext("Client work".into()));
        assert_eq!(Response::Success, reply);
        let (reply, switch) = answer(by_name.clone());
        assert_eq!((Response::Success, Some(by_name)), (reply, switch.clone()));

        s.run(create.unwrap());
        let client = ContextKey::Named(s.id("Client work"));
        let created = s.reactor.contexts.last_used(client);
        s.run(switch.unwrap());

        assert_eq!(client, s.reactor.contexts.active());
        assert_eq!(created + 1, s.reactor.contexts.last_used(client));
        assert_eq!(vec![wid(1), wid(2)], s.members("Client work"));
    }

    /// The command survives the RON round trip that recordings use.
    #[test]
    fn the_create_command_survives_a_ron_round_trip() {
        let event = context(ContextCommand::CreateContext("Client work".into()));
        let ron = ron::ser::to_string(&event).unwrap();
        assert_eq!("Command(create_context(\"Client work\"))", ron);
        let Event::Command(Command::Context(command)) = ron::de::from_str(&ron).unwrap() else {
            panic!("{ron}");
        };
        assert_eq!(ContextCommand::CreateContext("Client work".into()), command);
    }
}
