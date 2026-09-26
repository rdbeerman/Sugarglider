// Copyright The Glide Authors
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Reactor tests for membership: new windows, windows found at launch,
//! closed windows, titles, tabs, and the membership commands.

use test_log::test;

use super::*;
use crate::actor::reactor::testing::WindowState;

/// App `wid.pid` opens a window, which reaches the reactor as a new window
/// does: `WindowCreated`, then the window server's list, which names the
/// windows in `listed` and the new one, then `WindowBecameVisible`.
fn open_window(s: &mut Setup, wid: WindowId, info: WindowInfo, listed: &[WindowId]) {
    s.apps.windows.insert(
        wid,
        WindowState {
            frame: info.frame,
            ..Default::default()
        },
    );
    s.reactor.handle_event(Event::WindowCreated(wid, info, MouseState::Up));
    let mut listed = listed.to_vec();
    listed.push(wid);
    let on_screen = on_screen(s, &listed);
    s.reactor
        .handle_event(Event::WindowsOnScreenUpdated { pid: Some(wid.pid), on_screen });
    s.reactor.handle_event(Event::WindowBecameVisible(wid));
    s.apps.simulate_until_quiet(&mut s.reactor);
}

/// The members that `contexts.json` holds for the context.
fn saved_members(s: &Setup, key: ContextKey) -> Vec<String> {
    match ContextsStore::new(s.dir.path().join("contexts.json")).load(SystemTime::now()) {
        Loaded::Read { contexts, .. } => contexts
            .get(id_of(key))
            .unwrap()
            .members
            .iter()
            .map(|m| m.title.clone())
            .collect(),
        other => panic!("{other:?}"),
    }
}

/// R20, R38, L11. A new window joins the active context. It reaches the
/// layout through `WindowCreated`, the window list, and
/// `WindowBecameVisible`, and gets one node.
#[test]
fn r20_a_new_window_joins_the_active_context_with_one_node() {
    let mut s = Setup::new(2);
    let c = s.create("C", &[wid(1)]);
    s.switch(c);
    assert_eq!(vec![wid(2)], s.parked());

    let info = WindowInfo {
        frame: rect(700., 100., 50., 50.),
        ..make_window(3)
    };
    open_window(&mut s, wid(3), info, &[wid(1), wid(2)]);

    assert!(s.reactor.contexts.is_member(c, wid(3)));
    let tiles = vec![
        (wid(1), rect(0., 0., 600., 1000.)),
        (wid(3), rect(600., 0., 600., 1000.)),
    ];
    assert_eq!(tiles, s.tiles());
    assert_eq!(tiles, s.frames(&[wid(1), wid(3)]));
    assert_eq!(vec![wid(2)], s.parked());
    assert_eq!(vec!["Window1", "Window3"], saved_members(&s, c));

    // The window stays a member when it leaves the screen and comes back.
    report_visible(&mut s, &[wid(1), wid(2)]);
    report_visible(&mut s, &[wid(1), wid(2), wid(3)]);
    assert_eq!(tiles, s.tiles());
    assert_eq!(vec![wid(2)], s.parked());
}

/// R20. Under Everything and under Unsorted, a new window is unsorted, and
/// it shows.
#[test]
fn r20_under_everything_or_unsorted_a_new_window_stays_unsorted() {
    let mut s = Setup::new(1);
    let c = s.create("C", &[]);
    let info = |idx: usize| WindowInfo {
        frame: rect(700., 100., 50., 50.),
        ..make_window(idx)
    };
    open_window(&mut s, wid(2), info(2), &[wid(1)]);
    assert!(s.reactor.contexts.is_unsorted(wid(2)));
    s.switch(ContextKey::Unsorted);
    open_window(&mut s, wid(3), info(3), &[wid(1), wid(2)]);

    assert!(s.reactor.contexts.is_unsorted(wid(3)));
    assert!(s.reactor.contexts.get(id_of(c)).unwrap().members.is_empty());
    assert!(s.parked().is_empty());
    let tiles: Vec<WindowId> = s.tiles().into_iter().map(|(wid, _)| wid).collect();
    assert_eq!(vec![wid(1), wid(2), wid(3)], tiles);
}

