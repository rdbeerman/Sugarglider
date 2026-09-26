# Switcher JSON contract (M8)

This is the contract between the context switcher panel (Swift, `SugargliderUI`) and Rust. The Swift half implements it in `SugargliderUI/Sources/SugargliderUI/ContextSwitcherContract.swift`. The doc comment on `ContextSwitcherJSON` there summarizes it and repeats the examples. The examples below are also the test fixtures in `SugargliderUI/Tests/SugargliderUITests/ContractFixtures.swift`, so the Swift tests decode and re-encode exactly these literals. A test checks that the three copies of the examples are the same.

Design source: `docs/specs/contexts.md`, sections "Switcher" and "Swift bridge".

## General rules

- All keys are snake_case.
- A named context is always tagged as `{"id": N}`, never a bare number, so it can't be read as a context number (1–9).
- The strings `"everything"` and `"unsorted"` name the two built-in entries. This key type is not the key-binding `ContextRef`, where a bare string is a name. Rust must not parse these fields as `ContextRef`. The switcher never sends a name, except as the new name in `create` and `rename`.
- A window is Rust's `WindowId` in its serde shape, `{"pid": 812, "idx": 9123}` (`pid: pid_t`, `idx: NonZeroU32`). Rust can deserialize it with the existing derive. The switcher never reads the fields and sends the value back unchanged.

## Functions

Rust to Swift, exported by SugargliderUI with `@_cdecl`:

| Symbol | Signature | Does |
|---|---|---|
| `sugarglider_show_context_switcher` | `fn(json: *const c_char)` | Shows the panel with the show payload. The reactor calls it from its own thread when it handles `open_context_switcher`. Swift copies the string before it returns and shows the panel on the main queue. A call while the panel is open replaces it with a fresh panel (empty query, list view). A payload that doesn't decode is logged with `NSLog` and shows nothing. |
| `sugarglider_hide_context_switcher` | `fn()` | Hides the panel if it is open. Safe to call at any time. |

Swift to Rust, implemented by the Rust half:

| Symbol | Signature | Does |
|---|---|---|
| `sugarglider_rank_contexts` | `extern "C" fn(query: *const c_char) -> *mut c_char` | Returns `model::contexts::rank(query, contexts, unsorted_has_windows)` as the rank result below, computed from the latest published state (I1). Returns NULL when contexts are unavailable, for example because the feature is off. |
| `sugarglider_run_context_command` | `extern "C" fn(json: *const c_char) -> *mut c_char` | Takes one command below. Returns NULL when Rust accepted the command and sent it to the reactor (`WmEvent::Command` through the sender in `CONFIG_UPDATE_SENDER`). Otherwise returns an error message for the user. |

- Each input string is NUL-terminated UTF-8 and is valid only during the call.
- Swift frees every returned string with the existing `sugarglider_free_string`.
- Swift looks up both Swift-to-Rust functions with `dlsym(RTLD_DEFAULT, "<name>")` when the panel needs them, not with `@_silgen_name`. A binary without them still loads, and the panel shows an inline error instead of crashing. They must therefore stay exported from the executable, like the config functions.
- Build requirements for the Rust half (spec, "Swift bridge"): add `-Wl,-u,_sugarglider_rank_contexts` and `-Wl,-u,_sugarglider_run_context_command` to the symbol list in `build.rs`, add a `black_box` reference to each in `swift_bridge::init`, and declare `sugarglider_show_context_switcher(json: *const c_char)` and `sugarglider_hide_context_switcher()` in the `extern "C"` block of `src/ui/swift_bridge.rs`.

`sugarglider_run_context_command` checks against the published snapshot before it sends the command, and returns an error for:

- invalid JSON, or a command it doesn't know;
- the feature is off;
- a context id that no longer exists;
- a `create` or `rename` name that is empty, reserved, or taken (R4). The panel checks a new name against the reserved names and the payload before it sends `create`, but the payload can be out of date, and the panel checks a new name for `rename` only for being empty;
- a `set_number` number outside 1 to 9.

