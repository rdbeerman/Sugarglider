# Contexts

Status: draft, not implemented. Written 2026-09-25, revised 2026-09-26.

Code references point at commit `31eb378`. Each reference names the symbol first; line numbers are hints and will drift.

## Start here

This section is for whoever picks up the work next.

- Branch: `feat-rooms-context-switching`. The draft PR carries this spec only.
- The design decisions in [Settled decisions](#settled-decisions) came from the product owner. Build on them; don't reopen them.
- The first milestone is a spike on a real Mac ([M1](#m1-spike-answer-the-macos-questions)). It answers the questions in [What we don't know yet](#what-we-dont-know-yet). Several later designs depend on those answers.
- Agent sessions must not start the live window manager (`cargo run`, `sugarglider launch`). See `agents.md`. A person runs the spike and the manual QA checklist. Agents run `cargo test`, `cargo +nightly fmt --check`, and `devtool`.
- Rules are numbered (R1, R2, …) so commits and tests can cite them.

## Summary

A **context** is a named set of windows with its own tiling layout. Switching to a context shows its windows in that layout and hides every other window. Nothing is closed. One window can belong to several contexts.

You switch with a hotkey, the menu bar, a search panel called the **switcher**, or the `sugarglider context` command line. Raycast and SuperCmd can call the command line.

The idea comes from [Rooms](https://github.com/saragordic/rooms), an MIT-licensed Mac app. We take its switching model and its safety rules. We drop its layout templates, because Sugarglider already tiles.

## Problem

Sugarglider keeps one tiling layout per macOS Space. People who switch between several jobs keep all those windows open. On one Space, all of them share one layout. Tiles get small, or windows pile up in stacks.

macOS Spaces don't solve this. Each window lives on exactly one Space. A window manager can't move windows between Spaces unless the user partly disables System Integrity Protection, which is how yabai does it. So contexts work inside the Spaces that are visible, and they hide windows instead of moving them to other Spaces.

## Goals and non-goals

Goals:

- Switch the whole visible window set with one keystroke, or by typing a few letters of a context name.
- Give each context its own tiling layout.
- Let one window belong to several contexts.
- Never lose a window. Every window Sugarglider parks comes back after a quit, a crash, or a restart.
- Let launchers list and switch contexts.
- Change nothing for people who never create a context.

Non-goals for the first version:

- Browser tabs. A context holds windows. Put a project's tabs in their own browser window.
- Moving windows between macOS Spaces.
- Opening apps or windows that aren't running.
- Native full-screen windows and Stage Manager.
- Rooms' layout templates (Focus, Columns, Grid).

## Settled decisions

The product owner made these choices on 2026-09-25:

- The feature is called **context**. The search panel is called the **switcher**.
- A window can belong to several contexts. Example: WhatsApp is in both "Comms" and "Relax".
- Scope is a setting: `global` (a switch changes every screen) or `per_screen` (a switch changes only the focused screen).
- When the user focuses a window from another context, Sugarglider switches to that context.
- If two screens both want the same shared window, the window goes to the screen that switched most recently.
- Windows that are in no context appear in an "Unsorted" entry instead of being hidden everywhere.

And on 2026-09-26:

- Sugarglider hides windows only by parking them. It never hides apps.
- Global scope ships first. `per_screen` scope follows in its own milestone ([M9](#m9-per-screen-scope)).
- A window's records survive when its app quits, even if macOS reports the window closed before the app quit (R23).

## Terms

- **Context**: a named set of windows. It has its own layout on each Space, and on each screen size, like today's layouts.
- **Member**: a window in a context. Sugarglider records it by app bundle id, window title, and window server id.
- **Member record**: the stored description of a member. A record stays when the window is gone (for example, when the app quits), so the window can rejoin later.
- **Active context**: the context a screen shows. In `global` scope, all screens share one active context.
- **Everything**: a built-in view that shows every window in the Space's normal layout, the layout Sugarglider uses today. You can't add windows to it.
- **Unsorted**: a built-in context. Its members are the windows that belong to no named context.
- **Park a window**: move a window to a screen corner so that only 1 pixel stays on screen. AeroSpace uses this for every window it hides. Sugarglider parks every window that must not show.
- **Journal**: the file `~/.glide/parked.json`. Sugarglider writes each window there before it parks it.
- **Pinned window**: a window that is a member of every context.

## User stories

1. **Save a context.** You have the windows for a job open. You press ⌃⌥Space and type "Sugarglider". You choose "New context". The switcher lists the windows on screen, all checked. You uncheck the ones that don't belong and press Enter. The new context's layout starts as your current layout without the unchecked windows.
2. **Switch by name.** You press ⌃⌥Space, type "cli", and press Enter. The "Client work" windows appear in their layout. All other windows hide.
3. **Switch by number.** You press ⌃⌥2.
4. **Switch from a launcher.** In Raycast or SuperCmd you run "Switch Context" and type "cli".
5. **Keep working.** You open a new window. It joins the context you're in.
6. **Share a window.** WhatsApp is in "Comms" and "Relax". It has its own place in each layout.
7. **Fix a mistake.** A window is in the wrong context. You focus it and open the switcher. ⌘↩ adds it to the highlighted context. ⇧⌘↩ moves it there.
8. **Follow focus.** You ⌘-Tab to Chrome. Its windows are only in "Relax". Sugarglider switches to "Relax".
9. **Two monitors.** With `per_screen` scope, the left screen shows "Comms" and the right screen shows "Build".
10. **Get everything back.** ⌃⌥0 or "Show Everything" in the menu shows all windows. Quitting Sugarglider shows all windows. After a crash, the next launch shows them.
11. **An app restarts.** WhatsApp quits and opens again. Its window rejoins "Comms" and "Relax", because the app and the title match.

## Behavior rules

### Membership

- **R1.** A context holds windows, not apps. A window can be in any number of contexts.
- **R2.** A member has its own place in each context's layout. Each context keeps a layout per Space and per screen size, the same way Sugarglider keeps layouts today.
- **R3.** A pinned window is a member of every context, including contexts created later. `toggle_window_pinned` pins and unpins the focused window.
- **R4.** Context names are unique, ignoring case. "Everything" and "Unsorted" are reserved.
- **R5.** A context can have a number from 1 to 9. Numbers are unique. A new context gets the lowest free number, or none when 1 to 9 are taken.
- **R6.** Deleting a context never touches its windows. Windows that were only in that context become unsorted. If the deleted context was active, Unsorted becomes active, so those windows stay visible.

### Scope and active context

R8, R9, R11, and R26 apply to `per_screen` scope, which lands in [M9](#m9-per-screen-scope). Until then, only `global` scope exists.

- **R7.** In `global` scope, one active context covers all screens. A switch changes every screen. Windows stay on the screen they're on.
- **R8.** In `per_screen` scope, each screen has its own active context. A switch changes only the focused screen. The members that are on other visible screens move to the focused screen.
- **R9.** In `per_screen` scope, a shared window can only be on one screen. It goes to the screen that switched most recently. The other screen's layout closes the gap. The window returns when that screen switches again.
- **R10.** A screen's active context applies to whichever Space the screen shows. When the user changes Space, Sugarglider applies the active context to the newly visible Space.
- **R11.** Changing scope from `per_screen` to `global` makes the focused screen's context the global one. Changing from `global` to `per_screen` gives every screen the current global context.

### Switching

- **R12.** A switch to context C runs these steps in this order:
  1. Write the journal entries for every window the switch will park (R30).
  2. Make C's layout the active layout on each affected Space.
  3. Lay out C's members. Members that were parked return to their place in C's layout (H3).
  4. Park the windows that must not show.
  5. Focus C's most recently focused member with a quiet raise (see R25).
- **R13.** A window "must show" when it is a member of the active context of the screen it is on, or will move to (R8). Under Everything, every window on that screen's visible Space must show.
- **R14.** Sugarglider never parks:
  - its own windows;
  - windows it doesn't track (`classify_window` returns `Untracked`);
  - minimized windows (it leaves them minimized);
  - windows of apps that the user hid with ⌘H;
  - windows that are only on Spaces nobody can see (R10 handles them when their Space becomes visible).
- **R15.** Parking is the only way Sugarglider hides a window. It never hides an app or minimizes a window. Parked windows keep their place in every layout they belong to (L5).
- **R16.** Switching to the context that is already active applies it again. This parks windows that drifted in.
- **R17.** A switch has no animation. Windows appear in their final place.
- **R18.** `previous_context` switches back to the context used before the current one (per screen in `per_screen` scope).
- **R19.** Each switch updates a most-recently-used order. The switcher and R23 use it.

### New and closed windows

- **R20.** A new window that matches no member record (R22, steps 1 to 3) joins the active context of the screen where it appears. If that screen shows Everything or Unsorted, the window is unsorted. Sugarglider never parks a new window that matched no record.
- **R21.** A window that matches a member record (R22) rejoins the contexts that hold that record. It does not also join the active context. If none of its contexts is active, Sugarglider parks it like any other window that must not show. If the window takes focus, R24 applies.
- **R22.** Sugarglider matches windows to empty member records in this order:
  1. same window server id (valid within one login session);
  2. same app and exactly the same title;
  3. same app, a similar title, and the window is in no other context;
  4. only during a switch: any window of the same app that is in no context.

  Titles are similar when, after lowercasing and removing accents, both have at least 4 characters and one contains the other, or they share a prefix of at least min(12, two-thirds of the shorter title). This is Rooms' `SlotMatcher.similar`. Steps 3 and 4 never take a window that belongs to another context. Step 4 doesn't run when a window arrives, because it would claim, and then park, a window the user just opened.

  A member record's title follows its live window. Sugarglider already observes `kAXTitleChangedNotification` (`src/actor/app.rs`). When the app quits, the record keeps the last title, so a relaunched Chrome or editor window can match it at step 2 or 3.
- **R23.** When a window closes, its records become pending. Matching skips pending records. If the app then terminates, the records stop being pending and stay, so the windows can rejoin when the app runs again (R21). If the app shows that it is still running, the records are deleted. An app shows this when it creates a window, when the user activates it, or when a later window-server update lists one of its windows. When Sugarglider restarts, all records stay. Spike question Q4 checks the event order this depends on.

### Focus from outside

- **R24.** When the user focuses a window that is not a member of its screen's active context, Sugarglider switches to the most recently used context that contains the window. Ways to focus include ⌘-Tab, the Dock, a notification click, and launching an app. If the window is in no context, Sugarglider switches to Unsorted. If the screen shows Everything, nothing happens. These windows never cause a switch:
  - windows Sugarglider doesn't track, such as a Raycast or 1Password panel;
  - Sugarglider's own windows, including the switcher.

  Sugarglider decides a new window's membership (R20, R21) before it applies this rule to the window, so launching an app never switches to Unsorted. Spike question Q3 checks whether the focus event can arrive before the new window is known.
- **R25.** Only focus the user started counts. Sugarglider's own raises are quiet already (the `Quiet` flag in `src/actor/app.rs`, `on_activation_changed`). During a switch, until step 5 of R12 finishes, Sugarglider ignores activations. Parking the focused window may make macOS send an activation or main-window change that looks like the user's (spike question Q2).
- **R26.** In `per_screen` scope, the switch happens on the screen where the window was last shown, or on the focused screen when that is unknown.

### Everything and Unsorted

- **R27.** Showing Everything puts every parked window back at its journal frame, then lays out the Space's normal layout.
- **R28.** Everything is active until the first context exists. With the feature flag off, Everything is always active. People who never create a context see no change.
- **R29.** Unsorted behaves like a context whose members are computed. It has its own layouts. The switcher lists it only when it has windows. It can't be renamed, numbered, or deleted.

### Safety

- **R30.** Before Sugarglider parks a window, it writes the journal entry to disk. If the write fails, it doesn't park.
- **R31.** Sugarglider removes a journal entry only after it confirms the window is back. The frame read back from the app must be within 16 points of the target (the tolerance Rooms uses).
- **R32.** On quit (the Quit menu item and `save_and_exit`), Sugarglider shows Everything (R27) and then exits.
- **R33.** When Sugarglider turns off (`toggle_global_enabled`, `sugarglider pause`, or turning off the feature flag), it shows Everything first. When the user turns off one space (`toggle_space_activated`), it shows Everything on that space first.
- **R34.** On launch, Sugarglider restores every window in the journal before it applies any context. It drops entries for apps that aren't running. If the journal can't be read, Sugarglider moves it to `parked.unreadable-<unix time>.json`, logs an error, and starts a new one.
- **R35.** If Sugarglider dies and nobody starts it again, the user can still reach every window. A parked window still shows 1 pixel in a screen corner, and Mission Control shows it.

## User interface

### Switcher

`open_context_switcher` opens it. The suggested binding is ⌃⌥Space.

```
┌────────────────────────────────────────────────────┐
│ ⌕ Switch context…                                  │
├────────────────────────────────────────────────────┤
│ ● Sugarglider   Ghostty · Zed · Chrome    ⌃⌥1      │
│ ▸ Client work   Outlook · Word · Chrome   ⌃⌥2      │
│   Comms         Slack · Mail · Calendar   ⌃⌥3      │
│   Unsorted      2 windows                          │
│   Everything    show all windows          ⌃⌥0      │
├────────────────────────────────────────────────────┤
│ ↩ switch  ⌘↩ add window  ⇧⌘↩ move window  ⌘N new   │
└────────────────────────────────────────────────────┘
```

- ● marks the active context. ▸ marks the highlighted row.
- Each row shows the name, up to three app names, and the number shortcut.
- Typing filters the list. Ranking follows Rooms' `Matcher.rank`: exact name, then name prefix, then word prefix, then initials, then all words matched as prefixes, then letters in order. Ties go to the most recently used context. Names are compared lowercased and without accents.
- When no name matches exactly, a "New context “<text>”" row appears.
- The first time, the list is empty and says "Type a name to create your first context".

Keys:

| Key | Action |
|---|---|
| ↑ ↓ | Move the highlight |
| ↩ | Switch to the highlighted context |
| ⌘↩ | Add the target window to the highlighted context |
| ⇧⌘↩ | Move the target window to the highlighted context |
| ⌘N | New context from the windows on screen |
| ⌘E | Edit the highlighted context's windows |
| ⌘R | Rename the highlighted context |
| ⌘1 – ⌘9 | Give the highlighted context that number |
| ⌘P | Pin or unpin the target window |
| ⌘⌫ | Delete the highlighted context (R6) |
| Esc | Close |

The **target window** is the window that had focus when the switcher opened. Sugarglider records it at that moment and passes its id with the command, so the panel taking key focus can't change it.

The create and edit views list the windows on screen (the focused screen in `per_screen` scope) and the context's current members, each with a checkbox.

The panel is an `NSPanel` with the `nonactivatingPanel` and borderless styles, at floating level. It joins all Spaces and overrides `canBecomeKey` to return true. That lets the user type without making Sugarglider the frontmost app. Rooms' `PalettePanel` does the same. It appears centered on the focused screen.

### Menu bar

- The status item shows the name of the active context. In `per_screen` scope it shows the focused screen's context. It shows nothing extra when Everything is active. If `experimental.status_icon.space_index` is on, it shows both, for example "2 · Comms".
- The menu gains a section with each context (a checkmark on the active one, and the number as the key equivalent), "Show Everything", "New Context from Current Windows…", a "Send Window to" submenu, and "Open Switcher…".

### Key bindings

New commands (all are `snake_case` in TOML):

| Command | Argument | Does |
|---|---|---|
| `open_context_switcher` | none | Opens the switcher |
| `switch_context` | number 1–9 or name | Switches (R12) |
| `show_everything` | none | R27 |
| `previous_context` | none | R18 |
| `add_window_to_context` | number or name | Adds the focused window |
| `move_window_to_context` | number or name | Moves the focused window out of the active context and into the named one |
| `remove_window_from_context` | none | Removes the focused window from the active context |
| `toggle_window_pinned` | none | R3 |

The default config ships these bindings commented out while the feature is experimental. The hotkey event tap swallows a bound key even when the feature is off, so shipping them active would take ⌃⌥Space away from people who don't use contexts. macOS also binds ⌃⌥Space to "Select next source in Input menu" by default. Sugarglider's event tap would take the key first; the user docs must mention this.

```toml
# "Ctrl + Alt + Space" = "open_context_switcher"
# "Ctrl + Alt + 0" = "show_everything"
# "Ctrl + Alt + 1" = { switch_context = 1 }
# "Ctrl + Alt + 2" = { switch_context = 2 }
# ... through 9
# "Ctrl + Alt + Tab" = "previous_context"
```

### Command line

```
sugarglider context list [--json]
sugarglider context current [--json]
sugarglider context switch <query>
sugarglider context everything
sugarglider context previous
sugarglider context create <name>
sugarglider context add <query>          # adds the focused window
sugarglider context move <query>         # moves the focused window
sugarglider context remove               # removes the focused window from the active context
sugarglider context rename <query> <new name>
sugarglider context delete <query>
```

- `<query>` is a number or a name. The server matches names with the switcher's ranking and takes the best match.
- Human-readable output goes to stdout. `--json` prints the snapshot shape below.
- On failure (no match, server not running, feature off) the command prints the reason to stderr and exits with status 1.

`sugarglider context list --json`:

```json
{
  "scope": "global",
  "screens": [{ "id": 1, "active": "Comms" }],
  "contexts": [
    { "name": "Comms", "number": 1, "active": true, "apps": ["WhatsApp", "Microsoft Teams"], "windows": 2 },
    { "name": "Relax", "number": 2, "active": false, "apps": ["WhatsApp", "Google Chrome"], "windows": 2 }
  ],
  "unsorted": 3
}
```

### Raycast and SuperCmd

A Raycast [script command](https://github.com/raycast/script-commands) needs no published extension. Ship it as `contrib/raycast/switch-context.sh`:

```bash
#!/bin/bash
# @raycast.schemaVersion 1
# @raycast.title Switch Context
# @raycast.mode compact
# @raycast.packageName Sugarglider
# @raycast.argument1 { "type": "text", "placeholder": "Context" }
set -euo pipefail
/usr/local/bin/sugarglider context switch "$1" 2>&1
```

- `make install` puts the binary in `/usr/local/bin`. Raycast adds that directory to `PATH`, but the absolute path avoids depending on that.
- In `compact` mode Raycast shows the last output line, and treats a non-zero exit as a failure. `2>&1` makes the error text visible.
- SuperCmd's documentation says it imports Raycast script-command folders and runs most Raycast extensions. Nobody has tested this with Sugarglider yet.
- A full Raycast extension with a searchable list is a later item.

### Preferences

The Preferences window gets a "Contexts (experimental)" switch and a scope picker. `write_preferences_to_file` (`src/config.rs`) writes only a fixed list of keys, so these two keys must be added to it.

## Design

### Overview

```
hotkey · menu · switcher (FFI) · CLI (IPC)
                 │  ContextCommand
                 ▼
WmController ─► SpaceManager ─► Reactor ─► LayoutManager
                                  │          ├─ owns model::contexts::Contexts
                                  │          ├─ one layout per (Space, context)
                                  │          └─ returns a SwitchPlan
                                  ├─► journal ~/.glide/parked.json   (written first)
                                  ├─► app threads: SetWindowFrame, Raise (quiet)
                                  ├─► ~/.glide/contexts.json
                                  └─► snapshot ─► CLI, switcher, menu bar
```

### Model: `src/model/contexts.rs`

The model is pure, like the rest of `src/model/`. It does no I/O and reads no clock. Most-recently-used order is a sequence number that increments on each switch, not a timestamp.

Types (names are suggestions):

```rust
pub struct ContextId(u32);
pub enum ContextKey { Everything, Unsorted, Named(ContextId) }

pub struct MemberRecord {
    pub bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub title: String,
    pub window_server_id: Option<WindowServerId>,
    pub window: Option<WindowId>, // live window; not persisted
}

pub struct Context {
    pub id: ContextId,
    pub name: String,
    pub number: Option<u8>,
    pub members: Vec<MemberRecord>,
    pub last_used: u64,
}

pub struct Contexts {
    contexts: Vec<Context>,
    pinned: Vec<MemberRecord>,
    next_id: u32,
    use_seq: u64,
}
```

Pure functions, each with unit tests:

- `rank(query, &Contexts) -> Vec<(ContextKey, score)>`: the switcher ranking.
- `match_window(window, &Contexts, pass) -> Vec<ContextId>`: R22.
- `plan_switch(&SwitchInput) -> SwitchPlan`: R12 to R15.
  - `SwitchInput` lists each visible screen with its Space, its active context after the switch, and its windows. Each window carries its pid, bundle id, contexts, and whether it is minimized, untracked, in an app the user hid, or already parked.
  - `SwitchPlan` lists windows to park, windows to put back, windows that move to another screen (`per_screen` only), and the window to focus.

The layering rule allows `model` to use `sys` for geometry types only. Pass screens as plain indexes or keep screen ids in the actor.

### Layouts

Facts from the current code:

- `LayoutManager.layout_mapping: HashMap<SpaceId, SpaceLayoutMapping>` holds each Space's layouts. All layouts share one `LayoutTree` (`src/actor/layout.rs`, struct `LayoutManager`).
- `LayoutManager::layout(space)` returns the active layout of the Space. Every layout operation goes through it (`src/actor/layout.rs:2501`).
- One window can have a node in several layouts at once. `window_nodes: BTreeMap<WindowId, Vec<NodeId>>` in `src/model/window.rs` documents this, and `LayoutTree::window_node(layout, wid)` picks the node for one layout. `remove_window_from(layout, wid)` removes it from one layout; `remove_window(wid)` removes it from all.
- **When a window stops being visible, Sugarglider removes it from the active layout.** `set_windows_for_app` detaches its node. When the window is visible again, it is added back as the last child of the root, and its old place is lost (`src/model/layout_tree.rs:320-359`). Layouts that are not active keep their nodes.

This last fact is the main constraint. If a switch made windows invisible, every switch would throw away the layout of every hidden window. So the design separates membership from visibility:

- **L1.** Add `context_layouts: HashMap<(SpaceId, ContextKey), SpaceLayoutMapping>` and `active_context: HashMap<SpaceId, ContextKey>` to `LayoutManager`, both with `#[serde(default)]`. `Everything` keeps using `layout_mapping`. `layout(space)` looks up the active context first.
- **L2.** Each context mapping keeps the per-screen-size memory that `SpaceLayoutMapping` provides today. Context layouts don't take part in `next_layout`/`prev_layout`. Garbage collection must treat them as roots.
  - Today the `LayoutEvent::SpaceExposed` handler calls `activate_size` only on `layout_mapping` (`src/actor/layout.rs`, `LayoutManager::handle_event`). It must also call it on the mapping of the Space's active context.
  - When a context becomes active on a Space, Sugarglider calls `activate_size` on the context's mapping with the Space's current size. `layout_mapping` already holds that size.
  - `layout(space)` unwraps. The context's mapping for a Space must exist (L3) before any event reaches `layout(space)` with that context active.
- **L3.** When a context gets its first layout on a Space, Sugarglider clones the active layout (`LayoutTree::clone_layout`) and removes the windows that aren't members. Creating a context keeps the arrangement the user sees.
- **L4.** When a context layout is active, `WindowsOnScreenUpdated` and `WindowAdded` only add windows that are members. A visible window that isn't a member, and isn't parked yet, never gets a tile.
- **L5.** Parked windows stay in the visible-window list, because 1 pixel stays on screen. H2 keeps them out of the updates that reach `set_windows_for_app`, so parking never removes a node. A switch changes the active layout and the reactor's parked set in one reactor event. No visibility update can see a half-finished switch. No timer is involved; `CONTRIBUTING.md` asks us not to add timers.
- **L6.** Members that the user minimized, or whose app the user hid, leave the active layout as they do today. They stay members.
- **L7.** `floating_windows`, `floating_restore_frames`, and size locks are keyed by window. In the first version, a floating window shared by two contexts has the same frame in both.

### Parking

Facts from the current code:

- `SetWindowFrame` is the only request parking needs. The app requests are `Terminate`, `GetVisibleWindows`, `SetWindowFrame`, `AnimationFrame`, `BeginWindowAnimation`, `EndWindowAnimation`, `Raise`, and `WindowDestroyed` (`src/actor/app.rs`, enum `Request`).
- A window whose frame is on no screen belongs to no Space and drops out of the layout at the next refresh (`best_screen_idx_for_window`, test `windows_parked_off_screen_belong_to_no_screen`). A window with 1 pixel on screen does belong to that screen.

Design:

- **H1.** Parking sends `SetWindowFrame` to a corner position that keeps 1 pixel on screen. Port Rooms' `Geometry.parkingOrigin`. It tries the four corners and takes the first one where the parked window overlaps no other display.
- **H2.** The reactor keeps `parked: HashMap<WindowId, CGRect>`, with the frame from before parking. `send_visible_windows_to_layout` and `MouseMovedOverWindow` skip parked windows, whatever their geometry says.
- **H3.** Putting a member back needs no special request. The member is in the active layout, so `update_layout` writes its frame. The reactor clears its parked state when it sends that frame. Showing Everything puts non-members back at their journal frames (R27).
- **H4.** Writes use the existing transaction ids, so the reactor ignores stale frame reads from before the park.

A switch writes one frame per window that it parks or puts back. Q5 measures what that costs.

### Journal and state files

Both files live in `data_dir()` (`~/.glide`, `src/config.rs`), next to `layout.ron`. Both are JSON, because people and scripts read them. Both are written atomically with `tempfile::NamedTempFile::new_in(dir)` and `persist`, like `write_preferences_to_file`. (`LayoutManager::save` is not atomic; don't copy it.)

`~/.glide/parked.json`:

```json
{
  "version": 1,
  "entries": [
    { "pid": 812, "bundle_id": "com.google.Chrome", "window_server_id": 9123,
      "title": "Docs", "frame": { "x": 0, "y": 25, "w": 1440, "h": 875 } }
  ]
}
```

`~/.glide/contexts.json`:

```json
{
  "version": 1,
  "next_id": 3,
  "use_seq": 42,
  "contexts": [
    { "id": 1, "name": "Comms", "number": 1, "last_used": 42,
      "members": [
        { "bundle_id": "net.whatsapp.WhatsApp", "app_name": "WhatsApp", "title": "WhatsApp", "window_server_id": 81234 }
      ] }
  ],
  "pinned": [],
  "active": { "global": 1 }
}
```

- The reactor writes `contexts.json` after every change to contexts or membership, after each switch, when an app that has members quits, and on quit. A title change alone doesn't write the file, because terminals and browsers change titles constantly. The app-quit write keeps the last titles that R22 needs to match relaunched windows.
- `active` is `{ "global": <id> }` or `{ "per_screen": { "<display id>": <id> } }`. On launch, each screen gets its saved context. A screen without an entry shows Everything.
- An unreadable `contexts.json` is moved aside like the journal, and Sugarglider starts with no contexts.
- The files contain window titles. The user docs must say so, as Rooms does.
- Context layouts are part of `LayoutManager`, so they survive `save_and_exit` plus `--restore` like today's layouts. After a crash or a reboot, membership survives in `contexts.json`, but layouts start fresh from member order.

### Commands and dispatch

- Add `reactor::Command::Context(ContextCommand)` (`src/actor/reactor.rs`, enum `Command`). `Command` is untagged, so the variant names must not collide with `LayoutCommand` names.
- `open_context_switcher` is UI work on the main thread, so it is a `WmCmd` variant (`src/actor/wm_controller.rs`).
- `switch_context` takes an untagged `ContextRef`: a number, a name, or an exact id. The CLI and the switcher resolve names to an id before they send the command, so a rename in between can't redirect it.
- New commands must be added to the exhaustive matches in `WmController::handle_event`, `Reactor::handle_event`, and `describe_command` in `src/ui/preferences_json.rs`.
- `LayoutManager` handles context commands and returns a `SwitchPlan` in its response, the same way it returns windows to raise today. The reactor carries out the plan in R12 order and writes the journal first.

### IPC

Facts from the current code:

- Requests and responses are RON over a `CFMessagePort` named `org.glidewm.server`. The exchange is synchronous; the client waits up to 1000 ms (`src/bin/glide.rs`).
- `Response` has only `Pong`, `Success`, and `Error`. It can't carry data.
- The port callback runs on the main thread, and so does `WmController`. The reactor runs on its own thread. So the callback can't wait for a reply from the reactor; that would block the thread that must forward the request.

Design:

- **I1.** The reactor publishes a `ContextsSnapshot` into a global `OnceLock<RwLock<Arc<ContextsSnapshot>>>` after every change. `CURRENT_CONFIG` in `src/ui/swift_bridge.rs` is the precedent.
- **I2.** Add `Request::Context(ContextRequest)` with `List`, `Current`, and `Run(ContextCommand)`, and `Response::Contexts(ContextsSnapshot)`.
- **I3.** `List` and `Current` read the snapshot and reply at once. `Run` resolves the query against the snapshot, replies `Error` when nothing matches, and otherwise sends the command through `wm_tx` and replies `Success`. The switch itself happens after the reply.
- **I4.** An older server can't parse the new request and replies with nothing. Then the client prints "Deserializing response failed". Replace that message with "The running Sugarglider doesn't support contexts. Restart it."

### Swift bridge

- Rust to Swift: add `sugarglider_show_context_switcher(json)` and `sugarglider_hide_context_switcher()`. The JSON holds the snapshot, the target window, and the windows on screen with their titles and app names.
- Swift to Rust: add `sugarglider_get_contexts()` (returns JSON, freed with `sugarglider_free_string`) and `sugarglider_run_context_command(json)`.
- Today nothing lets Swift send a command. Add a sender next to `CONFIG_UPDATE_SENDER` (`src/ui/swift_bridge.rs`) that sends `WmEvent::Command`.
- Each new Swift-to-Rust symbol needs a `-Wl,-u,_<symbol>` line in `build.rs` and a `black_box` reference in `swift_bridge::init`. Commit `73ae968` explains why.

### Config

```toml
[settings.experimental.contexts]
# Named window sets you switch between. See docs/specs/contexts.md.
enable = false
# "global": a switch changes every screen. "per_screen": only the focused screen.
scope = "global"
```

- Declare the struct with `#[derive(PartialConfig!)]` inside `Experimental` (`src/config.rs`). Every field needs a value in `sugarglider.default.toml`; the tests `default_config_is_valid` and `default_settings_match_unspecified_setting_values` check this.
- `enable` arrives in M5. `scope` arrives in M9, so the config never has a key that does nothing.
- On a config reload that sets `enable = false`, Sugarglider shows Everything (R33). A reload that changes `scope` applies R11.

## Implementation traps

- The test harness hits `todo!()` for `Raise` and `WindowDestroyed` requests (`src/actor/reactor/testing.rs`, `simulate_events_for_requests`). Switching raises windows, so implement `Raise`.
- A test reactor replays its own recording when it is dropped (`testing.rs`, `impl Drop`). Every new reactor `Event` must survive a serde round trip.
- New `LayoutManager` fields need `#[serde(default)]`. Bless `tests/snapshots/current.ron` with `GLIDE_BLESS_SNAPSHOTS=1`, and keep the old snapshots restoring.
- `Status::update_space` sets the status item title on every Space change (`src/actor/status.rs`). It would overwrite the context name; merge the two.
- The status menu is built once and has no dynamic items (`src/ui/status_bar.rs`). The context list needs an `NSMenuDelegate` or a rebuild on change.
- `notification_center.rs` panics on any notification name it doesn't know. If you observe new NSWorkspace notifications, add a branch.
- Two spellings of the same hotkey (`"Alt + Ctrl + 1"` and `"Ctrl + Alt + 1"`) both survive the config merge, and the second registration panics (`src/sys/event.rs`, `register_wm` calls `unwrap`).
- A `WindowAdded` event arrives right after a `WindowsOnScreenUpdated` for the same app. Nothing guards against adding the window twice. Check this before adding membership logic to both paths.
- `ReactorCommand::SaveAndExit` calls `process::exit` on the reactor thread. R32 must run before that call.
- There are no signal handlers. After `SIGTERM` or `kill -9`, only the journal (R34) protects windows.
- `CONTRIBUTING.md` describes the log target as `glide_wm::…`; in this crate it is `sugarglider::…`.

## What we don't know yet

These are facts about macOS that the code can't answer. The spike (M1) answers Q1 to Q4 on a real Mac, and M5 answers Q5. Replace each question with the finding.

- **Q1.** Does macOS keep a window where Sugarglider puts it when only 1 pixel stays on screen? Check all four corners, with one display and with two. Check that the window stays parked when its app is activated, and whether apps move their own parked windows back.
- **Q2.** When Sugarglider parks the focused window, does macOS send an activation or main-window change, and does it arrive with `Quiet::No`? This decides whether R25's guard is needed.
- **Q3.** When the user launches an app, does `ApplicationActivated` or `ApplicationMainWindowChanged` reach the reactor before the new window does (`ApplicationLaunched`, `WindowsDiscovered`, or `WindowCreated`)? R24 depends on the order.
- **Q4.** When the user quits an app with ⌘Q, do `WindowDestroyed` events reach the reactor before `ApplicationTerminated`? How long is the gap? Check several apps, including Chrome and a single-window app. For an app with several windows, can a window-server update that lists its remaining windows arrive between the first `WindowDestroyed` and `ApplicationTerminated`? That update would delete the first window's records during the quit. R23 depends on these answers.
- **Q5.** How long does a full switch take with about 20 windows? Log the duration of every switch, as Rooms does, and set a target from the measurement.

## Milestones

Each milestone is a set of small commits that build and pass `cargo test`. Run `cargo +nightly fmt` before each commit. The feature stays behind `settings.experimental.contexts.enable` until the last milestone. Commits use `internal:` (or `refactor:`/`test:`) until the feature leaves experimental; then a `feat:` commit adds the release note.

M1 to M5 give a usable feature in global scope with a command line. The menu bar and the switcher follow. `per_screen` scope comes last because it moves windows between displays.

### M1. Spike: answer the macOS questions

- Add `devtool` subcommands: `park <pid> <window>`, which prints the frame it replaced, and `set-frame <pid> <window> <x> <y> <w> <h>`, which puts the window back. Reuse `devtool list ax` to read results.
- For Q1, a person runs them with Sugarglider stopped, so it doesn't lay the window out again.
- For Q2 to Q4, a person records a trace with `--record` while parking the focused window, launching an app, and quitting apps with ⌘Q. The trace shows the event order.
- The person uses a Mac with two displays and writes the answers to Q1–Q4 into this spec.
- Commit: `internal: add devtool commands to park windows`.

### M2. Model

- `src/model/contexts.rs` with the types, `rank`, `match_window`, and `plan_switch`, plus unit tests. `plan_switch` covers global scope; M9 adds `per_screen` inputs.
- No behavior change.

### M3. Parking and the journal

- H1 to H4, the journal with R30, R31, and R34, and R32 on quit.
- Extend the test harness (see Implementation traps).
- Nothing calls these paths yet, apart from launch recovery.

### M4. Context layouts

- L1 to L6 in `LayoutManager`, with serde defaults and a blessed snapshot.
- Model tests assert exact frames.

### M5. Switching in global scope, with a minimal command line

- The commands, R1 to R35 except R8, R9, R11, and R26, `contexts.json`, and the `enable` config flag.
- I1 to I4, and the `sugarglider context` subcommands `list`, `create`, `add`, `switch`, and `everything`. They are enough to use contexts every day and to test them by hand.
- Log the duration of every switch and answer Q5.
- Reactor integration tests (see Testing).

### M6. Full command line and Raycast

- The other `sugarglider context` subcommands and `contrib/raycast/switch-context.sh`.

### M7. Menu bar

- The status title and the context menu section.

### M8. Switcher

- The SwiftUI panel and the Swift bridge functions.

### M9. Per-screen scope

- R8, R9, R11, and R26, the `scope` config key, and `per_screen` inputs to `plan_switch`.
- R8 moves windows to another display, and so to another Space. Layouts are per Space. Decide here whether a window that R9 moved keeps its place in the other screen's layout. Until then, the test "switching away and back gives the same frames" covers one screen.
- Model and reactor tests with two screens.

### M10. Preferences and docs

- The Preferences switch and scope picker.
- A user page in `site/src/content/docs`.
- Check the Preferences save path first. The research for this spec found that `HotkeyBindingJson.sort_order` is required in Rust while the Swift `HotkeyBinding` has no such field. That may make saving fail whenever hotkeys exist. Nobody has confirmed this at runtime.

## Testing

Model tests (M2):

- A switch parks exactly the windows that must not show. An app with windows inside and outside the target context gets only its non-member windows parked (R12, R15).
- A switch doesn't park minimized, untracked, or unseen-Space windows, or windows of apps the user hid (R14).
- Matching follows the R22 order. Steps 3 and 4 never take another context's window. Step 4 never runs when a window arrives.
- A record follows its window's title. A relaunched window with the last title rejoins at step 2 (R22).
- Pending records match nothing. They stay when the app terminates, and they go when the app shows it is still running (R23).
- Ranking: "cli" finds "Client work", "cw" finds it by initials, and an exact name beats a prefix.
- M9: a window shared by two contexts, in `per_screen` scope, goes to the screen that switched last (R9).

Reactor integration tests (M3–M5), using `Apps`, `simulate_until_quiet`, and `layout.calculate_layout`:

- Switching away from a context and back gives exactly the same frames. This is the regression test for the layout constraint above.
- A switch parks every non-member window on the visible Spaces and nothing else (R13, R14).
- A new window joins the active context (R20). A closed window's records go once its app shows it is still running (R23).
- An app that quits and relaunches rejoins its contexts by title (R21, R22). This also works when `WindowDestroyed` arrives before `ApplicationTerminated` (R23).
- Focus on a window from another context switches (R24). An activation during a switch does not (R25). Focus on an untracked window does not. Launching an app never switches to Unsorted, whatever order the launch events arrive in (R24).
- A `SpaceExposed` with a new screen size while a context is active gives that context a separate layout for the new size. The old size brings the old layout back (L2).
- `save_and_exit` shows Everything first (R32). Launch with a journal restores windows (R34); make the journal path injectable for this test.
- Showing Everything restores all windows (R27).

Manual QA (a person, on a real Mac):

- Two displays in global scope. After M9, in both scopes.
- WhatsApp in two contexts. Chrome with one window in each of two contexts, before and after quitting and relaunching Chrome.
- Finder windows in and out of a context.
- ⌘-Tab, a Dock click, and a notification click into another context. Opening a Raycast or 1Password panel from inside a context.
- `kill -9` on the server while windows are parked, then relaunch. Also check R35 without relaunching.
- `save_and_exit` and `--restore`.
- The Raycast script. SuperCmd importing the script folder.
- ⌃⌥Space with two keyboard input sources enabled.

## Later

- Open the apps of a context that aren't running, and match their windows as they appear.
- A Raycast extension with a searchable list.
- Layout previews in the switcher.
- Keeping layout shapes across reboots.
- A `SIGTERM` handler that shows Everything before exit.
- A `sugarglider context recover` command that restores the journal without the server.
- A URL scheme (`sugarglider://context/<name>`). It needs app bundle work first: `glide_bundle()` requires a bundle id containing "glidewm", and the packaged id is `com.rdbeerman.sugarglider` (`src/sys/bundle.rs`).
- Per-context floating frames.

## References

- Rooms, MIT license: [README](https://github.com/saragordic/rooms). Relevant files: `Sources/Rooms/Windows/WindowEngine.swift` (parking and the resting ledger), `Sources/RoomsCore/Geometry.swift` (`parkingOrigin`, `keptOnScreen`), `Sources/RoomsCore/WindowSlot.swift` (`SlotMatcher`, `RestLedger`), `Sources/RoomsCore/Matcher.swift` (name ranking), `Sources/Rooms/Palette/PalettePanel.swift` (the panel).
- [AeroSpace](https://github.com/nikitabobko/AeroSpace) hides windows by moving them to a screen corner.
- [Raycast script commands](https://github.com/raycast/script-commands).
- [SuperCmd extension support](https://supercmd.sh/).