/// R38. An unsorted window that comes back from being minimized is not new,
/// so it doesn't join the active context.
#[test]
fn r38_a_window_back_from_being_minimized_is_not_new() {
    let mut s = Setup::new(3);
    report_visible(&mut s, &[wid(1), wid(2)]);
    let c = s.create("C", &[wid(1), wid(2)]);
    s.switch(c);
    assert!(s.parked().is_empty());

    report_visible(&mut s, &[wid(1), wid(2), wid(3)]);

    assert!(s.reactor.contexts.is_unsorted(wid(3)));
    assert_eq!(vec![wid(3)], s.parked());
}

/// R21, R20. An app launches while C is active. Its window 1 matches an
/// empty record of C by title and rejoins C. Its window 2 matches nothing
/// in the same launch and joins C, so nothing parks it.
#[test]
fn r21_r20_a_window_that_rejoins_and_an_unmatched_one_launched_together_both_show() {
    let mut s = Setup::new(2);
    let c = s.create("C", &[wid(1)]);
    let old = WindowId::new(3, 9);
    let desc = WindowDesc {
        wid: old,
        bundle_id: Some("com.testapp3".into()),
        app_name: Some("TestApp3".into()),
        title: "Window1".into(),
        window_server_id: Some(WindowServerId::new(99)),
    };
    s.reactor.contexts.add_window(id_of(c), &desc).unwrap();
    s.reactor.contexts.window_closed(old);
    s.reactor.contexts.app_terminated(3);
    s.switch(c);
    assert_eq!(vec![wid(2)], s.parked());

    let w = |idx: usize, wsid: u32, x: f64| WindowInfo {
        sys_id: Some(WindowServerId::new(wsid)),
        frame: rect(x, 100., 300., 300.),
        ..make_window(idx)
    };
    s.reactor
        .handle_events(s.apps.make_app(3, vec![w(1, 41, 100.), w(2, 42, 500.)]));
    s.apps.simulate_until_quiet(&mut s.reactor);

    let (rejoined, joined) = (WindowId::new(3, 1), WindowId::new(3, 2));
    let members = &s.reactor.contexts.get(id_of(c)).unwrap().members;
    let links: Vec<RecordLink> = members.iter().map(|m| m.link).collect();
    assert_eq!(
        vec![
            RecordLink::Live(wid(1)),
            RecordLink::Live(rejoined),
            RecordLink::Live(joined),
        ],
        links
    );
    assert_eq!(vec![wid(2)], s.parked());
    let tiled: Vec<WindowId> = s.tiles().into_iter().map(|(wid, _)| wid).collect();
    assert_eq!(vec![wid(1), rejoined, joined], tiled);
}