The reactor logs failures that come later, for example a failed journal write (R30).

## Show payload

```json
{
  "display_id": 1,
  "target_window": { "pid": 812, "idx": 9123 },
  "contexts": [
    {
      "id": 1, "name": "Sugarglider", "number": 1, "hotkey": "⌃⌥1",
      "active": true, "apps": ["Ghostty", "Zed", "Google Chrome"], "windows": 3,
      "members": [
        { "record": 0, "app": "Ghostty", "title": "~/src/sugarglider",
          "window": { "pid": 640, "idx": 8812 } },
        { "record": 1, "app": "Zed", "title": "reactor.rs",
          "window": { "pid": 701, "idx": 8920 } },
        { "record": 2, "app": "Google Chrome", "title": "Docs",
          "window": { "pid": 812, "idx": 9123 } },
        { "record": 3, "app": "Mail", "title": "Inbox", "window": null }
      ]
    },
    {
      "id": 4, "name": "Client work", "number": null, "hotkey": null,
      "active": false, "apps": [], "windows": 0, "members": []
    }
  ],
  "unsorted": { "windows": 2, "active": false },
  "everything": { "active": false, "hotkey": "⌃⌥0" },
  "windows": [
    { "id": { "pid": 640, "idx": 8812 }, "title": "~/src/sugarglider",
      "app": "Ghostty", "tab_count": 1, "pinned": false },
    { "id": { "pid": 812, "idx": 9123 }, "title": "Docs",
      "app": "Google Chrome", "tab_count": 3, "pinned": false },
    { "id": { "pid": 903, "idx": 9201 }, "title": "WhatsApp",
      "app": "WhatsApp", "tab_count": 1, "pinned": true },
    { "id": { "pid": 977, "idx": 9310 }, "title": "Downloads",
      "app": "Finder", "tab_count": 1, "pinned": false },
    { "id": { "pid": 988, "idx": 9402 }, "title": "general",
      "app": "Slack", "tab_count": 1, "pinned": false }
  ]
}
```

In this example the Zed window (record 1) is open but not on screen (another Space, minimized, or parked), and the Mail window (record 3) is gone. The Chrome window is the main tab of a group of 3 tabs. The WhatsApp window is pinned, so it doesn't count as unsorted; Finder and Slack are the 2 unsorted windows.

