// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tests that replay a recording made with the parked-window journal and the
//! contexts, as `devtool replay` does, and compare the frames the replay
//! writes with the frames the recorded run wrote.

use std::path::Path;

use test_log::test;

use super::*;
use crate::actor::reactor::testing::WindowState;
use crate::actor::reactor::{Record, replay};
use crate::model::contexts::Contexts;

/// A reactor that records to `trace.ron` in `dir`, with contexts on or off,
/// that opens its journal and `contexts.json` in `dir` as a launch does, and
/// records the state it read at launch. Its screen shows Space 1.
fn recording(dir: &TempDir, contexts: bool) -> Reactor {
    let (group_indicators_tx, _) = crate::actor::channel();
    let mut reactor = Reactor::new(
        config(contexts),
        LayoutManager::new_for_test(),
        Record::new(Some(&dir.path().join("trace.ron"))),
        group_indicators_tx,
        ParkedJournal::open(dir.path().join("parked.json"), SystemTime::now()),
    );
    reactor.process_lookup = Box::new(test_app_process);
    reactor.open_contexts(
        ContextsStore::new(dir.path().join("contexts.json")),
        Some("boot".into()),
        SystemTime::now(),
    );
    reactor.record_launch_state();
    reactor.handle_event(screens(vec![screen()], vec![Some(space())]));
    reactor
}

/// Answers the requests until the apps are quiet, and keeps each frame
/// write in `writes`.
fn settle(s: &mut Setup, writes: &mut Vec<(WindowId, CGRect)>) {
    loop {
        let requests = s.apps.requests();
        if requests.is_empty() {
            return;
        }
        for request in &requests {
            if let Request::SetWindowFrame(wid, frame, _) = request {
                writes.push((*wid, *frame));
            }
        }
        answer(s, requests);
    }
}

/// The frame writes of a replay of the recording at `path`. The replay hands
/// its requests over on a thread of its own, so they are taken until none
/// comes for a second.
fn replayed_writes(path: &Path) -> Vec<(WindowId, CGRect)> {
    let (tx, rx) = std::sync::mpsc::channel();
    replay(path, move |_, request| _ = tx.send(request)).unwrap();
    let mut writes = vec![];
    while let Ok(request) = rx.recv_timeout(Duration::from_secs(1)) {
        if let Request::SetWindowFrame(wid, frame, _) = request {
            writes.push((wid, frame));
        }
    }
    writes
}

/// A member record of window `idx` of app `pid`.
fn desc(pid: i32, idx: u32, title: &str, wsid: u32) -> WindowDesc {
    WindowDesc {
        wid: WindowId::new(pid, idx),
        bundle_id: Some(format!("com.testapp{pid}")),
        app_name: Some(format!("TestApp{pid}")),
        title: title.into(),
        window_server_id: Some(WindowServerId::new(wsid)),
    }
}

/// Saves `contexts.json` in `dir` as the boot "boot" leaves it.
fn save_contexts(dir: &TempDir, contexts: &Contexts) {
    ContextsStore::new(dir.path().join("contexts.json"))
        .save(contexts, Some("boot"))
        .unwrap();
}

/// Registers app `pid` with `windows`, and lets the window server list them
/// next to the windows in `listed`.
fn launch_listed(s: &mut Setup, pid: i32, windows: Vec<WindowInfo>, listed: &[WindowId]) {
    let mut launch = s.apps.make_app(pid, windows);
    for event in &mut launch {
        if let Event::WindowsOnScreenUpdated { on_screen, .. } = event {
            for (at, &wid) in listed.iter().enumerate() {
                let info = on_screen_info(s, wid);
                on_screen.visible.insert(at, info.id);
                on_screen.info.insert(at, info);
            }
        }
    }
    s.reactor.handle_events(launch);
}

