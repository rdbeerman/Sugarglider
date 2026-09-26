// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Parking hides a window by moving it into a corner of its screen, so that
//! only 1 point of it stays on screen.

use std::io;

use objc2_core_foundation::CGRect;
use tracing::{debug, info};

use super::Reactor;
use super::animation::Animation;
use crate::actor::app::{WindowId, pid_t};
use crate::actor::parked_journal::JournalEntry;
use crate::model::parking_origin;
use crate::sys::window_server::WindowServerId;

/// How far, in points, the frame a window reports may be from the frame
/// written to put it back, for the window to count as back.
const BACK_TOLERANCE: f64 = 16.0;

fn is_back(reported: CGRect, target: CGRect) -> bool {
    [
        reported.origin.x - target.origin.x,
        reported.origin.y - target.origin.y,
        reported.size.width - target.size.width,
        reported.size.height - target.size.height,
    ]
    .iter()
    .all(|difference| difference.abs() <= BACK_TOLERANCE)
}

impl Reactor {
    /// Parks each window in a corner of the screen it is on.
    ///
    /// The journal entries of all the windows are written first. If that
    /// write fails, no window is parked. Windows that are already parked, that
    /// have no window server id, or that are on no screen are left where they
    /// are.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only tests park windows until context switching")
    )]
    pub(super) fn park_windows(&mut self, wids: &[WindowId]) -> io::Result<()> {
        let mut entries = vec![];
        let mut parking = vec![];
        for &wid in wids {
            if self.parked.contains_key(&wid) {
                continue;
            }
            let Some(window) = self.windows.get(&wid) else { continue };
            let Some(app) = self.apps.get(&wid.pid) else { continue };
            let Some(wsid) = window.window_server_id else {
                debug!(?wid, "Not parking a window without a window server id");
                continue;
            };
            let frame = window.frame_monotonic;
            let Some(parked_frame) = self.parked_frame(frame) else {
                debug!(?wid, ?frame, "Not parking a window that is on no screen");
                continue;
            };
            entries.push(JournalEntry {
                pid: wid.pid,
                bundle_id: app.info.bundle_id.clone(),
                window_server_id: wsid,
                title: window.title.expose_secret().clone(),
                frame: frame.into(),
            });
            parking.push((wid, frame, parked_frame));
        }
        if parking.is_empty() {
            return Ok(());
        }
        self.journal.record(entries)?;
        let mut writes = vec![];
        for (wid, frame, parked_frame) in parking {
            self.parked.insert(wid, frame);
            writes.push((wid, parked_frame));
        }
        self.write_frames_now(&writes);
        Ok(())
    }

    /// Puts parked windows back at the frames they had before they were
    /// parked. Their journal entries stay until the windows report those
    /// frames.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "only tests park windows until context switching")
    )]
    pub(super) fn unpark_windows(&mut self, wids: &[WindowId]) {
        for wid in wids {
            let Some(frame) = self.parked.remove(wid) else { continue };
            self.frame_attempts.remove(wid);
            self.pending_frame_overrides.insert(*wid, frame);
        }
        self.update_layout(&[], true);
    }

    /// The frame that parks a window now at `frame`. It keeps 1 point in a
    /// corner of the visible frame of the window's screen, and the corner is
    /// chosen to stay clear of the full bounds of the other displays.
    fn parked_frame(&self, frame: CGRect) -> Option<CGRect> {
        let screen = self.best_screen_idx_for_window(&frame)?;
        let others: Vec<CGRect> = self
            .screens
            .iter()
            .enumerate()
            .filter(|&(idx, _)| idx != screen)
            .map(|(_, other)| other.bounds)
            .collect();
        let origin = parking_origin(frame.size, self.screens[screen].frame, &others);
        Some(CGRect { origin, size: frame.size })
    }

    /// Writes the frames at once, without animation and outside the layout.
    ///
    /// Each write takes a new transaction id and becomes the window's known
    /// frame. It also resets the window's count of repeated writes.
    fn write_frames_now(&mut self, frames: &[(WindowId, CGRect)]) {
        let mut anim = Animation::new();
        for &(wid, frame) in frames {
            let Some(window) = self.windows.get_mut(&wid) else {
                continue;
            };
            let Some(app) = self.apps.get(&wid.pid) else { continue };
            self.frame_attempts.remove(&wid);
            self.pending_frame_overrides.remove(&wid);
            let txid = window.next_txid();
            anim.add_window(&app.handle, wid, window.frame_monotonic, frame, false, txid);
            window.frame_monotonic = frame;
        }
        self.send_animation(anim, true);
    }

    /// Handles the echo of a frame write. If the window is back from parking
    /// and the echo is within 16 points of the frame written, the window's
    /// journal entry goes.
    ///
    /// The caller has checked that the echo belongs to the last write, whose
    /// target is the window's `frame_monotonic`.
    pub(super) fn confirm_unparked(&mut self, wid: WindowId, reported: CGRect) {
        if self.parked.contains_key(&wid) {
            return;
        }
        let Some(window) = self.windows.get(&wid) else { return };
        let Some(wsid) = window.window_server_id else { return };
        if self.journal.get(wid.pid, wsid).is_none() {
            return;
        }
        let target = window.frame_monotonic;
        if !is_back(reported, target) {
            debug!(?wid, ?reported, ?target, "Window is not back from parking yet");
            return;
        }
        self.journal.remove_window(wid.pid, wsid);
    }

    /// Forgets a destroyed window's parking state and journal entry.
    pub(super) fn forget_parked_window(&mut self, wid: WindowId, wsid: Option<WindowServerId>) {
        self.parked.remove(&wid);
        if let Some(wsid) = wsid {
            self.journal.remove_window(wid.pid, wsid);
        }
    }

    /// Forgets the parking state and journal entries of an app that is gone.
    pub(super) fn forget_parked_app(&mut self, pid: pid_t) {
        self.parked.retain(|wid, _| wid.pid != pid);
        self.journal.remove_app(pid);
    }

    /// Puts the app's windows that the journal listed at startup back at their
    /// frames from before parking. Each entry is put back once, and it stays
    /// in the journal until the window reports the frame.
    pub(super) fn restore_from_journal(&mut self, pid: pid_t) {
        let entries = self.journal.unrestored(pid);
        if entries.is_empty() {
            return;
        }
        let Some(app) = self.apps.get(&pid) else { return };
        let bundle_id = app.info.bundle_id.clone();
        let mut writes = vec![];
        for entry in entries {
            if entry.bundle_id.is_some() && bundle_id.is_some() && entry.bundle_id != bundle_id {
                info!(
                    pid,
                    journal = ?entry.bundle_id,
                    running = ?bundle_id,
                    "Dropping a journal entry whose pid belongs to another app now"
                );
                self.journal.remove_window(pid, entry.window_server_id);
                continue;
            }
            let Some(&wid) = self.windows.iter().find_map(|(wid, window)| {
                (wid.pid == pid && window.window_server_id == Some(entry.window_server_id))
                    .then_some(wid)
            }) else {
                continue;
            };
            self.journal.mark_restored(pid, entry.window_server_id);
            writes.push((wid, entry.frame.into()));
        }
        if !writes.is_empty() {
            info!(
                pid,
                count = writes.len(),
                "Putting back windows parked before a restart"
            );
            self.write_frames_now(&writes);
        }
    }

    /// Drops the journal entries of apps that are not running.
    pub(super) fn drop_journal_entries_of_absent_apps(&mut self) {
        let apps = &self.apps;
        self.journal.retain_apps(|pid| apps.contains_key(&pid));
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{Instant, SystemTime};

    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use tempfile::TempDir;
    use test_log::test;

    use super::super::testing::*;
    use super::super::{Event, FrameAttempt, MAX_FRAME_ATTEMPTS, Reactor, Requested};
    use crate::actor::app::{Request, WindowId};
    use crate::actor::layout::LayoutManager;
    use crate::actor::parked_journal::{JournalEntry, ParkedJournal};
    use crate::sys::app::WindowInfo;
    use crate::sys::screen::{CoordinateConverter, SpaceId};
    use crate::sys::window_server::WindowServerId;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
        CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))
    }

    fn screen() -> CGRect {
        rect(0., 0., 1000., 1000.)
    }

    fn space() -> SpaceId {
        SpaceId::new(1)
    }

    fn wid(idx: u32) -> WindowId {
        WindowId::new(1, idx)
    }

    /// A reactor with one app whose windows are tiled on one screen, and a
    /// journal in a temporary directory.
    struct Setup {
        reactor: Reactor,
        apps: Apps,
        dir: TempDir,
    }

    impl Setup {
        fn new(windows: usize) -> Setup {
            let mut s = Setup::launching(vec![]);
            s.reactor.handle_events(s.apps.make_app(1, make_windows(windows)));
            s.reactor.handle_event(Event::StartupComplete);
            s.apps.simulate_until_quiet(&mut s.reactor);
            s
        }

        /// A reactor on one screen that no app has reached yet, starting with
        /// the journal that an earlier run left with `entries`.
        fn launching(entries: Vec<JournalEntry>) -> Setup {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join("parked.json");
            if !entries.is_empty() {
                ParkedJournal::open(path.clone(), SystemTime::now()).record(entries).unwrap();
            }
            let mut reactor = Reactor::new_for_test(LayoutManager::new_for_test());
            reactor.journal = ParkedJournal::open(path, SystemTime::now());
            reactor.handle_event(Event::ScreenParametersChanged {
                frames: vec![screen()],
                bounds: vec![screen()],
                spaces: vec![Some(space())],
                scale_factors: vec![1.0],
                converter: CoordinateConverter::default(),
                on_screen: Default::default(),
            });
            Setup {
                reactor,
                apps: Apps::new(),
                dir,
            }
        }

        fn journal_path(&self) -> PathBuf {
            self.dir.path().join("parked.json")
        }

        /// The entries in the journal file.
        fn journal_on_disk(&self) -> Vec<JournalEntry> {
            ParkedJournal::open(self.journal_path(), SystemTime::now()).entries().to_vec()
        }

        fn frame(&self, wid: WindowId) -> CGRect {
            self.apps.windows[&wid].frame
        }

        fn tiles(&self) -> Vec<(WindowId, CGRect)> {
            let mut tiles =
                self.reactor.layout.calculate_layout(space(), screen(), &self.reactor.config);
            tiles.sort_by_key(|(wid, _)| *wid);
            tiles
        }

        /// Asks every app for its windows, which sends each app's visible
        /// windows to the layout again.
        fn refresh_visible_windows(&mut self) {
            self.reactor.update_visible_windows();
            self.apps.simulate_until_quiet(&mut self.reactor);
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

    #[test]
    fn h1_parks_in_a_corner_clear_of_the_other_displays_full_bounds() {
        let mut apps = Apps::new();
        let mut reactor = Reactor::new_for_test(LayoutManager::new_for_test());
        // A display above the main display. Both have a 25-point menu bar.
        let main_visible = rect(0., 25., 1920., 1055.);
        let main_bounds = rect(0., 0., 1920., 1080.);
        let above_visible = rect(0., -1055., 1920., 1055.);
        let above_bounds = rect(0., -1080., 1920., 1080.);
        reactor.handle_event(Event::ScreenParametersChanged {
            frames: vec![main_visible, above_visible],
            bounds: vec![main_bounds, above_bounds],
            spaces: vec![Some(space()), None],
            scale_factors: vec![1.0, 1.0],
            converter: CoordinateConverter::default(),
            on_screen: Default::default(),
        });
        let window = WindowInfo {
            frame: rect(100., -1000., 400., 20.),
            ..make_window(1)
        };
        reactor.handle_events(apps.make_app(1, vec![window]));
        reactor.handle_event(Event::StartupComplete);
        apps.simulate_until_quiet(&mut reactor);

        reactor.park_windows(&[wid(1)]).unwrap();

        // A bottom corner would reach into the main display's menu bar, which
        // its visible frame leaves out.
        assert_eq!(
            vec![rect(1919., -1074., 400., 20.)],
            frame_writes(&apps.requests(), wid(1))
        );
    }

    #[test]
    fn r30_the_journal_holds_the_frame_from_before_parking() {
        let mut s = Setup::new(2);
        let tile = s.frame(wid(1));

        s.reactor.park_windows(&[wid(1)]).unwrap();

        assert_eq!(
            vec![JournalEntry {
                pid: 1,
                bundle_id: Some("com.testapp1".into()),
                window_server_id: WindowServerId::new(1),
                title: "Window1".into(),
                frame: tile.into(),
            }],
            s.journal_on_disk()
        );
        let parked = rect(999., 999., tile.size.width, tile.size.height);
        assert_eq!(vec![parked], frame_writes(&s.apps.requests(), wid(1)));
    }

    #[test]
    fn r30_a_failed_journal_write_parks_nothing() {
        let mut s = Setup::new(2);
        let txid = s.reactor.windows[&wid(1)].last_sent_txid;
        let frame = s.reactor.windows[&wid(1)].frame_monotonic;

        fs::set_permissions(s.dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
        let result = s.reactor.park_windows(&[wid(1), wid(2)]);
        fs::set_permissions(s.dir.path(), fs::Permissions::from_mode(0o755)).unwrap();

        assert!(result.is_err());
        assert!(s.apps.requests().is_empty());
        assert!(s.reactor.parked.is_empty());
        assert!(s.reactor.journal.entries().is_empty());
        assert_eq!(txid, s.reactor.windows[&wid(1)].last_sent_txid);
        assert_eq!(frame, s.reactor.windows[&wid(1)].frame_monotonic);
        assert!(!s.journal_path().exists());
    }

    #[test]
    fn h2_a_parked_window_keeps_its_tile_and_the_others_keep_their_frames() {
        let mut s = Setup::new(3);
        let tiles = s.tiles();
        let before = [wid(1), wid(3)].map(|wid| s.frame(wid));

        s.reactor.park_windows(&[wid(2)]).unwrap();
        s.apps.simulate_until_quiet(&mut s.reactor);
        s.refresh_visible_windows();

        assert_eq!(tiles, s.tiles());
        assert_eq!(before, [wid(1), wid(3)].map(|wid| s.frame(wid)));
        let tile = tiles[1].1;
        assert_eq!(
            rect(999., 999., tile.size.width, tile.size.height),
            s.frame(wid(2))
        );
    }

    #[test]
    fn r31_unparking_restores_the_exact_frame_and_the_echo_removes_the_entry() {
        let mut s = Setup::new(2);
        let tile = s.frame(wid(1));
        s.reactor.park_windows(&[wid(1)]).unwrap();
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(1, s.journal_on_disk().len(), "the park echo keeps the entry");

        s.reactor.unpark_windows(&[wid(1)]);
        let requests = s.apps.requests();
        assert_eq!(vec![tile], frame_writes(&requests, wid(1)));
        assert_eq!(1, s.journal_on_disk().len(), "the entry waits for the echo");

        for event in s.apps.simulate_events_for_requests(requests) {
            s.reactor.handle_event(event);
        }
        assert_eq!(tile, s.frame(wid(1)));
        assert!(s.journal_on_disk().is_empty());
        assert!(s.reactor.journal.entries().is_empty());
    }

    #[test]
    fn r31_the_entry_stays_while_the_window_is_more_than_16_points_off() {
        let mut s = Setup::new(2);
        s.reactor.park_windows(&[wid(1)]).unwrap();
        s.apps.simulate_until_quiet(&mut s.reactor);

        s.reactor.unpark_windows(&[wid(1)]);
        let Some(Request::SetWindowFrame(_, target, txid)) =
            s.apps.requests().into_iter().find(|request| {
                matches!(request, Request::SetWindowFrame(request_wid, ..) if *request_wid == wid(1))
            })
        else {
            panic!("no unpark write");
        };
        let echo = |dx: f64| {
            Event::WindowFrameChanged(
                wid(1),
                rect(
                    target.origin.x + dx,
                    target.origin.y,
                    target.size.width,
                    target.size.height,
                ),
                txid,
                Requested(true),
                None,
            )
        };

        s.reactor.handle_event(echo(20.));
        assert_eq!(1, s.journal_on_disk().len());
        s.reactor.handle_event(echo(16.));
        assert!(s.journal_on_disk().is_empty());
    }

    #[test]
    fn r31_the_entry_goes_when_the_window_is_destroyed() {
        let mut s = Setup::new(2);
        s.reactor.park_windows(&[wid(1), wid(2)]).unwrap();
        s.apps.simulate_until_quiet(&mut s.reactor);

        s.reactor.handle_event(Event::WindowDestroyed(wid(1)));

        assert_eq!(
            vec![WindowServerId::new(2)],
            s.journal_on_disk()
                .iter()
                .map(|entry| entry.window_server_id)
                .collect::<Vec<_>>()
        );
        assert!(!s.reactor.parked.contains_key(&wid(1)));
    }

    #[test]
    fn r31_the_entries_go_when_the_app_thread_terminates() {
        let mut s = Setup::new(2);
        s.reactor.park_windows(&[wid(1), wid(2)]).unwrap();
        s.apps.simulate_until_quiet(&mut s.reactor);

        s.reactor.handle_event(Event::ApplicationThreadTerminated(1));

        assert!(s.journal_on_disk().is_empty());
        assert!(s.reactor.parked.is_empty());
    }

    #[test]
    fn h5_parked_windows_of_one_app_with_one_size_are_not_tabs() {
        let mut s = Setup::new(2);
        let tiles = s.tiles();
        assert_eq!(tiles[0].1.size, tiles[1].1.size);

        s.reactor.park_windows(&[wid(1), wid(2)]).unwrap();
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(s.frame(wid(1)), s.frame(wid(2)), "both share one corner");
        s.refresh_visible_windows();
        assert_eq!(tiles, s.tiles(), "both keep their tiles");

        s.reactor.handle_event(Event::WindowDestroyed(wid(1)));
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(
            vec![wid(2)],
            s.tiles().into_iter().map(|(wid, _)| wid).collect::<Vec<_>>(),
            "the closed window leaves no empty tile"
        );
        assert_eq!(1, s.journal_on_disk().len());
    }

    #[test]
    fn h4_park_and_unpark_writes_take_new_transaction_ids_and_reset_frame_attempts() {
        let mut s = Setup::new(2);
        let tile = s.frame(wid(1));
        let gave_up = || FrameAttempt {
            target: tile,
            count: MAX_FRAME_ATTEMPTS + 1,
            last: Instant::now(),
        };
        s.reactor.frame_attempts.insert(wid(1), gave_up());
        let txid = s.reactor.windows[&wid(1)].last_sent_txid;

        s.reactor.park_windows(&[wid(1)]).unwrap();
        let parked = rect(999., 999., tile.size.width, tile.size.height);
        let window = &s.reactor.windows[&wid(1)];
        assert_eq!(txid.0 + 1, window.last_sent_txid.0);
        assert_eq!(parked, window.frame_monotonic);
        assert!(!s.reactor.frame_attempts.contains_key(&wid(1)));
        s.apps.simulate_until_quiet(&mut s.reactor);

        s.reactor.frame_attempts.insert(wid(1), gave_up());
        s.reactor.unpark_windows(&[wid(1)]);
        let window = &s.reactor.windows[&wid(1)];
        assert_eq!(txid.0 + 2, window.last_sent_txid.0);
        assert_eq!(tile, window.frame_monotonic);
        assert_eq!(1, s.reactor.frame_attempts[&wid(1)].count);
        assert_eq!(vec![tile], frame_writes(&s.apps.requests(), wid(1)));
    }

    #[test]
    fn h4_parking_and_unparking_ten_times_quickly_ends_at_the_right_frames() {
        let mut s = Setup::new(2);
        let tiles = [wid(1), wid(2)].map(|wid| s.frame(wid));

        for _ in 0..10 {
            s.reactor.park_windows(&[wid(1), wid(2)]).unwrap();
            s.reactor.unpark_windows(&[wid(1), wid(2)]);
        }
        let requests = s.apps.requests();
        assert_eq!(20, frame_writes(&requests, wid(1)).len());
        assert_eq!(20, frame_writes(&requests, wid(2)).len());
        for event in s.apps.simulate_events_for_requests(requests) {
            s.reactor.handle_event(event);
        }
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(tiles, [wid(1), wid(2)].map(|wid| s.frame(wid)));
        assert!(s.journal_on_disk().is_empty());

        for _ in 0..10 {
            s.reactor.park_windows(&[wid(1), wid(2)]).unwrap();
            s.apps.simulate_until_quiet(&mut s.reactor);
            s.reactor.unpark_windows(&[wid(1), wid(2)]);
            s.apps.simulate_until_quiet(&mut s.reactor);
        }
        assert_eq!(tiles, [wid(1), wid(2)].map(|wid| s.frame(wid)));
        assert!(s.journal_on_disk().is_empty());
        assert!(s.reactor.parked.is_empty());
    }

    fn entry(pid: i32, wsid: u32, frame: CGRect) -> JournalEntry {
        JournalEntry {
            pid,
            bundle_id: Some(format!("com.testapp{pid}")),
            window_server_id: WindowServerId::new(wsid),
            title: format!("Window{wsid}"),
            frame: frame.into(),
        }
    }

    fn window_at(sys_id: u32, frame: CGRect) -> WindowInfo {
        WindowInfo {
            frame,
            sys_id: Some(WindowServerId::new(sys_id)),
            ..make_window(1)
        }
    }

    fn journal_wsids(entries: Vec<JournalEntry>) -> Vec<u32> {
        entries.iter().map(|entry| entry.window_server_id.as_u32()).collect()
    }

    #[test]
    fn a_launch_without_a_journal_writes_none() {
        let s = Setup::new(2);
        assert!(!s.journal_path().exists());
    }

    #[test]
    fn r34_launch_puts_back_each_apps_windows_when_the_app_arrives() {
        let left = rect(0., 0., 500., 1000.);
        let right = rect(500., 0., 500., 1000.);
        let elsewhere = rect(100., 100., 300., 300.);
        let parked = rect(999., 999., 500., 1000.);
        let mut s = Setup::launching(vec![
            entry(1, 11, right),
            entry(1, 12, left),
            entry(2, 21, elsewhere),
            entry(3, 31, left),
        ]);

        s.reactor
            .handle_events(s.apps.make_app(1, vec![window_at(11, parked), window_at(12, parked)]));
        // The windows go back before the layout sees them, so their tiles keep
        // the order they had before parking and need no second write.
        let requests = s.apps.requests();
        assert_eq!(vec![right], frame_writes(&requests, WindowId::new(1, 1)));
        assert_eq!(vec![left], frame_writes(&requests, WindowId::new(1, 2)));
        for event in s.apps.simulate_events_for_requests(requests) {
            s.reactor.handle_event(event);
        }
        assert_eq!(vec![21, 31], journal_wsids(s.journal_on_disk()));

        s.reactor.handle_events(s.apps.make_app(2, vec![window_at(21, parked)]));
        let requests = s.apps.requests();
        assert_eq!(
            Some(&elsewhere),
            frame_writes(&requests, WindowId::new(2, 1)).first()
        );
        for event in s.apps.simulate_events_for_requests(requests) {
            s.reactor.handle_event(event);
        }
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(vec![31], journal_wsids(s.journal_on_disk()));

        s.reactor.handle_event(Event::StartupComplete);
        assert!(s.journal_on_disk().is_empty());
    }

    #[test]
    fn r34_a_window_the_app_reports_later_goes_back_then() {
        let mut s = Setup::launching(vec![entry(1, 11, screen())]);
        s.reactor.handle_events(s.apps.make_app(1, vec![]));
        s.reactor.handle_event(Event::StartupComplete);
        assert!(s.apps.requests().is_empty());

        let wid = WindowId::new(1, 1);
        s.reactor.handle_event(Event::WindowsDiscovered {
            pid: 1,
            new: vec![(wid, window_at(11, rect(999., 999., 1000., 1000.)))],
            known_visible: vec![wid],
        });
        assert_eq!(vec![screen()], frame_writes(&s.apps.requests(), wid));
    }

    #[test]
    fn r34_each_entry_is_put_back_once() {
        let mut s = Setup::launching(vec![entry(1, 11, screen())]);
        let wid = WindowId::new(1, 1);
        s.reactor
            .handle_events(s.apps.make_app(1, vec![window_at(11, rect(999., 999., 1000., 1000.))]));
        assert_eq!(vec![screen()], frame_writes(&s.apps.requests(), wid));

        // The window hasn't reported the frame, so its entry is still there.
        s.reactor.handle_event(Event::WindowsDiscovered {
            pid: 1,
            new: vec![],
            known_visible: vec![wid],
        });
        assert!(frame_writes(&s.apps.requests(), wid).is_empty());
        assert_eq!(vec![11], journal_wsids(s.journal_on_disk()));
    }

    #[test]
    fn r34_an_entry_whose_pid_another_app_has_now_is_dropped() {
        let elsewhere = rect(100., 100., 300., 300.);
        let mut other_app = entry(1, 11, elsewhere);
        other_app.bundle_id = Some("com.example.other".into());
        let mut s = Setup::launching(vec![other_app]);

        s.reactor
            .handle_events(s.apps.make_app(1, vec![window_at(11, rect(999., 999., 1000., 1000.))]));

        let writes = frame_writes(&s.apps.requests(), WindowId::new(1, 1));
        assert!(!writes.contains(&elsewhere), "{writes:?}");
        assert!(s.journal_on_disk().is_empty());
    }
}