/// R38, R32. A restart while C is active. App 1 holds C's members, windows
/// 1 and 2, and the unsorted window 3. App 2 has one window, which is in no
/// context. Whichever app registers first, the windows that match no
/// record stay unsorted and are parked when startup completes, and C keeps
/// its members.
#[test]
fn r38_after_a_restart_windows_that_match_no_record_stay_unsorted_and_are_parked() {
    for app_2_first in [false, true] {
        let mut s = Setup::on(vec![screen()], vec![Some(space())]);
        let other = WindowId::new(2, 1);
        let other_window = WindowInfo {
            sys_id: Some(WindowServerId::new(30)),
            frame: rect(700., 100., 50., 50.),
            ..make_window(1)
        };
        s.reactor.handle_events(s.apps.make_app(1, make_windows(3)));
        s.reactor.handle_events(s.apps.make_app(2, vec![other_window.clone()]));
        s.reactor.handle_event(Event::WindowsOnScreenUpdated {
            pid: None,
            on_screen: on_screen(&s, &[wid(1), wid(2), wid(3), other]),
        });
        s.reactor.handle_event(Event::StartupComplete);
        s.apps.simulate_until_quiet(&mut s.reactor);
        let c = s.create("C", &[wid(1), wid(2)]);
        s.switch(c);
        assert_eq!(vec![wid(3), other], s.parked());
        let exits = catch_exits(&mut s);
        save_and_exit(&mut s);
        s.apps.simulate_until_quiet(&mut s.reactor);
        assert_eq!(vec![0], *exits.lock().unwrap());
        let all = [wid(1), wid(2), wid(3), other];
        let frames = s.frames(&all);

        let mut reactor = restore(&s, true);
        let mut apps = Apps::new();
        let app1: Vec<WindowInfo> = (1..=3)
            .map(|idx| WindowInfo {
                frame: frames[idx - 1].1,
                ..make_window(idx)
            })
            .collect();
        let app2 = vec![WindowInfo {
            frame: frames[3].1,
            ..other_window.clone()
        }];
        // Each app's window server list names every visible window.
        let listed = WindowsOnScreen::new(
            [(1, 1), (2, 1), (3, 1), (30, 2)]
                .iter()
                .zip(&frames)
                .map(|(&(id, pid), &(_, frame))| WindowServerInfo {
                    id: WindowServerId::new(id),
                    pid,
                    layer: 0,
                    frame,
                })
                .collect(),
        );
        let list_all = |events: Vec<Event>| -> Vec<Event> {
            events
                .into_iter()
                .map(|event| match event {
                    Event::WindowsOnScreenUpdated { pid, .. } => {
                        Event::WindowsOnScreenUpdated { pid, on_screen: listed.clone() }
                    }
                    other => other,
                })
                .collect()
        };
        let launches = [
            list_all(apps.make_app(1, app1)),
            list_all(apps.make_app(2, app2)),
        ];
        let [first, second] = if app_2_first {
            let [one, two] = launches;
            [two, one]
        } else {
            launches
        };
        reactor.handle_events(first);
        reactor.handle_events(second);
        reactor.handle_event(Event::StartupComplete);
        apps.simulate_until_quiet(&mut reactor);

        let mut tiles = reactor.layout.calculate_layout(space(), screen(), &reactor.config);
        tiles.sort_by_key(|(wid, _)| *wid);
        let in_c = vec![
            (wid(1), rect(0., 0., 600., 1000.)),
            (wid(2), rect(600., 0., 600., 1000.)),
        ];
        assert_eq!(in_c, tiles, "app 2 first: {app_2_first}");
        let mut parked: Vec<WindowId> = reactor.parked.keys().copied().collect();
        parked.sort();
        assert_eq!(vec![wid(3), other], parked, "app 2 first: {app_2_first}");
        assert!(reactor.contexts.is_unsorted(other));
        assert!(reactor.contexts.is_unsorted(wid(3)));
        assert_eq!(corner(frames[3].1.size), apps.windows[&other].frame);
    }
}

/// R38, R28. Windows that an app shows while contexts are off rejoin their
/// contexts when contexts are turned on again.
#[test]
fn r38_windows_found_while_contexts_are_off_rejoin_when_they_are_turned_on() {
    let mut s = Setup::new(1);
    let c = s.create("C", &[wid(1)]);
    record_of_app_2(&mut s, c);
    s.switch(c);
    s.reactor.handle_event(Event::ConfigChanged(config(false)));
    s.apps.simulate_until_quiet(&mut s.reactor);
    let window = WindowInfo {
        sys_id: Some(WindowServerId::new(21)),
        frame: rect(700., 100., 50., 50.),
        ..make_window(1)
    };
    s.reactor.handle_events(s.apps.make_app(2, vec![window]));
    let arrived = WindowId::new(2, 1);
    report_visible(&mut s, &[wid(1), arrived]);
    assert!(!s.reactor.contexts.is_member(c, arrived));

    s.reactor.handle_event(Event::ConfigChanged(config(true)));
    s.apps.simulate_until_quiet(&mut s.reactor);

    assert!(s.reactor.contexts.is_member(c, arrived));
    let tiles = vec![
        (wid(1), rect(0., 0., 600., 1000.)),
        (arrived, rect(600., 0., 600., 1000.)),
    ];
    assert_eq!(tiles, s.tiles());
    assert_eq!(tiles, s.frames(&[wid(1), arrived]));
    assert!(s.parked().is_empty());
}

/// The titles of C's records and whether each has an open window.
fn records(s: &Setup, key: ContextKey) -> Vec<(String, RecordLink)> {
    let members = &s.reactor.contexts.get(id_of(key)).unwrap().members;
    members.iter().map(|m| (m.title.clone(), m.link)).collect()
}