/// Journal and state files, R20, R21, R22, R24, R37. A run with contexts
/// read at launch: windows rejoin by their window server ids, a title
/// changes, a new window joins C, another app's window rejoins D, a window
/// moves to D, focus from outside switches to D, the app quits, and the user
/// switches back to C. A replay of its recording writes the same frames.
#[test]
fn a_replay_of_membership_and_focus_changes_writes_the_frames_of_the_run() {
    let dir = TempDir::new().unwrap();
    let mut contexts = Contexts::new();
    let c = contexts.create("C").unwrap();
    let d = contexts.create("D").unwrap();
    contexts.add_window(c, &desc(1, 1, "Window1", 1)).unwrap();
    contexts.add_window(d, &desc(1, 2, "Window2", 2)).unwrap();
    contexts.add_window(d, &desc(2, 1, "Doc", 21)).unwrap();
    contexts.switch_to(ContextKey::Named(c)).unwrap();
    save_contexts(&dir, &contexts);
    let reactor = recording(&dir, true);
    let mut s = Setup {
        reactor,
        apps: Apps::new(),
        dir,
    };
    let (c, d) = (ContextKey::Named(c), ContextKey::Named(d));
    let mut writes = vec![];

    s.reactor.handle_events(s.apps.make_app(1, make_windows(3)));
    s.reactor.handle_event(Event::StartupComplete);
    settle(&mut s, &mut writes);
    assert_eq!(vec![wid(2), wid(3)], s.parked());

    s.reactor
        .handle_event(Event::WindowTitleChanged(wid(1), "Renamed".to_string().into()));
    let info = WindowInfo {
        frame: rect(700., 100., 50., 50.),
        ..make_window(4)
    };
    s.apps.windows.insert(
        wid(4),
        WindowState {
            frame: info.frame,
            ..Default::default()
        },
    );
    s.reactor.handle_event(Event::WindowCreated(wid(4), info, MouseState::Up));
    let on_screen = on_screen(&s, &[wid(1), wid(2), wid(3), wid(4)]);
    s.reactor
        .handle_event(Event::WindowsOnScreenUpdated { pid: Some(1), on_screen });
    s.reactor.handle_event(Event::WindowBecameVisible(wid(4)));
    settle(&mut s, &mut writes);
    assert!(s.reactor.contexts.is_member(c, wid(4)));

    let doc = WindowId::new(2, 1);
    let window = WindowInfo {
        title: "Doc".to_string().into(),
        sys_id: Some(WindowServerId::new(21)),
        frame: rect(900., 100., 50., 50.),
        ..make_window(1)
    };
    launch_listed(&mut s, 2, vec![window], &[wid(1), wid(2), wid(3), wid(4)]);
    settle(&mut s, &mut writes);
    assert!(s.reactor.contexts.is_member(d, doc));

    s.reactor.handle_event(Event::ApplicationGloballyActivated(1));
    s.reactor.handle_event(Event::ApplicationActivated(1, Quiet::Yes));
    s.reactor
        .handle_event(Event::ApplicationMainWindowChanged(1, Some(wid(4)), Quiet::Yes));
    s.reactor.handle_event(Event::Command(Command::Context(
        ContextCommand::MoveWindowToContext(ContextRef::Name("D".into())),
    )));
    settle(&mut s, &mut writes);
    // The focusing raise goes out, and its own end follows.
    let sequence_id = s.reactor.raise_sequence;
    s.reactor.handle_event(Event::RaiseFocusSent { sequence_id });
    s.reactor.handle_event(Event::RaiseTimeout { sequence_id });

    s.reactor.handle_event(Event::ApplicationGloballyActivated(2));
    s.reactor
        .handle_event(Event::ApplicationMainWindowChanged(2, Some(doc), Quiet::No));
    s.reactor.handle_event(Event::ApplicationActivated(2, Quiet::No));
    assert_eq!(d, s.reactor.contexts.active());
    settle(&mut s, &mut writes);
    let sequence_id = s.reactor.raise_sequence;
    s.reactor.handle_event(Event::RaiseFocusSent { sequence_id });
    s.reactor.handle_event(Event::RaiseTimeout { sequence_id });

    s.apps.windows.remove(&doc);
    s.reactor.handle_event(Event::WindowDestroyed(doc));
    s.reactor.handle_event(Event::ApplicationTerminated(2));
    s.reactor.handle_event(Event::ApplicationThreadTerminated(2));
    settle(&mut s, &mut writes);
    s.command(c);
    settle(&mut s, &mut writes);
    assert_eq!(c, s.reactor.contexts.active());
    assert_eq!(vec![wid(2), wid(3), wid(4)], s.parked());

    let Setup { reactor, dir, .. } = s;
    drop(reactor);
    assert_eq!(writes, replayed_writes(&dir.path().join("trace.ron")));
}

