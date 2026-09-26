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
    use super::super::{ContextCommand, ContextRef, Event, Reactor};
    use crate::actor::app::WindowId;
    use crate::actor::contexts_snapshot::{ContextSummary, ContextsSnapshot, ScreenContext};
    use crate::actor::layout::LayoutManager;
    use crate::model::contexts::ContextKey;
    use crate::sys::app::WindowInfo;
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
}