/// R23. A closed window's records wait until its app shows it is still
/// running: it creates a window, the user activates it, or a window server
/// list names one of its windows. Then they go, and `contexts.json` is
/// written without them.
#[test]
fn r23_a_closed_windows_records_go_once_its_app_shows_it_still_runs() {
    for signal in ["new window", "activation", "window server list"] {
        let mut s = Setup::new(3);
        let c = s.create("C", &[wid(1), wid(2)]);
        s.switch(c);
        s.close(wid(2));
        assert_eq!(
            vec![
                ("Window1".to_string(), RecordLink::Live(wid(1))),
                ("Window2".to_string(), RecordLink::Pending(wid(2))),
            ],
            records(&s, c),
            "{signal}"
        );

        match signal {
            "new window" => {
                let info = WindowInfo {
                    frame: rect(700., 100., 50., 50.),
                    ..make_window(4)
                };
                open_window(&mut s, wid(4), info, &[wid(1), wid(3)]);
            }
            "activation" => {
                s.reactor.handle_event(Event::ApplicationActivated(1, Quiet::No));
            }
            _ => report_visible(&mut s, &[wid(1), wid(3)]),
        }

        let mut left = vec![("Window1".to_string(), RecordLink::Live(wid(1)))];
        if signal == "new window" {
            left.push(("Window4".to_string(), RecordLink::Live(wid(4))));
        }
        assert_eq!(left, records(&s, c), "{signal}");
        let saved: Vec<String> = left.into_iter().map(|(title, _)| title).collect();
        assert_eq!(saved, saved_members(&s, c), "{signal}");
    }
}

/// R23. An activation that Sugarglider's own raise caused doesn't show that
/// the app is still running.
#[test]
fn r23_a_quiet_activation_keeps_a_closed_windows_records() {
    let mut s = Setup::new(2);
    let c = s.create("C", &[wid(1), wid(2)]);
    s.switch(c);
    s.close(wid(2));

    s.reactor.handle_event(Event::ApplicationActivated(1, Quiet::Yes));

    assert_eq!(
        vec![
            ("Window1".to_string(), RecordLink::Live(wid(1))),
            ("Window2".to_string(), RecordLink::Pending(wid(2))),
        ],
        records(&s, c)
    );
}

/// App 2 with window 1, titled "Doc", on the right half of the screen, and
/// its window server id `wsid`. `pid` stands for a launch of app 2: a
/// relaunch has a new pid and the same bundle id.
fn doc_app(s: &mut Setup, pid: i32, wsid: u32) -> Vec<Event> {
    let window = WindowInfo {
        title: "Doc".to_string().into(),
        sys_id: Some(WindowServerId::new(wsid)),
        frame: rect(700., 100., 50., 50.),
        ..make_window(1)
    };
    let info = test_app_info(2);
    s.apps.make_app_with_info(pid, info, vec![window], None, false)
}

/// R21, R22, R23, L8. App 2 quits while D is active, and its window, a
/// member of C, is parked. When the app runs again with a new pid, its
/// window rejoins C by its title, and not D, and stays parked. Switching to
/// C puts it back and tiles it. This holds whether macOS reports the window
/// destroyed before the app terminates or not at all.
#[test]
fn r21_r22_r23_an_app_that_quits_and_relaunches_rejoins_its_context_by_title() {
    for destroyed_first in [true, false] {
        let mut s = Setup::new(1);
        let events = doc_app(&mut s, 2, 21);
        s.reactor.handle_events(events);
        let doc = WindowId::new(2, 1);
        report_visible(&mut s, &[wid(1), doc]);
        let c = s.create("C", &[wid(1), doc]);
        let d = s.create("D", &[wid(1)]);
        s.switch(c);
        s.switch(d);
        assert_eq!(vec![doc], s.parked());

        if destroyed_first {
            s.close(doc);
        }
        s.reactor.handle_event(Event::ApplicationTerminated(2));
        s.reactor.handle_event(Event::ApplicationThreadTerminated(2));
        s.apps.windows.remove(&doc);
        assert_eq!(
            vec![
                ("Window1".to_string(), RecordLink::Live(wid(1))),
                ("Doc".to_string(), RecordLink::Empty),
            ],
            records(&s, c),
            "destroyed first: {destroyed_first}"
        );
        assert_eq!(vec!["Window1", "Doc"], saved_members(&s, c));

        let events = doc_app(&mut s, 5, 51);
        s.reactor.handle_events(events);
        let relaunched = WindowId::new(5, 1);
        report_visible(&mut s, &[wid(1), relaunched]);

        assert!(s.reactor.contexts.is_member(c, relaunched));
        assert!(!s.reactor.contexts.is_member(d, relaunched));
        assert_eq!(vec![relaunched], s.parked());
        assert_eq!(vec![(wid(1), screen())], s.tiles());

        s.switch(c);

        let tiles = vec![
            (wid(1), rect(0., 0., 600., 1000.)),
            (relaunched, rect(600., 0., 600., 1000.)),
        ];
        assert_eq!(tiles, s.tiles(), "destroyed first: {destroyed_first}");
        assert_eq!(tiles, s.frames(&[wid(1), relaunched]));
        assert!(s.parked().is_empty());
    }
}