/// Journal and state files, R34. Window 1 of an app was parked before the
/// launch. The app registers after startup completes, while the process
/// lookup says it runs, and its window is put back and tiled in C. A replay
/// of the recording writes the same frames, whichever processes run on the
/// Mac that replays it. The app's pid runs on no Mac.
#[test]
fn a_replay_puts_back_a_window_whose_app_registers_after_startup_completes() {
    const PID: i32 = 2_000_000_000;
    let dir = TempDir::new().unwrap();
    let before = rect(700., 100., 300., 300.);
    let mut journal = ParkedJournal::open(dir.path().join("parked.json"), SystemTime::now());
    journal
        .record(vec![JournalEntry {
            pid: PID,
            bundle_id: Some(format!("com.testapp{PID}")),
            window_server_id: WindowServerId::new(71),
            title: "Window1".into(),
            frame: before.into(),
        }])
        .unwrap();
    let mut contexts = Contexts::new();
    let c = contexts.create("C").unwrap();
    contexts.add_window(c, &desc(1, 1, "Window1", 1)).unwrap();
    contexts.add_window(c, &desc(PID, 1, "Window1", 71)).unwrap();
    contexts.switch_to(ContextKey::Named(c)).unwrap();
    save_contexts(&dir, &contexts);
    let reactor = recording(&dir, true);
    let mut s = Setup {
        reactor,
        apps: Apps::new(),
        dir,
    };
    let mut writes = vec![];
    s.reactor.handle_events(s.apps.make_app(1, make_windows(1)));
    s.reactor.handle_event(Event::StartupComplete);
    settle(&mut s, &mut writes);

    let late = WindowId::new(PID, 1);
    let window = WindowInfo {
        sys_id: Some(WindowServerId::new(71)),
        frame: corner(before.size),
        ..make_window(1)
    };
    launch_listed(&mut s, PID, vec![window], &[wid(1)]);
    settle(&mut s, &mut writes);
    assert!(writes.contains(&(late, before)));
    assert_eq!(halves_of(wid(1), late), s.tiles());

    let Setup { reactor, dir, .. } = s;
    drop(reactor);
    assert_eq!(writes, replayed_writes(&dir.path().join("trace.ron")));
}

/// Journal and state files, R21, R28. Contexts are off at launch, so the
/// launch reads no `contexts.json`. A config reload turns them on, the file
/// is read then, and the window that isn't in the active context is parked.
/// A replay of the recording writes the same frames.
#[test]
fn a_replay_of_a_run_that_turns_contexts_on_writes_the_frames_of_the_run() {
    let dir = TempDir::new().unwrap();
    let mut contexts = Contexts::new();
    let c = contexts.create("C").unwrap();
    contexts.add_window(c, &desc(1, 1, "Window1", 1)).unwrap();
    contexts.switch_to(ContextKey::Named(c)).unwrap();
    save_contexts(&dir, &contexts);
    let reactor = recording(&dir, false);
    let mut s = Setup {
        reactor,
        apps: Apps::new(),
        dir,
    };
    let mut writes = vec![];
    s.reactor.handle_events(s.apps.make_app(1, make_windows(2)));
    s.reactor.handle_event(Event::StartupComplete);
    settle(&mut s, &mut writes);

    s.reactor.handle_event(Event::ConfigChanged(config(true)));
    settle(&mut s, &mut writes);
    assert_eq!(ContextKey::Named(c), s.reactor.contexts.active());
    assert_eq!(vec![wid(2)], s.parked());
    assert!(writes.contains(&(wid(2), corner(CGSize::new(600., 1000.)))));

    let Setup { reactor, dir, .. } = s;
    drop(reactor);
    assert_eq!(writes, replayed_writes(&dir.path().join("trace.ron")));
}

/// The halves of the screen for two windows, `left` on the left, sorted by
/// window.
fn halves_of(left: WindowId, right: WindowId) -> Vec<(WindowId, CGRect)> {
    let mut tiles = vec![
        (left, rect(0., 0., 600., 1000.)),
        (right, rect(600., 0., 600., 1000.)),
    ];
    tiles.sort_by_key(|(wid, _)| *wid);
    tiles
}
