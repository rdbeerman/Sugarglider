# Sugarglider

A tiling window manager for macOS with drag-to-rearrange and a native menu bar GUI.

<img width="1600" height="668" alt="Screen Recording 2026-09-18 at 17 07 42" src="https://github.com/user-attachments/assets/ca848321-c101-4528-a4cb-eb83ee730001" />


Based on [Glide](https://github.com/tmandry/glide) by Tyler Mandry.

## Features

- **Automatic Window Tiling** - Windows automatically arrange in a tree or column layout
- **Menu Bar GUI** - Quick access to controls, layout switching, and preferences
- **Drag-to-Rearrange** - Visually drag windows to reposition them in the layout
- **Native SwiftUI Preferences** - Native macOS preferences window
- **Keyboard-First** - Full keyboard control with customizable hotkeys
- **Smooth Animations** - Fluid window transitions

---

## Installation

### Prerequisites

- macOS 13.0 (Ventura) or later
- Rust 1.75+ (edition 2024)
- Swift 5.9+
- Xcode Command Line Tools

### Build from Source

```bash
# Clone the repository
git clone https://github.com/rdbeerman/sugarglider.git
cd sugarglider

# Build (debug)
make build

# Or build release
make release

# Install to /usr/local/bin
make install
```

### Grant Accessibility Permissions

Sugarglider requires Accessibility permissions to manage windows:

1. Open **System Settings** > **Privacy & Security** > **Accessibility**
2. Click the lock to make changes
3. Add `sugarglider_server` to the list and enable it

> **Important:** Sugarglider requires **"Displays have separate Spaces"** to be turned on for multi-monitor support. This is the default on macOS. Check it under System Settings > Desktop & Dock > Mission Control.

---

## Quick Start

```bash
# Launch Sugarglider
sugarglider launch

# Or run directly
sugarglider_server
```

A menu bar icon will appear. Click it to access controls.

Press **⌥Z** to start managing the current space. Press again to stop.

---

## How Tiling Works

Sugarglider organizes windows using two layout modes: **Tree** and **Column**.

### Tree Layout (Default)

Windows are arranged in a binary tree structure. Each split divides space proportionally.

```
┌─────────────────────────────────────────────────────────────┐
│                                                             │
│                      Single Window                          │
│                                                             │
└─────────────────────────────────────────────────────────────┘

        Open a second window → splits horizontally

┌────────────────────────────┬────────────────────────────────┐
│                            │                                │
│         Window 1           │           Window 2             │
│                            │                                │
└────────────────────────────┴────────────────────────────────┘

        Open a third window → splits the focused window

┌────────────────────────────┬────────────────────────────────┐
│                            │            Window 2            │
│         Window 1           ├────────────────────────────────┤
│                            │            Window 3            │
└────────────────────────────┴────────────────────────────────┘
```

#### Tree Structure Visualization

The layout forms a tree where each node is either a **split** or a **window**:

```
              Root (Horizontal Split)
              /                    \
         Window 1          Vertical Split
                           /            \
                      Window 2      Window 3
```

#### More Complex Layouts

```
┌──────────────┬──────────────┬──────────────┐
│              │              │              │
│              │   Window 2   │   Window 3   │
│   Window 1   │              │              │
│              ├──────────────┴──────────────┤
│              │                             │
│              │          Window 4           │
│              │                             │
└──────────────┴─────────────────────────────┘

Tree structure:
                    Root (H)
                   /        \
              Win 1         (V)
                           /   \
                        (H)    Win 4
                       /   \
                   Win 2   Win 3
```

### Column Layout

Windows arranged in scrollable columns, ideal for ultrawide monitors:

```
┌─────────────┬─────────────┬─────────────┬─────────────┬ ─ ─ ─
│             │             │             │             │
│  Column 1   │  Column 2   │  Column 3   │  Column 4   │  ...
│             │             │             │             │
└─────────────┴─────────────┴─────────────┴─────────────┴ ─ ─ ─
              ◄─────── Scroll horizontally ───────►
```

---

## Drag-to-Rearrange

Drag windows to visually rearrange them in the layout.

### How It Works

1. **Grab** a window's title bar or from anywhere in the window content (disabled by default)
2. **Drag** to where you want the window to appear, on top of another to swap or below to split horizontally
3. **Drop** on a zone to reposition the window

## Keyboard Shortcuts

All shortcuts use the **Option (⌥)** key by default. Customize in preferences or config file.

### Toggle Management

| Shortcut | Action |
|----------|--------|
| `⌥Z` | Toggle tiling globally |
| `⌥⇧Z` | Toggle tiling for current space |
| `⌥⇧E` | Save and exit Sugarglider |

### Focus Navigation

```
                    ⌥K (up)
                       ↑
                       │
          ⌥H (left) ←──●──→ ⌥L (right)
                       │
                       ↓
                    ⌥J (down)
```

| Shortcut | Action |
|----------|--------|
| `⌥H` | Focus window to the left |
| `⌥L` | Focus window to the right |
| `⌥K` | Focus window above |
| `⌥J` | Focus window below |
| `⌥;` | Focus parent container |

### Move Windows

| Shortcut | Action |
|----------|--------|
| `⌥⇧H` | Move/swap window left |
| `⌥⇧L` | Move/swap window right |
| `⌥⇧K` | Move/swap window up |
| `⌥⇧J` | Move/swap window down |

### Resize Windows

```
     ⌥⇧= (grow)
         ↑
         │
    ─────●───── ⌥0 (equalize)
         │
         ↓
     ⌥⇧- (shrink)
```

| Shortcut | Action |
|----------|--------|
| `⌥=` or `⌥⇧=` | Grow window size |
| `⌥-` | Shrink window size |
| `⌥0` | Reset to equal sizes |

### Layout Control

| Shortcut | Action |
|----------|--------|
| `⌥R` | Rotate split direction (H↔V) |
| `⌥F` | Toggle float/tile for current window |
| `⌥⇧F` | Toggle fullscreen |
| `⌥,` | Open preferences |

---

## Menu Bar

Click the menu bar icon to access controls:

```
┌─────────────────────────────┐
│ ● Enable Space             │  ← Toggle tiling on current space
├─────────────────────────────┤
│ Sugarglider v0.1.0         │
│ Documentation              │  ← Opens this guide
├─────────────────────────────┤
│ ● Enable Sugarglider       │  ← Toggle tiling globally
│ Quit                       │
└─────────────────────────────┘
```

---

## Configuration

Configuration file: `~/.config/sugarglider/sugarglider.toml`

### Example Configuration

```toml
# General settings
[general]
launch_at_login = true
show_menu_bar_icon = true
enable_animations = true

# Focus behavior
[focus]
focus_follows_mouse = false
mouse_follows_focus = false

# Window gaps (in pixels)
[gaps]
outer = 8
inner = 8

# Default layout settings
[layout]
default_mode = "tree"  # or "column"
default_split = "auto" # "horizontal", "vertical", or "auto"

# Hotkey customization
[hotkeys]
toggle_global = "alt+z"
toggle_space = "alt+shift+z"
focus_left = "alt+h"
focus_right = "alt+l"
focus_up = "alt+k"
focus_down = "alt+j"

# Per-app rules - apps that should always float
[[app_rules]]
app = "Finder"
behavior = "float"

[[app_rules]]
app = "System Settings"
behavior = "float"

[[app_rules]]
app = "Calculator"
behavior = "float"

[[app_rules]]
app = "Archive Utility"
behavior = "ignore"
```

### Apply Configuration Changes

```bash
# Reload config without restarting
sugarglider config update

# Watch for changes and auto-reload
sugarglider config update --watch
```

### Configuration Reference

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `general.launch_at_login` | bool | `false` | Start at login |
| `general.show_menu_bar_icon` | bool | `true` | Show menu bar icon |
| `general.enable_animations` | bool | `true` | Animate window transitions |
| `focus.focus_follows_mouse` | bool | `false` | Focus window under cursor |
| `focus.mouse_follows_focus` | bool | `false` | Move cursor to focused window |
| `gaps.outer` | int | `8` | Gap from screen edges (px) |
| `gaps.inner` | int | `8` | Gap between windows (px) |
| `layout.default_mode` | string | `"tree"` | `"tree"` or `"column"` |
| `layout.default_split` | string | `"auto"` | `"horizontal"`, `"vertical"`, `"auto"` |

### App Behaviors

| Behavior | Description |
|----------|-------------|
| `tile` | Normal tiling (default) |
| `float` | Always floating, never tiled |
| `ignore` | Completely ignored by Sugarglider |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                      SugargliderUI (Swift)                      │
│        Preferences Window  •  Drop Zone Overlay  •  Menu        │
└───────────────────────────────┬─────────────────────────────────┘
                                │ FFI
┌───────────────────────────────┴─────────────────────────────────┐
│                         Sugarglider (Rust)                      │
├─────────────────────────────────────────────────────────────────┤
│  actor     │  Event-driven actors, IPC, window management       │
│  model     │  Pure data structures, layout tree algorithms      │
│  sys       │  macOS API wrappers (Accessibility, SkyLight)      │
│  config    │  Configuration loading and validation              │
│  ui        │  Native AppKit components (status bar, groups)     │
└─────────────────────────────────────────────────────────────────┘
```

### Layer Responsibilities

| Layer | Purpose | Side Effects |
|-------|---------|--------------|
| **sys** | Thin wrappers around macOS APIs | None (just types) |
| **model** | Pure algorithms and data structures | None |
| **actor** | Event processing, IPC, I/O | All side effects |
| **config** | Configuration parsing/validation | File I/O only |
| **ui** | AppKit menu bar, overlays | UI only |

---

## Development

### Project Structure

```
sugarglider/
├── src/
│   ├── actor/              # Event-driven actors
│   │   ├── reactor.rs      # Central event processor
│   │   ├── layout.rs       # Layout manager
│   │   ├── app.rs          # Per-app management
│   │   └── mouse.rs        # Mouse/drag handling
│   ├── model/              # Pure data structures
│   │   ├── layout_tree.rs  # Tree structure
│   │   └── tree.rs         # Generic N-ary tree
│   ├── sys/                # macOS API wrappers
│   ├── config/             # Configuration system
│   ├── ui/                 # Native UI components
│   └── bin/                # Entry points
├── SugargliderUI/          # Swift UI package
│   └── Sources/
│       └── SugargliderUI/
│           ├── PreferencesView.swift
│           ├── DropZoneOverlay.swift
│           └── SugargliderUI.swift
├── crates/                 # Supporting crates
├── tests/                  # Integration tests
└── examples/               # Developer tools
```

### Build Commands

```bash
make build          # Build debug version (Swift + Rust)
make release        # Build release version
make run            # Build and run (debug)
make test           # Run all tests
make fmt            # Format all code
make clean          # Clean build artifacts
make install        # Install to /usr/local/bin
make help           # Show all commands
```

### Developer Tools

```bash
# List windows via accessibility API
cargo run --example devtool -- list ax

# Replay a recorded trace (for debugging)
cargo run --example devtool -- replay traces/example.ron
```

### Recording a Debug Trace

```bash
RUST_LOG=info sugarglider_server --record traces/debug-$(date +%Y%m%d-%H%M%S).ron
```

---

## Credits

- [Glide](https://github.com/tmandry/glide) by Tyler Mandry - The foundation for Sugarglider
- [Yabai](https://github.com/koekeishiya/yabai) - Inspiration and macOS window management techniques
- [objc2](https://github.com/madsmtm/objc2) - Rust bindings for Objective-C/macOS APIs
- Inspired by [i3](https://i3wm.org/), [Sway](https://swaywm.org/), and [Hyprland](https://hyprland.org/)

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