/// R22. A title change updates the window's member records, and doesn't
/// write `contexts.json`. After its app quits, the record keeps the last
/// title, and the relaunched window with that title rejoins C.
#[test]
fn r22_a_title_change_updates_the_member_records_and_a_relaunch_matches_it() {
    let mut s = Setup::new(1);
    let events = doc_app(&mut s, 2, 21);
    s.reactor.handle_events(events);
    let doc = WindowId::new(2, 1);
    report_visible(&mut s, &[wid(1), doc]);
    let c = s.create("C", &[wid(1), doc]);
    let d = s.create("D", &[wid(1)]);
    s.reactor.contexts.pin(&s.desc(doc));
    s.switch(c);
    let path = s.dir.path().join("contexts.json");
    fs::remove_file(&path).unwrap();

    s.reactor
        .handle_event(Event::WindowTitleChanged(doc, "Doc — edited".to_string().into()));

    assert_eq!(
        vec![
            ("Window1".to_string(), RecordLink::Live(wid(1))),
            ("Doc — edited".to_string(), RecordLink::Live(doc)),
        ],
        records(&s, c)
    );
    assert_eq!("Doc — edited", s.reactor.contexts.pinned()[0].title);
    assert_eq!("Doc — edited", s.desc(doc).title);
    assert!(!path.exists(), "a title change alone writes nothing");
    s.reactor.contexts.unpin(doc);
    s.switch(d);
    assert_eq!(vec![doc], s.parked());

    s.reactor.handle_event(Event::ApplicationTerminated(2));
    s.reactor.handle_event(Event::ApplicationThreadTerminated(2));
    s.apps.windows.remove(&doc);
    assert_eq!(vec!["Window1", "Doc — edited"], saved_members(&s, c));
    let window = WindowInfo {
        title: "Doc — edited".to_string().into(),
        sys_id: Some(WindowServerId::new(51)),
        frame: rect(700., 100., 50., 50.),
        ..make_window(1)
    };
    let events = s.apps.make_app_with_info(5, test_app_info(2), vec![window], None, false);
    s.reactor.handle_events(events);
    let relaunched = WindowId::new(5, 1);
    report_visible(&mut s, &[wid(1), relaunched]);

    assert!(s.reactor.contexts.is_member(c, relaunched));
    assert_eq!(vec![relaunched], s.parked());
}

/// C holds windows 1 and 2, window 3 is minimized, and C is active.
fn c_with_window_3_minimized() -> (Setup, ContextKey) {
    let mut s = Setup::new(3);
    report_visible(&mut s, &[wid(1), wid(2)]);
    let c = s.create("C", &[wid(1), wid(2)]);
    s.switch(c);
    assert!(s.parked().is_empty());
    (s, c)
}

fn in_c() -> Vec<(WindowId, CGRect)> {
    vec![
        (wid(1), rect(0., 0., 600., 1000.)),
        (wid(2), rect(600., 0., 600., 1000.)),
    ]
}

/// R39, R30. A known window that isn't a member becomes visible without
/// taking focus: its app reports it unminimized, or a Space change finds it
/// moved in from another Space. It is parked at once, with its journal
/// entry written first, and the members keep their tiles.
#[test]
fn r39_a_known_non_member_that_becomes_visible_is_parked() {
    for how in ["unminimized", "moved in from another Space"] {
        let (mut s, _) = c_with_window_3_minimized();
        let frame = s.frame(wid(3));
        let listed = on_screen(&s, &[wid(1), wid(2), wid(3)]);
        match how {
            "unminimized" => s.reactor.handle_event(Event::WindowsOnScreenUpdated {
                pid: Some(1),
                on_screen: listed,
            }),
            _ => s.reactor.handle_event(Event::SpaceChanged(vec![Some(space())], listed)),
        }
        let requests = s.apps.requests();
        assert_eq!(
            vec![corner(frame.size)],
            frame_writes(&requests, wid(3)),
            "{how}"
        );
        assert_eq!(vec![entry(3, frame)], s.journal_on_disk(), "{how}");
        answer(&mut s, requests);
        s.apps.simulate_until_quiet(&mut s.reactor);

        assert_eq!(vec![wid(3)], s.parked(), "{how}");
        assert_eq!(corner(frame.size), s.frame(wid(3)));
        assert_eq!(in_c(), s.tiles(), "{how}");
        assert_eq!(in_c(), s.frames(&[wid(1), wid(2)]), "{how}");
    }
}

