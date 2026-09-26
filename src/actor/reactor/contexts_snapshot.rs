// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Publishing the contexts for the threads that read them (I1).

use std::sync::Arc;

use super::Reactor;
use crate::actor::contexts_snapshot::{ContextsSnapshot, ScreenContext};

impl Reactor {
    /// The contexts as the command line and the switcher see them.
    fn contexts_snapshot(&self) -> ContextsSnapshot {
        if !self.contexts_enabled() {
            return ContextsSnapshot::off();
        }
        let screens = self
            .screens
            .iter()
            .zip(1..)
            .filter_map(|(screen, id)| {
                Some(ScreenContext {
                    id,
                    shows: self.shown_context(screen.space?),
                })
            })
            .collect();
        let unsorted = self.windows_on_visible_spaces(|wid| self.contexts.is_unsorted(wid)).len();
        ContextsSnapshot::new(&self.contexts, screens, unsorted)
    }

    /// Publishes the snapshot of the contexts when it differs from the one
    /// published last.
    pub(super) fn publish_contexts_snapshot(&mut self) {
        let snapshot = self.contexts_snapshot();
        if self.published_contexts.as_deref() == Some(&snapshot) {
            return;
        }
        let snapshot = Arc::new(snapshot);
        self.published_contexts = Some(snapshot.clone());
        (self.publish_contexts)(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use pretty_assertions::assert_eq;
    use test_log::test;

    use super::super::create_context::tests::*;
    use super::super::testing::*;
    use super::super::{Command, ContextCommand, ContextRef, Event, Reactor, ReactorCommand};
    use crate::actor::app::WindowId;
    use crate::actor::contexts_snapshot::{ContextSummary, ContextsSnapshot, ScreenContext};
    use crate::actor::layout::{LayoutCommand, LayoutEvent, LayoutManager};
    use crate::actor::parked_journal::FailingWrites;
    use crate::actor::server::{ContextRequest, Response, answer_context_request};
    use crate::model::Direction;
    use crate::model::contexts::{ContextId, ContextKey, WindowDesc};
    use crate::sys::app::WindowInfo;
    use crate::sys::screen::SpaceId;
    use crate::sys::window_server::{WindowServerId, WindowServerInfo, WindowsOnScreen};

    type Published = Arc<Mutex<Vec<Arc<ContextsSnapshot>>>>;

    /// Keeps every snapshot the reactor publishes from now on.
    fn capture(reactor: &mut Reactor) -> Published {
        let published = Published::default();
        let sink = published.clone();
        reactor.publish_contexts = Box::new(move |snapshot| sink.lock().unwrap().push(snapshot));
        reactor.published_contexts = None;
        published
    }

    fn count(published: &Published) -> usize {
        published.lock().unwrap().len()
    }

    fn last(published: &Published) -> ContextsSnapshot {
        (**published.lock().unwrap().last().unwrap()).clone()
    }

    fn shows(key: ContextKey) -> Vec<ScreenContext> {
        vec![ScreenContext { id: 1, shows: key }]
    }

    /// A window server snapshot that lists app 1's windows at their frames.
    fn listed(s: &Setup, idxs: &[u32]) -> WindowsOnScreen {
        WindowsOnScreen::new(
            idxs.iter()
                .map(|&idx| WindowServerInfo {
                    id: WindowServerId::new(idx),
                    pid: 1,
                    layer: 0,
                    frame: s.apps.windows[&wid(idx)].frame,
                })
                .collect(),
        )
    }

    /// I1. The reactor publishes a snapshot after each event that changes
    /// the contexts or the windows they count, and only then. The first
    /// snapshot is published even when contexts are off.
    #[test]
    fn a_snapshot_is_published_after_each_kind_of_change() {
        let mut reactor = Reactor::new_for_test(LayoutManager::new_for_test());
        let published = capture(&mut reactor);
        let mut s = Setup {
            reactor,
            apps: Apps::new(),
            dir: tempfile::TempDir::new().unwrap(),
        };

        s.reactor.handle_event(screens(vec![screen()], vec![Some(space())]));
        assert_eq!(1, count(&published));
        assert_eq!(ContextsSnapshot::off(), last(&published));

        s.reactor.handle_event(Event::ConfigChanged(config(true)));
        let on = last(&published);
        assert!(on.enabled);
        assert_eq!(ContextKey::Everything, on.active);
        assert_eq!(shows(ContextKey::Everything), on.screens);
        assert!(on.contexts.is_empty());
        assert_eq!(0, on.unsorted.windows);

        // Windows open.
        s.reactor.handle_events(s.apps.make_app(1, make_windows(3)));
        s.reactor.handle_event(Event::StartupComplete);
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(3, last(&published).unsorted.windows);

        // Nothing changes.
        let before = count(&published);
        s.reactor.handle_event(Event::MouseUp);
        s.reactor.handle_event(Event::StartupComplete);
        assert_eq!(before, count(&published));

        // A context is created.
        s.create("Work");
        let work = s.id("Work");
        let created = last(&published);
        assert_eq!(
            vec![ContextSummary {
                id: work,
                name: "Work".into(),
                number: Some(1),
                last_used: 1,
                apps: vec!["TestApp1".into()],
                windows: 3,
            }],
            created.contexts
        );
        assert_eq!(ContextKey::Named(work), created.active);
        assert_eq!(shows(ContextKey::Named(work)), created.screens);
        assert_eq!(0, created.unsorted.windows);

        // A switch.
        s.run(ContextCommand::ShowEverything);
        let switched = last(&published);
        assert_eq!(ContextKey::Everything, switched.active);
        assert_eq!(2, switched.everything.last_used);

        // A window opens and closes.
        let other = WindowId::new(2, 1);
        let other_window = WindowInfo {
            sys_id: Some(WindowServerId::new(20)),
            ..make_window(4)
        };
        s.reactor.handle_events(s.apps.make_app(2, vec![other_window]));
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(1, last(&published).unsorted.windows);
        s.apps.windows.remove(&other);
        s.reactor.handle_event(Event::WindowDestroyed(other));
        assert_eq!(0, last(&published).unsorted.windows);

        // The Space is about to be turned off, so it shows every window
        // until the next space change.
        s.run(ContextCommand::SwitchContext(ContextRef::Id(work)));
        s.reactor.handle_event(Event::ShowEverythingOn(vec![space()]));
        let turning_off = last(&published);
        assert_eq!(ContextKey::Named(work), turning_off.active);
        assert_eq!(shows(ContextKey::Everything), turning_off.screens);
        let all = listed(&s, &[1, 2, 3]);
        s.reactor.handle_event(Event::SpaceChanged(vec![Some(space())], all));
        assert_eq!(shows(ContextKey::Named(work)), last(&published).screens);

        // The context is deleted. No command deletes a context yet, so the
        // next event publishes the change.
        s.reactor.delete_context(work).unwrap();
        s.reactor.handle_event(Event::StartupComplete);
        let deleted = last(&published);
        assert!(deleted.contexts.is_empty());
        assert_eq!(ContextKey::Unsorted, deleted.active);
        assert_eq!(3, deleted.unsorted.windows);

        // Contexts are turned off.
        s.reactor.handle_event(Event::ConfigChanged(config(false)));
        assert_eq!(ContextsSnapshot::off(), last(&published));
    }

    /// The Unsorted count follows the windows on the visible Spaces: a
    /// window that leaves them, for example by being minimized, stops
    /// counting.
    #[test]
    fn unsorted_counts_the_windows_on_the_visible_spaces() {
        let mut s = Setup::new(3);
        let published = capture(&mut s.reactor);
        s.reactor.handle_event(Event::StartupComplete);
        assert_eq!(3, last(&published).unsorted.windows);

        let on_screen = listed(&s, &[1, 2]);
        s.reactor.handle_event(Event::WindowsOnScreenUpdated { pid: None, on_screen });

        assert_eq!(2, last(&published).unsorted.windows);
    }

    /// App 1's window as the contexts model describes it.
    fn desc(idx: u32) -> WindowDesc {
        WindowDesc {
            wid: wid(idx),
            bundle_id: Some("com.testapp1".into()),
            app_name: Some("TestApp1".into()),
            title: format!("Window{idx}"),
            window_server_id: Some(WindowServerId::new(idx)),
        }
    }

    /// Adds app 1's windows to the context by the model, as a membership
    /// command will.
    fn add(s: &mut Setup, id: ContextId, idxs: &[u32]) {
        for &idx in idxs {
            s.reactor.contexts.add_window(id, &desc(idx)).unwrap();
        }
    }

    fn summary(s: &Setup, name: &str, windows: usize) -> ContextSummary {
        let context = s.reactor.contexts.by_name(name).unwrap();
        ContextSummary {
            id: context.id,
            name: name.into(),
            number: context.number,
            last_used: context.last_used,
            apps: if windows > 0 {
                vec!["TestApp1".into()]
            } else {
                vec![]
            },
            windows,
        }
    }

    /// R1, R29. A window in two contexts counts in both. An unsorted window
    /// counts while it is parked, because it is still on a visible Space.
    #[test]
    fn a_window_counts_in_each_of_its_contexts_and_a_parked_unsorted_one_counts() {
        let mut s = Setup::new(4);
        let c = s.reactor.contexts.create("C").unwrap();
        let d = s.reactor.contexts.create("D").unwrap();
        add(&mut s, c, &[1, 2]);
        add(&mut s, d, &[2, 3]);
        let published = capture(&mut s.reactor);

        s.run(ContextCommand::SwitchContext(ContextRef::Id(c)));

        assert_eq!(vec![wid(3), wid(4)], s.parked());
        let switched = last(&published);
        assert_eq!(vec![summary(&s, "C", 2), summary(&s, "D", 2)], switched.contexts);
        assert_eq!(1, switched.unsorted.windows);
        assert_eq!(ContextKey::Named(c), switched.active);
        assert_eq!(shows(ContextKey::Named(c)), switched.screens);
    }

    /// R3. A pinned window shows under Unsorted but doesn't count as an
    /// unsorted window.
    #[test]
    fn a_pinned_window_does_not_count_as_unsorted() {
        let mut s = Setup::new(3);
        let c = s.reactor.contexts.create("C").unwrap();
        add(&mut s, c, &[1]);
        let published = capture(&mut s.reactor);
        s.reactor.handle_event(Event::StartupComplete);
        assert_eq!(2, last(&published).unsorted.windows);

        s.reactor.contexts.pin(&desc(3));
        s.run(ContextCommand::SwitchContext(ContextRef::Name(
            "Unsorted".into(),
        )));

        assert_eq!(ContextKey::Unsorted, last(&published).active);
        assert_eq!(vec![wid(1)], s.parked());
        assert_eq!(1, last(&published).unsorted.windows);
    }

    /// I1. A member window that closes no longer counts as open, in each of
    /// its contexts, and its app drops out of a context without another open
    /// window of it.
    #[test]
    #[ignore = "bug: the snapshot counts a closed member window as open, because the reactor never marks its records closed (R23)"]
    fn closing_a_member_window_publishes_the_lower_counts() {
        let mut s = Setup::new(3);
        let c = s.reactor.contexts.create("C").unwrap();
        let d = s.reactor.contexts.create("D").unwrap();
        add(&mut s, c, &[1, 2]);
        add(&mut s, d, &[2]);
        let published = capture(&mut s.reactor);
        s.reactor.handle_event(Event::StartupComplete);
        assert_eq!(
            vec![summary(&s, "C", 2), summary(&s, "D", 1)],
            last(&published).contexts
        );

        s.apps.windows.remove(&wid(2));
        s.reactor.handle_event(Event::WindowDestroyed(wid(2)));

        assert_eq!(
            vec![summary(&s, "C", 1), summary(&s, "D", 0)],
            last(&published).contexts
        );
        assert_eq!(1, last(&published).unsorted.windows);
    }

    /// I1. Each screen that shows a Space gets an entry, numbered by its
    /// place in the list of screens from 1. A screen that shows no Space
    /// gets none. A display that goes away takes its entry with it.
    #[test]
    fn each_screen_with_a_space_gets_an_entry_numbered_by_its_place() {
        let left = screen();
        let middle = rect(1200., 0., 1200., 1000.);
        let right = rect(2400., 0., 1200., 1000.);
        let mut s = Setup::on(
            vec![left, middle, right],
            vec![Some(space()), None, Some(SpaceId::new(2))],
        );
        let published = capture(&mut s.reactor);
        s.reactor.handle_event(Event::StartupComplete);
        let everywhere = |key| {
            vec![
                ScreenContext { id: 1, shows: key },
                ScreenContext { id: 3, shows: key },
            ]
        };
        assert_eq!(everywhere(ContextKey::Everything), last(&published).screens);

        s.create("Work");

        let work = ContextKey::Named(s.id("Work"));
        assert_eq!(everywhere(work), last(&published).screens);
        s.reactor.handle_event(screens(vec![left], vec![Some(space())]));
        assert_eq!(shows(work), last(&published).screens);
    }

    /// I1. Events and commands that change nothing the snapshot holds
    /// publish nothing: a layout command, a switch whose journal write
    /// fails, a switch to a context that doesn't exist or that no name
    /// matches, a refused new context, and a previous context that doesn't
    /// exist. Each change that follows publishes once.
    #[test]
    fn events_that_change_nothing_in_the_snapshot_publish_nothing() {
        let mut s = Setup::new(2);
        let c = s.reactor.contexts.create("C").unwrap();
        add(&mut s, c, &[1]);
        let published = capture(&mut s.reactor);
        s.reactor.handle_event(Event::StartupComplete);
        assert_eq!(1, count(&published));

        let frames = s.frames(&[wid(1), wid(2)]);
        s.reactor.send_layout_event(LayoutEvent::WindowFocused(vec![space()], wid(1)));
        s.reactor.handle_event(Event::Command(Command::Layout(LayoutCommand::MoveNode(
            Direction::Right,
        ))));
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_ne!(frames, s.frames(&[wid(1), wid(2)]));
        let failing = FailingWrites::start(s.dir.path());
        s.run(ContextCommand::SwitchContext(ContextRef::Id(c)));
        drop(failing);
        assert_eq!(ContextKey::Everything, s.reactor.contexts.active());
        let deleted = s.reactor.contexts.create("Gone").unwrap();
        s.reactor.contexts.delete(deleted).unwrap();
        s.run(ContextCommand::SwitchContext(ContextRef::Id(deleted)));
        s.run(ContextCommand::SwitchContext(ContextRef::Number(2)));
        s.run(ContextCommand::SwitchContext(ContextRef::Name("zzz".into())));
        for name in ["c", "Everything", " "] {
            s.create(name);
        }
        s.run(ContextCommand::PreviousContext);
        assert_eq!(1, count(&published));

        s.run(ContextCommand::SwitchContext(ContextRef::Id(c)));
        assert_eq!(2, count(&published));
        assert_eq!(ContextKey::Named(c), last(&published).active);
        s.run(ContextCommand::PreviousContext);
        assert_eq!(3, count(&published));
        assert_eq!(ContextKey::Everything, last(&published).active);
    }

    /// R16, R19. Switching to the active context again is a use of it, so
    /// the snapshot with its new use number is published.
    #[test]
    fn switching_to_the_active_context_again_publishes_its_new_use() {
        let mut s = Setup::new(2);
        s.create("Work");
        let work = s.id("Work");
        let published = capture(&mut s.reactor);
        s.reactor.handle_event(Event::StartupComplete);
        let used = last(&published).contexts[0].last_used;

        s.run(ContextCommand::SwitchContext(ContextRef::Id(work)));

        assert_eq!(2, count(&published));
        assert_eq!(used + 1, last(&published).contexts[0].last_used);
    }

    /// R28, I1. With contexts turned off, the snapshot is the one of contexts
    /// that are off, and nothing else is published until they are on again.
    /// The server then refuses every request. Turned on again, the contexts
    /// are published as they were.
    #[test]
    fn with_contexts_off_only_the_off_snapshot_is_published() {
        let mut s = Setup::new(2);
        s.create("Work");
        let work = s.id("Work");
        let published = capture(&mut s.reactor);
        s.reactor.handle_event(Event::StartupComplete);
        let on = last(&published);

        s.reactor.handle_event(Event::ConfigChanged(config(false)));
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(2, count(&published));
        let off = last(&published);
        assert_eq!(ContextsSnapshot::off(), off);
        s.run(ContextCommand::CreateContext("Other".into()));
        s.run(ContextCommand::SwitchContext(ContextRef::Id(work)));
        s.run(ContextCommand::ShowEverything);
        s.reactor.handle_event(Event::MouseUp);
        assert_eq!(2, count(&published));
        for request in [
            ContextRequest::List,
            ContextRequest::Current,
            ContextRequest::Run(ContextCommand::ShowEverything),
            ContextRequest::Run(ContextCommand::CreateContext("Other".into())),
        ] {
            let (reply, sent) = answer_context_request(request, Some(&off));
            assert_eq!(None, sent);
            assert!(
                matches!(&reply, Response::Error(reason) if reason.starts_with("Contexts are off.")),
                "{reply:?}"
            );
        }

        s.reactor.handle_event(Event::ConfigChanged(config(true)));
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(3, count(&published));
        assert_eq!(on, last(&published));
    }

    /// R32. While a quit waits for parked windows to come back, each screen
    /// shows Everything, and the active context stays.
    #[test]
    fn a_quit_that_waits_publishes_everything_on_each_screen() {
        let mut s = Setup::new(2);
        let c = s.reactor.contexts.create("C").unwrap();
        add(&mut s, c, &[1]);
        s.run(ContextCommand::SwitchContext(ContextRef::Id(c)));
        assert_eq!(vec![wid(2)], s.parked());
        let published = capture(&mut s.reactor);

        s.reactor
            .handle_event(Event::Command(Command::Reactor(ReactorCommand::SaveAndExit)));

        let quitting = last(&published);
        assert_eq!(ContextKey::Named(c), quitting.active);
        assert_eq!(shows(ContextKey::Everything), quitting.screens);
    }
}