| Key | Type | Meaning |
|---|---|---|
| `display_id` | u32 or null | The `CGDirectDisplayID` of the focused screen. The panel centers on it. Null or an unknown id means the main screen. |
| `target_window` | window or null | The window that had focus when the reactor handled `open_context_switcher` (spec: "target window"). Every command about a window carries this id, never the window focused when the command is sent. It is null when no window had focus, or when the focused window is untracked, parked (for example Finder's window after R12 step 6), or Sugarglider's own. Otherwise `windows` holds it. Null disables ⌘↩, ⇧⌘↩, and ⌘P: the panel dims their hints and says why. |
| `contexts` | array | Every named context. |
| `contexts[].id` | u32 | The context id. |
| `contexts[].name` | string | The name. |
| `contexts[].number` | 1–9 or null | The number (R5). |
| `contexts[].hotkey` | string or null | The binding that runs `switch_context` with that number, formatted like the Preferences hotkey list (for example `"⌃⌥1"`), or null when none is bound. The panel shows it, or the bare number when it is null. |
| `contexts[].active` | bool | Whether this is the active context. |
| `contexts[].apps` | string array | The distinct app names of the context's open member windows, in member order. The panel shows the first three. |
| `contexts[].windows` | integer | How many member windows are open. |
| `contexts[].members` | array | Every member record, in the model's order (`Context.members`). The edit view lists the ones that aren't in `windows`. |
| `members[].record` | integer | The record's index in `Context.members`, as `Contexts::remove_record` takes it. |
| `members[].app` | string | The app name (`app_name`, or the bundle id when the name is unknown). |
| `members[].title` | string | The record's title. |
| `members[].window` | window or null | The record's open window, or null when the window is gone (the record is empty or pending, R23). The key may be missing, which means null. |
| `unsorted.windows` | integer | The number of unsorted windows (R3, R29). |
| `unsorted.active` | bool | Whether Unsorted is active. |
| `everything.active` | bool | Whether Everything is active. |
| `everything.hotkey` | string or null | The binding that runs `show_everything`, or null. |
| `windows` | array | The windows on screen that the create and edit views list: the tracked windows that show now on the visible Spaces (the focused screen in `per_screen` scope), including the target window. Leave out parked windows, Sugarglider's own windows, and untracked windows. A native tab group is one entry, its main tab (R36). |
| `windows[].id` | window | The window. |
| `windows[].title`, `windows[].app` | string | Its title and app name. |
| `windows[].tab_count` | integer | How many tabs the window's native tab group has, or 1 for a window without tabs. The create and edit views show "(n tabs)" when it is more than 1. |
| `windows[].pinned` | bool | Whether it is pinned (R3). The create and edit views show a pinned window checked and fixed. |

Exactly one of the `active` flags is true. Keys whose type says "or null" may also be missing.

Tabs that share a frame share membership (R36). Every window id in the payload (`target_window`, `windows[].id`, and `members[].window`) names a tab group by its main tab, so the panel shows one row per group. Every command that carries a window (`add_window`, `move_window`, `toggle_pinned`, `create.windows`, `edit.add`, and `edit.remove`) applies to the window's whole group. Rust resolves a tab to its group, as R36 requires.

Which contexts hold a window is only in `contexts[].members`: a window is a member of a context when one of the context's records has it as `window`. The edit view checks a window on screen by that rule.

## Rank result

The entries of `rank`, best first. `key` is `{"id": N}`, `"unsorted"`, or `"everything"`. `match` is the `NameMatch` variant in snake_case (`#[serde(rename_all = "snake_case")]`): `exact`, `name_prefix`, `word_prefix`, `initials`, `all_word_prefixes`, `letters_in_order`, or `empty_query`.

For the query `"cli"`:

```json
[
  { "key": { "id": 4 }, "match": "name_prefix" }
]
```

For the empty query, every entry, most recently used first. Unsorted is listed only when it has windows (R29):

```json
[
  { "key": { "id": 1 }, "match": "empty_query" },
  { "key": { "id": 4 }, "match": "empty_query" },
  { "key": "unsorted", "match": "empty_query" },
  { "key": "everything", "match": "empty_query" }
]
```

How the panel uses it:

- It calls rank for every query, the empty one included, and shows the entries in this order. It skips ids that the show payload doesn't have.
- It shows the "New context “<text>”" row, last, when no entry has `match` equal to `exact` and the trimmed query can name a new context: it isn't empty, it isn't "Everything" or "Unsorted", and it isn't the name of a context in the payload. The panel compares names lowercased and without the accents of Latin letters, as `model::contexts::fold` does. Where its comparison differs from `fold`, it folds less, so it never refuses a name that Rust accepts. So "unsorted" offers no "New context" row, even when the rank result leaves Unsorted out.
- The first-time state ("Type a name to create your first context") shows when the payload has no named contexts and Everything is active. Then the list shows no entries, only the "New context" row once the user types.
- When `sugarglider_rank_contexts` is missing or returns NULL, the panel shows an inline error and lists the payload's entries unfiltered, in payload order (named contexts, Unsorted when it has windows, Everything).

## Commands

Each command is an object with exactly one key, the command's name.

### `switch`

```json
{ "switch": { "id": 4 } }
```

```json
{ "switch": "unsorted" }
```

```json
{ "switch": "everything" }
```

Switches to the entry (R12). `"everything"` shows Everything (R27). Sent by ↩ on a context row.

### `add_window`

```json
{ "add_window": { "window": { "pid": 812, "idx": 9123 }, "context": { "id": 4 } } }
```

Adds the target window to the context, as `add_window_to_context` does for the focused window (R37). Sent by ⌘↩ on a named context.

### `move_window`

```json
{ "move_window": { "window": { "pid": 812, "idx": 9123 }, "context": { "id": 4 } } }
```

Moves the target window out of the active context and into this one, as `move_window_to_context` does (R37). Sent by ⇧⌘↩ on a named context.

### `toggle_pinned`

```json
{ "toggle_pinned": { "window": { "pid": 812, "idx": 9123 } } }
```

Pins or unpins the target window, as `toggle_window_pinned` does (R3). Sent by ⌘P.

### `create`

```json
{ "create": { "name": "Sugarglider",
              "windows": [{ "pid": 640, "idx": 8812 }, { "pid": 812, "idx": 9123 }] } }
```

Creates a context with this name (R4) and the lowest free number (R5), whose members are exactly these windows, and switches to it, as `sugarglider context create` does. `windows` can be empty. Sent from the create view, which ↩ on the "New context" row opens. ⌘N first opens a naming view that holds the trimmed query, where the user can change the name. The panel checks the name as it does for the "New context" row before it lists the windows, and says why when the name can't be used. When Rust rejects `create`, the panel goes back to the naming view with the name and Rust's message, and keeps the windows the user checked.

`windows` never holds a pinned window. A pinned window is a member of every context already (R3), so the create view shows it checked and fixed and leaves it out. Rust ignores a pinned window in `windows` and gives it no record.

### `edit`

```json
{ "edit": { "context": { "id": 1 },
            "add": [{ "pid": 977, "idx": 9310 }],
            "remove": [{ "pid": 701, "idx": 8920 }],
            "remove_records": [{ "record": 3, "app": "Mail", "title": "Inbox" }] } }
```

Changes a context's members. Sent from the edit view (⌘E) with only what changed. In order, Rust:

1. Removes each record in `remove_records` (`Contexts::remove_record`, R23), from the highest `record` down. It removes a record only when the record at that index still has no open window and still has this app and title. Otherwise it skips the item and logs it, because R23 can change the list while the panel is open.
2. Removes the windows in `remove`. This takes effect at once, as `remove_window_from_context` does (R37). A window here is open; it may be off screen.
3. Adds the windows in `add`. This takes effect at the next switch, as `add_window_to_context` does (R37).

The panel puts an unchecked member with an open window in `remove`, and one without a window in `remove_records`. `add` and `remove` never hold a pinned window: the edit view shows it checked and fixed. Rust ignores a pinned window in them, so its records stay as they are.

### `rename`

```json
{ "rename": { "context": { "id": 4 }, "name": "Client work 2026" } }
```

Renames the context (R4). Sent by ⌘R, then ↩. The panel sends the trimmed name and never an empty one.

### `set_number`

```json
{ "set_number": { "context": { "id": 4 }, "number": 2 } }
```

Gives the context a number from 1 to 9, and takes it from the context that had it (R5). Sent by ⌘1–⌘9.

### `delete`

```json
{ "delete": { "id": 4 } }
```

Deletes the context (R6). Sent by ⌘⌫, then ↩ on the inline confirm row.

## Error return

`sugarglider_run_context_command` returns NULL on success. On failure it returns a message for the user, for example:

```text
A context named "Comms" already exists
```

The panel shows it inline and stays open. After a command succeeds, the panel closes.

## Panel behavior that Rust relies on

- Built-in entries: the panel never sends `add_window`, `move_window`, `edit`, `rename`, `set_number`, or `delete` for Unsorted or Everything (R29). It shows an inline message instead.
- The panel is a non-activating panel, so opening it doesn't change the frontmost app. It closes when it loses key status, after a successful command, and on Esc from the list.