/// R39, R24. A window that becomes visible as the main window has taken
/// focus, so R39 doesn't park it.
#[test]
fn r39_a_window_that_becomes_visible_as_the_main_window_is_not_parked() {
    let (mut s, _) = c_with_window_3_minimized();
    let frame = s.frame(wid(3));
    s.reactor.handle_event(Event::ApplicationGloballyActivated(1));
    s.reactor.handle_event(Event::ApplicationActivated(1, Quiet::Yes));
    s.reactor
        .handle_event(Event::ApplicationMainWindowChanged(1, Some(wid(3)), Quiet::Yes));
    assert_eq!(Some(wid(3)), s.reactor.main_window());

    let listed = on_screen(&s, &[wid(1), wid(2), wid(3)]);
    s.reactor.handle_event(Event::WindowsOnScreenUpdated {
        pid: Some(1),
        on_screen: listed,
    });
    s.apps.simulate_until_quiet(&mut s.reactor);

    assert!(s.parked().is_empty());
    assert_eq!(frame, s.frame(wid(3)));
    assert_eq!(in_c(), s.tiles());
}

/// R39, R31. A parked window that its app moves out of its corner is parked
/// again at once. Its journal entry keeps the frame from before it was
/// first parked.
#[test]
fn r39_a_parked_window_that_its_app_moves_back_is_parked_again_at_once() {
    let mut s = Setup::new(2);
    let c = s.create("C", &[wid(1)]);
    s.switch(c);
    let parked_at = corner(CGSize::new(600., 1000.));
    assert_eq!(parked_at, s.frame(wid(2)));
    let journal = s.journal_on_disk();
    assert_eq!(vec![entry(2, rect(600., 0., 600., 1000.))], journal);

    let moved = rect(300., 200., 600., 700.);
    let txid = s.reactor.windows[&wid(2)].last_sent_txid;
    s.apps.windows.get_mut(&wid(2)).unwrap().frame = moved;
    s.reactor.handle_event(Event::WindowFrameChanged(
        wid(2),
        moved,
        txid,
        Requested(false),
        None,
    ));

    let requests = s.apps.requests();
    assert_eq!(vec![parked_at], frame_writes(&requests, wid(2)));
    answer(&mut s, requests);
    s.apps.simulate_until_quiet(&mut s.reactor);
    assert_eq!(parked_at, s.frame(wid(2)));
    assert_eq!(vec![wid(2)], s.parked());
    assert_eq!(journal, s.journal_on_disk());
    assert_eq!(vec![(wid(1), screen())], s.tiles());
}

/// R36. A new native tab shares the frame of its group. It joins the
/// contexts of the group's main tab, here C and D, and not only the active
/// context, and it is pinned when the main tab is. It takes no tile of its
/// own.
#[test]
fn r36_a_new_tab_joins_the_contexts_of_its_groups_main_tab() {
    for pinned in [false, true] {
        let mut s = Setup::new(2);
        let c = s.create("C", &[wid(1), wid(2)]);
        let d = s.create("D", &[wid(1)]);
        if pinned {
            s.reactor.contexts.pin(&s.desc(wid(1)));
        }
        s.switch(c);
        let tiles = s.tiles();
        s.reactor.handle_event(Event::ApplicationGloballyActivated(1));
        s.reactor.handle_event(Event::ApplicationActivated(1, Quiet::Yes));
        s.reactor
            .handle_event(Event::ApplicationMainWindowChanged(1, Some(wid(1)), Quiet::Yes));

        let tab = WindowInfo {
            frame: s.frame(wid(1)),
            ..make_window(3)
        };
        open_window(&mut s, wid(3), tab, &[wid(1), wid(2)]);

        assert_eq!(
            vec![id_of(c), id_of(d)],
            s.reactor.contexts.contexts_of(wid(3)),
            "pinned: {pinned}"
        );
        assert_eq!(pinned, s.reactor.contexts.is_pinned(wid(3)));
        assert_eq!(tiles, s.tiles());
        assert!(s.parked().is_empty());
        s.switch(d);
        assert_eq!(vec![wid(2)], s.parked());
    }
}
